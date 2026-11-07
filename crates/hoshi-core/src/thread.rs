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
    /// The provider's own session, resumed on the next turn.
    pub session_id: Option<String>,
    pub active_turn_id: Option<String>,
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

/// One entry in a thread's transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThreadItem {
    pub id: String,
    pub thread_id: String,
    pub created_at: String,
    pub content: ItemContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ItemContent {
    UserMessage {
        text: String,
    },
    AssistantMessage {
        text: String,
    },
    ToolCall {
        name: String,
        /// The tool input as JSON, truncated for display.
        input: String,
        output: Option<String>,
        is_error: bool,
    },
    TurnFailed {
        reason: TurnFailure,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum TurnOutcome {
    Completed,
    Interrupted,
    Failed { reason: TurnFailure },
}

/// Why a turn failed. Codes for the UI; `detail` is for logs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "code",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum TurnFailure {
    ProviderDisabled,
    SpawnFailed { detail: String },
    ProcessExited { detail: String },
    ProviderError { detail: String },
}
