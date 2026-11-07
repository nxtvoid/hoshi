//! Domain model and orchestrator. Pure: no I/O, no async runtime, no SQLite.

mod project;
mod thread;

pub use project::Project;
pub use thread::{Thread, ThreadStatus};
