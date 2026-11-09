//! Domain model and orchestrator. Pure: no I/O, no async runtime, no SQLite.

mod composer;
mod diff;
mod model;
mod orchestration;
mod project;
mod provider;
mod provider_update;
mod pull_request;
mod request;
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
    answer_task, answers_message, cancel_task, close_stale_turn, decide, record_branch,
    record_delegation, record_output, record_title, record_turn_changes, record_worktree,
    record_worktree_left, send_task_result, start_queued_turn, subagent_thread_id, turn_failed,
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
pub use request::{ApprovalDecision, Question, QuestionOption, RequestResponse, RequestStatus};
pub use settings::{
    AppSettings, ClaudeCodeSettings, FavoriteModel, GeneralSettings, ProviderSettings, StartupView,
    ThreadTitleSettings,
};
pub use terminal::{TerminalAttach, TerminalEvent, TerminalExit};
pub use thread::{
    AccessMode, Effort, ItemContent, OpenToolCall, Task, TaskStatus, Thread, ThreadItem,
    ThreadStatus, ThreadWorkspace, ToolDetail, TranscriptPage, TurnFailure, TurnOutcome,
};
pub use usage::{ContextUsage, ThreadContextUsage, UsageLimits, UsageWindow, UsageWindowKind};
pub use workspace::{ChangesSummary, Editor};
