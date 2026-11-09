use crate::{ApprovalDecision, DiffFile, MessageAttachment, Question, RequestStatus};
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
    /// Its worktree existed before the thread moved into it, as one the user made; hoshi
    /// never removes it.
    #[serde(default)]
    pub worktree_adopted: bool,
    /// The thread whose agent delegated this one's task to it; `None` for one the user started.
    #[serde(default)]
    pub parent_thread_id: Option<String>,
    /// As a delegated task, it was cancelled: its answer never goes back.
    #[serde(default)]
    pub task_cancelled: bool,
    /// As a delegated task, its answer went back to the agent that delegated it.
    #[serde(default)]
    pub task_answered: bool,
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
    /// Asks before anything the agent's settings don't allow, file edits included.
    Ask,
    /// File edits apply on their own; anything else the agent's settings don't
    /// allow is asked first.
    #[default]
    AcceptEdits,
    /// The provider judges what is safe to run.
    Auto,
    /// Runs everything, except what the agent's own rules and safety checks ask about.
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

/// Part of a thread's transcript, latest turns first loaded, older ones on request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TranscriptPage {
    /// Oldest first, each as `ItemContent::summary` leaves it.
    pub items: Vec<ThreadItem>,
    /// Items before these remain.
    pub has_older: bool,
}

/// A tool call still waiting for its output.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OpenToolCall {
    /// The thread's transcript, or one of its subagents'.
    pub thread_id: String,
    pub item_id: String,
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
        /// Sent while the agent worked, to steer the running turn.
        #[serde(default)]
        steer: bool,
        /// The answer of a task this thread delegated, from that child thread; sent by hoshi,
        /// not the user.
        #[serde(default)]
        task_result: Option<String>,
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
        /// What the call did, beyond its output, once it completed.
        #[serde(default)]
        detail: Option<ToolDetail>,
        /// `output` is only its start: a page of the transcript leaves the rest out.
        #[serde(default)]
        output_omitted: bool,
        /// Its turn ended before it did, so it has no output.
        #[serde(default)]
        interrupted: bool,
    },
    TurnFailed {
        reason: TurnFailure,
    },
    /// The turn stopped before it finished, as the user asked or as the app closed.
    TurnInterrupted,
    /// The agent answered one run of a turn that went on, as each background task that
    /// ended gets a run of its own.
    RunEnded,
    /// A task that ran in the background ended; the agent's answer to it follows. The call
    /// that started it, when one did, is `tool_call_id`.
    BackgroundTaskEnded {
        tool_call_id: Option<String>,
        failed: bool,
    },
    /// The files a turn changed in its working tree, read from its checkpoints once it
    /// ended; `None` while they are still being read.
    TurnChanges {
        files: Option<Vec<DiffFile>>,
    },
    /// A tool call held back until the user approves it.
    Approval {
        /// The call it held back, in the thread's transcript or a subagent's; `None` in
        /// approvals recorded before calls were kept.
        tool_call_id: Option<String>,
        tool_name: String,
        /// The tool input as JSON, truncated for display.
        input: String,
        /// Why the agent asks, in its own words.
        reason: Option<String>,
        can_accept_for_session: bool,
        status: RequestStatus,
        decision: Option<ApprovalDecision>,
        feedback: Option<String>,
    },
    Questions {
        questions: Vec<Question>,
        status: RequestStatus,
        answers: Option<Vec<String>>,
    },
    /// The agent's task list as it stands, one per turn, kept up to date in place.
    TaskList {
        tasks: Vec<Task>,
    },
    /// A plan mode turn's plan, which the agent carries out once accepted.
    ProposedPlan {
        /// Markdown.
        plan: String,
        status: RequestStatus,
        decision: Option<ApprovalDecision>,
        feedback: Option<String>,
    },
}

/// How much of a tool's output a page of the transcript keeps; the item alone has it all.
const OUTPUT_PREVIEW_CHARS: usize = 200;

impl ItemContent {
    /// The item as a page of the transcript carries it: tool output cut to its start and
    /// diffs left out, which the item alone returns.
    pub fn summary(self) -> Self {
        match self {
            Self::ToolCall {
                name,
                input,
                output,
                is_error,
                detail,
                output_omitted,
                interrupted,
            } => {
                let long = output
                    .as_ref()
                    .is_some_and(|text| text.chars().nth(OUTPUT_PREVIEW_CHARS).is_some());
                Self::ToolCall {
                    name,
                    input,
                    output: match output {
                        Some(text) if long => {
                            Some(text.chars().take(OUTPUT_PREVIEW_CHARS).collect())
                        }
                        other => other,
                    },
                    is_error,
                    detail: detail.as_ref().map(ToolDetail::summary),
                    output_omitted: output_omitted || long,
                    interrupted,
                }
            }
            other => other,
        }
    }

    /// Set on the requests the agent makes mid-turn.
    pub fn request_status(&self) -> Option<RequestStatus> {
        match self {
            Self::Approval { status, .. }
            | Self::Questions { status, .. }
            | Self::ProposedPlan { status, .. } => Some(*status),
            _ => None,
        }
    }
}

/// One step of the agent's task list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Task {
    pub subject: String,
    /// How the agent words it while working on it, such as "Writing a.txt".
    pub active_form: Option<String>,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
}

/// What a tool call did, as the provider reported it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ToolDetail {
    FileChange {
        path: String,
        additions: u32,
        deletions: u32,
        /// The change as unified diff hunks, capped in size. `None` in a whole
        /// transcript, which leaves diffs out; the item alone carries it.
        diff: Option<String>,
    },
}

impl ToolDetail {
    /// The detail without what a whole transcript leaves out.
    pub fn summary(&self) -> Self {
        match self {
            Self::FileChange {
                path,
                additions,
                deletions,
                ..
            } => Self::FileChange {
                path: path.clone(),
                additions: *additions,
                deletions: *deletions,
                diff: None,
            },
        }
    }
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
