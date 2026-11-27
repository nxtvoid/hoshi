use crate::{
    ApprovalDecision, DiffFile, MessageAttachment, PullRequestUpdate, Question, RequestStatus,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    /// The provider's id for the last message of the conversation, a point a rewind can
    /// come back to.
    pub conversation_cursor: Option<String>,
    /// Set by a rewind: the next turn resumes the session from this message, leaving out
    /// what came after it.
    pub resume_at: Option<String>,
    /// Its agent asked to be woken on its pull request's news.
    pub watching_pull_request: bool,
    /// The agent CLI it runs on. Once it worked, another one the user picks waits in
    /// `pending_model` for the next message.
    #[serde(default = "crate::default_provider")]
    pub provider: String,
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
    /// RFC 3339: out of its section until then, unless it needs the user sooner. Kept once
    /// passed, until something wakes it; see [`crate::is_snoozed`].
    #[serde(default)]
    pub snoozed_until: Option<String>,
    /// When it was last snoozed; only what happens after it wakes the thread early.
    #[serde(default)]
    pub snoozed_at: Option<String>,
    /// When the user last woke it from a snooze, by hand or by pinning it.
    #[serde(default)]
    pub last_snooze_wake_at: Option<String>,
    /// When the user last opened it while it had something they hadn't seen: a turn that
    /// ended since their last visit, or a wake from a snooze.
    #[serde(default)]
    pub last_visited_at: Option<String>,
    /// When its latest turn ended, however it ended.
    #[serde(default)]
    pub last_turn_ended_at: Option<String>,
    /// The thread it was forked from, and from which of its turns.
    #[serde(default)]
    pub forked_from: Option<Box<ForkOrigin>>,
    /// Another provider the user picked, which the next message moves the thread to.
    #[serde(default)]
    pub pending_model: Option<Box<ModelChoice>>,
    /// What its last turn ran on, which the next tells a change of model or effort against.
    #[serde(default)]
    pub last_turn_model: Option<Box<ModelChoice>>,
    /// The sessions of the providers it ran on before, resumed on a switch back.
    #[serde(default)]
    pub provider_sessions: Vec<ProviderSession>,
    /// What its agent has yet to be told at the start of its next turn, oldest first.
    #[serde(default)]
    pub pending_handoffs: Vec<PendingHandoff>,
    pub created_at: String,
    /// RFC 3339 timestamp of the last activity.
    pub updated_at: String,
}

impl Thread {
    /// Its latest turn ended after the user last opened it.
    pub fn has_unseen_turn(&self) -> bool {
        let instant = |at: &Option<String>| {
            at.as_deref()
                .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        };
        let Some(ended) = instant(&self.last_turn_ended_at) else {
            return false;
        };
        instant(&self.last_visited_at).is_none_or(|visited| ended > visited)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreadStatus {
    Idle,
    Working,
    WaitingForInput,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptPage {
    /// Oldest first, each as `ItemContent::summary` leaves it.
    pub items: Vec<ThreadItem>,
    /// Items before these remain.
    pub has_older: bool,
}

/// A tool call still waiting for its output.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenToolCall {
    /// The thread's transcript, or one of its subagents'.
    pub thread_id: String,
    pub item_id: String,
}

/// A command the agent left running in the background, which its turn waits for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundTask {
    pub task_id: String,
    /// The call that started it, when one did.
    pub tool_call_id: Option<String>,
    /// The agent's own words for what it runs.
    pub description: String,
    /// Unix milliseconds.
    pub started_at: u64,
}

/// The commands a thread's running turn waits for; empty once none is left.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadBackgroundTasks {
    pub thread_id: String,
    pub tasks: Vec<BackgroundTask>,
    /// The agent works on nothing else: the turn only waits for these.
    pub waiting: bool,
}

/// One entry in a thread's transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadItem {
    pub id: String,
    pub thread_id: String,
    pub created_at: String,
    pub content: ItemContent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompactionStatus {
    Running,
    Done,
    Failed,
}

/// The point of the conversation a user message started from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Rewind {
    /// The first message: going back starts a new session.
    Start,
    /// After the provider's message `cursor` of its session `session`, when recorded.
    After {
        cursor: String,
        #[serde(default)]
        session: Option<String>,
    },
}

/// A change of model or effort between two turns of one provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSwitch {
    pub from: ModelChoice,
    pub to: ModelChoice,
}

/// A provider and model a thread runs on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelChoice {
    pub provider: String,
    /// `None` uses the provider's default.
    pub model: Option<String>,
    pub effort: Option<Effort>,
    pub fast_mode: bool,
    pub ultracode: bool,
}

