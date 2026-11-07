//! Domain model and orchestrator. Pure: no I/O, no async runtime, no SQLite.

mod orchestration;
mod project;
mod settings;
mod thread;

pub use orchestration::{Command, CommandError, Event, EventEnvelope, ReadModel, Snapshot, decide};
pub use project::Project;
pub use settings::{AppSettings, ClaudeCodeSettings, ProviderSettings};
pub use thread::{Thread, ThreadStatus};
