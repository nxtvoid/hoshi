use crate::MessageAttachment;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Thread {
    pub id: String,
    /// `None` for a chat, which belongs to no project and runs in a folder of its own.
    pub project_id: Option<String>,
    pub title: String,
    pub status: ThreadStatus,
    pub branch: Option<String>,
    /// When set, the agent runs here instead of the project root.
    pub worktree_path: Option<String>,
    /// The provider's own session, resumed on the next turn.
    pub session_id: Option<String>,
    /// Provider model alias; `None` uses the provider's default.
    pub model: Option<String>,
    pub effort: Option<Effort>,
    /// Faster output from models that support it, billed at a higher rate.
    pub fast_mode: bool,
    /// Lets the agent plan and run multi-agent workflows.
    pub ultracode: bool,
    pub access_mode: AccessMode,
    /// The agent only reads and proposes a plan; nothing is changed.
    pub plan_mode: bool,
    pub active_turn_id: Option<String>,
    /// Put away by the user; a new message brings it back.
    pub archived: bool,
    /// Kept at the top of the list by the user.
    pub pinned: bool,
    /// Its worktree folder was removed to free the disk; the next message recreates it at
    /// the same path from the branch, which stays.
    #[serde(default)]
    pub worktree_released: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }
}

/// What the agent may do without asking.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AccessMode {
    /// File edits apply on their own; anything else the agent's settings don't
    /// allow is refused.
    #[default]
    AcceptEdits,
    /// The provider judges what is safe to run.
    Auto,
    FullAccess,
}

/// Where a new thread works. The engine turns `NewWorktree` and `BranchWorktree` into
/// `Worktree` after creating it, so a thread never exists without its worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ThreadWorkspace {
    Local,
    NewWorktree,
    /// A new worktree on an existing branch, such as a pull request's head: the local one
    /// when there is one, otherwise origin's, fetched and tracked.
    BranchWorktree {
        branch: String,
    },
    Worktree {
        path: String,
        branch: String,
    },
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
        #[serde(default)]
        attachments: Vec<MessageAttachment>,
    },
    AssistantMessage {
        text: String,
    },
    Reasoning {
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
