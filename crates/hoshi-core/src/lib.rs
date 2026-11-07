//! Domain model and orchestrator. Pure: no I/O, no async runtime, no SQLite.

mod orchestration;
mod project;
mod provider;
mod settings;
mod thread;

pub use orchestration::{
    Command, CommandError, Event, EventEnvelope, ReadModel, Snapshot, StreamDelta, decide,
    record_branch, record_output, turn_failed,
};
pub use project::Project;
pub use provider::ProviderOutput;
pub use settings::{AppSettings, ClaudeCodeSettings, ProviderSettings};
pub use thread::{
    Effort, ItemContent, Thread, ThreadItem, ThreadStatus, ThreadWorkspace, TurnFailure,
    TurnOutcome,
};
