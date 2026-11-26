use crate::{Forge, ForgeError, NewPullRequest, RepositoryRef};
mod actions;
mod detail;

use crate::PullRequestFile;
use hoshi_core::{
    ChecksState, ForgeKind, PullRequest, PullRequestAction, PullRequestDetail, PullRequestLabel,
    PullRequestState, RemoteRepository, ReviewAction, ReviewApproval, ReviewAsk, ReviewTurn,
};
use serde::Deserialize;
use serde_json::json;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const API_URL: &str = "https://api.github.com";

macro_rules! pull_request_fields {
    () => {
        "number title url isDraft state additions deletions updatedAt headRefName headRefOid baseRefName
        reviewDecision mergeable
        author { login }
        commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }"
    };
}

/// Who spoke when, read for one pull request at a time: across a search it costs too much.
macro_rules! review_turn_fields {
    () => {
        "reviewThreads(first: 100) { nodes { isResolved comments(last: 1) { nodes { author { login } } } } }
        latestOpinionatedReviews(first: 20, writersOnly: true) { nodes { author { login } state } }
        timelineItems(last: 50, itemTypes: [ISSUE_COMMENT, PULL_REQUEST_REVIEW]) { nodes {
          ... on IssueComment { author { __typename login } createdAt }
          ... on PullRequestReview { author { __typename login } state submittedAt }
        } }"
    };
}

const PULL_REQUEST_QUERY: &str = concat!(
    "
query($owner: String!, $name: String!, $branch: String!, $first: Int!, $after: String) {
  repository(owner: $owner, name: $name) {
    defaultBranchRef { name }
    pullRequests(headRefName: $branch, first: $first, after: $after, orderBy: {field: UPDATED_AT, direction: DESC}) {
      pageInfo { hasNextPage endCursor }
      nodes { isCrossRepository ",
    pull_request_fields!(),
    review_turn_fields!(),
    " }
    }
  }
}"
);

const PULL_REQUEST_BY_NUMBER_QUERY: &str = concat!(
    "
query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) { ",
    pull_request_fields!(),
    review_turn_fields!(),
    " }
  }
}"
);

/// Past a fork's latest pull request, pull requests from `headRefName` are read in pages of
/// [`FORK_PAGE`], up to [`FORK_PAGES`] of them.
const FORK_PAGE: u32 = 50;
const FORK_PAGES: usize = 4;

const SEARCH_LIMIT: u32 = 20;

