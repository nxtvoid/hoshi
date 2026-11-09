use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A code host that serves pull requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ForgeKind {
    Github,
    /// Bitbucket Cloud (bitbucket.org).
    Bitbucket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PullRequestState {
    Open,
    Draft,
    Merged,
    Closed,
}

impl PullRequestState {
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Merged | Self::Closed)
    }
}

/// The combined result of a pull request's CI checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ChecksState {
    Passing,
    Failing,
    Pending,
}

/// A pull request as every forge reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullRequest {
    pub forge: ForgeKind,
    pub number: u32,
    pub title: String,
    pub url: String,
    /// `None` when the author's account was deleted.
    pub author: Option<String>,
    pub head_branch: String,
    /// The commit its head branch was at when the forge was last asked. `None` in reads saved
    /// before hoshi kept it.
    #[serde(default)]
    pub head_sha: Option<String>,
    pub base_branch: String,
    pub state: PullRequestState,
    /// `None` when the pull request has no checks.
    pub checks: Option<ChecksState>,
    pub unresolved_comments: u32,
    pub additions: u32,
    pub deletions: u32,
    /// RFC 3339.
    pub updated_at: String,
}

/// What an update on a watched pull request told the agent, for the UI to name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullRequestUpdate {
    pub number: u32,
    /// Checks that failed since the last update.
    pub failed_checks: Vec<String>,
    pub checks_passed: bool,
    /// Comments and reviews since the last update.
    pub comments: u32,
    pub conflicting: bool,
    pub closed: bool,
    /// The watch ended with this update.
    pub stopped: bool,
}

/// What an agent watching its pull request was told so far. Kept across restarts, and
/// after the watch ends, so news from while hoshi was closed still wakes the agent and
/// watching again never repeats it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestWatch {
    pub number: u32,
    /// The commit the checks below belong to; `None` until first read.
    pub head_sha: Option<String>,
    pub failed_checks: Vec<String>,
    pub passed: bool,
    pub conflicting: bool,
    /// Comments up to this moment were told. RFC 3339.
    pub remarks_through: String,
    /// The comments at `remarks_through` already told, since forges stamp them to the second.
    pub remark_ids: Vec<String>,
    /// Comment-only wakes in a row.
    pub wakes: u32,
}

/// One CI check of a pull request's latest commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullRequestCheck {
    pub name: String,
    pub state: ChecksState,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReviewComment {
    pub author: Option<String>,
    pub body: String,
    /// RFC 3339.
    pub created_at: String,
    pub url: String,
}

/// A conversation on a line of the diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReviewThread {
    /// The forge's own id, to reply to or resolve the thread.
    pub id: String,
    pub path: String,
    /// `None` when the line is no longer in the diff.
    pub line: Option<u32>,
    pub resolved: bool,
    pub comments: Vec<ReviewComment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ReviewState {
    /// Asked to review, no review yet.
    Requested,
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Reviewer {
    /// A user's login or a team's name.
    pub name: String,
    pub state: ReviewState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullRequestLabel {
    pub name: String,
    /// Hex without `#`, as the forge stores it.
    pub color: String,
}

/// What happened on a pull request, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum TimelineItem {
    Comment {
        author: Option<String>,
        body: String,
        created_at: String,
        url: String,
    },
    Review {
        author: Option<String>,
        state: ReviewState,
        body: String,
        created_at: String,
        url: String,
    },
    Commit {
        sha: String,
        message: String,
        author: Option<String>,
        created_at: String,
        url: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

/// Whether the forge would merge the pull request as it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Mergeability {
    Mergeable,
    Conflicting,
    /// Still being computed by the forge, or blocked by rules it does not explain.
    Unknown,
}

/// A change the user asks the forge to make to a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum PullRequestAction {
    Merge {
        method: MergeMethod,
    },
    /// Closes or reopens it.
    SetOpen {
        open: bool,
    },
    SetDraft {
        draft: bool,
    },
    Edit {
        title: String,
        body: String,
    },
    RequestReviewers {
        names: Vec<String>,
    },
    RemoveReviewers {
        names: Vec<String>,
    },
    AddLabels {
        names: Vec<String>,
    },
    RemoveLabel {
        name: String,
    },
    Comment {
        body: String,
    },
    ReplyToThread {
        thread_id: String,
        body: String,
    },
    SetThreadResolved {
        thread_id: String,
        resolved: bool,
    },
}

/// What the side panel shows of a pull request, read on demand and not cached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullRequestDetail {
    /// What the forge lets hoshi change; older cached details predate it and are GitHub's.
    #[serde(default = "PullRequestSupport::all")]
    pub supports: PullRequestSupport,
    /// The forge's own id, needed by some actions.
    pub forge_id: String,
    /// `owner/name` or the forge's equivalent.
    pub repository: String,
    pub mergeability: Mergeability,
    /// The merge methods the repository allows.
    pub merge_methods: Vec<MergeMethod>,
    /// The signed-in account may edit it: title, labels, reviewers, state.
    pub can_edit: bool,
    /// Markdown.
    pub body: String,
    pub changed_files: u32,
    pub reviewers: Vec<Reviewer>,
    pub labels: Vec<PullRequestLabel>,
    pub checks: Vec<PullRequestCheck>,
    pub review_threads: Vec<ReviewThread>,
    pub timeline: Vec<TimelineItem>,
}

/// The account a forge is signed in with; `None` when it is not signed in.
/// Parts of a pull request some forges have and others don't.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullRequestSupport {
    pub labels: bool,
    /// Moving it in and out of draft.
    pub draft: bool,
    /// Reopening it once closed.
    pub reopen: bool,
}

impl PullRequestSupport {
    pub fn all() -> Self {
        Self {
            labels: true,
            draft: true,
            reopen: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ForgeAccount {
    pub forge: ForgeKind,
    pub login: Option<String>,
}

/// A fresh read of a thread's pull request detail; `None` when the read failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThreadPullRequestDetail {
    pub thread_id: String,
    pub detail: Option<PullRequestDetail>,
}

/// The pull request of a thread's branch; `None` when the branch has none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThreadPullRequest {
    pub thread_id: String,
    pub pull_request: Option<PullRequest>,
}
