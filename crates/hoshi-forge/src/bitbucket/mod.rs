//! Bitbucket Cloud (bitbucket.org) through its REST API, signed in with an Atlassian API
//! token. Listings carry no checks, open threads or line counts, so those come from their
//! own endpoints and are kept per pull request until it changes.

mod actions;
mod model;
mod patch;
mod scopes;

use crate::{Forge, ForgeError, NewPullRequest, PullRequestFile, RepositoryRef};
use actions::Plan;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use hoshi_core::{
    ChecksState, ForgeKind, MergeMethod, Mergeability, PullRequest, PullRequestAction,
    PullRequestDetail, PullRequestLabel, PullRequestState, PullRequestSupport, RemoteRepository,
};
use model::{Page, RawComment, RawCommit, RawDiffStat, RawPullRequest, RawStatus, Summary, User};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use ureq::config::RedirectAuthHeaders;

pub use scopes::REQUIRED_SCOPES;

const API_URL: &str = "https://api.bitbucket.org/2.0";
/// Bitbucket answers an empty page, without an error, to anything larger.
const PAGE_LEN: u32 = 50;
const MAX_PAGES: usize = 10;
const SEARCH_LIMIT: u32 = 20;
/// Without `Retry-After`, a rate limit is waited out this long.
const DEFAULT_RETRY_SECS: u64 = 60;
/// Running checks are read again this often; finished ones only when the head moves.
const PENDING_CHECKS_FRESH_FOR: Duration = Duration::from_secs(30);
const CHECKS_FRESH_FOR: Duration = Duration::from_secs(2 * 60);
/// Comments bump `updated_on`; this only covers a missed bump.
const COMMENTS_FRESH_FOR: Duration = Duration::from_secs(10 * 60);
/// Diffs past this size come without patches, like GitHub's.
const MAX_DIFF_BYTES: u64 = 8 * 1024 * 1024;
const ALL_STATES: &str = "state=OPEN&state=MERGED&state=DECLINED&state=SUPERSEDED";

/// An Atlassian account's email and one of its API tokens.
#[derive(Clone, PartialEq, Eq)]
pub struct BitbucketCredentials {
    pub email: String,
    pub token: String,
}

impl std::fmt::Debug for BitbucketCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BitbucketCredentials")
            .field("email", &self.email)
            .finish_non_exhaustive()
    }
}

impl BitbucketCredentials {
    /// One line each, as the credential store keeps them.
    pub fn encode(&self) -> String {
        format!("{}\n{}", self.email, self.token)
    }

    pub fn decode(text: &str) -> Option<Self> {
        let (email, token) = text.split_once('\n')?;
        let credentials = Self {
            email: email.trim().to_owned(),
            token: token.trim().to_owned(),
        };
        credentials.is_usable().then_some(credentials)
    }

    /// A value a header can't carry would fail the request with the secret in the error.
    fn is_usable(&self) -> bool {
        let safe = |value: &str| {
            !value.is_empty() && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        };
        safe(&self.email) && safe(&self.token)
    }

    fn header(&self) -> String {
        let pair = format!("{}:{}", self.email, self.token);
        format!("Basic {}", BASE64.encode(pair))
    }
}

type CredentialSource = Box<dyn Fn() -> Option<BitbucketCredentials> + Send + Sync>;

/// The account a token signs in as, and the scopes hoshi needs that it lacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitbucketAccess {
    pub login: String,
    /// In [`REQUIRED_SCOPES`]'s form. When Bitbucket doesn't list the token's scopes, only
    /// the read scopes can be checked, by reading; `write:pullrequest` is then assumed.
    pub missing_scopes: Vec<String>,
}

/// What Bitbucket answered, before it is read as a result.
struct Reply {
    status: u16,
    text: String,
    retry_after: Option<u64>,
    /// `x-oauth-scopes`: what the token may do.
    granted: Option<String>,
    /// `x-accepted-oauth-scopes`: what the endpoint asks for.
    accepted: Option<String>,
}

