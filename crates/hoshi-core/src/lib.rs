//! Domain model and orchestrator. Pure: no I/O, no async runtime, no SQLite.

mod app_release;
mod browser;
mod composer;
mod diff;
mod fonts;
mod handoff;
mod idle_compaction;
mod model;
mod orchestration;
mod project;
mod provider;
mod provider_account;
mod provider_update;
mod pull_request;
mod request;
mod server;
mod settings;
mod snooze;
mod source_control;
mod terminal;
mod theme_import;
mod thread;
mod usage;
mod workspace;

pub use app_release::{AppInstaller, AppRelease};
pub use browser::{BrowserFrame, BrowserInput, BrowserPage, BrowserPoint};
pub use composer::{EntryKind, MessageAttachment, SlashCommand, WorkspaceEntry};
pub use diff::{DiffFile, DiffFileStatus, DiffScope, FileDiff};
pub use fonts::{SystemFont, group_font_families};
pub use handoff::{
    ContextHandoff, DEFAULT_HANDOFF_TOKEN_CAP, HandoffBudget, HandoffSection, HistoricalMessage,
    HistoricalRole, MergeBackSummary, TranscriptTurn, assemble_handoff, handoff_token_cap,
    history_section, item_text, transcript_turns, turn_point,
};
pub use idle_compaction::{
    IDLE_COMPACTION_LEAD_MS, compaction_before_expiry, compaction_threshold, compacts_on_return,
    is_compact_command, worth_compacting,
};
pub use model::{DEFAULT_PROVIDER, ModelInfo, ProviderModels, default_provider};
pub use orchestration::{
    Command, CommandError, Event, EventEnvelope, HeldMessage, MessageSender, ProjectBranch,
    QueueEntry, ReadModel, Snapshot, StreamDelta, StreamKind, TaskPreview, ThreadHeldMessage,
    ThreadQueue, answer_task, answers_message, cancel_task, close_stale_turn, consume_handoffs,
    decide, drop_queued_messages, fork_without_native, may_continue_alone, record_branch,
    record_conversation_cursor, record_delegation, record_output, record_pull_request_watch,
    record_title, record_turn_changes, record_worktree, record_worktree_left, reopen_task,
    restore_session, retry_settled, send_pull_request_update, send_task_result, start_queued_turn,
    start_resumed_turn, start_woken_turn, subagent_thread_id, task_preview, turn_failed,
};
pub use project::{
    EffectiveSetting, FolderListing, Project, ProjectSettings, RemoteRepository, SettingSource,
    WorktreeSetup, WorktreeSetupRun, WorktreeSetupStatus, setup_file_path, setup_file_paths,
};
pub use provider::ProviderOutput;
pub use provider_account::{
    ChatGptPlan, CodexAccount, ProviderAccount, ProviderAccountStatus, ProviderPlan,
    ProviderSignIn, ProviderSignInFailure,
};
pub use provider_update::{
    ProviderUpdate, ProviderUpdateFailure, ProviderVersion, ProviderVersionUpdate, UpdateStep,
};
pub use pull_request::{
    ChecksState, ForgeAccount, ForgeKind, MergeMethod, Mergeability, PullRequest,
    PullRequestAction, PullRequestCheck, PullRequestDetail, PullRequestLabel, PullRequestState,
    PullRequestSupport, PullRequestUpdate, PullRequestWatch, ReviewAction, ReviewApproval,
    ReviewAsk, ReviewComment, ReviewState, ReviewThread, ReviewTurn, Reviewer, ThreadPullRequest,
    ThreadPullRequestDetail, TimelineItem,
};
pub use request::{ApprovalDecision, Question, QuestionOption, RequestResponse, RequestStatus};
pub use server::{LocalServer, ServerOrigin, ThreadServers};
pub use settings::{
    AppSettings, BrowserSettings, ClaudeCodeSettings, CodexSettings, ContextMeterPlace,
    CopilotSettings, FavoriteModel, GeneralSettings, IDLE_COMPACTION_TOKENS, IdleCompaction,
    LinkTarget, NewThreadModel, NewThreadSettings, NewThreadWorkspace, PageColorScheme, PageSize,
    ProviderSettings, SearchEngine, StartupView, SubagentEffort, ThreadTitleSettings,
};
pub use snooze::{
    CustomSnooze, MAX_SNOOZE_MINUTES, MAX_SNOOZE_PRESETS, SnoozeAmount, SnoozeChoice, SnoozeDay,
    SnoozeLabel, SnoozeLeft, SnoozePreset, SnoozeUnit, active_since, custom_snooze_until,
    default_snooze_presets, is_snoozed, pick_snooze_choice, raised_hand_while_snoozed, shows_woke,
    snooze_choice, snooze_choices, snooze_left, snooze_pending, snooze_until_reset, woke_at,
};
pub use source_control::{
    GitAction, GitActionPhase, GitActionProgress, GitActionPullRequest, GitActionRequest,
    GitActionResult, GitStatus,
};
pub use terminal::{TerminalAttach, TerminalEvent, TerminalExit};
pub use theme_import::{ExtensionTheme, ThemeExtension, ThemeExtensionSort};
pub use thread::{
    AccessMode, BackgroundTask, CompactionStatus, Effort, ForkOrigin, ForkPoint, HandoffDelivery,
    HandoffSource, HandoffStrategy, ItemContent, ModelChoice, ModelSwitch, OpenToolCall,
    PendingHandoff, ProviderSession, Resumption, Rewind, Task, TaskStatus, Thread,
    ThreadBackgroundTasks, ThreadItem, ThreadStatus, ThreadWorkspace, ToolDetail, TranscriptPage,
    TurnFailure, TurnOutcome, TurnPoint, TurnRetry, TurnStatus,
};
pub use usage::{
    BankedGrant, BankedResetFailure, BankedResetSpend, BankedResets, ContextUsage, ExtraUsage,
    ResetKind, SpendUnit, ThreadContextUsage, UsageLimits, UsageWindow, UsageWindowKind,
};
pub use workspace::{ChangesSummary, Editor};
