//! Domain model and orchestrator. Pure: no I/O, no async runtime, no SQLite.

mod app_release;
mod browser;
mod composer;
mod diff;
mod fonts;
mod model;
mod orchestration;
mod project;
mod provider;
mod provider_account;
mod provider_update;
mod pull_request;
mod request;
mod settings;
mod source_control;
mod terminal;
mod theme_import;
mod thread;
mod usage;
mod workspace;

pub use app_release::AppRelease;
pub use browser::{BrowserFrame, BrowserInput, BrowserPage, BrowserPoint};
pub use composer::{EntryKind, MessageAttachment, SlashCommand, WorkspaceEntry};
pub use diff::{DiffFile, DiffFileStatus, DiffScope, FileDiff};
pub use fonts::{SystemFont, group_font_families};
pub use model::{ModelInfo, ProviderModels};
pub use orchestration::{
    Command, CommandError, Event, EventEnvelope, ReadModel, Snapshot, StreamDelta, StreamKind,
    answer_task, answers_message, cancel_task, close_stale_turn, decide, record_branch,
    record_conversation_cursor, record_delegation, record_output, record_pull_request_watch,
    record_title, record_turn_changes, record_worktree, record_worktree_left,
    send_pull_request_update, send_task_result, start_queued_turn, start_woken_turn,
    subagent_thread_id, turn_failed,
};
pub use project::{
    FolderListing, Project, RemoteRepository, WorktreeSetup, WorktreeSetupRun, WorktreeSetupStatus,
};
pub use provider::ProviderOutput;
pub use provider_account::ProviderAccountStatus;
pub use provider_update::{ProviderUpdate, ProviderUpdateFailure, ProviderVersion};
pub use pull_request::{
    ChecksState, ForgeAccount, ForgeKind, MergeMethod, Mergeability, PullRequest,
    PullRequestAction, PullRequestCheck, PullRequestDetail, PullRequestLabel, PullRequestState,
    PullRequestSupport, PullRequestUpdate, PullRequestWatch, ReviewComment, ReviewState,
    ReviewThread, Reviewer, ThreadPullRequest, ThreadPullRequestDetail, TimelineItem,
};
pub use request::{ApprovalDecision, Question, QuestionOption, RequestResponse, RequestStatus};
pub use settings::{
    AppSettings, ClaudeCodeSettings, FavoriteModel, GeneralSettings, ProviderSettings, StartupView,
    ThreadTitleSettings,
};
pub use source_control::{
    GitAction, GitActionPhase, GitActionProgress, GitActionPullRequest, GitActionRequest,
    GitActionResult, GitStatus,
};
pub use terminal::{TerminalAttach, TerminalEvent, TerminalExit};
pub use theme_import::{ExtensionTheme, ThemeExtension, ThemeExtensionSort};
pub use thread::{
    AccessMode, BackgroundTask, CompactionStatus, Effort, ItemContent, OpenToolCall, Rewind, Task,
    TaskStatus, Thread, ThreadBackgroundTasks, ThreadItem, ThreadStatus, ThreadWorkspace,
    ToolDetail, TranscriptPage, TurnFailure, TurnOutcome,
};
pub use usage::{ContextUsage, ThreadContextUsage, UsageLimits, UsageWindow, UsageWindowKind};
pub use workspace::{ChangesSummary, Editor};