const SEARCH_QUERY: &str = concat!(
    "
query($search: String!, $first: Int!) {
  search(query: $search, type: ISSUE, first: $first) {
    nodes { ... on PullRequest { ",
    pull_request_fields!(),
    "reviewThreads(first: 100) { nodes { isResolved } }",
    " } }
  }
}"
);

const VIEWER_QUERY: &str = "query { viewer { login } }";

type TokenSource = Box<dyn Fn() -> Option<String> + Send + Sync>;

/// GitHub through its GraphQL API.
pub struct GitHub {
    agent: ureq::Agent,
    token_source: TokenSource,
    token: Mutex<Option<String>>,
}

impl GitHub {
    pub fn new(token_source: impl Fn() -> Option<String> + Send + Sync + 'static) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(20)))
            // Keeps the connection between the background sync's reads, so opening a pull
            // request rarely pays a new TLS handshake.
            .max_idle_age(Duration::from_secs(90))
            .build()
            .into();
        Self {
            agent,
            token_source: Box::new(token_source),
            token: Mutex::new(None),
        }
    }

    /// Uses the account the GitHub CLI is signed in to (`gh auth login`).
    pub fn with_cli_credentials() -> Self {
        Self::new(cli_token)
    }

    fn token(&self) -> Result<String, ForgeError> {
        let mut token = self.token.lock().unwrap_or_else(|error| error.into_inner());
        if token.is_none() {
            *token = (self.token_source)();
        }
        token.clone().ok_or(ForgeError::Unauthorized)
    }

    fn forget_token(&self) {
        *self.token.lock().unwrap_or_else(|error| error.into_inner()) = None;
    }

    fn graphql<T: for<'de> Deserialize<'de>>(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<T, ForgeError> {
        let body = json!({ "query": query, "variables": variables }).to_string();
        let text = self.request(Method::Post, "/graphql", Some(&body))?;
        parse_graphql(&text)
    }

    /// A REST `GET` of `path` under the API root.
    fn rest_get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, ForgeError> {
        let text = self.request(Method::Get, path, None)?;
        serde_json::from_str(&text).map_err(|error| ForgeError::Unexpected(error.to_string()))
    }

    /// Sends a request to `path` under the API root; returns the body of a 2xx answer.
    fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&str>,
    ) -> Result<String, ForgeError> {
        let token = self.token()?;
        let url = format!("{API_URL}{path}");
        let request = match method {
            Method::Get => self.agent.get(&url).force_send_body(),
            Method::Delete => self.agent.delete(&url).force_send_body(),
            Method::Post => self.agent.post(&url),
            Method::Patch => self.agent.patch(&url),
            Method::Put => self.agent.put(&url),
        };
        let sent = request
            .header("Authorization", &format!("bearer {token}"))
            .header("User-Agent", "hoshi")
            .header("Accept", "application/vnd.github+json")
            .send(body.unwrap_or_default());
        let mut response = sent.map_err(|error| ForgeError::Network(error.to_string()))?;
        let status = response.status().as_u16();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        let retry_after = header("retry-after");
        let remaining = header("x-ratelimit-remaining");
        let reset = header("x-ratelimit-reset");
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|error| ForgeError::Network(error.to_string()))?;
        match status {
            200..=299 => Ok(text),
            401 => {
                self.forget_token();
                Err(ForgeError::Unauthorized)
            }
            403 | 429 if retry_after.is_some() || remaining.as_deref() == Some("0") => {
                Err(ForgeError::RateLimited {
                    retry_after_secs: retry_after_secs(retry_after, reset),
                })
            }
            404 => Err(ForgeError::NotFound),
            400..=499 => Err(ForgeError::Rejected(rejection_message(&text, status))),
            _ => Err(ForgeError::Unexpected(format!("HTTP {status}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Method {
    Get,
    Post,
    Patch,
    Put,
    Delete,
}

/// GitHub explains a refused request in `message`, sometimes with details in `errors`.
fn rejection_message(text: &str, status: u16) -> String {
    #[derive(Deserialize)]
    struct Rejection {
        message: Option<String>,
        #[serde(default)]
        errors: Vec<serde_json::Value>,
    }
    let Ok(rejection) = serde_json::from_str::<Rejection>(text) else {
        return format!("HTTP {status}");
    };
    let details: Vec<String> = rejection
        .errors
        .iter()
        .filter_map(|error| match error {
            serde_json::Value::String(text) => Some(text.clone()),
            other => other.get("message")?.as_str().map(str::to_owned),
        })
        .collect();
    let message = rejection
        .message
        .unwrap_or_else(|| format!("HTTP {status}"));
    if details.is_empty() {
        message
    } else {
        format!("{message}: {}", details.join("; "))
    }
}

impl Forge for GitHub {
    fn kind(&self) -> ForgeKind {
        ForgeKind::Github
    }

    fn forget_credentials(&self) {
        self.forget_token();
    }

    fn viewer(&self) -> Result<String, ForgeError> {
        let data: ViewerData = self.graphql(VIEWER_QUERY, json!({}))?;
        Ok(data.viewer.login)
    }

    fn pull_request_for_branch(
        &self,
        repository: &RepositoryRef,
        branch: &str,
    ) -> Result<Option<PullRequest>, ForgeError> {
        let read = |first: u32, after: Option<&str>| {
            let variables = json!({
                "owner": repository.owner,
                "name": repository.name,
                "branch": branch,
                "first": first,
                "after": after,
            });
            self.graphql::<RepositoryData>(PULL_REQUEST_QUERY, variables)
                .and_then(|data| pull_request_from(data, branch))
        };
        let mut found = read(1, None)?;
        for _ in 0..FORK_PAGES {
            let BranchPullRequest::OnlyForks { after } = found else {
                break;
            };
            found = read(FORK_PAGE, Some(&after))?;
        }
        Ok(match found {
            BranchPullRequest::Own(pull_request) => Some(*pull_request),
            BranchPullRequest::None | BranchPullRequest::OnlyForks { .. } => None,
        })
    }

    fn pull_request(
        &self,
        repository: &RepositoryRef,
        number: u32,
    ) -> Result<PullRequest, ForgeError> {
        let variables = json!({
            "owner": repository.owner,
            "name": repository.name,
            "number": number,
        });
        let data: NumberData = self.graphql(PULL_REQUEST_BY_NUMBER_QUERY, variables)?;
        pull_request_by_number(data)
    }

    fn search_pull_requests(
        &self,
        repository: &RepositoryRef,
        query: &str,
    ) -> Result<Vec<PullRequest>, ForgeError> {
        let query = query.trim();
        let exact = match query.parse::<u32>() {
            Ok(number) => match self.pull_request(repository, number) {
                Ok(pull_request) => Some(pull_request),
                Err(ForgeError::NotFound) => None,
                Err(error) => return Err(error),
            },
            Err(_) => None,
        };
        let variables = json!({
            "search": search_terms(repository, query),
            "first": SEARCH_LIMIT,
        });
        let data: SearchData = self.graphql(SEARCH_QUERY, variables)?;
        let exact_number = exact.as_ref().map(|pull_request| pull_request.number);
        let found = data
            .search
            .nodes
            .into_iter()
            .map(pull_request_of)
            .filter(|pull_request| Some(pull_request.number) != exact_number);
        let pull_requests = exact.into_iter().chain(found).collect();
        Ok(pull_requests)
    }

    fn pull_request_detail(
        &self,
        repository: &RepositoryRef,
        number: u32,
    ) -> Result<PullRequestDetail, ForgeError> {
        let variables = json!({
            "owner": repository.owner,
            "name": repository.name,
            "number": number,
        });
        let (core, activity) = std::thread::scope(|scope| {
            let activity = scope.spawn(|| {
                self.graphql::<detail::ActivityData>(detail::ACTIVITY_QUERY, variables.clone())
            });
            let core = self.graphql::<detail::DetailData>(detail::CORE_QUERY, variables.clone());
            let activity = activity
                .join()
                .unwrap_or_else(|_| Err(ForgeError::Unexpected("activity read panicked".into())));
            (core, activity)
        });
        detail::detail_from(detail::with_activity(core?, activity?))
    }

    fn pull_request_files(
        &self,
        repository: &RepositoryRef,
        number: u32,
    ) -> Result<Vec<PullRequestFile>, ForgeError> {
        let path = format!(
            "/repos/{}/{}/pulls/{number}/files?per_page=100",
            repository.owner, repository.name
        );
        let files: Vec<detail::RestFile> = self.rest_get(&path)?;
        Ok(files.into_iter().map(detail::file_from).collect())
    }

    fn act(
        &self,
        repository: &RepositoryRef,
        number: u32,
        forge_id: &str,
        action: &PullRequestAction,
    ) -> Result<(), ForgeError> {
        match actions::plan(repository, number, forge_id, action) {
            actions::Call::Rest { method, path, body } => {
                let body = body.map(|body| body.to_string());
                self.request(method, &path, body.as_deref()).map(|_| ())
            }
            actions::Call::Mutation { query, variables } => {
                let body = json!({ "query": query, "variables": variables }).to_string();
                let text = self.request(Method::Post, "/graphql", Some(&body))?;
                actions::check_mutation(&text)
            }
        }
    }

    fn create_pull_request(
        &self,
        repository: &RepositoryRef,
        pull_request: &NewPullRequest,
    ) -> Result<PullRequest, ForgeError> {
        #[derive(Deserialize)]
        struct Created {
            number: u32,
        }
        let path = format!("/repos/{}/{}/pulls", repository.owner, repository.name);
        let body = json!({
            "head": pull_request.head,
            "base": pull_request.base,
            "title": pull_request.title,
            "body": pull_request.body,
            "draft": pull_request.draft,
            "maintainer_can_modify": true,
        })
        .to_string();
        let text = self.request(Method::Post, &path, Some(&body))?;
        let created: Created = serde_json::from_str(&text)
            .map_err(|error| ForgeError::Unexpected(error.to_string()))?;
        self.pull_request(repository, created.number)
    }

    fn reviewer_candidates(
        &self,
        repository: &RepositoryRef,
        query: &str,
    ) -> Result<Vec<String>, ForgeError> {
        let variables = json!({
            "owner": repository.owner,
            "name": repository.name,
            "query": query,
        });
        let data: actions::AssignableData =
            self.graphql(actions::ASSIGNABLE_USERS_QUERY, variables)?;
        Ok(actions::logins_from(data))
    }

    fn labels(&self, repository: &RepositoryRef) -> Result<Vec<PullRequestLabel>, ForgeError> {
        #[derive(Deserialize)]
        struct Label {
            name: String,
            color: String,
        }
        let path = format!(
            "/repos/{}/{}/labels?per_page=100",
            repository.owner, repository.name
        );
        let labels: Vec<Label> = self.rest_get(&path)?;
        Ok(labels
            .into_iter()
            .map(|label| PullRequestLabel {
                name: label.name,
                color: label.color,
            })
            .collect())
    }

    fn repository(&self, repository: &RepositoryRef) -> Result<RemoteRepository, ForgeError> {
        #[derive(Deserialize)]
        struct Repository {
            full_name: String,
            name: String,
            description: Option<String>,
            html_url: String,
            clone_url: String,
            private: bool,
        }
        let found: Repository =
            self.rest_get(&format!("/repos/{}/{}", repository.owner, repository.name))?;
        Ok(RemoteRepository {
            forge: ForgeKind::Github,
            full_name: found.full_name,
            name: found.name,
            description: found.description.filter(|text| !text.trim().is_empty()),
            web_url: found.html_url,
            clone_url: found.clone_url,
            private: found.private,
        })
    }
}

fn cli_token() -> Option<String> {
    let mut command = Command::new("gh");
    command
        .args(["auth", "token", "--hostname", "github.com"])
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().ok()?;
    let token = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (output.status.success() && !token.is_empty()).then_some(token)
}

fn retry_after_secs(retry_after: Option<String>, reset: Option<String>) -> u64 {
    if let Some(seconds) = retry_after.and_then(|value| value.parse().ok()) {
        return seconds;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    reset
        .and_then(|value| value.parse::<u64>().ok())
        .map_or(60, |reset| reset.saturating_sub(now).max(1))
}

#[derive(Deserialize)]
struct Envelope<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GraphqlError>,
}

#[derive(Deserialize)]
struct GraphqlError {
    #[serde(rename = "type")]
    kind: Option<String>,
    message: String,
}

pub(super) fn parse_graphql<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T, ForgeError> {
    let envelope: Envelope<T> =
        serde_json::from_str(text).map_err(|error| ForgeError::Unexpected(error.to_string()))?;
    if let Some(error) = envelope.errors.first() {
        return Err(match error.kind.as_deref() {
            Some("NOT_FOUND") => ForgeError::NotFound,
            _ => ForgeError::Unexpected(error.message.clone()),
        });
    }
    envelope
        .data
        .ok_or_else(|| ForgeError::Unexpected("response without data".into()))
}

#[derive(Deserialize)]
struct ViewerData {
    viewer: Viewer,
}

#[derive(Deserialize)]
struct Viewer {
    login: String,
}

#[derive(Deserialize)]
struct SearchData {
    search: Nodes<PullRequestNode>,
}

/// GitHub's search syntax for the repository's pull requests matching `query`.
fn search_terms(repository: &RepositoryRef, query: &str) -> String {
    let words: String = query
        .split_whitespace()
        .map(|word| format!(" {}", word.replace('"', "")))
        .collect();
    format!(
        "repo:{}/{} is:pr sort:updated-desc{words}",
        repository.owner, repository.name
    )
}

#[derive(Deserialize)]
struct RepositoryData {
    repository: Option<Repository>,
}

#[derive(Deserialize)]
struct NumberData {
    repository: Option<NumberRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NumberRepository {
    pull_request: Option<PullRequestNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Repository {
    default_branch_ref: Option<Named>,
    pull_requests: Page<PullRequestNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page<T> {
    nodes: Vec<T>,
    #[serde(default)]
    page_info: Option<PageInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

#[derive(Deserialize)]
struct Nodes<T> {
    nodes: Vec<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestNode {
    number: u32,
    #[serde(default)]
    is_cross_repository: bool,
    title: String,
    url: String,
    is_draft: bool,
    state: String,
    additions: u32,
    deletions: u32,
    updated_at: String,
    head_ref_name: String,
    #[serde(default)]
    head_ref_oid: Option<String>,
    base_ref_name: String,
    #[serde(default)]
    review_decision: Option<String>,
    #[serde(default)]
    mergeable: Option<String>,
    author: Option<Author>,
    commits: Nodes<CommitNode>,
    review_threads: Nodes<ReviewThread>,
    #[serde(default)]
    latest_opinionated_reviews: Option<Nodes<OpinionatedReview>>,
    #[serde(default)]
    timeline_items: Option<Nodes<TimelineNode>>,
}

#[derive(Deserialize)]
struct Author {
    login: String,
}

#[derive(Deserialize)]
struct CommitNode {
    commit: Commit,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Commit {
    status_check_rollup: Option<Rollup>,
}

#[derive(Deserialize)]
struct Rollup {
    state: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewThread {
    is_resolved: bool,
    #[serde(default)]
    comments: Option<Nodes<ThreadComment>>,
}

#[derive(Deserialize)]
struct ThreadComment {
    author: Option<Author>,
}

#[derive(Deserialize)]
struct OpinionatedReview {
    author: Option<Author>,
    state: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimelineNode {
    author: Option<Actor>,
    state: Option<String>,
    created_at: Option<String>,
    submitted_at: Option<String>,
}

#[derive(Deserialize)]
struct Actor {
    #[serde(rename = "__typename")]
    kind: Option<String>,
    login: String,
}

impl TimelineNode {
    fn approval(&self) -> Option<ReviewApproval> {
        if self.state.as_deref() != Some("APPROVED") {
            return None;
        }
        Some(ReviewApproval {
            by: self.author.as_ref()?.login.clone(),
            at: self.submitted_at.clone()?,
        })
    }

    fn action(&self) -> Option<ReviewAction> {
        let author = self.author.as_ref()?;
        if author.kind.as_deref() == Some("Bot") {
            return None;
        }
        let (ask, at) = match self.state.as_deref() {
            None => (ReviewAsk::Comment, self.created_at.clone()?),
            Some("COMMENTED") => (ReviewAsk::Comment, self.submitted_at.clone()?),
            Some("CHANGES_REQUESTED") => (ReviewAsk::ChangesRequested, self.submitted_at.clone()?),
            _ => return None,
        };
        Some(ReviewAction {
            by: author.login.clone(),
            ask,
            at,
        })
    }
}

/// A branch's pull request. A fork's branch of the same name is another branch.
#[derive(Debug, PartialEq)]
enum BranchPullRequest {
    Own(Box<PullRequest>),
    None,
    /// Every one read came from a fork; those after `after` may not.
    OnlyForks {
        after: String,
    },
}

fn pull_request_from(data: RepositoryData, branch: &str) -> Result<BranchPullRequest, ForgeError> {
    let repository = data.repository.ok_or(ForgeError::NotFound)?;
    if repository
        .default_branch_ref
        .is_some_and(|default| default.name == branch)
    {
        return Ok(BranchPullRequest::None);
    }
    let page = repository.pull_requests;
    if let Some(node) = page
        .nodes
        .into_iter()
        .find(|node| !node.is_cross_repository)
    {
        return Ok(BranchPullRequest::Own(Box::new(pull_request_of(node))));
    }
    Ok(match page.page_info {
        Some(PageInfo {
            has_next_page: true,
            end_cursor: Some(after),
        }) => BranchPullRequest::OnlyForks { after },
        _ => BranchPullRequest::None,
    })
}

fn pull_request_by_number(data: NumberData) -> Result<PullRequest, ForgeError> {
    data.repository
        .and_then(|repository| repository.pull_request)
        .map(pull_request_of)
        .ok_or(ForgeError::NotFound)
}

fn pull_request_of(node: PullRequestNode) -> PullRequest {
    let state = match (node.state.as_str(), node.is_draft) {
        ("MERGED", _) => PullRequestState::Merged,
        ("CLOSED", _) => PullRequestState::Closed,
        (_, true) => PullRequestState::Draft,
        _ => PullRequestState::Open,
    };
    let checks = node
        .commits
        .nodes
        .last()
        .and_then(|commit| commit.commit.status_check_rollup.as_ref())
        .and_then(|rollup| match rollup.state.as_str() {
            "SUCCESS" => Some(ChecksState::Passing),
            "FAILURE" | "ERROR" => Some(ChecksState::Failing),
            "PENDING" | "EXPECTED" => Some(ChecksState::Pending),
            _ => None,
        });
    let author = node.author.map(|author| author.login);
    let answered_by_author = |thread: &ReviewThread| {
        let last = thread
            .comments
            .as_ref()
            .and_then(|comments| comments.nodes.last())
            .and_then(|comment| comment.author.as_ref());
        matches!((last, &author), (Some(last), Some(author)) if last.login.eq_ignore_ascii_case(author))
    };
    let unresolved_comments = node
        .review_threads
        .nodes
        .iter()
        .filter(|thread| !thread.is_resolved && !answered_by_author(thread))
        .count() as u32;
    let changes_requested_by: Vec<String> = node
        .latest_opinionated_reviews
        .iter()
        .flat_map(|reviews| &reviews.nodes)
        .filter(|review| review.state == "CHANGES_REQUESTED")
        .filter_map(|review| Some(review.author.as_ref()?.login.clone()))
        .collect();
    let actions: Vec<ReviewAction> = node
        .timeline_items
        .iter()
        .flat_map(|items| &items.nodes)
        .filter_map(TimelineNode::action)
        .collect();
    let approvals: Vec<ReviewApproval> = node
        .timeline_items
        .iter()
        .flat_map(|items| &items.nodes)
        .filter_map(TimelineNode::approval)
        .collect();
    let turn = ReviewTurn::of(
        author.as_deref(),
        &actions,
        &changes_requested_by,
        &approvals,
    );
    PullRequest {
        forge: ForgeKind::Github,
        number: node.number,
        title: node.title,
        url: node.url,
        author,
        head_branch: node.head_ref_name,
        head_sha: node.head_ref_oid,
        base_branch: node.base_ref_name,
        state,
        checks,
        unresolved_comments,
        open_tasks: 0,
        changes_requested: node.review_decision.as_deref() == Some("CHANGES_REQUESTED")
            || !changes_requested_by.is_empty(),
        turn,
        conflicts: node.mergeable.as_deref() == Some("CONFLICTING"),
        additions: node.additions,
        deletions: node.deletions,
        updated_at: node.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(pull_requests: &str) -> String {
        format!(
            r#"{{"data":{{"repository":{{"defaultBranchRef":{{"name":"main"}},
                "pullRequests":{{"nodes":[{pull_requests}]}}}}}}}}"#
        )
    }

    #[test]
    fn search_reads_pull_requests_and_scopes_the_query_to_the_repository() {
        let text = format!(r#"{{"data":{{"search":{{"nodes":[{OPEN_PR}]}}}}}}"#);

        let data = parse_graphql::<SearchData>(&text).unwrap();
        let repository = RepositoryRef {
            forge: ForgeKind::Github,
            host: "github.com".into(),
            owner: "o".into(),
            name: "r".into(),
        };

        assert_eq!(data.search.nodes.len(), 1);
        assert_eq!(
            pull_request_of(data.search.nodes.into_iter().next().unwrap()).number,
            42
        );
        assert_eq!(
            search_terms(&repository, "  side \"bar\" "),
            "repo:o/r is:pr sort:updated-desc side bar"
        );
    }

    const OPEN_PR: &str = r#"{
        "number": 42, "title": "Fix the sidebar", "url": "https://github.com/o/r/pull/42",
        "isDraft": false, "state": "OPEN", "additions": 120, "deletions": 34,
        "updatedAt": "2026-11-07T12:00:00Z", "headRefName": "fix/sidebar", "baseRefName": "main",
        "author": {"login": "nxtvoid"},
        "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": "FAILURE"}}}]},
        "reviewThreads": {"nodes": [{"isResolved": false}, {"isResolved": true}, {"isResolved": false}]}
    }"#;

    fn lookup(text: &str, branch: &str) -> Result<BranchPullRequest, ForgeError> {
        parse_graphql::<RepositoryData>(text).and_then(|data| pull_request_from(data, branch))
    }

    fn parse(text: &str, branch: &str) -> Result<Option<PullRequest>, ForgeError> {
        lookup(text, branch).map(|found| match found {
            BranchPullRequest::Own(pull_request) => Some(*pull_request),
            BranchPullRequest::None | BranchPullRequest::OnlyForks { .. } => None,
        })
    }

    #[test]
    fn a_branch_matches_only_pull_requests_from_the_repository_itself() {
        let fork = OPEN_PR
            .replace(r#""number": 42"#, r#""number": 86"#)
            .replace(r#""isDraft""#, r#""isCrossRepository": true, "isDraft""#);
        let own = OPEN_PR.replace(r#""isDraft""#, r#""isCrossRepository": false, "isDraft""#);
        let with_more = |nodes: &str| {
            response(nodes).replace(
                r#""pullRequests":{"#,
                r#""pullRequests":{"pageInfo":{"hasNextPage":true,"endCursor":"c1"},"#,
            )
        };

        assert_eq!(
            lookup(&with_more(&fork), "fix/sidebar"),
            Ok(BranchPullRequest::OnlyForks { after: "c1".into() })
        );
        assert_eq!(
            lookup(&response(&fork), "fix/sidebar"),
            Ok(BranchPullRequest::None)
        );
        let found = parse(&response(&format!("{fork},{own}")), "fix/sidebar")
            .unwrap()
            .unwrap();
        assert_eq!(found.number, 42);
        assert_eq!(
            lookup(&response(&fork), "main"),
            Ok(BranchPullRequest::None)
        );
    }

    #[test]
    fn reads_a_pull_request_by_number() {
        let text = format!(r#"{{"data":{{"repository":{{"pullRequest":{OPEN_PR}}}}}}}"#);
        let pull_request = parse_graphql::<NumberData>(&text)
            .and_then(pull_request_by_number)
            .unwrap();

        assert_eq!(pull_request.number, 42);
        assert_eq!(pull_request.author.as_deref(), Some("nxtvoid"));

        let missing = r#"{"data":{"repository":{"pullRequest":null}}}"#;
        assert_eq!(
            parse_graphql::<NumberData>(missing).and_then(pull_request_by_number),
            Err(ForgeError::NotFound)
        );
    }

    #[test]
    fn reads_state_checks_and_unresolved_threads() {
        let pull_request = parse(&response(OPEN_PR), "fix/sidebar").unwrap().unwrap();

        assert_eq!(pull_request.number, 42);
        assert_eq!(pull_request.state, PullRequestState::Open);
        assert_eq!(pull_request.checks, Some(ChecksState::Failing));
        assert_eq!(pull_request.unresolved_comments, 2);
        assert_eq!(pull_request.author.as_deref(), Some("nxtvoid"));
        assert_eq!(pull_request.base_branch, "main");
    }

    #[test]
    fn a_draft_without_checks_and_a_merged_one() {
        let draft = OPEN_PR
            .replace(r#""isDraft": false"#, r#""isDraft": true"#)
            .replace(r#"{"state": "FAILURE"}"#, "null");
        let merged = OPEN_PR.replace(r#""state": "OPEN""#, r#""state": "MERGED""#);

        let draft = parse(&response(&draft), "fix/sidebar").unwrap().unwrap();
        let merged = parse(&response(&merged), "fix/sidebar").unwrap().unwrap();

        assert_eq!(draft.state, PullRequestState::Draft);
        assert_eq!(draft.checks, None);
        assert_eq!(merged.state, PullRequestState::Merged);
    }

    #[test]
    fn the_default_branch_and_a_branch_without_pr_have_none() {
        assert_eq!(parse(&response(OPEN_PR), "main").unwrap(), None);
        assert_eq!(parse(&response(""), "feat/x").unwrap(), None);
    }

    #[test]
    fn a_missing_repository_is_not_found() {
        let text = r#"{"data":{"repository":null},
            "errors":[{"type":"NOT_FOUND","message":"Could not resolve to a Repository"}]}"#;

        assert_eq!(parse(text, "x"), Err(ForgeError::NotFound));
    }

    #[test]
    fn reads_requested_changes_and_conflicts() {
        let blocked = OPEN_PR.replacen(
            "\"isDraft\"",
            "\"reviewDecision\": \"CHANGES_REQUESTED\", \"mergeable\": \"CONFLICTING\", \"isDraft\"",
            1,
        );
        let blocked = parse(&response(&blocked), "fix/sidebar").unwrap().unwrap();
        let plain = parse(&response(OPEN_PR), "fix/sidebar").unwrap().unwrap();

        assert!(blocked.changes_requested && blocked.conflicts);
        assert!(!plain.changes_requested && !plain.conflicts);
    }

    #[test]
    fn whose_turn_comes_from_people_not_commits_or_bots() {
        let reviewed = OPEN_PR.replacen(
            "\"isDraft\"",
            r#""reviewThreads": {"nodes": [
                {"isResolved": false, "comments": {"nodes": [{"author": {"login": "NXTVOID"}}]}},
                {"isResolved": false, "comments": {"nodes": [{"author": {"login": "luis"}}]}}
            ]},
            "latestOpinionatedReviews": {"nodes": [{"author": {"login": "luis"}, "state": "CHANGES_REQUESTED"}]},
            "timelineItems": {"nodes": [
                {"author": {"__typename": "User", "login": "luis"}, "state": "CHANGES_REQUESTED", "submittedAt": "2026-11-07T10:00:00Z"},
                {"author": {"__typename": "User", "login": "nxtvoid"}, "createdAt": "2026-11-07T11:00:00Z"},
                {"author": {"__typename": "Bot", "login": "ci"}, "createdAt": "2026-11-07T12:00:00Z"},
                {"author": {"__typename": "User", "login": "eva"}, "state": "APPROVED", "submittedAt": "2026-11-07T12:30:00Z"},
                {}
            ]},
            "isDraft""#,
            1,
        );
        let reviewed = reviewed.replacen(
            r#""reviewThreads": {"nodes": [{"isResolved": false}, {"isResolved": true}, {"isResolved": false}]}"#,
            r#""unused": null"#,
            1,
        );
        let pull_request = parse(&response(&reviewed), "fix/sidebar").unwrap().unwrap();

        assert_eq!(pull_request.unresolved_comments, 1);
        assert!(pull_request.changes_requested);
        assert_eq!(
            pull_request.turn,
            Some(ReviewTurn::Reviewers {
                waiting_on: vec!["luis".into()],
                replied_at: "2026-11-07T11:00:00Z".into(),
            })
        );
    }
}
