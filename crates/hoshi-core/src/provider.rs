use crate::{
    BackgroundTask, ContextUsage, ItemContent, Task, ToolDetail, TurnOutcome, UsageLimits,
};

/// What a provider adapter reports while it runs a turn, in provider-neutral terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderOutput {
    SessionStarted {
        session_id: String,
    },
    /// Streaming text for an assistant message that is still being written.
    TextDelta {
        item_id: String,
        delta: String,
    },
    /// Streaming text of the model's reasoning that is still being written.
    ReasoningDelta {
        item_id: String,
        delta: String,
    },
    /// A tool's output so far while it runs, replacing what was reported before. Its final
    /// output comes with `ToolCallCompleted`.
    ToolOutput {
        item_id: String,
        output: String,
    },
    /// The model's reasoning before it answers or calls a tool, as the provider shows it.
    Reasoning {
        item_id: String,
        text: String,
    },
    AssistantMessage {
        item_id: String,
        text: String,
    },
    /// Where the agent's session works: reported as a turn starts, and again whenever the
    /// agent moves, such as into a worktree.
    WorkingDirectory {
        path: String,
    },
    ToolCallStarted {
        item_id: String,
        name: String,
        input: String,
    },
    ToolCallCompleted {
        item_id: String,
        output: String,
        is_error: bool,
        detail: Option<ToolDetail>,
    },
    /// What a subagent did, which the agent's `tool_use_id` call started. Only its messages,
    /// reasoning and tool calls are kept.
    Subagent {
        tool_use_id: String,
        output: Box<ProviderOutput>,
    },
    /// The agent's whole task list, after it changed.
    TaskList {
        tasks: Vec<Task>,
    },
    /// The agent waits for the user: `content` is an approval, questions or a proposed plan,
    /// pending.
    RequestOpened {
        request_id: String,
        content: ItemContent,
    },
    /// The agent stopped waiting for an answer to the request.
    RequestWithdrawn {
        request_id: String,
    },
    /// The user stopped the call before it finished, as a subagent stopped on its own.
    /// Follows its `ToolCallCompleted`.
    ToolCallStopped {
        item_id: String,
    },
    /// After each model call of the thread's own agent, not its subagents.
    ContextUsage(ContextUsage),
    /// Plan limits the provider reported mid-turn; may cover only some windows.
    UsageLimits(UsageLimits),
    /// A task that ran in the background ended, and the agent's next run answers it.
    /// `tool_use_id` is the call that started it, when one did.
    BackgroundTaskEnded {
        task_id: String,
        tool_use_id: Option<String>,
        failed: bool,
    },
    /// The commands the agent left running in the background, whenever they change or a run
    /// starts or ends beside them. The turn waits only for those the agent said it awaits;
    /// the others outlive it. `waiting` when no run goes on: the turn only waits.
    BackgroundTasks {
        tasks: Vec<BackgroundTask>,
        waiting: bool,
    },
    /// The agent answered, but its turn goes on: a background task that ended or a message
    /// it took in starts another run. `run` counts the turn's runs from 1.
    RunEnded {
        run: u32,
    },
    /// The agent began compacting its conversation into a summary, as `/compact` asks.
    CompactionStarted,
    /// The conversation was compacted: its context went from `before_tokens` to
    /// `after_tokens`.
    CompactionFinished {
        before_tokens: u64,
        after_tokens: u64,
    },
    /// Compacting failed; `error` is the provider's own message.
    CompactionFailed {
        error: String,
    },
    /// The provider's id for its latest message of the conversation, where a rewind can
    /// later resume it.
    ConversationCursor {
        cursor: String,
    },
    TurnCompleted {
        outcome: TurnOutcome,
    },
    /// After its turn completed, the agent went on by itself, as a command it left running in
    /// the background ended: what follows, up to the next `TurnCompleted`, is a turn of its
    /// own.
    Woke,
    /// The process a completed turn left running for its background commands ended; nothing
    /// more comes from it.
    ProcessEnded,
}
