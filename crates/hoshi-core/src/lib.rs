//! Domain model and orchestrator. Pure: no I/O, no async runtime, no SQLite.

mod composer;
mod diff;
mod model;
mod orchestration;
mod project;
mod provider;
mod provider_update;
mod pull_request;
mod settings;
mod terminal;
mod thread;
mod usage;
mod workspace;

pub use composer::{EntryKind, MessageAttachment, SlashCommand, WorkspaceEntry};
pub use diff::{DiffFile, DiffFileStatus, DiffScope, FileDiff};
pub use model::{ModelInfo, ProviderModels};
pub use orchestration::{
    Command, CommandError, Event, EventEnvelope, ReadModel, Snapshot, StreamDelta, StreamKind,
    decide, record_branch, record_output, record_title, record_worktree, record_worktree_left,
    turn_failed,
};
pub use project::{FolderListing, Project, RemoteRepository};
pub use provider::ProviderOutput;
pub use provider_update::{ProviderUpdate, ProviderUpdateFailure, ProviderVersion};
pub use pull_request::{
    ChecksState, ForgeAccount, ForgeKind, MergeMethod, Mergeability, PullRequest,
    PullRequestAction, PullRequestCheck, PullRequestDetail, PullRequestLabel, PullRequestState,
    PullRequestSupport, ReviewComment, ReviewState, ReviewThread, Reviewer, ThreadPullRequest,
    ThreadPullRequestDetail, TimelineItem,
};
pub use settings::{
    AppSettings, ClaudeCodeSettings, FavoriteModel, GeneralSettings, ProviderSettings, StartupView,
    ThreadTitleSettings,
};
pub use terminal::{TerminalAttach, TerminalEvent, TerminalExit};
pub use thread::{
    AccessMode, Effort, ItemContent, Thread, ThreadItem, ThreadStatus, ThreadWorkspace,
    TurnFailure, TurnOutcome,
};
pub use usage::{ContextUsage, ThreadContextUsage, UsageLimits, UsageWindow, UsageWindowKind};
pub use workspace::{ChangesSummary, Editor};
