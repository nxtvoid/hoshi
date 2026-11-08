//! The detail view of a pull request: description, people, checks, timeline and files.

use crate::{ForgeError, PullRequestFile};
use hoshi_core::{
    ChecksState, DiffFile, DiffFileStatus, MergeMethod, Mergeability, PullRequestCheck,
    PullRequestDetail, PullRequestLabel, PullRequestSupport, ReviewComment, ReviewState,
    ReviewThread, Reviewer, TimelineItem,
};
use serde::Deserialize;

/// Everything but the activity, which [`ACTIVITY_QUERY`] reads at the same time.
pub(super) const CORE_QUERY: &str = "
query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    nameWithOwner mergeCommitAllowed squashMergeAllowed rebaseMergeAllowed
    pullRequest(number: $number) {
      id body changedFiles mergeable viewerCanUpdate
      reviewRequests(first: 20) { nodes { requestedReviewer {
        __typename
        ... on User { login }
        ... on Bot { login }
        ... on Mannequin { login }
        ... on Team { name }
      } } }
      latestReviews(first: 20) { nodes { author { login } state } }
      labels(first: 20) { nodes { name color } }
      commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 100) { nodes {
        __typename
        ... on CheckRun { name status conclusion detailsUrl }
        ... on StatusContext { context state targetUrl }
      } } } } } }
    }
  }
}";

/// The timeline and review threads, the slowest part, read in parallel with [`CORE_QUERY`].
pub(super) const ACTIVITY_QUERY: &str = "
query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      timelineItems(last: 100, itemTypes: [ISSUE_COMMENT, PULL_REQUEST_REVIEW, PULL_REQUEST_COMMIT]) {
        nodes {
          __typename
          ... on IssueComment { author { login } body createdAt url }
          ... on PullRequestReview { author { login } state body submittedAt url }
          ... on PullRequestCommit {
            url
            commit { abbreviatedOid messageHeadline committedDate author { name user { login } } }
          }
        }
      }
      reviewThreads(first: 100) { nodes {
        id isResolved path line
        comments(first: 50) { nodes { author { login } body createdAt url } }
      } }
    }
  }
}";

