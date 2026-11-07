use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Thread {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub status: ThreadStatus,
    pub branch: Option<String>,
    /// When set, the agent runs here instead of the project root.
    pub worktree_path: Option<String>,
    pub created_at: String,
    /// RFC 3339 timestamp of the last activity.
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ThreadStatus {
    Idle,
    Working,
    WaitingForInput,
    Error,
}