impl ModelChoice {
    pub fn of(thread: &Thread) -> Self {
        Self {
            provider: thread.provider.clone(),
            model: thread.model.clone(),
            effort: thread.effort,
            fast_mode: thread.fast_mode,
            ultracode: thread.ultracode,
        }
    }
}

/// The session a thread keeps with a provider it no longer runs on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSession {
    pub provider: String,
    pub session_id: String,
    pub conversation_cursor: Option<String>,
    pub resume_at: Option<String>,
    /// The message the thread moved away with: the provider saw everything before it.
    pub seen_before: String,
}

/// How a turn of a thread ended, as its transcript tells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TurnStatus {
    Running,
    Completed,
    Failed,
    Interrupted,
}

impl TurnStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }
}

/// A turn a fork or a merge-back starts from, read by the engine from the transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnPoint {
    /// The message that started it.
    pub turn_id: String,
    /// Its last item.
    pub through_item_id: String,
    /// Its place among the thread's turns, from 1.
    pub ordinal: u32,
    pub status: TurnStatus,
    /// The provider that ran it.
    pub provider: String,
    /// That provider's session, and its last message of the conversation once the turn
    /// ended: where a native fork resumes. `None` when not recorded.
    pub session_id: Option<String>,
    pub cursor: Option<String>,
}

/// Where a fork or a merge-back starts from, as the user asks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ForkPoint {
    /// The latest turn that completed.
    LatestStable,
    /// The turn that holds item `item_id`, such as an answer of the agent.
    Turn { item_id: String },
}

/// Where a fork came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkOrigin {
    pub thread_id: String,
    /// The source's turn it goes on from: it inherits the source's items through
    /// `turn.through_item_id`.
    pub turn: TurnPoint,
}

/// How a context handoff covers what the agent did not see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HandoffStrategy {
    /// The whole conversation, to an agent that saw none of it.
    FullThreadSummary,
    /// The turns since the agent last saw the thread.
    DeltaSinceTargetLastSeen,
    /// A fork's turns, merged back into the thread it came from.
    ForkDeltaSummary,
}

impl HandoffStrategy {
    /// As the handoff names it to the agent.
    pub fn label(self) -> &'static str {
        match self {
            Self::FullThreadSummary => "full_thread_summary",
            Self::DeltaSinceTargetLastSeen => "delta_since_target_last_seen",
            Self::ForkDeltaSummary => "merge_back / fork_delta_summary",
        }
    }
}

/// Context a thread's next turn hands its agent first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PendingHandoff {
    /// The thread moved from `from` to `to`: the items from `since` (the whole thread when
    /// `None`) up to message `before` are what `to` never saw.
    ProviderSwitch {
        from: ModelChoice,
        to: ModelChoice,
        strategy: HandoffStrategy,
        since: Option<String>,
        before: String,
    },
    /// A fork's first turn takes the source's conversation through the fork point: by forking
    /// the source's session when `native` and its provider can, otherwise as a transcript.
    Fork { native: bool },
    /// A fork merged back through its turn `turn`.
    MergeBack {
        fork_thread_id: String,
        turn: TurnPoint,
    },
}

