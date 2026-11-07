//! Provider adapters. Each one drives an agent CLI (Claude Code first) and
//! translates its native protocol into provider-neutral `ProviderOutput`.

mod claude;
mod process;

pub use claude::{ClaudeCodeAdapter, ClaudeParser};
use hoshi_core::{Effort, ProviderOutput, TurnFailure};

pub struct TurnRequest {
    /// The thread's working directory: its worktree, or the project root.
    pub cwd: String,
    pub prompt: String,
    pub resume_session_id: Option<String>,
    /// Overrides the adapter's default model.
    pub model: Option<String>,
    pub effort: Option<Effort>,
}

/// Receives everything a turn reports, ending with `ProviderOutput::TurnCompleted`.
pub type OutputSink = Box<dyn FnMut(ProviderOutput) + Send>;

pub trait ProviderAdapter: Send + Sync {
    fn start_turn(
        &self,
        request: TurnRequest,
        sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure>;
}

pub trait RunningTurn: Send {
    /// Stops the turn. The sink still receives a final `TurnCompleted`.
    fn interrupt(&self);
}

/// Truncates long tool inputs and outputs so a transcript stays light.
pub(crate) fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => format!("{}…", &text[..index]),
        None => text.to_owned(),
    }
}
