use crate::{ContextUsage, TurnOutcome, UsageLimits};

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
    /// The model's reasoning before it answers or calls a tool, as the provider shows it.
    Reasoning {
        item_id: String,
        text: String,
    },
    AssistantMessage {
        item_id: String,
        text: String,
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
    },
    /// After each model call of the thread's own agent, not its subagents.
    ContextUsage(ContextUsage),
    /// Plan limits the provider reported mid-turn; may cover only some windows.
    UsageLimits(UsageLimits),
    TurnCompleted {
        outcome: TurnOutcome,
    },
}
