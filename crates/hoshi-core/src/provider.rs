use crate::TurnOutcome;

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
    TurnCompleted {
        outcome: TurnOutcome,
    },
}