#[derive(Deserialize)]
pub(super) struct DetailData {
    repository: Option<Repository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Repository {
    name_with_owner: String,
    merge_commit_allowed: bool,
    squash_merge_allowed: bool,
    rebase_merge_allowed: bool,
    pull_request: Option<DetailPullRequest>,
}

#[derive(Deserialize)]
struct Nodes<T> {
    nodes: Vec<T>,
}

impl<T> Default for Nodes<T> {
    fn default() -> Self {
        Self { nodes: Vec::new() }
    }
}

#[derive(Deserialize)]
pub(super) struct ActivityData {
    repository: Option<ActivityRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActivityRepository {
    pull_request: Option<ActivityPullRequest>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActivityPullRequest {
    timeline_items: Nodes<TimelineNode>,
    review_threads: Nodes<ThreadNode>,
}

/// Puts the activity read by [`ACTIVITY_QUERY`] into the rest of the detail.
pub(super) fn with_activity(mut core: DetailData, activity: ActivityData) -> DetailData {
    let activity = activity
        .repository
        .and_then(|repository| repository.pull_request);
    let pull_request = core
        .repository
        .as_mut()
        .and_then(|repository| repository.pull_request.as_mut());
    if let (Some(pull_request), Some(activity)) = (pull_request, activity) {
        pull_request.timeline_items = activity.timeline_items;
        pull_request.review_threads = activity.review_threads;
    }
    core
}

#[derive(Deserialize)]
struct Author {
    login: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DetailPullRequest {
    id: String,
    body: String,
    changed_files: u32,
    mergeable: String,
    viewer_can_update: bool,
    review_requests: Nodes<ReviewRequest>,
    latest_reviews: Nodes<LatestReview>,
    labels: Nodes<Label>,
    #[serde(default)]
    timeline_items: Nodes<TimelineNode>,
    commits: Nodes<CommitNode>,
    #[serde(default)]
    review_threads: Nodes<ThreadNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewRequest {
    requested_reviewer: Option<RequestedReviewer>,
}

/// A user, bot or mannequin has a `login`; a team has a `name`.
#[derive(Deserialize)]
struct RequestedReviewer {
    login: Option<String>,
    name: Option<String>,
}

#[derive(Deserialize)]
struct LatestReview {
    author: Option<Author>,
    state: String,
}

#[derive(Deserialize)]
struct Label {
    name: String,
    color: String,
}

/// One of `IssueComment`, `PullRequestReview` or `PullRequestCommit`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimelineNode {
    #[serde(rename = "__typename")]
    typename: String,
    author: Option<Author>,
    body: Option<String>,
    created_at: Option<String>,
    submitted_at: Option<String>,
    state: Option<String>,
    url: Option<String>,
    commit: Option<TimelineCommit>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimelineCommit {
    abbreviated_oid: String,
    message_headline: String,
    committed_date: String,
    author: Option<GitActor>,
}

#[derive(Deserialize)]
struct GitActor {
    name: Option<String>,
    user: Option<Author>,
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
    contexts: Nodes<CheckContext>,
}

/// A `CheckRun` (GitHub Actions and apps) or a `StatusContext` (the older status API).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckContext {
    #[serde(rename = "__typename")]
    typename: String,
    name: Option<String>,
    status: Option<String>,
    conclusion: Option<String>,
    details_url: Option<String>,
    context: Option<String>,
    state: Option<String>,
    target_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadNode {
    id: String,
    is_resolved: bool,
    path: String,
    line: Option<u32>,
    comments: Nodes<CommentNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentNode {
    author: Option<Author>,
    body: String,
    created_at: String,
    url: String,
}

/// An entry of `GET /repos/{owner}/{repo}/pulls/{number}/files`.
#[derive(Deserialize)]
pub(super) struct RestFile {
    filename: String,
    previous_filename: Option<String>,
    status: String,
    additions: u32,
    deletions: u32,
    /// Missing for binary files and diffs too large for the API.
    patch: Option<String>,
}

pub(super) fn detail_from(data: DetailData) -> Result<PullRequestDetail, ForgeError> {
    let repository = data.repository.ok_or(ForgeError::NotFound)?;
    let merge_methods = [
        (repository.merge_commit_allowed, MergeMethod::Merge),
        (repository.squash_merge_allowed, MergeMethod::Squash),
        (repository.rebase_merge_allowed, MergeMethod::Rebase),
    ]
    .into_iter()
    .filter_map(|(allowed, method)| allowed.then_some(method))
    .collect();
    let pull_request = repository.pull_request.ok_or(ForgeError::NotFound)?;
    let mut reviewers: Vec<Reviewer> = pull_request
        .latest_reviews
        .nodes
        .into_iter()
        .filter_map(|review| {
            Some(Reviewer {
                name: review.author?.login,
                state: review_state(&review.state),
            })
        })
        .collect();
    for request in pull_request.review_requests.nodes {
        let Some(reviewer) = request.requested_reviewer else {
            continue;
        };
        let Some(name) = reviewer.login.or(reviewer.name) else {
            continue;
        };
        if !reviewers.iter().any(|existing| existing.name == name) {
            reviewers.push(Reviewer {
                name,
                state: ReviewState::Requested,
            });
        }
    }
    let checks = pull_request
        .commits
        .nodes
        .into_iter()
        .last()
        .and_then(|node| node.commit.status_check_rollup)
        .map(|rollup| {
            rollup
                .contexts
                .nodes
                .into_iter()
                .filter_map(check_from)
                .collect()
        })
        .unwrap_or_default();
    let mergeability = match pull_request.mergeable.as_str() {
        "MERGEABLE" => Mergeability::Mergeable,
        "CONFLICTING" => Mergeability::Conflicting,
        _ => Mergeability::Unknown,
    };
    Ok(PullRequestDetail {
        supports: PullRequestSupport::all(),
        forge_id: pull_request.id,
        repository: repository.name_with_owner,
        mergeability,
        merge_methods,
        can_edit: pull_request.viewer_can_update,
        body: pull_request.body,
        changed_files: pull_request.changed_files,
        reviewers,
        labels: pull_request
            .labels
            .nodes
            .into_iter()
            .map(|label| PullRequestLabel {
                name: label.name,
                color: label.color,
            })
            .collect(),
        checks,
        review_threads: pull_request
            .review_threads
            .nodes
            .into_iter()
            .map(thread_from)
            .collect(),
        timeline: pull_request
            .timeline_items
            .nodes
            .into_iter()
            .filter_map(timeline_item_from)
            .collect(),
    })
}

pub(super) fn file_from(file: RestFile) -> PullRequestFile {
    let status = match file.status.as_str() {
        "added" | "copied" => DiffFileStatus::Added,
        "removed" => DiffFileStatus::Deleted,
        "renamed" => DiffFileStatus::Renamed,
        _ => DiffFileStatus::Modified,
    };
    let binary = file.patch.is_none() && file.additions == 0 && file.deletions == 0;
    PullRequestFile {
        file: DiffFile {
            path: file.filename,
            old_path: file.previous_filename,
            status,
            additions: (!binary).then_some(file.additions),
            deletions: (!binary).then_some(file.deletions),
        },
        patch: file.patch,
    }
}

fn review_state(state: &str) -> ReviewState {
    match state {
        "APPROVED" => ReviewState::Approved,
        "CHANGES_REQUESTED" => ReviewState::ChangesRequested,
        "DISMISSED" => ReviewState::Dismissed,
        _ => ReviewState::Commented,
    }
}

fn check_from(context: CheckContext) -> Option<PullRequestCheck> {
    let (name, state, url) = if context.typename == "CheckRun" {
        let state = if context.status.as_deref() != Some("COMPLETED") {
            ChecksState::Pending
        } else {
            match context.conclusion.as_deref() {
                Some("SUCCESS" | "NEUTRAL" | "SKIPPED") => ChecksState::Passing,
                _ => ChecksState::Failing,
            }
        };
        (context.name?, state, context.details_url)
    } else {
        let state = match context.state.as_deref() {
            Some("SUCCESS") => ChecksState::Passing,
            Some("PENDING" | "EXPECTED") => ChecksState::Pending,
            _ => ChecksState::Failing,
        };
        (context.context?, state, context.target_url)
    };
    Some(PullRequestCheck { name, state, url })
}

fn thread_from(thread: ThreadNode) -> ReviewThread {
    ReviewThread {
        id: thread.id,
        path: thread.path,
        line: thread.line,
        resolved: thread.is_resolved,
        comments: thread
            .comments
            .nodes
            .into_iter()
            .map(|comment| ReviewComment {
                author: comment.author.map(|author| author.login),
                body: comment.body,
                created_at: comment.created_at,
                url: comment.url,
            })
            .collect(),
    }
}

fn timeline_item_from(node: TimelineNode) -> Option<TimelineItem> {
    let author = node.author.map(|author| author.login);
    match node.typename.as_str() {
        "IssueComment" => Some(TimelineItem::Comment {
            author,
            body: node.body.unwrap_or_default(),
            created_at: node.created_at?,
            url: node.url.unwrap_or_default(),
        }),
        "PullRequestReview" => Some(TimelineItem::Review {
            author,
            state: review_state(node.state.as_deref().unwrap_or_default()),
            body: node.body.unwrap_or_default(),
            created_at: node.submitted_at?,
            url: node.url.unwrap_or_default(),
        }),
        "PullRequestCommit" => {
            let commit = node.commit?;
            let author = commit
                .author
                .and_then(|actor| actor.user.map(|user| user.login).or(actor.name));
            Some(TimelineItem::Commit {
                sha: commit.abbreviated_oid,
                message: commit.message_headline,
                author,
                created_at: commit.committed_date,
                url: node.url.unwrap_or_default(),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::parse_graphql;

    const RESPONSE: &str = r###"{"data":{"repository":{"nameWithOwner":"nxtvoid/avzx",
        "mergeCommitAllowed":false,"squashMergeAllowed":true,"rebaseMergeAllowed":true,
        "pullRequest":{"id":"PR_kw1","mergeable":"CONFLICTING","viewerCanUpdate":true,
        "body":"## summary\nupdates deps","changedFiles":21,
        "reviewRequests":{"nodes":[
            {"requestedReviewer":{"__typename":"User","login":"ana"}},
            {"requestedReviewer":{"__typename":"Team","name":"core"}},
            {"requestedReviewer":{"__typename":"User","login":"luis"}}
        ]},
        "latestReviews":{"nodes":[{"author":{"login":"luis"},"state":"APPROVED"}]},
        "labels":{"nodes":[{"name":"deps","color":"0366d6"}]},
        "timelineItems":{"nodes":[
            {"__typename":"PullRequestCommit","url":"https://c/1","commit":{"abbreviatedOid":"abc1234","messageHeadline":"chore: bump","committedDate":"2026-11-07T10:00:00Z","author":{"name":"Me","user":{"login":"nxtvoid"}}}},
            {"__typename":"IssueComment","author":{"login":"ana"},"body":"looks good","createdAt":"2026-11-07T11:00:00Z","url":"https://c/2"},
            {"__typename":"PullRequestReview","author":{"login":"luis"},"state":"APPROVED","body":"","submittedAt":"2026-11-07T12:00:00Z","url":"https://c/3"}
        ]},
        "commits":{"nodes":[{"commit":{"statusCheckRollup":{"contexts":{"nodes":[
            {"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":"FAILURE","detailsUrl":"https://ci/1"},
            {"__typename":"CheckRun","name":"lint","status":"IN_PROGRESS","conclusion":null,"detailsUrl":null},
            {"__typename":"CheckRun","name":"docs","status":"COMPLETED","conclusion":"SKIPPED","detailsUrl":null},
            {"__typename":"StatusContext","context":"deploy","state":"SUCCESS","targetUrl":"https://deploy"}
        ]}}}}]},
        "reviewThreads":{"nodes":[
            {"id":"PRRT_1","isResolved":false,"path":"src/a.rs","line":12,"comments":{"nodes":[
                {"author":{"login":"ana"},"body":"Rename this","createdAt":"2026-11-07T10:00:00Z","url":"https://c/4"}
            ]}}
        ]}
    }}}}"###;

    #[test]
    fn the_two_halves_make_the_same_detail_as_one_response() {
        let core = r#"{"data":{"repository":{"nameWithOwner":"o/r",
            "mergeCommitAllowed":true,"squashMergeAllowed":true,"rebaseMergeAllowed":true,
            "pullRequest":{"id":"PR_1","mergeable":"MERGEABLE","viewerCanUpdate":false,
            "body":"","changedFiles":1,"reviewRequests":{"nodes":[]},
            "latestReviews":{"nodes":[]},"labels":{"nodes":[]},"commits":{"nodes":[]}}}}}"#;
        let activity = r#"{"data":{"repository":{"pullRequest":{
            "timelineItems":{"nodes":[{"__typename":"IssueComment","author":{"login":"ana"},
                "body":"hi","createdAt":"2026-11-07T11:00:00Z","url":"https://c/2"}]},
            "reviewThreads":{"nodes":[{"id":"PRRT_1","isResolved":true,"path":"a.rs",
                "line":null,"comments":{"nodes":[]}}]}}}}}"#;

