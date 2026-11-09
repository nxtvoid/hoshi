use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What a thread's checkout allows: whether there is something to commit, push or pull.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GitStatus {
    /// `None` on a detached HEAD.
    pub branch: Option<String>,
    /// The repository's default branch, such as `main`; `None` when it can't be told.
    pub default_branch: Option<String>,
    pub has_changes: bool,
    pub has_upstream: bool,
    /// Whether `origin` is configured.
    pub has_remote: bool,
    /// Commits the branch has that its upstream lacks.
    pub ahead: u32,
    /// Commits its upstream has that the branch lacks.
    pub behind: u32,
    /// Commits the branch has that the default branch lacks.
    pub ahead_of_default: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum GitAction {
    Commit,
    CommitPush,
    CommitPushPullRequest,
    Push,
    /// Pushes first when the branch has commits its upstream lacks.
    CreatePullRequest,
    Pull,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GitActionRequest {
    pub action: GitAction,
    /// The user's own commit message; written for them when `None`.
    pub message: Option<String>,
    /// The files to commit; every change when `None`.
    pub paths: Option<Vec<String>>,
    /// Commits on a new branch named for the change, leaving the current one as it is.
    pub new_branch: bool,
    /// Opens the pull request as a draft.
    pub draft: bool,
}

/// The step a git action is at, as it reports it while running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum GitActionPhase {
    WritingCommitMessage,
    CreatingBranch,
    Committing,
    Pushing,
    WritingPullRequest,
    CreatingPullRequest,
    Pulling,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GitActionProgress {
    pub thread_id: String,
    pub phase: GitActionPhase,
}

/// What a git action did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GitActionResult {
    /// The new commit's short hash.
    pub commit: Option<String>,
    pub subject: Option<String>,
    /// The branch it pushed to.
    pub pushed: Option<String>,
    pub pulled: bool,
    /// The pull request it opened, or found already open.
    pub pull_request: Option<GitActionPullRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GitActionPullRequest {
    pub number: u32,
    pub title: String,
    pub url: String,
    /// It was open before; nothing new was created.
    pub existing: bool,
}