/// Where a context handoff's material came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HandoffSource {
    ProviderSwitch {
        from: ModelChoice,
        to: ModelChoice,
        strategy: HandoffStrategy,
    },
    /// The history of the thread a fork came from.
    Fork {
        thread_id: String,
    },
    MergeBack {
        fork_thread_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HandoffDelivery {
    /// Prepared; the provider has yet to take it.
    Pending,
    /// Written ahead of the user's message.
    Inline,
    /// Added to the provider's conversation as items of their own.
    Injected,
    /// Its turn ended before the provider took it, or it did not fit; the next turn tries
    /// again.
    Failed,
}

/// Why hoshi went on with a thread by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Resumption {
    /// A restart cut the turn short; the agent goes on where it left off.
    Restart,
    /// A restart ended background work the agent ran; it is told so.
    RestartEndedWork,
    /// The usage limit that failed the turn reset.
    UsageLimitReset,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
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
        /// News on a pull request the thread watches, sent by hoshi, not the user.
        #[serde(default)]
        pull_request_update: Option<PullRequestUpdate>,
        /// Sent by hoshi to go on with the thread by itself, as after a restart.
        #[serde(default)]
        resumed: Option<Resumption>,
        /// Where the conversation goes back to when the user edits from this message; `None`
        /// for a message that can't be edited from, such as a steer.
        #[serde(default)]
        rewind: Option<Rewind>,
        /// Sent to a running turn that ended before its agent read it.
        #[serde(default)]
        dropped: bool,
        /// The turn it started runs on another model or effort of the same provider than the
        /// turn before.
        #[serde(default)]
        model_switch: Option<Box<ModelSwitch>>,
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
    /// The app closed or crashed while the turn ran, which cut it short.
    TurnCut,
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
    /// The conversation compacted into a summary, to free its context window.
    Compaction {
        status: CompactionStatus,
        /// The context's size before and after, once done.
        before_tokens: Option<u64>,
        after_tokens: Option<u64>,
        /// Why it failed, in the provider's words.
        error: Option<String>,
    },
    /// The provider ran the turn on `model` rather than the one asked, for `reason`.
    ModelRerouted {
        model: String,
        reason: Option<String>,
    },
    /// The provider got through to the model after retrying, `attempt` times of
    /// `max_attempts` when it said how many it makes.
    ProviderRecovered {
        attempt: u32,
        max_attempts: Option<u32>,
    },
    /// An MCP server the agent was given failed to start; `error` is the provider's own.
    McpServerFailed {
        name: String,
        error: String,
    },
    /// A hook the user configured for `event` stopped the turn, saying `reason`; `source` is
    /// where the hook comes from.
    HookStopped {
        event: String,
        source: Option<String>,
        reason: String,
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
    /// A secret the agent asked the user for, such as a token. Its value is never recorded:
    /// it goes to a file only the agent's commands read.
    SecretRequest {
        /// What it is, such as "GitHub token".
        label: String,
        /// What it is for and where to get it, in the agent's words.
        reason: String,
        placeholder: Option<String>,
        status: RequestStatus,
        /// Once resolved: whether the user saved one or declined.
        saved: Option<bool>,
    },
    /// Context handed to the agent at the start of its turn, from `sources`: `included_items`
    /// items whole, `omitted_items` left out to fit.
    ContextHandoff {
        sources: Vec<HandoffSource>,
        delivery: HandoffDelivery,
        included_items: u32,
        omitted_items: u32,
    },
    /// Where a turn that runs on another model or effort of the same provider than the turn
    /// before starts, for a message that waited for its turn.
    ModelSwitched {
        from: ModelChoice,
        to: ModelChoice,
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
            | Self::ProposedPlan { status, .. }
            | Self::SecretRequest { status, .. } => Some(*status),
            _ => None,
        }
    }
}

/// The provider failed to reach the model and tries again; holds until the run's next output
/// or its end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnRetry {
    pub attempt: u32,
    pub max_attempts: Option<u32>,
    /// How long after `at` the next attempt goes.
    pub retry_in_ms: Option<u64>,
    /// What failed, in the provider's words.
    pub error: Option<String>,
    /// RFC 3339 timestamp of when the provider said so.
    pub at: String,
}

/// One step of the agent's task list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub subject: String,
    /// How the agent words it while working on it, such as "Writing a.txt".
    pub active_form: Option<String>,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
}

/// What a tool call did, as the provider reported it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ToolDetail {
    FileChange {
        path: String,
        additions: u32,
        deletions: u32,
        /// The change as unified diff hunks, capped in size. `None` in a whole
        /// transcript, which leaves diffs out; the item alone carries it.
        diff: Option<String>,
    },
    /// A call that started a subagent: the model it runs on and the effort it runs at, as
    /// far as known. `effort` is `None` for a model without effort levels. `resume_id` is
    /// the provider's id for the subagent, by which its agent can go on with it, once known.
    Subagent {
        model: String,
        effort: Option<Effort>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resume_id: Option<String>,
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
            Self::Subagent { .. } => self.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TurnOutcome {
    Completed,
    Interrupted,
    Failed { reason: TurnFailure },
}

/// Why a turn failed. Codes for the UI; `detail` is for logs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "code",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TurnFailure {
    ProviderDisabled,
    SpawnFailed {
        detail: String,
    },
    ProcessExited {
        detail: String,
    },
    ProviderError {
        detail: String,
    },
    /// The plan's usage limit was reached; `resets_at` (ISO 8601) is when it resets, when
    /// the provider told.
    UsageLimited {
        detail: String,
        resets_at: Option<String>,
    },
    /// The provider is overloaded or throttled the request: a later try may work.
    Overloaded {
        detail: String,
    },
    /// The provider refused the account's credentials: the user has to sign in again.
    AuthenticationFailed {
        detail: String,
    },
    /// The conversation no longer fits the model's context.
    ContextFull {
        detail: String,
    },
    /// The agent did not finish starting in time.
    StartupTimedOut {
        detail: String,
    },
    /// The provider could not reopen the thread's conversation; no new one was started in
    /// its place.
    ResumeFailed {
        detail: String,
    },
    /// The turn asks for what the agent's process can't change, and starting another would
    /// end the background work it runs.
    BackgroundWorkRunning,
    /// The context the agent had to be handed first leaves too little room for the message;
    /// nothing was cut.
    ContextHandoffTooLarge,
}