        let detail = parse_graphql::<DetailData>(core)
            .and_then(|core| {
                let activity = parse_graphql::<ActivityData>(activity)?;
                detail_from(with_activity(core, activity))
            })
            .unwrap();

        assert_eq!(detail.timeline.len(), 1);
        assert_eq!(detail.review_threads[0].id, "PRRT_1");
        assert_eq!(detail.merge_methods.len(), 3);
    }

    #[test]
    fn reads_people_labels_checks_threads_and_timeline() {
        let detail = parse_graphql::<DetailData>(RESPONSE)
            .and_then(detail_from)
            .unwrap();

        assert_eq!(detail.repository, "nxtvoid/avzx");
        assert_eq!(detail.forge_id, "PR_kw1");
        assert_eq!(detail.mergeability, Mergeability::Conflicting);
        assert_eq!(
            detail.merge_methods,
            [MergeMethod::Squash, MergeMethod::Rebase]
        );
        assert!(detail.can_edit);
        assert_eq!(detail.review_threads[0].id, "PRRT_1");
        assert_eq!(detail.changed_files, 21);
        let reviewers: Vec<_> = detail
            .reviewers
            .iter()
            .map(|reviewer| (reviewer.name.as_str(), reviewer.state))
            .collect();
        assert_eq!(
            reviewers,
            [
                ("luis", ReviewState::Approved),
                ("ana", ReviewState::Requested),
                ("core", ReviewState::Requested),
            ]
        );
        assert_eq!(detail.labels[0].color, "0366d6");
        let checks: Vec<_> = detail
            .checks
            .iter()
            .map(|check| (check.name.as_str(), check.state))
            .collect();
        assert_eq!(
            checks,
            [
                ("build", ChecksState::Failing),
                ("lint", ChecksState::Pending),
                ("docs", ChecksState::Passing),
                ("deploy", ChecksState::Passing),
            ]
        );
        assert_eq!(detail.review_threads[0].comments[0].body, "Rename this");
        assert!(matches!(
            &detail.timeline[..],
            [
                TimelineItem::Commit { sha, author: Some(author), .. },
                TimelineItem::Comment { .. },
                TimelineItem::Review { state: ReviewState::Approved, .. },
            ] if sha == "abc1234" && author == "nxtvoid"
        ));
    }

    #[test]
    fn reads_rest_files_and_spots_binaries() {
        let files: Vec<RestFile> = serde_json::from_str(
            r#"[
                {"filename":"src/a.rs","status":"modified","additions":2,"deletions":1,"patch":"@@ -1 +1,2 @@\n-a\n+b\n+c"},
                {"filename":"new.rs","previous_filename":"old.rs","status":"renamed","additions":0,"deletions":0,"patch":""},
                {"filename":"logo.png","status":"added","additions":0,"deletions":0}
            ]"#,
        )
        .unwrap();

        let files: Vec<_> = files.into_iter().map(file_from).collect();

        assert_eq!(files[0].file.status, DiffFileStatus::Modified);
        assert_eq!(files[0].file.additions, Some(2));
        assert_eq!(files[1].file.old_path.as_deref(), Some("old.rs"));
        assert_eq!(files[1].file.status, DiffFileStatus::Renamed);
        assert_eq!(files[2].file.additions, None);
        assert!(files[2].patch.is_none());
    }
}