impl Reply {
    fn into_result(self) -> Result<String, ForgeError> {
        let message = || rejection_message(&self.text, self.status);
        match self.status {
            200..=299 => Ok(self.text),
            401 => Err(ForgeError::Unauthorized),
            403 => Err(ForgeError::Forbidden {
                message: message(),
                scopes: scopes::lacking(
                    &self.text,
                    self.accepted.as_deref(),
                    self.granted.as_deref(),
                ),
            }),
            404 => Err(ForgeError::NotFound),
            409 => Err(ForgeError::Conflict(message())),
            429 => Err(ForgeError::RateLimited {
                retry_after_secs: self.retry_after.unwrap_or(DEFAULT_RETRY_SECS),
            }),
            400..=499 => Err(ForgeError::Rejected(message())),
            status => Err(ForgeError::Unexpected(format!("HTTP {status}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

/// What was last read beside a pull request, and the state it belongs to.
#[derive(Debug, Clone)]
struct Extras {
    updated_on: String,
    commit: Option<String>,
    summary: Summary,
    checks_read_at: Instant,
    comments_read_at: Instant,
}

type PullRequestKey = (String, String, u32);

pub struct Bitbucket {
    agent: ureq::Agent,
    source: CredentialSource,
    credentials: Mutex<Option<BitbucketCredentials>>,
    main_branches: Mutex<HashMap<(String, String), Option<String>>>,
    extras: Mutex<HashMap<PullRequestKey, Extras>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

impl Bitbucket {
    pub fn new(source: impl Fn() -> Option<BitbucketCredentials> + Send + Sync + 'static) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(20)))
            .max_idle_age(Duration::from_secs(90))
            // Diffs redirect within the API; private repositories need the header there too.
            .redirect_auth_headers(RedirectAuthHeaders::SameHost)
            .build()
            .into();
        Self {
            agent,
            source: Box::new(source),
            credentials: Mutex::new(None),
            main_branches: Mutex::new(HashMap::new()),
            extras: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_credentials(credentials: BitbucketCredentials) -> Self {
        Self::new(move || Some(credentials.clone()))
    }

    fn credentials(&self) -> Result<BitbucketCredentials, ForgeError> {
        let mut credentials = lock(&self.credentials);
        if credentials.is_none() {
            *credentials = (self.source)().filter(BitbucketCredentials::is_usable);
        }
        credentials.clone().ok_or(ForgeError::Unauthorized)
    }

    /// `path` is under the API root, or a `next` link Bitbucket sent, which must stay on it.
    fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<String, ForgeError> {
        let reply = self.send(method, path, body)?;
        if reply.status == 401 {
            self.forget_credentials();
        }
        reply.into_result()
    }

    fn send(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Reply, ForgeError> {
        let url = if path.starts_with("https://") {
            if !path.starts_with(&format!("{API_URL}/")) {
                return Err(ForgeError::Unexpected(
                    "a link off the Bitbucket API".into(),
                ));
            }
            path.to_owned()
        } else {
            format!("{API_URL}{path}")
        };
        let credentials = self.credentials()?;
        let authorization = credentials.header();
        let payload = body.map(Value::to_string).unwrap_or_default();
        let sent = match method {
            Method::Get => self
                .agent
                .get(&url)
                .header("Authorization", &authorization)
                .call(),
            Method::Delete => self
                .agent
                .delete(&url)
                .header("Authorization", &authorization)
                .call(),
            Method::Post => self
                .agent
                .post(&url)
                .header("Authorization", &authorization)
                .header("Content-Type", "application/json")
                .send(payload.as_str()),
            Method::Put => self
                .agent
                .put(&url)
                .header("Authorization", &authorization)
                .header("Content-Type", "application/json")
                .send(payload.as_str()),
        };
        let mut response = sent.map_err(|error| ForgeError::Network(error.to_string()))?;
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        let retry_after = header("retry-after").and_then(|value| value.trim().parse().ok());
        let granted = header("x-oauth-scopes");
        let accepted = header("x-accepted-oauth-scopes");
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_DIFF_BYTES)
            .read_to_string()
            .map_err(|error| ForgeError::Network(error.to_string()))?;
        Ok(Reply {
            status,
            text,
            retry_after,
            granted,
            accepted,
        })
    }

    /// The account the token signs in as, and which of [`REQUIRED_SCOPES`] it lacks. Only
    /// reads: the token's scopes come from the header Bitbucket answers with, or, without
    /// it, from one read per read scope.
    pub fn check_access(&self) -> Result<BitbucketAccess, ForgeError> {
        let reply = self.send(Method::Get, "/user", None)?;
        if reply.status == 401 {
            self.forget_credentials();
        }
        let granted = reply.granted.as_deref().and_then(scopes::from_header);
        let user: User = serde_json::from_str(&reply.into_result()?)
            .map_err(|error| ForgeError::Unexpected(error.to_string()))?;
        let login = user
            .login()
            .ok_or_else(|| ForgeError::Unexpected("the account has no name".into()))?;
        let missing_scopes = match granted {
            Some(granted) => scopes::missing(&granted, &REQUIRED_SCOPES),
            None => self.probe_read_scopes(&user)?,
        };
        Ok(BitbucketAccess {
            login,
            missing_scopes,
        })
    }

    /// The read scopes whose read was refused.
    fn probe_read_scopes(&self, user: &User) -> Result<Vec<String>, ForgeError> {
        let mut probes = vec![
            (
                "read:workspace:bitbucket",
                "/user/workspaces?pagelen=1".to_owned(),
            ),
            (
                "read:repository:bitbucket",
                "/repositories?role=member&pagelen=1".to_owned(),
            ),
        ];
        if let Some(uuid) = &user.uuid {
            probes.push((
                "read:pullrequest:bitbucket",
                format!("/pullrequests/{}?pagelen=1", encode(uuid)),
            ));
        }
        let mut missing = Vec::new();
        for (scope, path) in probes {
            match self.request(Method::Get, &path, None) {
                Ok(_) => {}
                Err(ForgeError::Forbidden { .. }) => missing.push(scope.to_owned()),
                Err(error @ (ForgeError::Unauthorized | ForgeError::RateLimited { .. })) => {
                    return Err(error);
                }
                Err(error) => eprintln!("hoshi: couldn't check a Bitbucket scope: {error}"),
            }
        }
        Ok(missing)
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ForgeError> {
        let text = self.request(Method::Get, path, None)?;
        serde_json::from_str(&text).map_err(|error| ForgeError::Unexpected(error.to_string()))
    }

    /// Every page of a listing, up to [`MAX_PAGES`].
    fn get_all<T: DeserializeOwned>(&self, path: &str) -> Result<Vec<T>, ForgeError> {
        let mut values = Vec::new();
        let mut next = Some(path.to_owned());
        for _ in 0..MAX_PAGES {
            let Some(url) = next.take() else { break };
            let page: Page<T> = self.get(&url)?;
            values.extend(page.values);
            next = page.next;
        }
        Ok(values)
    }

    fn main_branch(&self, repository: &RepositoryRef) -> Result<Option<String>, ForgeError> {
        let key = (repository.owner.clone(), repository.name.clone());
        if let Some(branch) = lock(&self.main_branches).get(&key) {
            return Ok(branch.clone());
        }
        #[derive(Deserialize)]
        struct Repository {
            mainbranch: Option<model::Branch>,
        }
        let found: Repository = self.get(&format!(
            "{}?fields=mainbranch.name",
            repository_path(repository)
        ))?;
        let branch = found.mainbranch.map(|branch| branch.name);
        lock(&self.main_branches).insert(key, branch.clone());
        Ok(branch)
    }

    fn statuses(
        &self,
        repository: &RepositoryRef,
        number: u32,
    ) -> Result<Vec<RawStatus>, ForgeError> {
        self.get_all(&format!(
            "{}/statuses?pagelen={PAGE_LEN}",
            pull_request_path(repository, number)
        ))
    }

    fn comments(
        &self,
        repository: &RepositoryRef,
        number: u32,
    ) -> Result<Vec<RawComment>, ForgeError> {
        self.get_all(&format!(
            "{}/comments?pagelen={PAGE_LEN}",
            pull_request_path(repository, number)
        ))
    }

    fn diffstat(
        &self,
        repository: &RepositoryRef,
        number: u32,
    ) -> Result<Vec<RawDiffStat>, ForgeError> {
        self.get_all(&format!(
            "{}/diffstat?pagelen={PAGE_LEN}",
            pull_request_path(repository, number)
        ))
    }

    /// The pull request with its checks, open threads and line counts, reading only what
    /// changed since the last time.
    fn summarize(
        &self,
        repository: &RepositoryRef,
        raw: &RawPullRequest,
    ) -> Result<PullRequest, ForgeError> {
        let key = (repository.owner.clone(), repository.name.clone(), raw.id);
        let cached = lock(&self.extras).get(&key).cloned();
        let now = Instant::now();
        let commit = raw.head_commit().map(str::to_owned);
        let same_head = cached
            .as_ref()
            .is_some_and(|extras| extras.commit == commit);
        let same_update = cached
            .as_ref()
            .is_some_and(|extras| extras.updated_on == raw.updated_on);
        let open = !raw.state().is_finished();
        let mut summary = cached
            .as_ref()
            .map(|extras| extras.summary)
            .unwrap_or_default();

        let mut checks_read_at = cached.as_ref().map_or(now, |extras| extras.checks_read_at);
        let checks_age = now.duration_since(checks_read_at);
        let checks_stale = match summary.checks {
            Some(ChecksState::Pending) => checks_age >= PENDING_CHECKS_FRESH_FOR,
            _ => checks_age >= CHECKS_FRESH_FOR,
        };
        if (!same_head || (open && checks_stale))
            && let Some(statuses) = best_effort(self.statuses(repository, raw.id))?
        {
            summary.checks = model::checks_state(&model::checks(&statuses));
            checks_read_at = now;
        }
        if !same_head && let Some(stats) = best_effort(self.diffstat(repository, raw.id))? {
            summary.additions = stats.iter().filter_map(|stat| stat.lines_added).sum();
            summary.deletions = stats.iter().filter_map(|stat| stat.lines_removed).sum();
        }
        let mut comments_read_at = cached
            .as_ref()
            .map_or(now, |extras| extras.comments_read_at);
        if (!same_update || now.duration_since(comments_read_at) >= COMMENTS_FRESH_FOR)
            && let Some(comments) = best_effort(self.comments(repository, raw.id))?
        {
            summary.unresolved_comments = model::unresolved_threads(&comments);
            comments_read_at = now;
        }
        lock(&self.extras).insert(
            key,
            Extras {
                updated_on: raw.updated_on.clone(),
                commit,
                summary,
                checks_read_at,
                comments_read_at,
            },
        );
        Ok(model::pull_request(raw, summary))
    }

    fn workspace_members(&self, repository: &RepositoryRef) -> Result<Vec<User>, ForgeError> {
        #[derive(Deserialize)]
        struct Membership {
            user: User,
        }
        let members: Vec<Membership> = self.get_all(&format!(
            "/workspaces/{}/members?pagelen={PAGE_LEN}",
            encode(&repository.owner)
        ))?;
        Ok(members.into_iter().map(|member| member.user).collect())
    }

    /// Who a comment may mention: the pull request's people, then the workspace's members.
    fn mentionable(&self, repository: &RepositoryRef, number: Option<u32>) -> Vec<User> {
        let mut users = Vec::new();
        if let Some(number) = number
            && let Ok(raw) = self.get::<RawPullRequest>(&pull_request_path(repository, number))
        {
            users.extend(raw.author.clone());
            users.extend(raw.reviewers.iter().cloned());
            users.extend(raw.participants.iter().filter_map(|each| each.user.clone()));
        }
        if let Ok(Some(members)) = best_effort(self.workspace_members(repository)) {
            users.extend(members);
        }
        users
    }

    fn mentions_in(&self, repository: &RepositoryRef, number: Option<u32>, text: &str) -> String {
        if !text.contains('@') {
            return text.to_owned();
        }
        model::encode_mentions(text, &self.mentionable(repository, number))
    }

    fn with_mentions(
        &self,
        repository: &RepositoryRef,
        number: Option<u32>,
        action: &PullRequestAction,
    ) -> PullRequestAction {
        let mut action = action.clone();
        match &mut action {
            PullRequestAction::Comment { body }
            | PullRequestAction::ReplyToThread { body, .. }
            | PullRequestAction::Edit { body, .. } => {
                *body = self.mentions_in(repository, number, body);
            }
            _ => {}
        }
        action
    }

    fn change_reviewers(
        &self,
        repository: &RepositoryRef,
        number: u32,
        add: &[String],
        remove: &[String],
    ) -> Result<(), ForgeError> {
        let path = pull_request_path(repository, number);
        let current: RawPullRequest = self.get(&path)?;
        let uuid_of = |users: &[User], names: &[String]| -> Vec<String> {
            names
                .iter()
                .filter_map(|name| {
                    users
                        .iter()
                        .find(|user| user.login().as_deref() == Some(name.as_str()))
                        .and_then(|user| user.uuid.clone())
                })
                .collect()
        };
        let added = if add.is_empty() {
            Vec::new()
        } else {
            uuid_of(&self.workspace_members(repository)?, add)
        };
        let removed = uuid_of(&current.reviewers, remove);
        let current_uuids: Vec<String> = current
            .reviewers
            .iter()
            .filter_map(|user| user.uuid.clone())
            .collect();
        let body = actions::reviewer_list(&current_uuids, &added, &removed);
        self.request(Method::Put, &path, Some(&body)).map(drop)
    }
}

/// A read beside the main one: a refusal leaves the old value, but a rate limit or a lost
/// sign-in stops the whole read.
fn best_effort<T>(result: Result<T, ForgeError>) -> Result<Option<T>, ForgeError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error @ (ForgeError::RateLimited { .. } | ForgeError::Unauthorized)) => Err(error),
        Err(error) => {
            eprintln!("hoshi: a Bitbucket read beside a pull request failed: {error}");
            Ok(None)
        }
    }
}

/// Bitbucket explains a refusal in `error.message`, sometimes with `error.detail`.
fn rejection_message(text: &str, status: u16) -> String {
    let parsed: Option<Value> = serde_json::from_str(text).ok();
    let error = parsed.as_ref().and_then(|value| value.get("error"));
    let message = error
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str);
    let detail = error
        .and_then(|error| error.get("detail"))
        .and_then(Value::as_str);
    match (message, detail) {
        (Some(message), Some(detail)) if !detail.is_empty() => format!("{message}: {detail}"),
        (Some(message), _) => message.to_owned(),
        _ => format!("HTTP {status}"),
    }
}

/// Percent-encodes everything but RFC 3986's unreserved characters.
fn encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// A string inside a `q=` filter.
fn filter_literal(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn repository_path(repository: &RepositoryRef) -> String {
    format!(
        "/repositories/{}/{}",
        encode(&repository.owner),
        encode(&repository.name)
    )
}

fn pull_request_path(repository: &RepositoryRef, number: u32) -> String {
    format!("{}/pullrequests/{number}", repository_path(repository))
}

impl Forge for Bitbucket {
    fn kind(&self) -> ForgeKind {
        ForgeKind::Bitbucket
    }

    fn viewer(&self) -> Result<String, ForgeError> {
        let user: User = self.get("/user")?;
        user.login()
            .ok_or_else(|| ForgeError::Unexpected("the account has no name".into()))
    }

    fn forget_credentials(&self) {
        *lock(&self.credentials) = None;
    }

    fn pull_request_for_branch(
        &self,
        repository: &RepositoryRef,
        branch: &str,
    ) -> Result<Option<PullRequest>, ForgeError> {
        if self.main_branch(repository)?.as_deref() == Some(branch) {
            return Ok(None);
        }
        let filter = format!("source.branch.name = {}", filter_literal(branch));
        let page: Page<RawPullRequest> = self.get(&format!(
            "{}/pullrequests?{ALL_STATES}&sort=-updated_on&pagelen=1&q={}",
            repository_path(repository),
            encode(&filter)
        ))?;
        match page.values.first() {
            Some(raw) => self.summarize(repository, raw).map(Some),
            None => Ok(None),
        }
    }

    fn pull_request(
        &self,
        repository: &RepositoryRef,
        number: u32,
    ) -> Result<PullRequest, ForgeError> {
        let raw: RawPullRequest = self.get(&pull_request_path(repository, number))?;
        self.summarize(repository, &raw)
    }

    fn create_pull_request(
        &self,
        repository: &RepositoryRef,
        pull_request: &NewPullRequest,
    ) -> Result<PullRequest, ForgeError> {
        let path = format!("{}/pullrequests", repository_path(repository));
        let body = json!({
            "title": pull_request.title,
            "description": self.mentions_in(repository, None, &pull_request.body),
            "source": { "branch": { "name": pull_request.head } },
            "destination": { "branch": { "name": pull_request.base } },
            "draft": pull_request.draft,
        });
        let text = self.request(Method::Post, &path, Some(&body))?;
        let raw: RawPullRequest = serde_json::from_str(&text)
            .map_err(|error| ForgeError::Unexpected(error.to_string()))?;
        self.summarize(repository, &raw)
    }

    fn search_pull_requests(
        &self,
        repository: &RepositoryRef,
        query: &str,
    ) -> Result<Vec<PullRequest>, ForgeError> {
        let query = query.trim().trim_start_matches('#');
        if let Ok(number) = query.parse::<u32>() {
            return match self.pull_request(repository, number) {
                Ok(pull_request) => Ok(vec![pull_request]),
                Err(ForgeError::NotFound) => Ok(Vec::new()),
                Err(error) => Err(error),
            };
        }
        let mut path = format!(
            "{}/pullrequests?{ALL_STATES}&sort=-updated_on&pagelen={SEARCH_LIMIT}",
            repository_path(repository)
        );
        if !query.is_empty() {
            let literal = filter_literal(query);
            let filter = format!("(title ~ {literal} OR source.branch.name ~ {literal})");
            path.push_str(&format!("&q={}", encode(&filter)));
        }
        let page: Page<RawPullRequest> = self.get(&path)?;
        Ok(page
            .values
            .iter()
            .map(|raw| model::pull_request(raw, Summary::default()))
            .collect())
    }

    fn pull_request_detail(
        &self,
        repository: &RepositoryRef,
        number: u32,
    ) -> Result<PullRequestDetail, ForgeError> {
        let path = pull_request_path(repository, number);
        let raw: RawPullRequest = self.get(&path)?;
        let statuses = best_effort(self.statuses(repository, number))?.unwrap_or_default();
        let comments = best_effort(self.comments(repository, number))?.unwrap_or_default();
        let stats = best_effort(self.diffstat(repository, number))?.unwrap_or_default();
        let commits: Vec<RawCommit> =
            best_effort(self.get_all(&format!("{path}/commits?pagelen={PAGE_LEN}")))?
                .unwrap_or_default();
        let mergeability = if raw.state() == PullRequestState::Merged {
            Mergeability::Unknown
        } else {
            mergeability(best_effort(
                self.get::<Page<Value>>(&format!("{path}/conflicts")),
            )?)
        };
        let checks = model::checks(&statuses);
        Ok(PullRequestDetail {
            supports: PullRequestSupport {
                labels: false,
                draft: false,
                reopen: false,
            },
            forge_id: raw.id.to_string(),
            repository: format!("{}/{}", repository.owner, repository.name),
            mergeability,
            merge_methods: vec![MergeMethod::Merge, MergeMethod::Squash, MergeMethod::Rebase],
            // Bitbucket doesn't say up front; the forge refuses what the account can't do.
            can_edit: true,
            body: raw.body(),
            changed_files: stats.len() as u32,
            reviewers: model::reviewers(&raw),
            labels: Vec::new(),
            review_threads: model::review_threads(&comments, &raw.url()),
            timeline: model::timeline(&raw, &comments, &commits),
            checks,
        })
    }

    fn pull_request_files(
        &self,
        repository: &RepositoryRef,
        number: u32,
    ) -> Result<Vec<PullRequestFile>, ForgeError> {
        let stats = self.diffstat(repository, number)?;
        let patches = match self.request(
            Method::Get,
            &format!("{}/diff", pull_request_path(repository, number)),
            None,
        ) {
            Ok(diff) => patch::split(&diff),
            Err(error @ (ForgeError::RateLimited { .. } | ForgeError::Unauthorized)) => {
                return Err(error);
            }
            Err(_) => HashMap::new(),
        };
        Ok(model::diff_files(&stats)
            .into_iter()
            .map(|file| PullRequestFile {
                patch: patches
                    .get(&file.path)
                    .filter(|hunks| !hunks.is_empty())
                    .cloned(),
                file,
            })
            .collect())
    }

    fn act(
        &self,
        repository: &RepositoryRef,
        number: u32,
        _forge_id: &str,
        action: &PullRequestAction,
    ) -> Result<(), ForgeError> {
        let action = self.with_mentions(repository, Some(number), action);
        let result = match actions::plan(&action) {
            Plan::Request { method, path, body } => self
                .request(
                    method,
                    &format!("{}{path}", pull_request_path(repository, number)),
                    body.as_ref(),
                )
                .map(drop),
            Plan::Reviewers { add, remove } => {
                self.change_reviewers(repository, number, &add, &remove)
            }
            Plan::Unsupported(reason) => Err(ForgeError::Rejected(reason.to_owned())),
        };
        lock(&self.extras).remove(&(repository.owner.clone(), repository.name.clone(), number));
        result
    }

    fn reviewer_candidates(
        &self,
        repository: &RepositoryRef,
        query: &str,
    ) -> Result<Vec<String>, ForgeError> {
        let needle = query.trim().to_lowercase();
        let mut names: Vec<String> = self
            .workspace_members(repository)?
            .iter()
            .filter_map(User::login)
            .filter(|name| name.to_lowercase().contains(&needle))
            .collect();
        names.sort_by_key(|name| name.to_lowercase());
        names.dedup();
        Ok(names)
    }

    fn labels(&self, _repository: &RepositoryRef) -> Result<Vec<PullRequestLabel>, ForgeError> {
        Ok(Vec::new())
    }

    fn repository(&self, repository: &RepositoryRef) -> Result<RemoteRepository, ForgeError> {
        #[derive(Deserialize)]
        struct Link {
            href: String,
            #[serde(default)]
            name: String,
        }
        #[derive(Deserialize)]
        struct Links {
            html: Link,
            #[serde(default)]
            clone: Vec<Link>,
        }
        #[derive(Deserialize)]
        struct Repository {
            full_name: String,
            name: String,
            #[serde(default)]
            description: String,
            is_private: bool,
            links: Links,
        }
        let found: Repository = self.get(&format!(
            "{}?fields=full_name,name,description,is_private,links.html,links.clone",
            repository_path(repository)
        ))?;
        let clone_url = found
            .links
            .clone
            .into_iter()
            .find(|link| link.name == "https")
            .map(|link| link.href)
            .unwrap_or_else(|| format!("https://bitbucket.org/{}.git", found.full_name));
        Ok(RemoteRepository {
            forge: ForgeKind::Bitbucket,
            name: found.name,
            description: Some(found.description).filter(|text| !text.trim().is_empty()),
            web_url: found.links.html.href,
            clone_url,
            private: found.is_private,
            full_name: found.full_name,
        })
    }
}

/// An empty conflict list is mergeable; an unreadable one, unknown.
fn mergeability(conflicts: Option<Page<Value>>) -> Mergeability {
    match conflicts {
        Some(page) if page.values.is_empty() => Mergeability::Mergeable,
        Some(_) => Mergeability::Conflicting,
        None => Mergeability::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_round_trip_and_refuse_unsafe_values() {
        let credentials = BitbucketCredentials {
            email: "ana@example.com".into(),
            token: "ATATT3x-abc".into(),
        };
        assert_eq!(
            BitbucketCredentials::decode(&credentials.encode()),
            Some(credentials.clone())
        );
        assert_eq!(BitbucketCredentials::decode("ana@example.com\n"), None);
        assert_eq!(
            BitbucketCredentials::decode("ana@example.com\nto ken"),
            None
        );
        assert_eq!(
            credentials.header(),
            format!("Basic {}", BASE64.encode("ana@example.com:ATATT3x-abc"))
        );
        assert!(!format!("{credentials:?}").contains("ATATT"));
    }

    #[test]
    fn encodes_branch_filters() {
        assert_eq!(
            filter_literal(r#"fix/"quoted"\x"#),
            r#""fix/\"quoted\"\\x""#
        );
        assert_eq!(
            encode("source.branch.name = \"a b\""),
            "source.branch.name%20%3D%20%22a%20b%22"
        );
    }

    fn reply(status: u16, text: &str, granted: &str) -> Reply {
        Reply {
            status,
            text: text.into(),
            retry_after: None,
            granted: Some(granted.into()),
            accepted: Some("write:pullrequest:bitbucket".into()),
        }
    }

    #[test]
    fn reads_each_kind_of_refusal() {
        let reads = "read:user:bitbucket, read:pullrequest:bitbucket";
        let scopes = r#"{"type":"error","error":{"message":"Your credentials lack one or more required privilege scopes.","detail":{"granted":["read:user:bitbucket"],"required":["write:pullrequest:bitbucket"]}}}"#;
        assert_eq!(
            reply(403, scopes, reads).into_result(),
            Err(ForgeError::Forbidden {
                message: "Your credentials lack one or more required privilege scopes.".into(),
                scopes: vec!["write:pullrequest:bitbucket".into()],
            })
        );
        let access =
            r#"{"type":"error","error":{"message":"You do not have access to this repository."}}"#;
        assert_eq!(
            reply(403, access, reads).into_result(),
            Err(ForgeError::Forbidden {
                message: "You do not have access to this repository.".into(),
                scopes: vec!["write:pullrequest:bitbucket".into()],
            })
        );
        assert_eq!(
            reply(403, access, "write:pullrequest:bitbucket").into_result(),
            Err(ForgeError::Forbidden {
                message: "You do not have access to this repository.".into(),
                scopes: Vec::new(),
            })
        );
        let conflict = r#"{"type":"error","error":{"message":"Conflict"}}"#;
        assert_eq!(
            reply(409, conflict, reads).into_result(),
            Err(ForgeError::Conflict("Conflict".into()))
        );
        assert_eq!(
            reply(401, "", reads).into_result(),
            Err(ForgeError::Unauthorized)
        );
        assert_eq!(
            reply(404, "", reads).into_result(),
            Err(ForgeError::NotFound)
        );
        assert_eq!(reply(201, "{}", reads).into_result(), Ok("{}".into()));
    }

    #[test]
    fn reads_bitbucket_refusals() {
        assert_eq!(
            rejection_message(
                r#"{"type":"error","error":{"message":"Nope","detail":"Not allowed"}}"#,
                403
            ),
            "Nope: Not allowed"
        );
        assert_eq!(rejection_message("<html>", 400), "HTTP 400");
    }
}
