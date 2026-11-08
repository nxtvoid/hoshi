use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What a thread's diff compares its working tree against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DiffScope {
    /// Changes not committed yet: `HEAD` against the working tree.
    Uncommitted,
    /// Everything the branch changed since it left the default branch, committed or not.
    Branch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DiffFileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    /// New and not yet known to git.
    Untracked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiffFile {
    pub path: String,
    /// The path before a rename.
    pub old_path: Option<String>,
    pub status: DiffFileStatus,
    /// `None` for binary files.
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

/// One file's unified diff, cut short when it is too long to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileDiff {
    pub text: String,
    pub truncated: bool,
}
