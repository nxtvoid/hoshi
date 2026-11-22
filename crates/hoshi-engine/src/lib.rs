//! Runtime loop: takes commands, lets the core decide events, persists them
//! through the store, notifies the UI, then runs the resulting effects
//! (provider turns, git). Provider output comes back through the same path.

mod attachments;
mod background;
mod browser;
mod checkpoint;
mod delegation;
mod diff;
mod document_text;
mod editors;
mod folders;
mod git;
mod git_actions;
mod handoffs;
mod html_pages;
mod mcp;
mod merged_worktrees;
use delegation::Delegation;
use mcp::{McpServer, ToolFilter, ToolHandler};
mod paths;
mod project_icon;
mod provider_cache;
mod pull_request_cache;
mod pull_request_tool;
mod pull_request_watch;
mod pull_requests;
mod recent_folders;
mod recovery;
mod refreshing;
mod secret_requests;
mod secrets;
mod servers;
mod setup_files;
mod source_control;
mod stream_batch;
mod terminal;
mod thread_tools;
mod throttle;
mod titles;
mod watch;
mod worktree;
mod worktree_setup;

use attachments::Attachments;
/// The name hoshi's own MCP server goes by in the agents' configs.
pub use attachments::MAX_FILE_BYTES;
use background::Lingering;
use browser::tools::Browsing;
use checkpoint::{Checkpointer, Moment};
use chrono::{SecondsFormat, Utc};
use hoshi_core::{
    BankedResetFailure, BankedResetSpend, HeldMessage, ModelChoice, ThreadHeldMessage,
};
use hoshi_core::{
    BrowserFrame, BrowserInput, BrowserPage, BrowserSettings, GitActionProgress, ProviderUpdate,
    ProviderUpdateFailure, ProviderVersion, ProviderVersionUpdate, TranscriptPage,
};
use hoshi_core::{
    ChangesSummary, Command, CommandError, ContextUsage, DiffFile, DiffScope, Editor, Event,
    EventEnvelope, FileDiff, FolderListing, ForgeAccount, ForgeKind, ItemContent,
    MessageAttachment, ProviderModels, ProviderOutput, PullRequest, PullRequestAction,
    PullRequestDetail, PullRequestLabel, ReadModel, RemoteRepository, SlashCommand, Snapshot,
    StreamDelta, StreamKind, SubagentEffort, TerminalAttach, ThreadBackgroundTasks,
    ThreadContextUsage, ThreadItem, ThreadPullRequest, ThreadPullRequestDetail, ThreadStatus,
    ThreadTitleSettings, ThreadWorkspace, TurnFailure, TurnOutcome, UsageLimits, WorkspaceEntry,
    answers_message, close_stale_turn, decide, record_branch, record_conversation_cursor,
    record_output, record_title, record_turn_changes, record_worktree, record_worktree_left,
    retry_settled, start_queued_turn, turn_failed,
};
use hoshi_core::{
    CodexAccount, DEFAULT_PROVIDER, ProviderAccountStatus, ProviderSignIn, ProviderSignInFailure,
};
use hoshi_core::{IdleCompaction, compaction_before_expiry, snooze_pending};
use hoshi_forge::{
    Bitbucket, BitbucketCredentials, Forge, ForgeError, GitHub, RepositoryRef,
    parse_pull_request_url, parse_remote, parse_repository_name,
};
pub use hoshi_providers::MCP_SERVER_NAME;
use hoshi_providers::{
    CodexSetup, Installation, ProviderAdapter, RunningTurn, TitleRequest, TurnAttachment,
    TurnRequest, UpdateProgress, Updater, codex_account, codex_sign_out, run_update,
    start_codex_sign_in,
};
use hoshi_store::{CachedPullRequestDetail, Store, StoreError};
use html_pages::Pages;
pub use merged_worktrees::{KeptReason, MergedCleanup};
use pull_request_cache::{PullRequestCache, Version};
use pull_request_watch::Wake;
use pull_requests::PullRequestSync;
pub use recent_folders::RecentFolder;
use refreshing::Refreshing;
use secrets::SecretKey;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError, Weak, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime};
pub use terminal::TerminalSink;
use terminal::{Launch, Terminals};
use watch::GitWatcher;
use worktree::TurnWorkspace;
use worktree_setup::{SetupLaunch, WorktreeSetups};

pub enum Notification {
    Event(EventEnvelope),
    /// Text still streaming, batched.
    Stream(Vec<StreamDelta>),
    /// Read from a forge; cached, not part of the event log.
    PullRequest(ThreadPullRequest),
    /// A fresh read of a pull request's detail, after the panel showed the cached one.
    PullRequestDetail(ThreadPullRequestDetail),
    /// Cached, not part of the event log: it changes on every model call.
    ContextUsage(ThreadContextUsage),
    /// The commands a thread's agent runs in the background, during its turn or after it;
    /// empty once none is left.
    BackgroundTasks(ThreadBackgroundTasks),
    /// The provider plan's limits, as a running turn last reported them.
    UsageLimits(UsageLimits),
    ProviderVersion(ProviderVersionUpdate),
    /// The step a running git action reached.
    GitActionProgress(GitActionProgress),
    /// What a thread's browser page painted, while the panel shows it.
    BrowserFrame(BrowserFrame),
    /// Where a thread's browser page is, and whether the agent just acted on it.
    BrowserPage(BrowserPage),
    /// The message a thread holds until it is compacted, or that it holds none any more.
    HeldMessage(ThreadHeldMessage),
    /// A thread's queue, or whether the user can resume it, changed.
    Queue(hoshi_core::ThreadQueue),
    /// A worktree's setup command started, wrote output, or ended.
    WorktreeSetup(hoshi_core::WorktreeSetupRun),
    /// The servers of a thread a view watches, when they change.
    Servers(hoshi_core::ThreadServers),
    /// The branch a project's root folder is on, when it changes.
    ProjectBranch(hoshi_core::ProjectBranch),
    /// The providers as now known, after any change to them.
    Providers(ProvidersStatus),
    /// What a sweep of merged worktrees just did.
    MergedCleanup(MergedCleanup),
}

pub type Notify = Arc<dyn Fn(Notification) + Send + Sync>;

/// The enabled providers' CLIs and accounts, by provider id, as last checked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProvidersStatus {
    pub versions: HashMap<String, ProviderVersion>,
    pub accounts: HashMap<String, ProviderAccountStatus>,
    /// When [`Engine::check_providers`] last ran, as an RFC 3339 time.
    pub checked_at: Option<String>,
    /// The sign-ins hoshi ran, by provider id.
    pub sign_ins: HashMap<String, ProviderSignIn>,
    /// Who each provider that reports it is signed in as, by provider id.
    pub logins: HashMap<String, String>,
    /// Who Codex is signed in as; `None` when signed out, unknown or Codex isn't enabled.
    pub codex_account: Option<CodexAccount>,
    /// Each provider plan's limits as last read or reported, possibly in an earlier run, by
    /// provider id.
    pub usage_limits: HashMap<String, UsageLimits>,
    /// Grows with each status taken, so an older one never replaces a newer one.
    pub revision: u64,
}

/// Shared by every thread that talks to the engine: the app and the reader threads of
/// running turns.
pub struct Engine {
    state: Arc<Mutex<State>>,
    terminals: Arc<Terminals>,
    /// The headless browser agents open pages in; apart from the state, as pages are slow.
    browsing: Arc<Browsing>,
    /// How its pages open; set without waiting for a page that is busy.
    browser_settings: Arc<Mutex<BrowserSettings>>,
    /// Per working folder.
    paths: Refreshing<Vec<paths::Indexed>>,
    /// Per working folder, which may add its own commands and skills.
    slash_commands: Refreshing<Vec<SlashCommand>>,
    /// Held while a provider is asked for its limits, so one ask serves everyone waiting.
    usage_probes: Arc<UsageProbes>,
    /// Held while a banked reset is spent: by provider and grant, the request id a retry of
    /// that grant reuses until the provider answers.
    reset_spend: Mutex<HashMap<(String, String), String>>,
    /// Held while the provider's CLI is checked against its releases or updated.
    maintenance: Arc<Mutex<()>>,
    servers: servers::Servers,
}

const LINKED_PULL_REQUEST_FRESH_FOR: Duration = Duration::from_secs(60);
const PATHS_FRESH_FOR: Duration = Duration::from_secs(10);
const SLASH_COMMANDS_FRESH_FOR: Duration = Duration::from_secs(5 * 60);
const USAGE_LIMITS_FRESH_FOR: Duration = Duration::from_secs(60);
const PROVIDER_VERSION_CHECK_EVERY: Duration = Duration::from_secs(60 * 60);
const PROVIDER_VERSION_FRESH_FOR: Duration = Duration::from_secs(10 * 60);
const UPDATE_PROGRESS_EVERY: Duration = Duration::from_millis(250);
const CODEX: &str = "codex";
const IDLE_BROWSER_CHECK: Duration = Duration::from_secs(60);
const IDLE_LINGERING_CHECK: Duration = Duration::from_secs(60);
/// The folder under hoshi's home that notes the agent processes each run of hoshi runs.
const AGENT_PROCESSES_DIR: &str = "agent-processes";
/// The most quitting waits for the browser, which may be busy with a slow page.
const BROWSER_CLOSE_WAIT: Duration = Duration::from_secs(2);
/// How long the agent gets to take a command it was told to stop as stopped.
const STOPPED_WITHIN: Duration = Duration::from_secs(5);
const STOPPED_POLL: Duration = Duration::from_millis(100);
const PATH_SEARCH_LIMIT: usize = 50;
/// A first title is tried this many times, waiting longer before each retry.
const TITLE_ATTEMPTS: u32 = 3;
/// A transcript loads this many turns at a time,
const PAGE_TURNS: usize = 20;
/// and no more items, however long a turn ran.
const PAGE_MAX_ITEMS: usize = 600;
const TITLE_RETRY_DELAY: Duration = Duration::from_secs(2);
/// What asks the agent to compact its conversation.
const COMPACT_COMMAND: &str = "/compact";
/// How often secret files are checked for being past their day.
const SECRETS_SWEPT_EVERY: Duration = Duration::from_secs(60 * 60);

struct State {
    /// Wakes the pull request sync before its next tick.
    sync_wake: Option<mpsc::Sender<()>>,
    store: Store,
    model: ReadModel,
    sequence: u64,
    /// Running turns by thread id.
    turns: HashMap<String, Turn>,
    /// Agent processes kept after their turn for the commands it left running in the
    /// background, by thread id.
    lingering: HashMap<String, Lingering>,
    /// How many agent processes the engine started, which numbers each one.
    processes: u64,
    /// The default provider's adapter, which also answers for the plan's limits and the CLI's
    /// version; `None` when it is disabled.
    adapter: Option<Arc<dyn ProviderAdapter>>,
    /// Every enabled provider's adapter, the default one included.
    adapters: Vec<Arc<dyn ProviderAdapter>>,
    notify: Notify,
    /// New worktrees go under here, one folder per project.
    worktrees_dir: PathBuf,
    /// Each provider's models as its CLI last listed them.
    provider_cache_dir: PathBuf,
    /// Each chat works in its own folder under here, named by its id.
    chats_dir: PathBuf,
    attachments: Attachments,
    /// Pages agents showed in their threads.
    pages: Pages,
    /// Lets background work (branch refreshes) reach the state without keeping it alive.
    this: Weak<Mutex<State>>,
    /// The projects' setup commands running in their new worktrees.
    setups: WorktreeSetups,
    /// `None` when the platform watcher failed to start; branches then refresh after turns only.
    watcher: Option<GitWatcher>,
    forges: HashMap<ForgeKind, Arc<dyn Forge>>,
    /// The forge repository of each project whose `origin` a forge serves.
    repositories: HashMap<String, RepositoryRef>,
    /// The branch each project's root folder is on, as last read.
    root_branches: HashMap<String, Option<String>>,
    pull_requests: PullRequestSync,
    /// The files of the pull request last read for each thread, with their patches, so the
    /// UI can ask for one patch at a time.
    /// What the side panel last read of each thread's pull request.
    pull_request_cache: PullRequestCache,
    /// Pull requests read for links in the transcript, with when they were read.
    linked_pull_requests: HashMap<(RepositoryRef, u32), (Instant, PullRequest)>,
    context_usages: HashMap<String, ContextUsage>,
    thread_titles: ThreadTitleSettings,
    browser: BrowserSettings,
    subagent_effort: SubagentEffort,
    /// Threads whose generated title missed the subject; named again after their turn.
    titles_to_refine: HashSet<String>,
    /// By provider, once it reports or is asked for them; kept across restarts.
    usage_limits: HashMap<String, UsageLimits>,
    /// When each provider last answered for its limits in this run.
    usage_checked_at: HashMap<String, Instant>,
    /// Each enabled provider's CLI, once checked, by provider id.
    provider_versions: HashMap<String, ProviderVersion>,
    /// When the default provider's CLI was last checked.
    provider_version_checked_at: Option<Instant>,
    /// Whether each enabled provider's CLI is installed and signed in, once checked.
    provider_accounts: HashMap<String, ProviderAccountStatus>,
    /// When the providers were last checked, as an RFC 3339 time.
    providers_checked_at: Option<String>,
    /// The revision the next providers status takes.
    providers_revision: AtomicU64,
    /// Where Codex is found and keeps its sign-in; `None` while it isn't enabled.
    codex: Option<CodexSetup>,
    provider_sign_ins: HashMap<String, ProviderSignIn>,
    /// The account each provider that reports one that way is signed in as.
    provider_logins: HashMap<String, String>,
    codex_account: Option<CodexAccount>,
    /// Stops the Codex sign-in that waits for the user.
    codex_sign_in: Option<Arc<AtomicBool>>,
    /// Where each thread's agent last moved its session, until git confirmed it.
    session_moves: HashMap<String, String>,
    /// Messages sent while a thread's turn was ending, too late to join it, oldest first.
    /// Each starts a turn of its own once the running one ends.
    queued: HashMap<String, VecDeque<QueuedMessage>>,
    /// The server agents call to delegate tasks; `None` when it couldn't start.
    mcp: Option<McpServer>,
    /// Tasks delegated to child threads, by child thread id, until their answer goes back.
    delegations: HashMap<String, Delegation>,
    /// Messages waiting for their thread's `/compact` turn to end, sent once it does.
    after_compaction: HashMap<String, QueuedMessage>,
    idle_compaction: IdleCompaction,
    /// Idle threads that compact before their prompt cache expires, with when, in Unix
    /// milliseconds. Kept in memory only: after a restart, they compact when the user returns.
    idle_compactions: HashMap<String, u64>,
    /// Folders git changes outside the lock, normalized: a worktree being created, restored
    /// or removed, or the files a rewind puts back. No turn starts in one until it's done.
    folder_ops: HashSet<String>,
    /// Turns that were to start in a folder git was changing, by that folder.
    parked: HashMap<String, Vec<ParkedTurn>>,
    /// Threads git works on outside the lock, with the messages sent to them meanwhile.
    thread_ops: HashMap<String, Vec<Command>>,
    checkpoints: Arc<Checkpointer>,
    /// Secrets the user typed for agents, one file each.
    secrets_dir: PathBuf,
    /// The `request_secret` calls waiting for the user, by request id: they get the file the
    /// secret was saved to, or `None` when the user declined.
    secret_waits: HashMap<String, mpsc::Sender<Option<PathBuf>>>,
    /// What each thread's agent was told about its pull request, watched now or before.
    watches: HashMap<String, hoshi_core::PullRequestWatch>,
    /// When each watched pull request was last read in detail, and as what.
    watch_reads: HashMap<String, (Instant, hoshi_core::PullRequest)>,
    /// Updates for watching threads whose turn ran, held until it ends.
    pending_wakes: HashMap<String, Vec<Wake>>,
    /// The account each forge is read with, whose own comments never wake its agent.
    viewers: HashMap<ForgeKind, String>,
    /// The threads whose transcripts the UI holds, the only ones whose text streams to it.
    streamed: Vec<String>,
    recovery: recovery::Recovery,
    merged_worktrees: merged_worktrees::MergedWorktrees,
}

/// A message waiting for the thread's running turn to end.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueuedMessage {
    id: String,
    text: String,
    attachments: Vec<MessageAttachment>,
    /// The user sent it, so a running turn reads it at once. News hoshi brings, such as a
    /// task's answer, waits for the agent to finish its step.
    from_user: bool,
    /// It waits for the user to go on with the thread, as after its turn failed.
    #[serde(default)]
    held: bool,
}

fn held_message(thread_id: &str, message: &QueuedMessage) -> HeldMessage {
    HeldMessage {
        thread_id: thread_id.to_owned(),
        message_id: message.id.clone(),
        text: message.text.clone(),
        attachments: message.attachments.clone(),
    }
}

/// A thread whose pull request is due to be read.
/// Where a new thread's worktree goes, read under the lock.
struct WorktreePlan {
    root: String,
    path: PathBuf,
    branch: String,
    /// Checks out a branch that exists, fetched first, instead of creating one.
    existing: bool,
}

impl WorktreePlan {
    /// Creates the worktree; on a new branch, it keeps the branch it started from.
    fn create(&self) -> Result<ThreadWorkspace, CommandError> {
        let failed = |detail: String| CommandError::WorktreeFailed { detail };
        let (root, branch) = (&self.root, &self.branch);
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| failed(error.to_string()))?;
        }
        if self.existing {
            git::add_branch_worktree(root, &self.path, branch).map_err(failed)?;
        } else {
            let base = git::current_branch(root);
            git::add_worktree(root, &self.path, branch).map_err(failed)?;
            if let Some(base) = base
                && let Err(error) = git::set_branch_config(root, branch, git::BASE_CONFIG, &base)
            {
                eprintln!("hoshi: didn't keep the base of {branch}: {error}");
            }
        }
        Ok(ThreadWorkspace::Worktree {
            path: self.path.to_string_lossy().into_owned(),
            branch: branch.clone(),
        })
    }

    /// Takes back a worktree made for a thread that couldn't be created.
    fn discard(&self) {
        let path = self.path.to_string_lossy().into_owned();
        if let Err(error) = git::remove_worktree(&self.root, &path) {
            eprintln!("hoshi: kept worktree {path}: {error}");
        }
        if !self.existing
            && let Err(error) = git::delete_branch(&self.root, &self.branch)
        {
            eprintln!("hoshi: kept branch {}: {error}", self.branch);
        }
    }
}

/// What cleaning up a thread whose pull request merged takes, read under the lock.
struct CleanUpPlan {
    root: String,
    pull_request: PullRequest,
    worktree: Option<String>,
    /// The merged branch and its tip.
    branch: Option<(String, String)>,
    /// The thread works in the project root, on the merged branch.
    in_root: bool,
    sharing_root: Vec<String>,
}

impl CleanUpPlan {
    /// Refuses when anything would be lost, then removes the worktree and the branch. On
    /// failure, also tells whether the worktree was already removed.
    fn run(&self, thread_id: &str, checkpoints: &Checkpointer) -> Result<(), (bool, CommandError)> {
        let git_failed = |detail: String| (false, CommandError::GitFailed { detail });
        let dirty = || {
            (
                false,
                CommandError::WorktreeDirty {
                    thread_id: thread_id.to_owned(),
                },
            )
        };
        let root = &self.root;
        if let Some(path) = &self.worktree
            && Path::new(path).is_dir()
            && !git::is_clean(path).map_err(git_failed)?
        {
            return Err(dirty());
        }
        if let Some((tip, _)) = &self.branch {
            let held_by_pull_request = self.pull_request.head_sha.as_deref().is_some_and(|head| {
                head.starts_with(tip.as_str())
                    || tip.starts_with(head)
                    || git::is_ancestor(root, tip, head)
            });
            if !held_by_pull_request && !git::on_a_remote(root, tip) {
                return Err((
                    false,
                    CommandError::UnpushedCommits {
                        thread_id: thread_id.to_owned(),
                    },
                ));
            }
        }
        if self.in_root {
            if !git::is_clean(root).map_err(git_failed)? {
                return Err(dirty());
            }
            git::switch(root, &self.pull_request.base_branch).map_err(git_failed)?;
        }
        if let Some(path) = &self.worktree {
            checkpoints.settle(path);
            State::remove_clean_worktree(root, path, thread_id).map_err(|error| (false, error))?;
        }
        if let Some((_, name)) = &self.branch
            && let Err(detail) = git::delete_branch(root, name)
        {
            return Err((self.worktree.is_some(), CommandError::GitFailed { detail }));
        }
        Ok(())
    }
}

/// Git work on a thread's folders that runs outside the lock; see [`State::begin_op`].
#[must_use]
struct WorkspaceOp {
    thread_id: String,
    folders: Vec<String>,
}

/// A turn whose events are recorded, waiting for git to finish with its folder.
struct ParkedTurn {
    thread_id: String,
    turn_id: String,
    prompt: String,
    attachments: Vec<MessageAttachment>,
    /// What the turn was started on, which a choice made while it waits doesn't change.
    runs_on: RunsOn,
}

/// The model and its options a turn runs on.
#[derive(Debug, Clone)]
struct RunsOn {
    model: Option<String>,
    effort: Option<hoshi_core::Effort>,
    fast_mode: bool,
    ultracode: bool,
}

impl RunsOn {
    fn of(thread: &hoshi_core::Thread) -> Self {
        Self {
            model: thread.model.clone(),
            effort: thread.effort,
            fast_mode: thread.fast_mode,
            ultracode: thread.ultracode,
        }
    }
}

struct PullRequestJob {
    thread_id: String,
    branch: String,
    repository: RepositoryRef,
    forge: Arc<dyn Forge>,
}

struct Turn {
    id: String,
    /// The agent process it runs in, by number.
    process: u64,
    running: Box<dyn RunningTurn>,
    /// `None` outside a git repository.
    workspace: Option<TurnWorkspace>,
    /// The folder the turn's changes are read in: where it started, or where its thread
    /// went since.
    checkpointed: Option<String>,
    /// How many times the thread changed folder during the turn.
    arrivals: usize,
    /// The provider's latest message of the conversation so far.
    cursor: Option<String>,
    /// The commands running in the background, once the turn reported any.
    background: Option<ThreadBackgroundTasks>,
    /// Started by `/compact`, which never compacts again once idle.
    compacting: bool,
    /// What it hands its agent first, until delivered.
    handoff: Option<handoffs::TurnHandoff>,
}

/// What a background check for a worktree the turn created needs from the state.
struct WorktreeCheck {
    turn_id: String,
    workspace: TurnWorkspace,
    /// Worktrees of other threads.
    claimed: Vec<String>,
}

impl Engine {
    /// Keeps everything under `data_dir`: the database, worktrees, chats and attachments. Ends
    /// the agent processes an earlier run of hoshi left running.
    pub fn open(data_dir: &Path, notify: Notify) -> Result<Self, StoreError> {
        hoshi_providers::agent_processes::record_in(&data_dir.join(AGENT_PROCESSES_DIR));
        Self::from_store(
            Store::open(&data_dir.join("state.sqlite"))?,
            data_dir,
            notify,
        )
    }

    /// Turns that were running when the app last closed are recorded as
    /// interrupted, and every thread's branch is read again. Files go under `data_dir`.
    pub fn from_store(store: Store, data_dir: &Path, notify: Notify) -> Result<Self, StoreError> {
        let notify = stream_batch::batch_streams(notify);
        let snapshot = store.snapshot()?;
        let known = snapshot.pull_requests.iter().filter_map(|update| {
            let pull_request = update.pull_request.clone()?;
            Some((update.thread_id.clone(), pull_request))
        });
        let pull_requests = PullRequestSync::new(known);
        let watches = store.pull_request_watches()?.into_iter().collect();
        let context_usages = snapshot
            .context_usages
            .iter()
            .map(|cached| (cached.thread_id.clone(), cached.usage))
            .collect();
        let usage_limits = store.provider_usage_limits().unwrap_or_else(|error| {
            eprintln!("hoshi: failed to read the cached usage limits: {error}");
            HashMap::new()
        });
        let terminals = Arc::new(Terminals::default());
        let setups = WorktreeSetups::new(terminals.clone(), notify.clone());
        let notify_servers = notify.clone();
        let mut state = State {
            setups,
            model: ReadModel::from_snapshot(&snapshot),
            sequence: snapshot.sequence,
            store,
            turns: HashMap::new(),
            lingering: HashMap::new(),
            processes: 0,
            adapter: None,
            adapters: Vec::new(),
            notify,
            worktrees_dir: data_dir.join("worktrees"),
            provider_cache_dir: provider_cache::dir(data_dir),
            chats_dir: data_dir.join("chats"),
            attachments: Attachments::new(data_dir.join("attachments")),
            pages: Pages::new(data_dir.join("attachments").join("pages")),
            this: Weak::new(),
            watcher: None,
            sync_wake: None,
            forges: HashMap::new(),
            repositories: HashMap::new(),
            root_branches: HashMap::new(),
            pull_requests,
            pull_request_cache: PullRequestCache::default(),
            linked_pull_requests: HashMap::new(),
            context_usages,
            usage_limits,
            usage_checked_at: HashMap::new(),
            provider_versions: HashMap::new(),
            provider_version_checked_at: None,
            provider_accounts: HashMap::new(),
            providers_checked_at: None,
            providers_revision: AtomicU64::new(1),
            codex: None,
            provider_sign_ins: HashMap::new(),
            provider_logins: HashMap::new(),
            codex_account: None,
            codex_sign_in: None,
            session_moves: HashMap::new(),
            queued: HashMap::new(),
            mcp: None,
            delegations: HashMap::new(),
            after_compaction: HashMap::new(),
            idle_compaction: IdleCompaction::default(),
            idle_compactions: HashMap::new(),
            folder_ops: HashSet::new(),
            parked: HashMap::new(),
            thread_ops: HashMap::new(),
            checkpoints: Arc::new(Checkpointer::new()),
            secrets_dir: data_dir.join("secrets"),
            secret_waits: HashMap::new(),
            watches,
            watch_reads: HashMap::new(),
            pending_wakes: HashMap::new(),
            streamed: Vec::new(),
            viewers: HashMap::new(),
            thread_titles: ThreadTitleSettings::default(),
            browser: BrowserSettings::default(),
            subagent_effort: SubagentEffort::default(),
            titles_to_refine: HashSet::new(),
            recovery: recovery::Recovery::default(),
            merged_worktrees: merged_worktrees::MergedWorktrees::default(),
        };
        state.recover_from_restart();
        let merged_cleanup = state.store.merged_cleanup().unwrap_or_else(|error| {
            eprintln!("hoshi: failed to read the last merged worktree cleanup: {error}");
            None
        });
        state.merged_worktrees.restore(merged_cleanup);
        let stale: Vec<_> = state
            .model
            .threads()
            .filter_map(|thread| Some((thread.id.clone(), thread.active_turn_id.clone()?)))
            .collect();
        for (thread_id, turn_id) in stale {
            let now = now();
            let events = close_stale_turn(&state.model, &thread_id, &turn_id, &now);
            if let Err(error) = state.commit(&events, &now) {
                eprintln!("hoshi: failed to close a turn the app's close cut short: {error}");
            }
        }
        // A task the close cut short answers once the user has it finish.
        state.delegations = state.pending_delegations();
        secret_requests::forget_old(&state.secrets_dir);
        let thread_ids = state
            .model
            .threads()
            .filter(|thread| follows_its_folder(thread))
            .map(|thread| thread.id.clone())
            .collect();
        let shared = Arc::new_cyclic(|this| {
            state.this = this.clone();
            Mutex::new(state)
        });
        restore_pull_request_details(Arc::downgrade(&shared));
        refresh_branches(Arc::downgrade(&shared), thread_ids);
        end_idle_lingering(Arc::downgrade(&shared));
        let weak = Arc::downgrade(&shared);
        let browsing = Arc::new(Browsing::new(
            data_dir.join("browser"),
            lock(&shared).notify.clone(),
        ));
        let browser_settings = browsing.settings_handle();
        let idle = Arc::downgrade(&browsing);
        thread::spawn(move || {
            while let Some(browsing) = idle.upgrade() {
                browsing.end_idle_browser();
                drop(browsing);
                thread::sleep(IDLE_BROWSER_CHECK);
            }
        });
        let pages = browsing.clone();
        let html_pages = lock(&shared).pages.clone();
        let tools: Arc<ToolHandler> = Arc::new(move |caller, tool, arguments| {
            if tool.starts_with("preview_") {
                return pages.call(caller, tool, arguments);
            }
            if matches!(tool, html_pages::PREVIEW_TOOL | html_pages::RENDER_TOOL) {
                let shared = weak.upgrade().ok_or("hoshi is closing")?;
                let files = {
                    let state = lock(&shared);
                    let full_access = state.model.thread(caller).is_some_and(|thread| {
                        thread.access_mode == hoshi_core::AccessMode::FullAccess
                    });
                    if full_access {
                        html_pages::PageFiles::Any
                    } else {
                        html_pages::PageFiles::within_folder(state.cwd(caller).as_deref())
                    }
                };
                drop(shared);
                return if tool == html_pages::PREVIEW_TOOL {
                    html_pages::preview(arguments, &files)
                } else {
                    html_pages::render(&html_pages, caller, arguments, &files)
                };
            }
            match tool {
                "request_secret" => {
                    return secret_requests::request_secret(&weak, caller, arguments);
                }
                "thread_wait" => return thread_tools::wait_for_thread(&weak, caller, arguments),
                "delegate_task" => return delegation::delegate_task(&weak, caller, arguments),
                pull_request_tool::TOOL => {
                    return pull_request_tool::create_pull_request(&weak, caller, arguments);
                }
                _ => {}
            }
            let shared = weak.upgrade().ok_or("hoshi is closing")?;
            lock(&shared).call_tool(caller, tool, arguments)
        });
        let weak = Arc::downgrade(&shared);
        let offered: Arc<ToolFilter> = Arc::new(move |caller, tool| {
            weak.upgrade()
                .is_some_and(|shared| lock(&shared).offers_tool(caller, tool))
        });
        match McpServer::start(tools, offered) {
            Ok(server) => lock(&shared).mcp = Some(server),
            Err(error) => eprintln!("hoshi: MCP server unavailable: {error}"),
        }
        let weak = Arc::downgrade(&shared);
        match GitWatcher::new(move |project_ids| refresh_projects(&weak, &project_ids)) {
            Ok(watcher) => lock(&shared).watcher = Some(watcher),
            Err(error) => eprintln!("hoshi: git watcher unavailable: {error}"),
        }
        let projects = lock(&shared)
            .model
            .projects()
            .map(|project| (project.id.clone(), project.workspace_root.clone()))
            .collect();
        watch_projects(Arc::downgrade(&shared), projects);
        sync_pull_requests(Arc::downgrade(&shared));
        let maintenance = Arc::<Mutex<()>>::default();
        watch_provider_version(Arc::downgrade(&shared), maintenance.clone());
        thread::spawn(editors::installed);
        merged_worktrees::watch(Arc::downgrade(&shared), Arc::downgrade(&terminals));
        Ok(Self {
            state: shared,
            terminals,
            browsing,
            browser_settings,
            paths: Refreshing::new(PATHS_FRESH_FOR),
            slash_commands: Refreshing::new(SLASH_COMMANDS_FRESH_FOR),
            usage_probes: Arc::default(),
            reset_spend: Mutex::default(),
            maintenance,
            servers: servers::Servers::new(notify_servers),
        })
    }

    pub fn set_forges(&self, forges: Vec<Arc<dyn Forge>>) {
        lock(&self.state).forges = forges
            .into_iter()
            .map(|forge| (forge.kind(), forge))
            .collect();
    }

    /// Reads again the pull requests not read in the last few seconds, as when the
    /// window gets focus.
    pub fn refresh_pull_requests(&self) {
        let mut state = lock(&self.state);
        state.pull_requests.check_stale(Instant::now());
        state.wake_sync();
    }

    /// Checks the provider's CLI again when the last check is a few minutes old, as when
    /// the window gets focus.
    pub fn refresh_provider_version(&self) {
        let stale = lock(&self.state)
            .provider_version_checked_at
            .is_none_or(|at| at.elapsed() >= PROVIDER_VERSION_FRESH_FOR);
        if stale {
            self.check_provider_version();
        }
    }

    /// The detail of a thread's pull request as last read, answered at once. When it is
    /// missing or old, it is read again in the background and arrives as a
    /// [`Notification::PullRequestDetail`].
    pub fn pull_request_detail(&self, thread_id: &str) -> Option<PullRequestDetail> {
        let mut state = lock(&self.state);
        let version = Version::of(state.pull_requests.get(thread_id)?);
        let (detail, read) = state
            .pull_request_cache
            .detail(thread_id, &version, Instant::now());
        if read {
            read_detail(state.this.clone(), thread_id.to_owned(), version);
        }
        detail
    }

    /// The pull request a web link points to. `None` when the link is not a pull request
    /// of a forge with an account. A link read in the last minute is answered from memory.
    pub fn linked_pull_request(&self, url: &str) -> Result<Option<PullRequest>, CommandError> {
        let Some(key) = parse_pull_request_url(url) else {
            return Ok(None);
        };
        let forge = {
            let state = lock(&self.state);
            let fresh = state
                .linked_pull_requests
                .get(&key)
                .filter(|(read_at, _)| read_at.elapsed() < LINKED_PULL_REQUEST_FRESH_FOR);
            if let Some((_, pull_request)) = fresh {
                return Ok(Some(pull_request.clone()));
            }
            state.forges.get(&key.0.forge).cloned()
        };
        let Some(forge) = forge else {
            return Ok(None);
        };
        let pull_request = forge.pull_request(&key.0, key.1).map_err(forge_error)?;
        lock(&self.state)
            .linked_pull_requests
            .insert(key, (Instant::now(), pull_request.clone()));
        Ok(Some(pull_request))
    }

    /// Warms the detail and the files of a thread's pull request before they are shown,
    /// as when the pointer rests on it.
    pub fn prefetch_pull_request(&self, thread_id: &str) {
        let _ = self.pull_request_detail(thread_id);
        let mut state = lock(&self.state);
        let Some(version) = state.pull_requests.get(thread_id).map(Version::of) else {
            return;
        };
        let (_, read) = state
            .pull_request_cache
            .files(thread_id, &version, Instant::now());
        if read {
            let this = state.this.clone();
            let thread_id = thread_id.to_owned();
            thread::spawn(move || read_files(&this, &thread_id, version));
        }
    }

    /// The files a thread's pull request changes. Their patches stay here until
    /// [`Self::pull_request_file_diff`] asks for one. Waits for a read already on its way
    /// instead of starting another.
    pub fn pull_request_files(&self, thread_id: &str) -> Option<Vec<DiffFile>> {
        let waited_since = Instant::now();
        loop {
            let mut state = lock(&self.state);
            let version = Version::of(state.pull_requests.get(thread_id)?);
            let (files, read) = state
                .pull_request_cache
                .files(thread_id, &version, Instant::now());
            if let Some(files) = files {
                return Some(files.iter().map(|file| file.file.clone()).collect());
            }
            drop(state);
            if read {
                return read_files(&Arc::downgrade(&self.state), thread_id, version);
            }
            if waited_since.elapsed() > FILES_WAIT_LIMIT {
                return None;
            }
            thread::sleep(FILES_WAIT_STEP);
        }
    }

    /// One file's patch from the last [`Self::pull_request_files`] read; `None` for binaries
    /// and diffs the forge left out.
    pub fn pull_request_file_diff(&self, thread_id: &str, path: &str) -> Option<FileDiff> {
        let state = lock(&self.state);
        let file = state
            .pull_request_cache
            .current_files(thread_id)?
            .iter()
            .find(|file| file.file.path == path)?;
        file.patch.as_deref().map(diff::truncate)
    }

    /// Asks the forge to change the thread's pull request, then reads it again soon.
    pub fn run_pull_request_action(
        &self,
        thread_id: &str,
        forge_id: &str,
        action: &PullRequestAction,
    ) -> Result<(), CommandError> {
        let (forge, repository, number) = self
            .pull_request_target(thread_id)
            .ok_or(CommandError::ForgeUnavailable)?;
        forge
            .act(&repository, number, forge_id, action)
            .map_err(forge_error)?;
        {
            let mut state = lock(&self.state);
            state.pull_requests.check_soon(thread_id);
            state.pull_request_cache.invalidate(thread_id);
        }
        let _ = self.pull_request_detail(thread_id);
        Ok(())
    }

    /// Accounts the thread's pull request can ask for a review, matching `query`.
    pub fn reviewer_candidates(
        &self,
        thread_id: &str,
        query: &str,
    ) -> Result<Vec<String>, CommandError> {
        let (forge, repository, _) = self
            .pull_request_target(thread_id)
            .ok_or(CommandError::ForgeUnavailable)?;
        forge
            .reviewer_candidates(&repository, query)
            .map_err(forge_error)
    }

    /// The labels of the repository the thread's pull request belongs to.
    pub fn repository_labels(
        &self,
        thread_id: &str,
    ) -> Result<Vec<PullRequestLabel>, CommandError> {
        let (forge, repository, _) = self
            .pull_request_target(thread_id)
            .ok_or(CommandError::ForgeUnavailable)?;
        forge.labels(&repository).map_err(forge_error)
    }

    fn pull_request_target(&self, thread_id: &str) -> Option<(Arc<dyn Forge>, RepositoryRef, u32)> {
        let state = lock(&self.state);
        let thread = state.model.thread(thread_id)?;
        let repository = state.repositories.get(thread.project_id.as_ref()?)?.clone();
        let forge = state.forges.get(&repository.forge)?.clone();
        let number = state.pull_requests.number(thread_id)?;
        Some((forge, repository, number))
    }

    /// The files a thread changed, compared as `scope` says.
    pub fn thread_diff_files(
        &self,
        thread_id: &str,
        scope: DiffScope,
    ) -> Result<Vec<DiffFile>, CommandError> {
        let cwd = self.repository_cwd(thread_id)?;
        diff::files(&cwd, scope).map_err(|detail| CommandError::Internal { detail })
    }

    pub fn thread_file_diff(
        &self,
        thread_id: &str,
        scope: DiffScope,
        file: &DiffFile,
    ) -> Result<FileDiff, CommandError> {
        let cwd = self.repository_cwd(thread_id)?;
        diff::file(&cwd, scope, file).map_err(|detail| CommandError::Internal { detail })
    }

    /// Takes the thread back to before `message_id`: the transcript from there on goes,
    /// and the next turn resumes the conversation as it was. With `revert_files`, the
    /// working tree goes back to how it was before that message's turn too.
    pub fn rewind_thread(
        &self,
        thread_id: &str,
        message_id: &str,
        revert_files: bool,
    ) -> Result<(), CommandError> {
        let cannot = || CommandError::CannotRewind {
            thread_id: thread_id.to_owned(),
            message_id: message_id.to_owned(),
        };
        let rewind_files = |cwd: &str| {
            let before = checkpoint::reference(thread_id, message_id, Moment::Before);
            if !checkpoint::exists(cwd, &before) || !checkpoint::taken_in(cwd, &before) {
                return Err(CommandError::NoCheckpoint {
                    thread_id: thread_id.to_owned(),
                    message_id: message_id.to_owned(),
                });
            }
            checkpoint::restore(cwd, &before).map_err(|detail| CommandError::GitFailed { detail })
        };
        let mut state = lock(&self.state);
        let thread = state
            .model
            .thread(thread_id)
            .ok_or_else(|| CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            })?;
        if thread.active_turn_id.is_some() {
            return Err(CommandError::ThreadBusy {
                thread_id: thread_id.to_owned(),
            });
        }
        let item = state
            .store
            .thread_item(thread_id, message_id)
            .map_err(internal)?
            .ok_or_else(cannot)?;
        let ItemContent::UserMessage {
            rewind: Some(rewind),
            ..
        } = item.content
        else {
            return Err(cannot());
        };
        let command = Command::ThreadRewind {
            thread_id: thread_id.to_owned(),
            message_id: message_id.to_owned(),
            rewind,
        };
        if !revert_files {
            return state.decide_and_commit(command);
        }
        let cwd = state
            .cwd(thread_id)
            .ok_or_else(|| CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            })?;
        if let Some(busy) = state.working_in(&cwd, thread_id) {
            return Err(CommandError::ThreadBusy { thread_id: busy });
        }
        let op = state.begin_op(thread_id, &[&cwd])?;
        let checkpoints = state.checkpoints.clone();
        drop(state);
        checkpoints.settle(&cwd);
        let reverted = rewind_files(&cwd);
        let mut state = lock(&self.state);
        let rewound = reverted.and_then(|()| state.decide_and_commit(command));
        state.end_op(op, None)?;
        rewound
    }

    /// The files a finished turn changed, read from its checkpoints.
    pub fn turn_diff_files(
        &self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<Vec<DiffFile>, CommandError> {
        let cwd = self.repository_cwd(thread_id)?;
        let before = checkpoint::reference(thread_id, turn_id, Moment::Before);
        let after = checkpoint::reference(thread_id, turn_id, Moment::After);
        checkpoint::changes(&cwd, &before, &after)
            .map_err(|detail| CommandError::Internal { detail })
    }

    /// How one file listed in a turn's changes changed during that turn.
    pub fn turn_file_diff(
        &self,
        thread_id: &str,
        turn_id: &str,
        file: &DiffFile,
    ) -> Result<FileDiff, CommandError> {
        let cwd = self.repository_cwd(thread_id)?;
        let before = checkpoint::reference(thread_id, turn_id, Moment::Before);
        let after = checkpoint::reference(thread_id, turn_id, Moment::After);
        checkpoint::file_diff(&cwd, &before, &after, file)
            .map_err(|detail| CommandError::Internal { detail })
    }

    /// The code editors installed on this machine; found once, then answered at once.
    pub fn editors(&self) -> Vec<Editor> {
        editors::installed()
    }

    /// The local branches of a thread's repository.
    pub fn thread_branches(&self, thread_id: &str) -> Result<Vec<String>, CommandError> {
        let cwd = self.repository_cwd(thread_id)?;
        Ok(git::local_branches(&cwd))
    }

    /// What the thread's working tree changed and has not committed.
    pub fn thread_changes(&self, thread_id: &str) -> Result<ChangesSummary, CommandError> {
        let cwd = self.repository_cwd(thread_id)?;
        let files = diff::files(&cwd, DiffScope::Uncommitted)
            .map_err(|detail| CommandError::Internal { detail })?;
        Ok(ChangesSummary {
            files: files.len() as u32,
            additions: files.iter().filter_map(|file| file.additions).sum(),
            deletions: files.iter().filter_map(|file| file.deletions).sum(),
        })
    }

    /// Where the thread's agent runs; for a chat, its own folder.
    pub fn thread_folder(&self, thread_id: &str) -> Option<String> {
        lock(&self.state).cwd(thread_id)
    }

    pub fn open_in_editor(&self, thread_id: &str, editor: Editor) -> Result<(), CommandError> {
        let cwd = lock(&self.state)
            .cwd(thread_id)
            .ok_or_else(|| CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            })?;
        editors::open(editor, &cwd).map_err(|detail| CommandError::Internal { detail })
    }

    /// Opens a project's folder, for a thread not started yet.
    pub fn open_project_in_editor(
        &self,
        project_id: &str,
        editor: Editor,
    ) -> Result<(), CommandError> {
        let root = lock(&self.state)
            .model
            .project(project_id)
            .map(|project| project.workspace_root.clone())
            .ok_or_else(|| CommandError::ProjectNotFound {
                project_id: project_id.to_owned(),
            })?;
        if !Path::new(&root).is_dir() {
            return Err(CommandError::WorkspaceRootNotFound { path: root });
        }
        editors::open(editor, &root).map_err(|detail| CommandError::Internal { detail })
    }

    /// Checks out another local branch in the thread's folder. Refused while a turn runs,
    /// so the agent never finds its branch changed under it.
    pub fn switch_branch(&self, thread_id: &str, branch: &str) -> Result<(), CommandError> {
        let cwd = self.repository_cwd(thread_id)?;
        let op = {
            let mut state = lock(&self.state);
            let working = state
                .model
                .thread(thread_id)
                .is_some_and(|thread| thread.status == ThreadStatus::Working);
            if working {
                return Err(CommandError::ThreadBusy {
                    thread_id: thread_id.to_owned(),
                });
            }
            state.begin_op(thread_id, &[&cwd])?
        };
        let switched =
            git::switch(&cwd, branch).map_err(|detail| CommandError::GitFailed { detail });
        lock(&self.state).end_op(op, None)?;
        switched?;
        let state = Arc::downgrade(&self.state);
        read_branches(&state, &[thread_id.to_owned()]);
        let project_id = lock(&self.state)
            .model
            .thread(thread_id)
            .filter(|thread| thread.worktree_path.is_none())
            .and_then(|thread| thread.project_id.clone());
        if let Some(project_id) = project_id {
            read_root_branches(&state, &[project_id]);
        }
        Ok(())
    }

    fn repository_cwd(&self, thread_id: &str) -> Result<String, CommandError> {
        let cwd = lock(&self.state)
            .cwd(thread_id)
            .ok_or_else(|| CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            })?;
        if !git::is_repository(&cwd) {
            return Err(CommandError::NotAGitRepository { path: cwd });
        }
        Ok(cwd)
    }

    /// Checks the token and its scopes against Bitbucket, then keeps it in the system's
    /// credential store.
    pub fn connect_bitbucket(
        &self,
        email: &str,
        token: &str,
    ) -> Result<ForgeAccount, CommandError> {
        let credentials = BitbucketCredentials {
            email: email.trim().to_owned(),
            token: token.trim().to_owned(),
        };
        let access = Bitbucket::with_credentials(credentials.clone())
            .check_access()
            .map_err(forge_error)?;
        if !access.missing_scopes.is_empty() {
            return Err(CommandError::ForgeScopesMissing {
                scopes: access.missing_scopes,
            });
        }
        secrets::write(SecretKey::Bitbucket, &credentials.encode())
            .map_err(|detail| CommandError::Internal { detail })?;
        self.credentials_changed(ForgeKind::Bitbucket);
        Ok(ForgeAccount {
            forge: ForgeKind::Bitbucket,
            login: Some(access.login),
        })
    }

    /// Forgets a forge's stored credentials. GitHub's belong to its CLI and stay.
    pub fn disconnect_forge(&self, forge: ForgeKind) -> Result<(), CommandError> {
        match forge {
            ForgeKind::Github => return Ok(()),
            ForgeKind::Bitbucket => secrets::delete(SecretKey::Bitbucket)
                .map_err(|detail| CommandError::Internal { detail })?,
        }
        self.credentials_changed(forge);
        Ok(())
    }

    fn credentials_changed(&self, kind: ForgeKind) {
        let mut state = lock(&self.state);
        if let Some(forge) = state.forges.get(&kind) {
            forge.forget_credentials();
        }
        state.pull_requests.resume(kind);
        state.wake_sync();
    }

    /// Removes the thread's worktree folder to free the disk and keeps its branch; the next
    /// message creates the folder again. Refused while it has uncommitted or untracked files,
    /// while the agent works, or while a terminal is open in it. Everything stays local.
    pub fn release_worktree(&self, thread_id: &str) -> Result<(), CommandError> {
        release_worktree(&self.state, &self.terminals, thread_id)
    }

    /// Removes now the worktree folders [`merged_worktrees`] would, and tells what it did.
    pub fn delete_merged_worktrees(&self) -> MergedCleanup {
        merged_worktrees::sweep(&self.state, &self.terminals)
    }

    /// What the last sweep of merged worktrees did in this run, if one ran.
    pub fn merged_cleanup(&self) -> Option<MergedCleanup> {
        lock(&self.state).merged_worktrees.last()
    }

    /// The worktree folders of threads whose pull request merged are removed by themselves.
    pub fn set_delete_merged_worktrees(&self, on: bool) {
        lock(&self.state).merged_worktrees.set(on);
    }

    /// Once the thread's pull request is merged: removes its worktree and its local branch,
    /// and archives it. A thread in the project root on that branch first switches the root
    /// to the pull request's base. Refused when anything would be lost: uncommitted files,
    /// or commits that neither a remote branch nor the pull request holds. The forge is never
    /// touched; deleting the branch there is the forge's own business.
    pub fn clean_up_thread(&self, thread_id: &str) -> Result<(), CommandError> {
        if self.terminals.any_open_for(thread_id) {
            return Err(CommandError::TerminalsOpen {
                thread_id: thread_id.to_owned(),
            });
        }
        let switched = self.clean_up_locked(thread_id)?;
        if !switched.is_empty() {
            read_branches(&Arc::downgrade(&self.state), &switched);
        }
        Ok(())
    }

    /// The cleanup itself; returns the root threads whose branch changed under them. What
    /// git does runs outside the lock.
    fn clean_up_locked(&self, thread_id: &str) -> Result<Vec<String>, CommandError> {
        let (op, plan, checkpoints) = {
            let mut state = lock(&self.state);
            let (thread, root) = state.removable(thread_id)?;
            let thread = thread.clone();
            let pull_request = state
                .pull_requests
                .get(thread_id)
                .filter(|pull_request| pull_request.state == hoshi_core::PullRequestState::Merged)
                .cloned()
                .ok_or_else(|| CommandError::PullRequestNotMerged {
                    thread_id: thread_id.to_owned(),
                })?;
            let worktree = thread
                .worktree_path
                .clone()
                .filter(|_| !thread.worktree_released);
            // Only the branch the pull request merged; one the thread moved to since stays.
            let branch = thread
                .branch
                .clone()
                .filter(|branch| *branch == pull_request.head_branch)
                .and_then(|branch| Some((git::branch_tip(&root, &branch)?, branch)));
            let in_root = thread.worktree_path.is_none()
                && branch.is_some()
                && git::current_branch(&root).as_deref() == Some(pull_request.head_branch.as_str());
            let sharing_root = if in_root {
                state.root_threads_sharing(&thread)?
            } else {
                Vec::new()
            };
            let mut folders: Vec<&str> = worktree.as_deref().into_iter().collect();
            if in_root {
                folders.push(&root);
            }
            let op = state.begin_op(thread_id, &folders)?;
            state.close_lingering(thread_id);
            let plan = CleanUpPlan {
                root,
                pull_request,
                worktree,
                branch,
                in_root,
                sharing_root,
            };
            (op, plan, state.checkpoints.clone())
        };
        let cleaned = plan.run(thread_id, &checkpoints);
        let mut state = lock(&self.state);
        let committed = match cleaned {
            Ok(()) => state.decide_and_commit(Command::ThreadCleanUp {
                thread_id: thread_id.to_owned(),
            }),
            Err((worktree_removed, error)) => {
                if worktree_removed
                    && let Err(released) = state.decide_and_commit(Command::ThreadWorktreeRelease {
                        thread_id: thread_id.to_owned(),
                    })
                {
                    eprintln!(
                        "hoshi: removed the worktree of {thread_id} but didn't record it: {released}"
                    );
                }
                Err(error)
            }
        };
        state.end_op(op, None)?;
        committed.map(|()| plan.sharing_root)
    }

    /// The folders inside `path`, to choose where a project lives.
    pub fn list_folders(&self, path: &str) -> Result<FolderListing, CommandError> {
        folders::list(path)
    }

    /// Creates a folder for a project; with `init_repository`, a new repository in an
    /// empty folder. Returns its normalized path.
    pub fn create_folder(&self, path: &str, init_repository: bool) -> Result<String, CommandError> {
        folders::create(path, init_repository)
    }

    /// Clones into a new or empty folder and returns its normalized path. Takes as long
    /// as the download.
    pub fn clone_repository(&self, url: &str, destination: &str) -> Result<String, CommandError> {
        folders::clone(url, destination)
    }

    /// Looks up a repository typed as `owner/name` or a link. Asks the forge.
    pub fn find_repository(
        &self,
        forge: ForgeKind,
        input: &str,
    ) -> Result<RemoteRepository, CommandError> {
        let repository = parse_repository_name(forge, input).ok_or_else(|| {
            CommandError::RepositoryNameInvalid {
                input: input.to_owned(),
            }
        })?;
        let adapter = lock(&self.state)
            .forges
            .get(&forge)
            .cloned()
            .ok_or(CommandError::ForgeUnavailable)?;
        adapter.repository(&repository).map_err(forge_error)
    }

    /// Who each forge is signed in as now, reading the credentials again in case the account
    /// changed. Asks the forges, so it can take a moment.
    pub fn forge_accounts(&self) -> Vec<ForgeAccount> {
        let forges: Vec<_> = lock(&self.state).forges.values().cloned().collect();
        let accounts: Vec<ForgeAccount> = forges
            .into_iter()
            .map(|forge| {
                forge.forget_credentials();
                ForgeAccount {
                    forge: forge.kind(),
                    login: forge.viewer().ok(),
                }
            })
            .collect();
        let mut state = lock(&self.state);
        for account in &accounts {
            match &account.login {
                Some(login) => state.viewers.insert(account.forge, login.clone()),
                None => state.viewers.remove(&account.forge),
            };
        }
        accounts
    }

    /// The project's icon as a data URL, found among its files. Reads the disk.
    pub fn project_icon(&self, project_id: &str) -> Option<String> {
        let root = lock(&self.state)
            .model
            .project(project_id)?
            .workspace_root
            .clone();
        project_icon::data_url(Path::new(&root))
    }

    /// Where the project's icon is among its files. Reads the disk.
    pub fn project_icon_path(&self, project_id: &str) -> Option<std::path::PathBuf> {
        let root = lock(&self.state)
            .model
            .project(project_id)?
            .workspace_root
            .clone();
        project_icon::find(Path::new(&root))
    }

    /// Also reads the plan's limits in the background, so they are ready when asked for.
    /// Also reads the plan's limits and checks the CLI against its releases in the background.
    pub fn set_adapter(&self, adapter: Option<Arc<dyn ProviderAdapter>>) {
        self.set_adapters(adapter.into_iter().collect());
    }

    /// The enabled providers' adapters, one per provider. Each that reports them also reads
    /// its plan's limits in the background, and the CLIs are checked against their releases.
    pub fn set_adapters(&self, adapters: Vec<Arc<dyn ProviderAdapter>>) {
        let adapter = adapters
            .iter()
            .find(|adapter| adapter.provider() == DEFAULT_PROVIDER)
            .cloned();
        let enabled = adapter.is_some();
        let any_enabled = !adapters.is_empty();
        let reporting: Vec<String> = adapters
            .iter()
            .filter(|adapter| adapter.reports_usage_limits())
            .map(|adapter| adapter.provider().to_owned())
            .collect();
        {
            let mut state = lock(&self.state);
            let mut gone: Vec<String> = state
                .adapters
                .iter()
                .map(|adapter| adapter.provider().to_owned())
                .filter(|provider| !adapters.iter().any(|kept| kept.provider() == provider))
                .collect();
            if !enabled && !gone.iter().any(|provider| provider == DEFAULT_PROVIDER) {
                gone.push(DEFAULT_PROVIDER.to_owned());
            }
            state.adapter = adapter;
            state.adapters = adapters;
            for provider in gone {
                state.forget_provider(provider);
            }
        }
        self.slash_commands.clear();
        for provider in reporting {
            let state = Arc::downgrade(&self.state);
            let probes = self.usage_probes.clone();
            thread::spawn(move || {
                if let Some(state) = state.upgrade() {
                    check_usage_limits(&state, &probes, &provider, false);
                }
            });
        }
        if any_enabled {
            let state = Arc::downgrade(&self.state);
            let maintenance = self.maintenance.clone();
            thread::spawn(move || {
                if let Some(state) = state.upgrade() {
                    check_provider_versions(&state, &maintenance, None);
                }
            });
        }
    }

    /// The default provider's CLI as last checked, without checking it.
    pub fn provider_version(&self) -> Option<ProviderVersion> {
        self.provider_version_of(DEFAULT_PROVIDER)
    }

    /// Provider `provider`'s CLI as last checked, without checking it.
    pub fn provider_version_of(&self, provider: &str) -> Option<ProviderVersion> {
        lock(&self.state).provider_versions.get(provider).cloned()
    }

    /// Checks the default provider's CLI against its latest release. While an update or
    /// another check runs, answers with what is known.
    pub fn check_provider_version(&self) -> Option<ProviderVersion> {
        check_provider_versions(&self.state, &self.maintenance, Some(DEFAULT_PROVIDER));
        self.provider_version()
    }

    /// Whether the default provider's CLI is installed and signed in. Runs the CLI.
    pub fn provider_account_status(&self) -> ProviderAccountStatus {
        let adapter = lock(&self.state).adapter.clone();
        adapter.map_or(ProviderAccountStatus::Unknown, |adapter| {
            adapter.account_status()
        })
    }

    /// Every enabled provider's CLI and account as last checked, without checking them.
    pub fn providers_status(&self) -> ProvidersStatus {
        lock(&self.state).providers_status()
    }

    /// Checks every enabled provider's CLI against its latest release, and whether it is
    /// installed and signed in. Runs each CLI, so it can take a few seconds.
    pub fn check_providers(&self) -> ProvidersStatus {
        let adapters = lock(&self.state).adapters.clone();
        let accounts: Vec<(String, ProviderAccountStatus, Option<String>)> =
            thread::scope(|scope| {
                let accounts: Vec<_> = adapters
                    .iter()
                    .map(|adapter| {
                        scope.spawn(|| {
                            let status = adapter.account_status();
                            (
                                adapter.provider().to_owned(),
                                status,
                                adapter.signed_in_as(),
                            )
                        })
                    })
                    .collect();
                check_provider_versions(&self.state, &self.maintenance, None);
                accounts
                    .into_iter()
                    .filter_map(|account| account.join().ok())
                    .collect()
            });
        let codex = lock(&self.state).codex.clone();
        let codex_account = codex.and_then(|codex| codex_account(codex.home.as_deref()));
        let mut state = lock(&self.state);
        state.provider_logins.clear();
        for (provider, account, login) in accounts {
            if state.adapter_for(&provider).is_some() {
                if let Some(login) = login {
                    state.provider_logins.insert(provider.clone(), login);
                }
                state.provider_accounts.insert(provider, account);
            }
        }
        state.codex_account = codex_account.filter(|_| state.codex.is_some());
        state.providers_checked_at = Some(now());
        state.providers_status()
    }

    /// Where Codex is found and keeps its sign-in; `None` while it isn't enabled.
    pub fn set_codex(&self, setup: Option<CodexSetup>) {
        let mut state = lock(&self.state);
        if setup.is_none() {
            state.codex_account = None;
            if let Some(cancel) = state.codex_sign_in.take() {
                cancel.store(true, Ordering::SeqCst);
            }
            state.provider_sign_ins.remove(CODEX);
        }
        state.codex = setup;
    }

    /// Starts signing Codex in with ChatGPT and returns the page the user signs in on, for
    /// their browser. The sign-in then waits in the background; how it ends arrives as
    /// `Notification::Providers`. Runs the CLI.
    pub fn sign_in_codex(&self) -> Result<String, ProviderSignInFailure> {
        let setup = {
            let mut state = lock(&self.state);
            let setup = state
                .codex
                .clone()
                .ok_or(ProviderSignInFailure::NotInstalled)?;
            if let Some(cancel) = state.codex_sign_in.take() {
                cancel.store(true, Ordering::SeqCst);
            }
            state.set_sign_in(CODEX, ProviderSignIn::Starting);
            setup
        };
        let sign_in = match start_codex_sign_in(&setup) {
            Ok(sign_in) => sign_in,
            Err(reason) => {
                lock(&self.state).set_sign_in(
                    CODEX,
                    ProviderSignIn::Failed {
                        reason: reason.clone(),
                    },
                );
                return Err(reason);
            }
        };
        let url = sign_in.url().to_owned();
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut state = lock(&self.state);
            state.codex_sign_in = Some(cancel.clone());
            state.set_sign_in(CODEX, ProviderSignIn::Waiting { url: url.clone() });
        }
        let state = Arc::downgrade(&self.state);
        thread::spawn(move || {
            let result = sign_in.wait(&cancel);
            let account = codex_account(setup.home.as_deref());
            let Some(state) = state.upgrade() else {
                return;
            };
            let mut state = lock(&state);
            let current = state
                .codex_sign_in
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &cancel));
            if !current {
                return;
            }
            state.codex_sign_in = None;
            match result {
                Ok(()) => {
                    state.keep_codex_account(account, ProviderAccountStatus::Ready);
                    state.set_sign_in(CODEX, ProviderSignIn::Succeeded);
                }
                Err(reason) => state.set_sign_in(CODEX, ProviderSignIn::Failed { reason }),
            }
        });
        Ok(url)
    }

    /// Stops the Codex sign-in that waits for the user.
    pub fn cancel_codex_sign_in(&self) {
        let mut state = lock(&self.state);
        if let Some(cancel) = state.codex_sign_in.take() {
            cancel.store(true, Ordering::SeqCst);
            state.set_sign_in(CODEX, ProviderSignIn::Idle);
        }
    }

    /// Signs Codex out through Codex itself. Runs the CLI.
    pub fn sign_out_codex(&self) -> Result<(), ProviderSignInFailure> {
        let setup = lock(&self.state)
            .codex
            .clone()
            .ok_or(ProviderSignInFailure::NotInstalled)?;
        codex_sign_out(&setup)?;
        let mut state = lock(&self.state);
        state.keep_codex_account(None, ProviderAccountStatus::SignedOut);
        state.set_sign_in(CODEX, ProviderSignIn::Idle);
        Ok(())
    }

    /// Updates the default provider's CLI in the background; its progress arrives as
    /// `Notification::ProviderVersion`.
    pub fn update_provider(&self) {
        self.update_provider_of(DEFAULT_PROVIDER);
    }

    /// Updates provider `provider`'s CLI in the background; its progress arrives as
    /// `Notification::ProviderVersion`.
    pub fn update_provider_of(&self, provider: &str) {
        let state = Arc::downgrade(&self.state);
        let maintenance = self.maintenance.clone();
        let provider = provider.to_owned();
        thread::spawn(move || {
            if let Some(state) = state.upgrade() {
                update_provider(&state, &maintenance, &provider);
            }
        });
    }

    pub fn set_thread_titles(&self, settings: ThreadTitleSettings) {
        lock(&self.state).thread_titles = settings;
    }

    /// Who uses the browser and how its pages open. Agents running now keep the tools they
    /// were given until their session starts again; they are refused once access is off.
    pub fn set_browser(&self, settings: BrowserSettings) {
        lock(&self.state).browser = settings.clone();
        *hold(&self.browser_settings) = settings;
    }

    /// How agents choose the model and effort of their subagents, from their next turn on.
    pub fn set_subagent_effort(&self, policy: SubagentEffort) {
        lock(&self.state).subagent_effort = policy;
    }

    /// When idle threads with a large context compact. Threads already idle under another
    /// mode wait for the user.
    pub fn set_idle_compaction(&self, mode: IdleCompaction) {
        let mut state = lock(&self.state);
        state.idle_compaction = mode;
        if mode != IdleCompaction::BeforeCacheExpires {
            state.idle_compactions.clear();
        }
    }

    /// Names the thread again from its whole conversation, in the background. The
    /// title only changes if the user did not rename it meanwhile.
    /// Whether threads a restart cut short go on by themselves; see
    /// [`Engine::continue_cut_threads`].
    pub fn set_continue_after_restart(&self, on: bool) {
        lock(&self.state).recovery.continue_after_restart = on;
    }

    /// Whether a thread whose turn failed on its plan's usage limit goes on by itself once the
    /// limit resets.
    pub fn set_resume_after_usage_limit(&self, on: bool) {
        lock(&self.state).recovery.resume_after_usage_limit = on;
    }

    /// Has the threads whose turn the last close or crash cut short go on where they left off,
    /// told of the background work that ended with it. Once, after the adapters and settings
    /// are set.
    pub fn continue_cut_threads(&self) {
        lock(&self.state).continue_cut_threads();
    }

    /// The messages queued for the thread that wait for the user to go on with it, as after a
    /// restart or a failed turn.
    pub fn held_queue(&self, thread_id: &str) -> Vec<HeldMessage> {
        lock(&self.state).held_queue(thread_id)
    }

    /// Sends what waits in the thread's held queue, one turn each.
    pub fn resume_queue(&self, thread_id: &str) {
        lock(&self.state).release_queue(thread_id);
    }

    /// The messages waiting in the thread's queue, in the order they go, and whether the
    /// user can resume it. [`Notification::Queue`] tells of each change.
    pub fn thread_queue(&self, thread_id: &str) -> hoshi_core::ThreadQueue {
        lock(&self.state).thread_queue(thread_id)
    }

    /// Goes on with the stopped thread: when its last turn was stopped, cut short or failed on
    /// its usage limit and nothing waits for an answer, its agent is told to continue first;
    /// then what waited in its queue follows.
    pub fn resume_thread(&self, thread_id: &str) -> Result<(), CommandError> {
        let resumable = {
            let state = lock(&self.state);
            if state.model.thread(thread_id).is_none() {
                return Err(CommandError::ThreadNotFound {
                    thread_id: thread_id.to_owned(),
                });
            }
            state.resumable(thread_id) && state.model.pending_requests(thread_id).is_empty()
        };
        if resumable {
            self.dispatch(Command::MessageSend {
                thread_id: thread_id.to_owned(),
                message_id: delegation::new_id(),
                text: recovery::LIMIT_RESET_PROMPT.to_owned(),
                attachments: Vec::new(),
            })?;
        }
        lock(&self.state).release_queue(thread_id);
        Ok(())
    }

    /// Starts a turn with one of the user's queued messages, while the thread runs none; the
    /// rest of its queue follows.
    pub fn send_queued_now(&self, thread_id: &str, message_id: &str) -> Result<(), CommandError> {
        lock(&self.state).send_queued_now(thread_id, message_id)
    }

    /// Takes a message out of the thread's queue; the transcript shows it never read.
    pub fn remove_queued_message(
        &self,
        thread_id: &str,
        message_id: &str,
    ) -> Result<(), CommandError> {
        lock(&self.state).remove_queued(thread_id, Some(message_id))
    }

    /// Takes every message out of the thread's queue.
    pub fn clear_queue(&self, thread_id: &str) -> Result<(), CommandError> {
        lock(&self.state).remove_queued(thread_id, None)
    }

    /// Whether the project's threads a restart cut short go on by themselves, as it sets for
    /// itself or inherits from `general.continue_after_restart`; `None` for chats.
    pub fn continue_after_restart_in(
        &self,
        project_id: Option<&str>,
    ) -> hoshi_core::EffectiveSetting<bool> {
        lock(&self.state).continue_after_restart_in(project_id)
    }

    /// When the thread goes on by itself, as RFC 3339: once the usage limit that failed its
    /// turn resets.
    pub fn usage_limit_resume(&self, thread_id: &str) -> Option<String> {
        lock(&self.state).limit_resume(thread_id)
    }

    /// The thread no longer goes on by itself once its usage limit resets, whatever the
    /// settings say.
    pub fn cancel_usage_limit_resume(&self, thread_id: &str) {
        lock(&self.state).choose_limit_resume(thread_id, false);
    }

    /// The thread goes on by itself once the usage limit that failed its turn resets, whatever
    /// the settings say. Whether it will: not when no reset is ahead.
    pub fn resume_at_usage_limit_reset(&self, thread_id: &str) -> bool {
        lock(&self.state).choose_limit_resume(thread_id, true)
    }

    /// Threads whose turn fails on their usage limit are snoozed until it resets.
    pub fn set_snooze_until_limit_resets(&self, on: bool) {
        lock(&self.state).recovery.snooze_until_limit_resets = on;
    }

    /// Snoozes the thread until the usage limit that failed its turn resets. Whether it did:
    /// not when no reset is ahead, or the thread is archived or waits for the user.
    pub fn snooze_until_usage_limit_resets(&self, thread_id: &str) -> bool {
        lock(&self.state).snooze_until_limit_resets(thread_id)
    }

    /// The thread's snooze is the one its usage limit put it in, until the reset; waking it
    /// with [`Command::ThreadWake`] drops only that snooze, not the resumption.
    pub fn usage_limit_snoozed(&self, thread_id: &str) -> bool {
        lock(&self.state).limit_snoozed(thread_id)
    }

    pub fn regenerate_title(&self, thread_id: &str) -> Result<(), CommandError> {
        let mut state = lock(&self.state);
        if state.model.thread(thread_id).is_none() {
            return Err(CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            });
        }
        state.rename_from_conversation(thread_id, true);
        Ok(())
    }

    /// The thread's folder, or the project's root for a thread not created yet. `None` for
    /// a chat not created yet, which has no folder.
    fn composer_cwd(
        &self,
        project_id: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<Option<String>, CommandError> {
        let state = lock(&self.state);
        match (thread_id, project_id) {
            (Some(thread_id), _) => {
                state
                    .cwd(thread_id)
                    .map(Some)
                    .ok_or_else(|| CommandError::ThreadNotFound {
                        thread_id: thread_id.to_owned(),
                    })
            }
            (None, Some(project_id)) => state
                .model
                .project(project_id)
                .map(|project| Some(project.workspace_root.clone()))
                .ok_or_else(|| CommandError::ProjectNotFound {
                    project_id: project_id.to_owned(),
                }),
            (None, None) => Ok(None),
        }
    }

    /// The files and folders that best match `query`, for `@` mentions. Reads
    /// the folder the first time.
    pub fn search_paths(
        &self,
        project_id: Option<&str>,
        thread_id: Option<&str>,
        query: &str,
    ) -> Result<Vec<WorkspaceEntry>, CommandError> {
        let Some(cwd) = self.composer_cwd(project_id, thread_id)? else {
            return Ok(Vec::new());
        };
        let root = PathBuf::from(&cwd);
        let entries = self
            .paths
            .get(&cwd, move || Some(paths::read(&root)))
            .unwrap_or_default();
        Ok(paths::search(&entries, query, PATH_SEARCH_LIMIT))
    }

    /// What a prompt can start with in the thread's folder, or in the chats folder for a
    /// chat not created yet. `None` while no provider is enabled or when it can't say. Can
    /// run the provider's CLI.
    pub fn slash_commands(
        &self,
        project_id: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<Option<Vec<SlashCommand>>, CommandError> {
        let cwd = match self.composer_cwd(project_id, thread_id)? {
            Some(cwd) => cwd,
            None => {
                let chats_dir = lock(&self.state).chats_dir.clone();
                std::fs::create_dir_all(&chats_dir).map_err(|error| CommandError::Internal {
                    detail: error.to_string(),
                })?;
                chats_dir.to_string_lossy().into_owned()
            }
        };
        let adapter = {
            let state = lock(&self.state);
            let provider = thread_id
                .and_then(|thread_id| state.model.thread(thread_id))
                .map_or(DEFAULT_PROVIDER, |thread| thread.provider.as_str());
            state.adapter_for(provider)
        };
        let Some(adapter) = adapter else {
            return Ok(None);
        };
        let root = PathBuf::from(&cwd);
        let key = format!("{}\n{cwd}", adapter.provider());
        let commands = self
            .slash_commands
            .get(&key, move || adapter.slash_commands(&root));
        Ok(commands.map(|commands| commands.as_ref().clone()))
    }

    /// The default provider's models. Can run the provider's CLI.
    pub fn models(&self) -> Option<ProviderModels> {
        let adapter = lock(&self.state).adapter.clone()?;
        Some(adapter.models())
    }

    /// Every enabled provider's models, the default provider first. Runs the providers' CLIs
    /// side by side, and keeps what they list for [`Engine::known_provider_models`].
    pub fn provider_models(&self) -> Vec<ProviderModels> {
        let (adapters, cache_dir) = {
            let state = lock(&self.state);
            (state.adapters.clone(), state.provider_cache_dir.clone())
        };
        let mut catalogs: Vec<ProviderModels> = thread::scope(|scope| {
            let reads: Vec<_> = adapters
                .iter()
                .map(|adapter| {
                    scope.spawn(|| ProviderModels {
                        compacts_idle_threads: adapter.compacts_idle_threads(),
                        ..adapter.models()
                    })
                })
                .collect();
            reads
                .into_iter()
                .filter_map(|read| read.join().ok())
                .collect()
        });
        catalogs.sort_by_key(|catalog| catalog.provider != DEFAULT_PROVIDER);
        for catalog in catalogs.iter().filter(|catalog| !catalog.models.is_empty()) {
            if let Err(error) = provider_cache::write(&cache_dir, catalog) {
                eprintln!(
                    "hoshi: failed to keep the models of {}: {error}",
                    catalog.provider
                );
            }
        }
        catalogs
    }

    /// Every enabled provider's models as known without running a CLI, the default provider
    /// first: its offline catalog, then what [`Engine::provider_models`] last kept. Providers
    /// with neither are left out.
    pub fn known_provider_models(&self) -> Vec<ProviderModels> {
        let (adapters, cache_dir) = {
            let state = lock(&self.state);
            (state.adapters.clone(), state.provider_cache_dir.clone())
        };
        let mut catalogs: Vec<ProviderModels> = adapters
            .iter()
            .filter_map(|adapter| {
                let kept = provider_cache::read(&cache_dir, adapter.provider());
                let catalog = provider_cache::hydrate(adapter.offline_models(), kept)?;
                Some(ProviderModels {
                    compacts_idle_threads: adapter.compacts_idle_threads(),
                    ..catalog
                })
            })
            .collect();
        catalogs.sort_by_key(|catalog| catalog.provider != DEFAULT_PROVIDER);
        catalogs
    }

    /// The default provider's limits last read or reported, possibly in an earlier run,
    /// without asking it.
    pub fn known_usage_limits(&self) -> Option<UsageLimits> {
        self.known_usage_limits_of(DEFAULT_PROVIDER)
    }

    /// Provider `provider`'s limits last read or reported, possibly in an earlier run, without
    /// asking it.
    pub fn known_usage_limits_of(&self, provider: &str) -> Option<UsageLimits> {
        lock(&self.state).usage_limits.get(provider).cloned()
    }

    /// The default provider plan's limits; see [`Engine::usage_limits_of`].
    pub fn usage_limits(&self, force: bool) -> Option<UsageLimits> {
        self.usage_limits_of(DEFAULT_PROVIDER, force)
    }

    /// Provider `provider`'s plan limits, asking it unless it answered within the last minute.
    /// `force` always waits for an answer given after the call. `None` when it can't say or
    /// isn't enabled.
    pub fn usage_limits_of(&self, provider: &str, force: bool) -> Option<UsageLimits> {
        check_usage_limits(&self.state, &self.usage_probes, provider, force)
    }

    /// The enabled providers that can read their plan's limits, in the order they were set.
    pub fn usage_limit_providers(&self) -> Vec<String> {
        lock(&self.state)
            .adapters
            .iter()
            .filter(|adapter| adapter.reports_usage_limits())
            .map(|adapter| adapter.provider().to_owned())
            .collect()
    }

    /// Spends the default provider's next banked reset; see [`Engine::spend_banked_reset_of`].
    pub fn spend_banked_reset(&self) -> BankedResetSpend {
        self.spend_banked_reset_of(DEFAULT_PROVIDER)
    }

    /// Spends provider `provider`'s next banked reset, then reads its limits again. Reaches
    /// the network.
    pub fn spend_banked_reset_of(&self, provider: &str) -> BankedResetSpend {
        self.spend_banked_grant(provider, None)
    }

    /// Spends a banked reset of `provider`'s grant `grant_id`, one its limits list, or its
    /// next one when `None`; then reads its limits again. Reaches the network.
    pub fn spend_banked_grant(&self, provider: &str, grant_id: Option<&str>) -> BankedResetSpend {
        let mut pending = self
            .reset_spend
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (adapter, grant) = {
            let state = lock(&self.state);
            let grant = state
                .usage_limits
                .get(provider)
                .and_then(|limits| limits.banked_resets.as_ref())
                .filter(|banked| banked.count > 0)
                .and_then(|banked| match grant_id {
                    Some(id) => banked
                        .grants
                        .iter()
                        .any(|grant| grant.id == id && grant.resets > 0)
                        .then(|| id.to_owned()),
                    None => banked.next_grant_id.clone(),
                });
            (state.adapter_for(provider), grant)
        };
        let (Some(adapter), Some(grant)) = (adapter, grant) else {
            return BankedResetSpend::NoneLeft;
        };
        let key = (provider.to_owned(), grant);
        let request_id = pending.remove(&key).unwrap_or_else(delegation::new_id);
        let spend = adapter.spend_banked_reset(&key.1, &request_id);
        if !spend.settled() {
            pending.insert(key, request_id);
            return spend;
        }
        if matches!(spend, BankedResetSpend::Failed { .. }) {
            return spend;
        }
        let refreshed = check_usage_limits(&self.state, &self.usage_probes, provider, true);
        if spend == BankedResetSpend::Reset && refreshed.is_none() {
            return BankedResetSpend::Failed {
                reason: BankedResetFailure::LimitsUnread,
            };
        }
        spend
    }

    /// Hands a delegated task's answer back to its thread now, as it stands, with its plan
    /// when it still plans.
    pub fn send_task_answer(&self, thread_id: &str) {
        lock(&self.state).hand_back(thread_id, &TurnOutcome::Completed);
    }

    /// The user cancels a delegated task: it stops if it runs, its answer never goes back,
    /// and the thread that still waited for it hears it was stopped.
    pub fn cancel_task(&self, thread_id: &str) {
        lock(&self.state).stop_task(thread_id);
    }

    pub fn snapshot(&self) -> Snapshot {
        let state = lock(&self.state);
        Snapshot {
            sequence: state.sequence,
            projects: state.model.projects().cloned().collect(),
            threads: state.model.threads().cloned().collect(),
            pending_requests: state.model.all_pending_requests(),
            pending_tasks: state
                .delegations
                .keys()
                .filter(|task| state.awaited(task))
                .cloned()
                .collect(),
            open_tool_calls: state.model.open_tool_calls().cloned().collect(),
            pull_requests: state
                .pull_requests
                .known()
                .map(|(thread_id, pull_request)| ThreadPullRequest {
                    thread_id: thread_id.clone(),
                    pull_request: Some(pull_request.clone()),
                })
                .collect(),
            context_usages: state
                .context_usages
                .iter()
                .map(|(thread_id, usage)| ThreadContextUsage {
                    thread_id: thread_id.clone(),
                    usage: *usage,
                })
                .collect(),
            background_tasks: state
                .turns
                .values()
                .filter_map(|turn| turn.background.clone())
                .chain(
                    state
                        .lingering
                        .values()
                        .filter_map(|lingering| lingering.background.clone()),
                )
                .filter(|background| !background.tasks.is_empty())
                .collect(),
            task_previews: state.model.task_previews(),
            held_messages: state
                .after_compaction
                .iter()
                .map(|(thread_id, message)| held_message(thread_id, message))
                .collect(),
            project_branches: state
                .root_branches
                .iter()
                .map(|(project_id, branch)| hoshi_core::ProjectBranch {
                    project_id: project_id.clone(),
                    branch: branch.clone(),
                })
                .collect(),
        }
    }

    /// One transcript item, with what the whole transcript leaves out.
    pub fn thread_item(
        &self,
        thread_id: &str,
        item_id: &str,
    ) -> Result<Option<ThreadItem>, CommandError> {
        lock(&self.state)
            .store
            .thread_item(thread_id, item_id)
            .map_err(internal)
    }

    /// The latest turns before item `before`, the end when `None`, light enough to send
    /// as a whole: `thread_item` returns what they leave out.
    pub fn transcript(
        &self,
        thread_id: &str,
        before: Option<&str>,
    ) -> Result<TranscriptPage, CommandError> {
        let (items, has_older) = lock(&self.state)
            .store
            .transcript_page(thread_id, before, PAGE_TURNS, PAGE_MAX_ITEMS)
            .map_err(internal)?;
        let items = items
            .into_iter()
            .map(|item| ThreadItem {
                content: item.content.summary(),
                ..item
            })
            .collect();
        Ok(TranscriptPage { items, has_older })
    }

    /// Carries out a command from the UI. Git work it needs, such as creating a worktree,
    /// runs outside the lock.
    pub fn dispatch(&self, command: Command) -> Result<(), CommandError> {
        let command = match command {
            Command::ThreadCreate {
                workspace: ThreadWorkspace::NewWorktree | ThreadWorkspace::BranchWorktree { .. },
                ..
            } => return self.create_worktree_thread(command),
            Command::MessageSend { .. } => match self.restore_before_sending(command)? {
                Some(command) => command,
                None => return Ok(()),
            },
            other => other,
        };
        let mut state = lock(&self.state);
        let deleted: Vec<String> = state
            .threads_deleted_by(&command)
            .into_iter()
            .map(|thread| thread.id.clone())
            .collect();
        let stopped = match &command {
            Command::BackgroundTaskStop { thread_id, task_id } => state
                .sole_background_command(thread_id, task_id)
                .map(|since| (thread_id.clone(), task_id.clone(), since)),
            _ => None,
        };
        let sent_to = match &command {
            Command::MessageSend {
                thread_id,
                message_id,
                ..
            } => Some((thread_id.clone(), message_id.clone())),
            _ => None,
        };
        if let Command::TurnInterrupt { thread_id } = &command
            && state.hold_for_stop(thread_id)
            && state
                .model
                .thread(thread_id)
                .is_some_and(|thread| thread.active_turn_id.is_none())
        {
            return Ok(());
        }
        state.run_command(command)?;
        if let Some((thread_id, message_id)) = sent_to {
            state.release_queue_after(&thread_id, Some(&message_id));
        }
        let model = &state.model;
        self.terminals
            .retain_threads(|thread_id| model.thread(thread_id).is_some());
        drop(state);
        if let Some((thread_id, task_id, since)) = stopped {
            self.end_servers_of(thread_id, task_id, since);
        }
        if !deleted.is_empty() {
            let browsing = self.browsing.clone();
            thread::spawn(move || {
                for thread_id in &deleted {
                    browsing.forget(thread_id);
                }
            });
        }
        Ok(())
    }

    /// Creates a new thread's worktree outside the lock, then the thread in it. Messages
    /// sent to the thread meanwhile wait for it.
    fn create_worktree_thread(&self, command: Command) -> Result<(), CommandError> {
        let Command::ThreadCreate {
            thread_id,
            project_id,
            title,
            workspace,
            provider,
            model,
            effort,
            fast_mode,
            ultracode,
            access_mode,
            plan_mode,
        } = command
        else {
            return self.dispatch(command);
        };
        let existing = match workspace {
            ThreadWorkspace::BranchWorktree { branch } => Some(branch),
            _ => None,
        };
        let (op, plan) = {
            let mut state = lock(&self.state);
            let Some(project) = project_id.as_deref() else {
                return Err(CommandError::ChatNotLocal { thread_id });
            };
            let plan = state.plan_worktree(&thread_id, project, existing.as_deref())?;
            let path = plan.path.to_string_lossy().into_owned();
            let op = state.begin_op(&thread_id, &[&path])?;
            (op, plan)
        };
        let created = plan.create();
        let mut state = lock(&self.state);
        let workspace = match created {
            Ok(workspace) => workspace,
            Err(error) => {
                state.end_op(op, None)?;
                return Err(error);
            }
        };
        let create = Command::ThreadCreate {
            thread_id: thread_id.clone(),
            project_id,
            title,
            workspace,
            provider,
            model,
            effort,
            fast_mode,
            ultracode,
            access_mode,
            plan_mode,
        };
        let result = state.run_prepared(create);
        match &result {
            Ok(()) => state.start_worktree_setup(&thread_id),
            Err(_) => {
                thread::spawn(move || plan.discard());
            }
        }
        state.end_op(op, None)?;
        result
    }

    /// Creates again outside the lock the worktree a message's released thread had. `None`
    /// when the message went, or waits for git work on its thread.
    fn restore_before_sending(&self, command: Command) -> Result<Option<Command>, CommandError> {
        let Command::MessageSend { thread_id, .. } = &command else {
            return Ok(Some(command));
        };
        let thread_id = thread_id.clone();
        let (op, root, path, branch) = {
            let mut state = lock(&self.state);
            let restore = state
                .model
                .thread(&thread_id)
                .filter(|_| !state.thread_ops.contains_key(&thread_id))
                .filter(|thread| thread.worktree_released && !thread.worktree_adopted)
                .and_then(|thread| {
                    let path = thread.worktree_path.clone()?;
                    let missing = !Path::new(&path).is_dir()
                        && !state.folder_ops.contains(&git::normalize(&path));
                    missing.then(|| (thread.clone(), path))
                });
            let Some((thread, path)) = restore else {
                return Ok(Some(command));
            };
            let branch = thread
                .branch
                .clone()
                .ok_or_else(|| CommandError::WorktreeFailed {
                    detail: "the thread has no branch to check out".into(),
                })?;
            let root = state.project_root(&thread)?;
            let op = state.begin_op(&thread_id, &[&path])?;
            (op, root, path, branch)
        };
        let restored = git::restore_worktree(&root, &path, &branch);
        let mut state = lock(&self.state);
        match restored {
            Ok(()) => {
                state.start_worktree_setup(&thread_id);
                state.end_op(op, Some(command))?;
            }
            Err(detail) => {
                state.end_op(op, None)?;
                return Err(CommandError::WorktreeFailed { detail });
            }
        }
        Ok(None)
    }

    fn browsing(&self) -> &Browsing {
        &self.browsing
    }

    /// The UI shows this thread's transcript, `None` for none: only its text streams to it.
    pub fn stream_thread(&self, thread_id: Option<String>) {
        self.stream_threads(thread_id.into_iter().collect());
    }

    /// The UI holds these threads' transcripts, shown or kept for a quick return: only their
    /// text streams to it.
    pub fn stream_threads(&self, thread_ids: Vec<String>) {
        lock(&self.state).streamed = thread_ids;
    }

    /// The panel shows the thread's browser page, which then streams to it, or stopped.
    pub fn browser_view(&self, thread_id: &str, viewing: bool) -> Result<(), CommandError> {
        self.browsing()
            .view(thread_id, viewing)
            .map_err(|detail| CommandError::Internal { detail })
    }

    pub fn browser_open(&self, thread_id: &str, url: &str) -> Result<(), CommandError> {
        self.browsing()
            .user_open(thread_id, url)
            .map_err(|detail| CommandError::BrowserFailed { detail })
    }

    /// Back (`-1`), forward (`1`) or reload (`0`).
    pub fn browser_go(&self, thread_id: &str, delta: i32) -> Result<(), CommandError> {
        self.browsing()
            .user_go(thread_id, delta)
            .map_err(|detail| CommandError::BrowserFailed { detail })
    }

    pub fn browser_resize(
        &self,
        thread_id: &str,
        width: u32,
        height: u32,
    ) -> Result<(), CommandError> {
        self.browsing()
            .user_resize(thread_id, (width, height))
            .map_err(|detail| CommandError::BrowserFailed { detail })
    }

    /// The panel is `width` × `height` and the page fills it.
    pub fn browser_fit(
        &self,
        thread_id: &str,
        width: u32,
        height: u32,
    ) -> Result<(), CommandError> {
        self.browsing()
            .user_fit(thread_id, (width, height))
            .map_err(|detail| CommandError::BrowserFailed { detail })
    }

    pub fn browser_input(&self, thread_id: &str, input: &BrowserInput) -> Result<(), CommandError> {
        self.browsing()
            .user_input(thread_id, input)
            .map_err(|detail| CommandError::BrowserFailed { detail })
    }

    /// The screen showing the thread's page has `scale` device pixels per pixel, which the
    /// page then paints at.
    pub fn browser_scale(&self, thread_id: &str, scale: f64) -> Result<(), CommandError> {
        self.browsing()
            .user_scale(thread_id, scale)
            .map_err(|detail| CommandError::BrowserFailed { detail })
    }

    /// Runs `expression` in the thread's open page, awaiting a promise, and gives its value.
    pub fn browser_evaluate(
        &self,
        thread_id: &str,
        expression: &str,
    ) -> Result<serde_json::Value, CommandError> {
        self.browsing()
            .user_evaluate(thread_id, expression)
            .map_err(|detail| CommandError::BrowserFailed { detail })
    }

    /// The user closed the thread's page from the panel.
    pub fn browser_close(&self, thread_id: &str) {
        self.browsing().user_close(thread_id);
    }

    /// Closes every page and the headless browser, as when the app quits.
    /// Ends the agent processes kept between turns, and the commands running in them, as
    /// when the app quits.
    /// Ends the agent processes kept between turns, as hoshi quits.
    pub fn close_lingering_agents(&self) {
        let mut state = lock(&self.state);
        state.recovery.quitting = true;
        state.close_all_lingering();
    }

    /// Ends every agent process, those running turns and those kept between turns, and
    /// everything they started, as when the app quits. Waits a moment for them to end.
    pub fn end_agents(&self) {
        {
            let mut state = lock(&self.state);
            state.recovery.quitting = true;
            state.close_all_lingering();
            for turn in state.turns.values() {
                turn.running.close();
            }
        }
        hoshi_providers::agent_processes::end_all();
    }

    pub fn close_browser(&self) {
        let browsing = self.browsing.clone();
        let (closed, done) = mpsc::channel();
        thread::spawn(move || {
            browsing.close();
            let _ = closed.send(());
        });
        let _ = done.recv_timeout(BROWSER_CLOSE_WAIT);
    }

    /// Compacts the thread's conversation with `/compact`, then sends the message, as when
    /// the user comes back to a long thread whose prompt cache ran out. A thread whose turn
    /// runs takes the message at once instead.
    pub fn compact_then_send(
        &self,
        thread_id: &str,
        message_id: &str,
        text: String,
        attachments: Vec<MessageAttachment>,
    ) -> Result<(), CommandError> {
        let mut state = lock(&self.state);
        let busy = state
            .model
            .thread(thread_id)
            .ok_or_else(|| CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            })?
            .active_turn_id
            .is_some();
        let send = Command::MessageSend {
            thread_id: thread_id.to_owned(),
            message_id: message_id.to_owned(),
            text: text.clone(),
            attachments: attachments.clone(),
        };
        if busy {
            state.run_command(send)?;
            state.release_queue_after(thread_id, Some(message_id));
            return Ok(());
        }
        state.hold_until_compacted(
            thread_id,
            QueuedMessage {
                id: message_id.to_owned(),
                text,
                attachments,
                from_user: true,
                held: false,
            },
        );
        let compacted = state.run_command(Command::MessageSend {
            thread_id: thread_id.to_owned(),
            message_id: delegation::new_id(),
            text: COMPACT_COMMAND.to_owned(),
            attachments: Vec::new(),
        });
        if compacted.is_err()
            && state
                .after_compaction
                .get(thread_id)
                .is_some_and(|message| message.id == message_id)
        {
            state.take_held(thread_id);
        }
        compacted
    }

    /// Stops compacting the thread, so the message it holds goes at once.
    pub fn send_held_message_now(&self, thread_id: &str) -> Result<(), CommandError> {
        let mut state = lock(&self.state);
        if !state.after_compaction.contains_key(thread_id) {
            return Ok(());
        }
        // A restart ended the compaction it waited for.
        if !state.turns.contains_key(thread_id) {
            state.send_after_compaction(thread_id);
            return Ok(());
        }
        state.run_command(Command::TurnInterrupt {
            thread_id: thread_id.to_owned(),
        })
    }

    /// Takes back the message the thread holds until it is compacted, for the user to edit
    /// it. `discard` drops it, with the images it carried.
    pub fn take_held_message(&self, thread_id: &str, discard: bool) -> Option<HeldMessage> {
        let mut state = lock(&self.state);
        let message = state.take_held(thread_id)?;
        if discard {
            for attachment in &message.attachments {
                state.attachments.remove(attachment);
            }
        }
        Some(held_message(thread_id, &message))
    }

    /// Saves an image the user is about to send. `base64` holds its bytes.
    pub fn save_image(
        &self,
        name: &str,
        mime_type: &str,
        base64: &str,
    ) -> Result<MessageAttachment, CommandError> {
        lock(&self.state)
            .attachments
            .save_image(name, mime_type, base64)
            .map_err(|detail| CommandError::Internal { detail })
    }

    /// Saves a file the user is about to send, and the text of a document beside it.
    pub fn save_file(
        &self,
        name: &str,
        mime_type: &str,
        bytes: &[u8],
    ) -> Result<MessageAttachment, CommandError> {
        let attachments = lock(&self.state).attachments.clone();
        attachments
            .save_file(name, mime_type, bytes)
            .map_err(|detail| CommandError::Internal { detail })
    }

    /// An image or file the user removed before sending it.
    pub fn discard_attachment(&self, attachment: &MessageAttachment) {
        lock(&self.state).attachments.remove(attachment);
    }

    /// Where a saved image or file is, to open it; `None` when it is gone.
    pub fn attachment_path(&self, attachment: &MessageAttachment) -> Option<String> {
        let path = lock(&self.state).attachments.path_of(attachment)?;
        path.is_file().then(|| path.to_string_lossy().into_owned())
    }

    /// An image on this machine an agent's answer shows by its absolute path, as a data URL;
    /// `None` when it isn't there or isn't an image.
    pub fn local_image(&self, path: &str) -> Option<String> {
        html_pages::image_data_url(Path::new(path))
    }

    /// Opens a file an agent's answer links to by its absolute path: in `editor`, or else
    /// in the app the system has for it.
    pub fn open_local_file(&self, path: &str, editor: Option<Editor>) -> Result<(), CommandError> {
        let file = Path::new(path);
        if !file.is_absolute() || !file.exists() {
            return Err(CommandError::Internal {
                detail: format!("{path} isn't there"),
            });
        }
        editors::open(editor.unwrap_or(Editor::FileManager), path)
            .map_err(|detail| CommandError::Internal { detail })
    }

    /// A page an agent showed, by the id `html_render` gave it; `None` when it is gone.
    pub fn html_page(&self, id: &str) -> Option<Vec<u8>> {
        let pages = lock(&self.state).pages.clone();
        pages.read(id)
    }

    /// The file of a page an agent showed; `None` when it is gone.
    pub fn html_page_path(&self, id: &str) -> Option<PathBuf> {
        let pages = lock(&self.state).pages.clone();
        pages.path(id).filter(|path| path.is_file())
    }

    /// A sent image as a data URL; `None` when its file is gone.
    pub fn image_data_url(&self, id: &str, mime_type: &str) -> Option<String> {
        let path = lock(&self.state).attachments.path(id, mime_type)?;
        attachments::data_url(&path, mime_type)
    }

    /// Pull requests of the project's repository matching `query`, for `#` mentions.
    /// Asks the forge.
    pub fn search_pull_requests(
        &self,
        project_id: &str,
        query: &str,
    ) -> Result<Vec<PullRequest>, CommandError> {
        let (forge, repository) = {
            let state = lock(&self.state);
            let repository = state
                .repositories
                .get(project_id)
                .ok_or(CommandError::ForgeUnavailable)?
                .clone();
            let forge = state
                .forges
                .get(&repository.forge)
                .ok_or(CommandError::ForgeUnavailable)?
                .clone();
            (forge, repository)
        };
        forge
            .search_pull_requests(&repository, query)
            .map_err(forge_error)
    }

    /// Shows a thread's terminal through `sink`, starting its shell in the thread's
    /// folder (its worktree when it has one) if it is not running.
    pub fn open_terminal(
        &self,
        thread_id: &str,
        terminal_id: &str,
        size: (u16, u16),
        sink: TerminalSink,
    ) -> Result<TerminalAttach, CommandError> {
        let (cwd, project_root, worktree_path) = {
            let state = lock(&self.state);
            let not_found = || CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            };
            let thread = state.model.thread(thread_id).ok_or_else(not_found)?;
            let cwd = state.cwd(thread_id).ok_or_else(not_found)?;
            let project_root = thread
                .project_id
                .as_deref()
                .and_then(|project_id| state.model.project(project_id))
                .map_or_else(|| cwd.clone(), |project| project.workspace_root.clone());
            (cwd, project_root, thread.worktree_path.clone())
        };
        let launch = Launch {
            thread_id,
            cwd: &cwd,
            project_root: &project_root,
            worktree_path: worktree_path.as_deref(),
            command: None,
        };
        self.terminals.open(terminal_id, launch, size, sink)
    }

    /// Runs `command` in a shell in the home folder, shown through `sink` like a thread's
    /// terminal; the shell exits with the command.
    pub fn run_in_terminal(
        &self,
        terminal_id: &str,
        command: &str,
        size: (u16, u16),
        sink: TerminalSink,
    ) -> Result<TerminalAttach, CommandError> {
        let home = std::env::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .to_string_lossy()
            .into_owned();
        let launch = Launch {
            thread_id: "",
            cwd: &home,
            project_root: &home,
            worktree_path: None,
            command: Some(command),
        };
        self.terminals.open(terminal_id, launch, size, sink)
    }

    /// The folders the enabled agents ran sessions in, newest first, read from their own
    /// history. Reads many files, so it takes a moment.
    pub fn recent_folders(&self) -> Vec<RecentFolder> {
        let (adapters, data_dir, roots) = {
            let state = lock(&self.state);
            let data_dir = state
                .worktrees_dir
                .parent()
                .map_or_else(|| state.worktrees_dir.clone(), Path::to_path_buf);
            let roots: Vec<String> = state
                .model
                .projects()
                .map(|project| project.workspace_root.clone())
                .collect();
            (state.adapters.clone(), data_dir, roots)
        };
        let found = thread::scope(|scope| {
            let scans: Vec<_> = adapters
                .iter()
                .map(|adapter| {
                    scope.spawn(|| (adapter.provider().to_owned(), adapter.session_folders()))
                })
                .collect();
            scans
                .into_iter()
                .filter_map(|scan| scan.join().ok())
                .collect()
        });
        let home = std::env::home_dir();
        recent_folders::gather(
            found,
            &recent_folders::Excluded::new(home.as_deref(), &data_dir, &std::env::temp_dir()),
            &roots,
        )
    }

    /// Every worktree setup since the app started, running or ended.
    pub fn worktree_setups(&self) -> Vec<hoshi_core::WorktreeSetupRun> {
        lock(&self.state).setups.runs()
    }

    /// Runs the thread's worktree setup again, with the project's command as it is now.
    pub fn rerun_worktree_setup(&self, thread_id: &str) -> Result<(), CommandError> {
        let state = lock(&self.state);
        let launch = state
            .setup_launch(thread_id)
            .or_else(|| state.setups.launch(thread_id))
            .ok_or_else(|| CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            })?;
        state.run_worktree_setup(launch);
        Ok(())
    }

    /// Lets the agent start without the thread's setup, sending what it held.
    pub fn skip_worktree_setup(&self, thread_id: &str) -> Result<(), CommandError> {
        let mut state = lock(&self.state);
        for command in state.setups.stop_waiting(thread_id) {
            state.run_command(command)?;
        }
        Ok(())
    }

    pub fn write_terminal(&self, terminal_id: &str, data: &str) -> Result<(), CommandError> {
        self.terminals.write(terminal_id, data)
    }

    pub fn resize_terminal(&self, terminal_id: &str, size: (u16, u16)) -> Result<(), CommandError> {
        self.terminals.resize(terminal_id, size)
    }

    pub fn detach_terminal(&self, terminal_id: &str) {
        self.terminals.detach(terminal_id);
    }

    pub fn close_terminal(&self, terminal_id: &str) {
        self.terminals.close(terminal_id);
    }

    /// The thread's servers now; while watched, changes arrive as notifications.
    pub fn watch_servers(&self, thread_id: &str) -> Vec<hoshi_core::LocalServer> {
        self.servers.watch(thread_id)
    }

    pub fn unwatch_servers(&self, thread_id: &str) {
        self.servers.unwatch(thread_id);
    }

    /// Ends the server on `port` and what its process started; never hoshi, an agent or
    /// a shell.
    pub fn stop_server(&self, thread_id: &str, pid: u32, port: u16) -> Result<(), CommandError> {
        self.servers.stop(thread_id, pid, port)
    }

    /// The agent ends a command it is told to stop, but not always what the command
    /// started: a server run through a script keeps listening. Once the agent has taken the
    /// command as stopped, the servers it started since go too; ending them earlier would
    /// have the agent take the command as failed and go on by itself.
    fn end_servers_of(&self, thread_id: String, task_id: String, since: u64) {
        let (state, servers) = (Arc::downgrade(&self.state), self.servers.clone());
        thread::spawn(move || {
            let deadline = Instant::now() + STOPPED_WITHIN;
            loop {
                let Some(state) = state.upgrade() else {
                    return;
                };
                if !lock(&state).runs_background_command(&thread_id, &task_id) {
                    break;
                }
                if Instant::now() >= deadline {
                    return;
                }
                drop(state);
                thread::sleep(STOPPED_POLL);
            }
            servers.stop_agent_servers_since(&thread_id, since);
        });
    }
}

impl State {
    /// The adapter of `provider`, when it is enabled.
    fn adapter_for(&self, provider: &str) -> Option<Arc<dyn ProviderAdapter>> {
        self.adapters
            .iter()
            .find(|adapter| adapter.provider() == provider)
            .cloned()
    }

    /// Where the thread's agent runs. A chat's own folder is created when first asked for.
    fn cwd(&self, thread_id: &str) -> Option<String> {
        let thread = self.model.thread(thread_id)?;
        if thread.project_id.is_some() {
            return self.model.thread_cwd(thread_id).map(str::to_owned);
        }
        let folder = self.chat_folder(thread_id)?;
        std::fs::create_dir_all(&folder).ok()?;
        Some(folder.to_string_lossy().into_owned())
    }

    /// `None` for ids that could reach outside the chats folder.
    fn chat_folder(&self, thread_id: &str) -> Option<PathBuf> {
        let safe = !thread_id.is_empty()
            && thread_id
                .chars()
                .all(|char| char.is_ascii_alphanumeric() || char == '-');
        safe.then(|| self.chats_dir.join(thread_id))
    }

    /// Does the I/O a command needs before the core can decide it: resolves a
    /// project's folder, and creates a new thread's worktree.
    fn prepare(&self, command: Command) -> Result<Command, CommandError> {
        match command {
            Command::ProjectCreate {
                project_id,
                title,
                workspace_root,
            } => {
                let resolved = dunce::canonicalize(&workspace_root)
                    .ok()
                    .filter(|path| path.is_dir())
                    .ok_or_else(|| CommandError::WorkspaceRootNotFound {
                        path: workspace_root.clone(),
                    })?;
                Ok(Command::ProjectCreate {
                    project_id,
                    title,
                    workspace_root: resolved.to_string_lossy().into_owned(),
                })
            }
            Command::ThreadCreate {
                workspace: ThreadWorkspace::Worktree { .. },
                ..
            } => Err(CommandError::Internal {
                detail: "only the engine creates worktrees".into(),
            }),
            Command::ThreadWorktreeRelease { .. } | Command::ThreadCleanUp { .. } => {
                Err(CommandError::Internal {
                    detail: "worktrees and branches are removed through the engine's own calls"
                        .into(),
                })
            }
            Command::MessageSend { ref thread_id, .. } => {
                let switching = self.model.thread(thread_id).is_some_and(|thread| {
                    thread.active_turn_id.is_none() && thread.pending_model.is_some()
                });
                if switching {
                    self.refuse_ending_background_work(thread_id)?;
                }
                self.restore_released_worktree(thread_id)?;
                Ok(command)
            }
            Command::ThreadSnooze { ref thread_id, .. } if self.holds_user_message(thread_id) => {
                Err(CommandError::MessageQueued {
                    thread_id: thread_id.clone(),
                })
            }
            Command::ThreadModelSet {
                ref thread_id,
                ref provider,
                ..
            } => {
                if self
                    .model
                    .thread(thread_id)
                    .is_some_and(|thread| thread.provider != *provider)
                {
                    self.refuse_ending_background_work(thread_id)?;
                }
                Ok(command)
            }
            Command::ThreadCreate {
                thread_id,
                project_id,
                title,
                workspace:
                    workspace @ (ThreadWorkspace::NewWorktree | ThreadWorkspace::BranchWorktree { .. }),
                provider,
                model,
                effort,
                fast_mode,
                ultracode,
                access_mode,
                plan_mode,
            } => {
                let Some(project) = project_id.as_deref() else {
                    return Err(CommandError::ChatNotLocal { thread_id });
                };
                let existing = match workspace {
                    ThreadWorkspace::BranchWorktree { branch } => Some(branch),
                    _ => None,
                };
                let workspace = self.create_worktree(&thread_id, project, existing.as_deref())?;
                Ok(Command::ThreadCreate {
                    thread_id,
                    project_id,
                    title,
                    workspace,
                    provider,
                    model,
                    effort,
                    fast_mode,
                    ultracode,
                    access_mode,
                    plan_mode,
                })
            }
            other => Ok(other),
        }
    }

    /// A message the user sent waits for the thread: for its turn, its compaction, its git
    /// work or its worktree's setup.
    fn holds_user_message(&self, thread_id: &str) -> bool {
        self.queued
            .get(thread_id)
            .is_some_and(|queued| queued.iter().any(|message| message.from_user))
            || self
                .after_compaction
                .get(thread_id)
                .is_some_and(|message| message.from_user)
            || self
                .thread_ops
                .get(thread_id)
                .is_some_and(|held| !held.is_empty())
            || self.setups.holds(thread_id)
    }

    /// The thread's snooze is still ahead, whatever woke it since.
    fn snoozed(&self, thread_id: &str) -> bool {
        self.model
            .thread(thread_id)
            .is_some_and(|thread| snooze_pending(thread, Utc::now()))
    }

    /// [`Self::snoozed`] at `now`, in Unix milliseconds.
    fn snoozed_at_millis(&self, thread_id: &str, now: u64) -> bool {
        let now = i64::try_from(now)
            .ok()
            .and_then(chrono::DateTime::from_timestamp_millis);
        self.model
            .thread(thread_id)
            .zip(now)
            .is_some_and(|(thread, now)| snooze_pending(thread, now))
    }

    /// Refuses what would close the process the thread's completed turn kept while it still
    /// runs commands in the background.
    fn refuse_ending_background_work(&self, thread_id: &str) -> Result<(), CommandError> {
        if self
            .lingering
            .get(thread_id)
            .is_some_and(background::Lingering::runs_background_work)
        {
            return Err(CommandError::BackgroundWorkRunning {
                thread_id: thread_id.to_owned(),
            });
        }
        Ok(())
    }

    /// Creates again the worktree a released thread had, before a message runs in it.
    /// Left to the git work that runs in its folder, if any, which starts the turn after.
    fn restore_released_worktree(&self, thread_id: &str) -> Result<(), CommandError> {
        let released = self.model.thread(thread_id).is_some_and(|thread| {
            thread.worktree_released
                && !thread
                    .worktree_path
                    .as_deref()
                    .is_some_and(|path| self.folder_ops.contains(&git::normalize(path)))
        });
        if !released {
            return Ok(());
        }
        self.restore_worktree(thread_id)
    }

    /// Creates the thread's worktree again from its branch when its folder is gone.
    fn restore_worktree(&self, thread_id: &str) -> Result<(), CommandError> {
        let Some(thread) = self.model.thread(thread_id) else {
            return Ok(());
        };
        let Some(path) = thread
            .worktree_path
            .as_deref()
            .filter(|_| !thread.worktree_adopted)
        else {
            return Ok(());
        };
        if Path::new(path).is_dir() {
            return Ok(());
        }
        let failed = |detail: String| CommandError::WorktreeFailed { detail };
        let branch = thread
            .branch
            .as_deref()
            .ok_or_else(|| failed("the thread has no branch to check out".into()))?;
        let root = self.project_root(thread)?;
        git::restore_worktree(&root, path, branch).map_err(failed)
    }

    /// Decides `command` and commits its events, for work the engine did itself first.
    fn decide_and_commit(&mut self, command: Command) -> Result<(), CommandError> {
        let now = now();
        let events = decide(&self.model, command, &now)?;
        self.commit(&events, &now)
    }

    /// The thread, its project root, and whether it is free to have its workspace removed.
    fn removable(&self, thread_id: &str) -> Result<(&hoshi_core::Thread, String), CommandError> {
        let thread = self
            .model
            .thread(thread_id)
            .ok_or_else(|| CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            })?;
        if thread.active_turn_id.is_some() {
            return Err(CommandError::ThreadBusy {
                thread_id: thread_id.to_owned(),
            });
        }
        if thread.worktree_adopted {
            return Err(CommandError::WorktreeAdopted {
                thread_id: thread_id.to_owned(),
            });
        }
        if let Some(other) = self.sharing_worktree(thread, &[]) {
            return Err(CommandError::WorktreeShared {
                thread_id: thread_id.to_owned(),
                other_thread_id: other.to_owned(),
            });
        }
        let root = self.project_root(thread)?;
        Ok((thread, root))
    }

    /// Another thread that works in `thread`'s worktree as its own, such as a fork, besides
    /// those in `leaving`.
    fn sharing_worktree(&self, thread: &hoshi_core::Thread, leaving: &[String]) -> Option<&str> {
        let path = git::normalize(thread.worktree_path.as_deref()?);
        self.model
            .threads()
            .filter(|other| other.id != thread.id && !other.worktree_adopted)
            .filter(|other| !leaving.contains(&other.id))
            .find(|other| {
                other
                    .worktree_path
                    .as_deref()
                    .is_some_and(|other| git::normalize(other) == path)
            })
            .map(|other| other.id.as_str())
    }

    /// The other threads that run in `thread`'s project root, and so change branch with it.
    /// Refused while one of them works.
    fn root_threads_sharing(
        &self,
        thread: &hoshi_core::Thread,
    ) -> Result<Vec<String>, CommandError> {
        let sharing = self.model.threads().filter(|other| {
            other.id != thread.id
                && other.project_id == thread.project_id
                && other.worktree_path.is_none()
        });
        let mut ids = Vec::new();
        for other in sharing {
            if other.active_turn_id.is_some() {
                return Err(CommandError::ThreadBusy {
                    thread_id: other.id.clone(),
                });
            }
            ids.push(other.id.clone());
        }
        Ok(ids)
    }

    /// A thread other than `thread_id` whose turn runs in `folder`.
    fn working_in(&self, folder: &str, thread_id: &str) -> Option<String> {
        let folder = git::normalize(folder);
        self.model
            .threads()
            .filter(|other| other.id != thread_id && other.active_turn_id.is_some())
            .find(|other| {
                self.model
                    .thread_cwd(&other.id)
                    .is_some_and(|cwd| git::normalize(cwd) == folder)
            })
            .map(|other| other.id.clone())
    }

    /// The root of the thread's project; a chat has none, so it has no worktree either.
    fn project_root(&self, thread: &hoshi_core::Thread) -> Result<String, CommandError> {
        let project_id =
            thread
                .project_id
                .as_deref()
                .ok_or_else(|| CommandError::NothingToClean {
                    thread_id: thread.id.clone(),
                })?;
        self.model
            .project(project_id)
            .map(|project| project.workspace_root.clone())
            .ok_or_else(|| CommandError::ProjectNotFound {
                project_id: project_id.to_owned(),
            })
    }

    /// Removes a worktree folder, refusing when that would lose uncommitted work. A folder
    /// already gone only leaves git's record of it, which is pruned.
    fn remove_clean_worktree(root: &str, path: &str, thread_id: &str) -> Result<(), CommandError> {
        if !Path::new(path).is_dir() {
            git::prune_worktrees(root);
            return Ok(());
        }
        let git_failed = |detail: String| CommandError::GitFailed { detail };
        if !git::is_clean(path).map_err(git_failed)? {
            return Err(CommandError::WorktreeDirty {
                thread_id: thread_id.to_owned(),
            });
        }
        if let Some(reason) = git::lock_reason(root, path) {
            // Claude Code locks the worktrees it enters and leaves the lock behind.
            if !worktree::is_stale_agent_lock(&reason) {
                return Err(CommandError::WorktreeLocked {
                    thread_id: thread_id.to_owned(),
                    reason,
                });
            }
            git::unlock_worktree(root, path).map_err(git_failed)?;
        }
        git::remove_worktree(root, path).map_err(git_failed)
    }

    /// A worktree for a new thread: on a new `hoshi/<id>` branch, or on `existing`.
    fn create_worktree(
        &self,
        thread_id: &str,
        project_id: &str,
        existing: Option<&str>,
    ) -> Result<ThreadWorkspace, CommandError> {
        self.plan_worktree(thread_id, project_id, existing)?
            .create()
    }

    /// Where a new thread's worktree goes; [`WorktreePlan::create`] makes it.
    fn plan_worktree(
        &self,
        thread_id: &str,
        project_id: &str,
        existing: Option<&str>,
    ) -> Result<WorktreePlan, CommandError> {
        let project =
            self.model
                .project(project_id)
                .ok_or_else(|| CommandError::ProjectNotFound {
                    project_id: project_id.to_owned(),
                })?;
        let root = project.workspace_root.clone();
        if !git::is_repository(&root) {
            return Err(CommandError::NotAGitRepository { path: root });
        }
        let short_id: String = thread_id.chars().take(8).collect();
        let branch = existing.map_or_else(|| format!("hoshi/{short_id}"), str::to_owned);
        let folder = format!(
            "{}-{}",
            slug(&project.title),
            project_id.chars().take(8).collect::<String>()
        );
        Ok(WorktreePlan {
            path: self.worktrees_dir.join(folder).join(&short_id),
            root,
            branch,
            existing: existing.is_some(),
        })
    }

    /// Worktrees (with their repository) that a deletion leaves without a thread.
    /// The images and files in the transcripts of the threads `command` deletes.
    fn images_freed_by(&self, command: &Command) -> Vec<MessageAttachment> {
        self.threads_deleted_by(command)
            .into_iter()
            .flat_map(|thread| self.store.thread_items(&thread.id).unwrap_or_default())
            .flat_map(|item| match item.content {
                ItemContent::UserMessage { attachments, .. } => attachments,
                _ => Vec::new(),
            })
            .filter(|attachment| {
                matches!(
                    attachment,
                    MessageAttachment::Image { .. } | MessageAttachment::File { .. }
                )
            })
            .collect()
    }

    fn threads_deleted_by(&self, command: &Command) -> Vec<&hoshi_core::Thread> {
        match command {
            Command::ThreadDelete { thread_id } => {
                self.model.thread(thread_id).into_iter().collect()
            }
            Command::ProjectRemove { project_id } => self
                .model
                .threads()
                .filter(|thread| thread.project_id.as_ref() == Some(project_id))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn worktrees_freed_by(&self, command: &Command) -> Vec<(String, String)> {
        let deleted: Vec<String> = self
            .threads_deleted_by(command)
            .into_iter()
            .map(|thread| thread.id.clone())
            .collect();
        self.threads_deleted_by(command)
            .into_iter()
            .filter(|thread| !thread.worktree_adopted)
            .filter(|thread| self.sharing_worktree(thread, &deleted).is_none())
            .filter_map(|thread| {
                let path = thread.worktree_path.clone()?;
                let project = self.model.project(thread.project_id.as_deref()?)?;
                Some((project.workspace_root.clone(), path))
            })
            .fold(Vec::new(), |mut freed, (root, path)| {
                if !freed.iter().any(|(_, kept): &(String, String)| {
                    git::normalize(kept) == git::normalize(&path)
                }) {
                    freed.push((root, path));
                }
                freed
            })
    }

    fn commit(&mut self, events: &[Event], now: &str) -> Result<(), CommandError> {
        if events.is_empty() {
            return Ok(());
        }
        let resolved = self.with_models_named(events);
        let events = resolved.as_deref().unwrap_or(events);
        let sequences = self.store.append(events, now).map_err(internal)?;
        for (sequence, event) in sequences.into_iter().zip(events) {
            self.model.apply(event);
            self.sequence = sequence;
            (self.notify)(Notification::Event(EventEnvelope {
                sequence,
                event: event.clone(),
            }));
        }
        self.close_lingering_after(events);
        self.recovery_after(events);
        Ok(())
    }

    /// `events` with what each turn runs on as the provider's catalog names it where it stood
    /// for its defaults, so a change of default later doesn't rewrite what a turn ran on; a
    /// change that turns out to be none goes. A rewind and a fork get what the turn before
    /// their point ran on. `None` when nothing needed it.
    fn with_models_named(&self, events: &[Event]) -> Option<Vec<Event>> {
        let concerned = events.iter().any(|event| match event {
            Event::TurnStarted { model, .. } => model.is_some(),
            Event::ThreadRewound { .. } => true,
            Event::ThreadCreated { thread } => thread.forked_from.is_some(),
            Event::ItemAdded { item } => matches!(
                item.content,
                ItemContent::UserMessage {
                    model_switch: Some(_),
                    ..
                } | ItemContent::ModelSwitched { .. }
            ),
            _ => false,
        });
        if !concerned {
            return None;
        }
        let mut catalogs: HashMap<String, Option<ProviderModels>> = HashMap::new();
        // What a turn ran on is kept as recorded; only what it left to the defaults is named.
        let mut name = |choice: &mut ModelChoice, recorded: bool| {
            let catalog = catalogs
                .entry(choice.provider.clone())
                .or_insert_with(|| self.known_models(&choice.provider));
            if let Some(catalog) = catalog {
                name_choice(choice, catalog, recorded);
            }
        };
        let same = |from: &ModelChoice, to: &ModelChoice| {
            from.model == to.model && from.effort == to.effort
        };
        // A fork's history before its first own turn is its source's, up to the fork's turn.
        let before = |thread_id: &str, item_id: &str| {
            let (mut thread_id, mut item_id) = (thread_id.to_owned(), item_id.to_owned());
            for _ in 0..MAX_FORK_DEPTH {
                match self.store.turn_model_before(&thread_id, &item_id) {
                    Ok(Some(model)) => return Some(model),
                    Ok(None) => {}
                    Err(error) => {
                        eprintln!("hoshi: could not read what a turn ran on: {error}");
                        return None;
                    }
                }
                let origin = self.model.thread(&thread_id)?.forked_from.as_deref()?;
                thread_id.clone_from(&origin.thread_id);
                item_id.clone_from(&origin.turn.through_item_id);
            }
            None
        };
        let mut events = events.to_vec();
        for event in &mut events {
            match event {
                Event::TurnStarted {
                    model: Some(model), ..
                } => name(model, false),
                Event::ThreadRewound {
                    thread_id,
                    message_id,
                    last_turn_model,
                    ..
                } => *last_turn_model = before(thread_id, message_id),
                Event::ThreadCreated { thread } => {
                    if let Some(origin) = &thread.forked_from {
                        thread.last_turn_model =
                            before(&origin.thread_id, &origin.turn.through_item_id).map(Box::new);
                    }
                }
                Event::ItemAdded { item } => match &mut item.content {
                    ItemContent::UserMessage { model_switch, .. } => {
                        if let Some(switch) = model_switch {
                            name(&mut switch.from, true);
                            name(&mut switch.to, false);
                            if same(&switch.from, &switch.to) {
                                *model_switch = None;
                            }
                        }
                    }
                    ItemContent::ModelSwitched { from, to } => {
                        name(from, true);
                        name(to, false);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        events.retain(|event| {
            !matches!(
                event,
                Event::ItemAdded {
                    item: ThreadItem {
                        content: ItemContent::ModelSwitched { from, to },
                        ..
                    },
                } if same(from, to)
            )
        });
        Some(events)
    }

    /// The models `provider` offers as last listed, without asking its CLI.
    fn known_models(&self, provider: &str) -> Option<ProviderModels> {
        let adapter = self.adapter_for(provider)?;
        let kept = provider_cache::read(&self.provider_cache_dir, provider);
        provider_cache::hydrate(adapter.offline_models(), kept)
    }

    fn receive(&mut self, thread_id: &str, turn_id: &str, output: ProviderOutput) {
        if self.recovery.quitting {
            return;
        }
        let now = now();
        let settled = retry_settled(&self.model, thread_id, turn_id, &output, &now);
        if let Err(error) = self.commit(&settled, &now) {
            eprintln!("hoshi: failed to record a retry's end: {error}");
        }
        let (item_id, kind, delta) = match output {
            ProviderOutput::TextDelta { item_id, delta } => (item_id, StreamKind::Text, delta),
            ProviderOutput::ReasoningDelta { item_id, delta } => {
                (item_id, StreamKind::Reasoning, delta)
            }
            ProviderOutput::ToolOutput { item_id, output } => {
                (item_id, StreamKind::ToolOutput, output)
            }
            other => return self.record(thread_id, turn_id, other),
        };
        let shown = self.streamed.iter().any(|streamed| streamed == thread_id);
        let active = self
            .model
            .thread(thread_id)
            .is_some_and(|thread| thread.active_turn_id.as_deref() == Some(turn_id));
        if shown && active {
            (self.notify)(Notification::Stream(vec![StreamDelta {
                thread_id: thread_id.to_owned(),
                item_id,
                kind,
                delta,
            }]));
        }
    }

    fn record(&mut self, thread_id: &str, turn_id: &str, output: ProviderOutput) {
        match &output {
            ProviderOutput::ContextHandoffDelivered { injected } => {
                self.handoff_delivered(thread_id, turn_id, *injected);
                return;
            }
            ProviderOutput::SessionStarted { .. } => self.native_fork_started(thread_id, turn_id),
            ProviderOutput::TurnCompleted { outcome } => {
                self.handoff_turn_ended(thread_id, turn_id, outcome);
            }
            _ => {}
        }
        match output {
            ProviderOutput::ContextUsage(usage) => {
                self.record_context_usage(thread_id, turn_id, usage);
                return;
            }
            ProviderOutput::UsageLimits(update) => {
                let provider = self.model.thread(thread_id).map(|t| t.provider.clone());
                if let Some(provider) = provider {
                    let mut limits = self
                        .usage_limits
                        .get(&provider)
                        .cloned()
                        .unwrap_or_default();
                    limits.merge(update);
                    self.keep_usage_limits(&provider, limits);
                }
                return;
            }
            ProviderOutput::WorkingDirectory { path } => {
                self.follow_session(thread_id, turn_id, &path);
                return;
            }
            ProviderOutput::ConversationCursor { cursor } => {
                if let Some(turn) = self.turns.get_mut(thread_id)
                    && turn.id == turn_id
                {
                    turn.cursor = Some(cursor);
                }
                return;
            }
            ProviderOutput::BackgroundTasks { tasks, waiting } => {
                if let Some(turn) = self.turns.get_mut(thread_id)
                    && turn.id == turn_id
                {
                    let background = ThreadBackgroundTasks {
                        thread_id: thread_id.to_owned(),
                        tasks,
                        waiting,
                    };
                    turn.background = Some(background.clone());
                    (self.notify)(Notification::BackgroundTasks(background));
                    self.save_recovery(thread_id);
                }
                return;
            }
            _ => {}
        }
        if let ProviderOutput::ToolCallStarted { input, .. } = &output
            && let Some(turn) = self.turns.get_mut(thread_id)
            && turn.id == turn_id
            && let Some(workspace) = &mut turn.workspace
        {
            workspace.record_tool_input(input);
        }
        let ended = match &output {
            ProviderOutput::TurnCompleted { outcome } => Some(outcome.clone()),
            _ => None,
        };
        let completed = ended.is_some();
        let now = now();
        let ending = self
            .turns
            .get(thread_id)
            .filter(|turn| completed && turn.id == turn_id);
        let checkpointed =
            ending.and_then(|turn| Some((turn.checkpointed.clone()?, turn.arrivals)));
        if let Some(cursor) = ending.and_then(|turn| turn.cursor.clone()) {
            let events = record_conversation_cursor(&self.model, thread_id, cursor);
            if let Err(error) = self.commit(&events, &now) {
                eprintln!("hoshi: failed to record where the conversation is: {error}");
            }
        }
        if checkpointed.is_some() {
            // Holds the turn's place in the transcript while its changes are read.
            let pending = record_turn_changes(&self.model, thread_id, turn_id, None, &now);
            if let Err(error) = self.commit(&pending, &now) {
                eprintln!("hoshi: failed to record a turn's changes: {error}");
            }
        }
        let events = record_output(&self.model, thread_id, turn_id, output, &now);
        if let Err(error) = self.commit(&events, &now) {
            eprintln!("hoshi: failed to record provider output: {error}");
        }
        let current = !events.is_empty();
        if completed {
            let ended_turn = match self.turns.get(thread_id) {
                Some(turn) if turn.id == turn_id => self.turns.remove(thread_id),
                _ => None,
            };
            let compacts_when_idle = ended == Some(TurnOutcome::Completed)
                && ended_turn.as_ref().is_some_and(|turn| !turn.compacting);
            let workspace = ended_turn.and_then(|mut turn| {
                let workspace = turn.workspace.take();
                self.keep_between_turns(thread_id, turn);
                workspace
            });
            // The agent may have created a worktree or switched branches.
            let check = workspace
                .filter(|workspace| !workspace.moved)
                .map(|workspace| self.worktree_check(thread_id, turn_id, workspace));
            match checkpointed {
                Some((dir, arrivals)) => {
                    self.read_turn_changes(thread_id, turn_id, dir, arrivals, check);
                }
                None => refresh_after_turn(self.this.clone(), thread_id.to_owned(), check),
            }
            if self.titles_to_refine.remove(thread_id) {
                self.rename_from_conversation(thread_id, false);
            }
            if let Some(outcome) = &ended {
                if current {
                    self.turn_ended(thread_id, turn_id, outcome);
                }
                self.deliver_task_result(thread_id, outcome);
            }
            self.start_next_queued(thread_id);
            self.send_after_compaction(thread_id);
            self.deliver_pending_wakes(thread_id);
            if compacts_when_idle {
                self.schedule_idle_compaction(thread_id);
            }
        }
    }

    /// Checkpoints the tree a finished turn left, then records what changed since it began
    /// in the folder it ended in. A worktree the turn created and worked in becomes the
    /// thread's folder first, its changes counted from how it was checked out.
    fn read_turn_changes(
        &self,
        thread_id: &str,
        turn_id: &str,
        dir: String,
        arrivals: usize,
        check: Option<WorktreeCheck>,
    ) {
        let weak = self.this.clone();
        let (thread_id, turn_id) = (thread_id.to_owned(), turn_id.to_owned());
        self.checkpoints.queue(&dir.clone(), move || {
            let created = check
                .as_ref()
                .and_then(created_worktree)
                .filter(|(path, _)| *path != git::normalize(&dir));
            let (folder, arrivals) = match &created {
                Some((path, _)) => {
                    let reference =
                        checkpoint::reference(&thread_id, &turn_id, Moment::Arrived(arrivals + 1));
                    if let Err(error) = checkpoint::capture_created(path, &reference) {
                        eprintln!("hoshi: no checkpoint of the worktree {path}: {error}");
                    }
                    (path.clone(), arrivals + 1)
                }
                None => (dir, arrivals),
            };
            let files = changes_since_arrival(&folder, &thread_id, &turn_id, arrivals)
                .unwrap_or_else(|error| {
                    eprintln!("hoshi: no checkpoint after the turn in {folder}: {error}");
                    Vec::new()
                });
            thread::spawn(move || {
                let Some(shared) = weak.upgrade() else { return };
                let mut state = lock(&shared);
                let now = now();
                if let Some((path, branch)) = created {
                    let events = record_worktree(&state.model, &thread_id, path, branch, true);
                    if let Err(error) = state.commit(&events, &now) {
                        eprintln!("hoshi: failed to record a worktree: {error}");
                    }
                }
                let events =
                    record_turn_changes(&state.model, &thread_id, &turn_id, Some(files), &now);
                if let Err(error) = state.commit(&events, &now) {
                    eprintln!("hoshi: failed to record a turn's changes: {error}");
                }
                drop(state);
                refresh_after_turn(weak, thread_id, None);
            });
        });
    }

    /// Reads a running turn's changes in the folder its thread moved to from now on,
    /// counted from how that folder is now, or from how a worktree the turn created was
    /// checked out.
    fn follow_turn(&mut self, thread_id: &str, turn_id: &str, folder: &str, created: bool) {
        let Some(turn) = self
            .turns
            .get_mut(thread_id)
            .filter(|turn| turn.id == turn_id)
        else {
            return;
        };
        let same = |dir: &String| git::normalize(dir) == git::normalize(folder);
        if turn.checkpointed.as_ref().is_none_or(same) {
            return;
        }
        turn.arrivals += 1;
        turn.checkpointed = Some(folder.to_owned());
        let reference = checkpoint::reference(thread_id, turn_id, Moment::Arrived(turn.arrivals));
        let dir = folder.to_owned();
        self.checkpoints.queue(&dir.clone(), move || {
            let captured = if created {
                checkpoint::capture_created(&dir, &reference)
            } else {
                checkpoint::capture(&dir, &reference)
            };
            if let Err(error) = captured {
                eprintln!("hoshi: no checkpoint where the turn went in {dir}: {error}");
            }
        });
    }

    /// Carries out a command: decides its events, records them, and starts what they call for.
    /// A message to a thread git works on waits for that work to end.
    fn run_command(&mut self, command: Command) -> Result<(), CommandError> {
        // A thread goes with the tasks it delegated, which nothing else would list.
        if let Command::ThreadDelete { thread_id } = &command {
            let tasks: Vec<String> = self
                .model
                .threads()
                .filter(|thread| thread.parent_thread_id.as_deref() == Some(thread_id))
                .map(|thread| thread.id.clone())
                .collect();
            for task in tasks {
                self.run_command(Command::ThreadDelete { thread_id: task })?;
            }
        }
        if let Command::MessageSend { thread_id, .. } = &command
            && let Some(held) = self.thread_ops.get_mut(thread_id)
        {
            held.push(command);
            return Ok(());
        }
        let restoring = match &command {
            Command::MessageSend { thread_id, .. } if self.worktree_missing(thread_id) => {
                Some(thread_id.clone())
            }
            _ => None,
        };
        let creating = match &command {
            Command::ThreadCreate {
                thread_id,
                workspace: ThreadWorkspace::NewWorktree | ThreadWorkspace::BranchWorktree { .. },
                ..
            } => Some(thread_id.clone()),
            _ => None,
        };
        let command = self.prepare(command)?;
        if let Some(thread_id) = restoring {
            self.start_worktree_setup(&thread_id);
        }
        let Some(command) = self.setups.hold(command) else {
            return Ok(());
        };
        self.run_prepared(command)?;
        if let Some(thread_id) = creating {
            self.start_worktree_setup(&thread_id);
        }
        Ok(())
    }

    /// Whether the thread's worktree was released and its folder is gone.
    fn worktree_missing(&self, thread_id: &str) -> bool {
        self.model.thread(thread_id).is_some_and(|thread| {
            thread.worktree_released
                && thread
                    .worktree_path
                    .as_deref()
                    .is_some_and(|path| !Path::new(path).is_dir())
        })
    }

    /// Runs the project's setup in the thread's worktree, if it has one.
    fn start_worktree_setup(&self, thread_id: &str) {
        if let Some(launch) = self.setup_launch(thread_id) {
            self.run_worktree_setup(launch);
        }
    }

    fn setup_launch(&self, thread_id: &str) -> Option<SetupLaunch> {
        let thread = self.model.thread(thread_id)?;
        let worktree_path = thread.worktree_path.clone()?;
        let project = self.model.project(thread.project_id.as_deref()?)?;
        let setup = project.worktree_setup.clone()?;
        Some(SetupLaunch {
            thread_id: thread_id.to_owned(),
            project_root: project.workspace_root.clone(),
            worktree_path,
            files: setup.files,
            command: setup.command,
            wait: setup.wait,
        })
    }

    /// The messages a setup held are sent apart from its terminal, whose lock it ends under.
    fn run_worktree_setup(&self, launch: SetupLaunch) {
        let this = self.this.clone();
        self.setups.start(launch, move |held| {
            let this = this.clone();
            thread::spawn(move || {
                let Some(state) = this.upgrade() else {
                    return;
                };
                let mut state = lock(&state);
                for command in held {
                    if let Err(error) = state.run_command(command) {
                        eprintln!("hoshi: failed to send a message held for setup: {error}");
                    }
                }
            });
        })
    }

    /// Carries out a command `prepare` already went through, or one the engine itself made.
    fn run_prepared(&mut self, command: Command) -> Result<(), CommandError> {
        let interrupt = match &command {
            Command::TurnInterrupt { thread_id } => Some(thread_id.clone()),
            _ => None,
        };
        let stop = match &command {
            Command::SubagentStop {
                thread_id,
                tool_use_id,
            } => Some((thread_id.clone(), tool_use_id.clone())),
            _ => None,
        };
        let stop_background = match &command {
            Command::BackgroundTaskStop { thread_id, task_id } => {
                Some((thread_id.clone(), task_id.clone()))
            }
            _ => None,
        };
        let answer = match &command {
            Command::RequestRespond {
                thread_id,
                request_id,
                response,
            } => Some((thread_id.clone(), request_id.clone(), response.clone())),
            _ => None,
        };
        // A question whose turn is gone, as after a restart, is answered in a message.
        let by_message = answer
            .as_ref()
            .and_then(|(thread_id, request_id, response)| {
                if self.turns.contains_key(thread_id) {
                    return None;
                }
                let asked = self.model.pending_request(request_id)?;
                Some((
                    thread_id.clone(),
                    answers_message(&asked.content, response)?,
                ))
            });
        let freed_worktrees = self.worktrees_freed_by(&command);
        let freed_images = self.images_freed_by(&command);
        let deleted_threads: Vec<String> = self
            .threads_deleted_by(&command)
            .into_iter()
            .map(|thread| thread.id.clone())
            .collect();
        let forgotten_checkpoints: Vec<(String, String)> = self
            .threads_deleted_by(&command)
            .into_iter()
            .filter_map(|thread| Some((self.project_root(thread).ok()?, thread.id.clone())))
            .collect();
        let removed_with_project: Vec<String> = match &command {
            Command::ProjectRemove { .. } => self
                .threads_deleted_by(&command)
                .into_iter()
                .map(|thread| thread.id.clone())
                .collect(),
            _ => Vec::new(),
        };
        let now = now();
        let events = decide(&self.model, command, &now)?;
        self.commit(&events, &now)?;
        self.react(&events);
        for thread_id in &removed_with_project {
            self.forget_thread(thread_id);
        }
        if let Some(thread_id) = interrupt {
            match self.turns.get(&thread_id) {
                Some(turn) => turn.running.interrupt(),
                None => self.interrupt_parked(&thread_id),
            }
        }
        if let Some((thread_id, request_id, response)) = answer
            && let Some(turn) = self.turns.get(&thread_id)
        {
            turn.running.respond(&request_id, &response);
        }
        if let Some((thread_id, text)) = by_message {
            self.run_command(Command::MessageSend {
                thread_id,
                message_id: delegation::new_id(),
                text,
                attachments: Vec::new(),
            })?;
        }
        if let Some((thread_id, tool_use_id)) = stop
            && let Some(turn) = self.turns.get(&thread_id)
        {
            turn.running.stop_subagent(&tool_use_id);
        }
        if let Some((thread_id, task_id)) = stop_background {
            self.stop_background_command(&thread_id, &task_id);
        }
        // A deleted thread (or a removed project's threads) must not keep an agent running.
        let orphaned: Vec<_> = self
            .turns
            .keys()
            .filter(|thread_id| self.model.thread(thread_id).is_none())
            .cloned()
            .collect();
        for thread_id in orphaned {
            if let Some(turn) = self.turns.remove(&thread_id) {
                turn.running.interrupt();
            }
        }
        let orphaned: Vec<_> = self
            .lingering
            .keys()
            .filter(|thread_id| self.model.thread(thread_id).is_none())
            .cloned()
            .collect();
        for thread_id in orphaned {
            self.close_lingering(&thread_id);
        }
        let State {
            model,
            context_usages,
            ..
        } = self;
        context_usages.retain(|thread_id, _| model.thread(thread_id).is_some());
        self.setups
            .retain_threads(|thread_id| self.model.thread(thread_id).is_some());
        if !freed_worktrees.is_empty() {
            let checkpoints = self.checkpoints.clone();
            thread::spawn(move || {
                for (repository, path) in freed_worktrees {
                    checkpoints.settle(&path);
                    if let Err(error) = git::remove_worktree(&repository, &path) {
                        eprintln!("hoshi: kept worktree {path}: {error}");
                    }
                }
            });
        }
        for image in &freed_images {
            self.attachments.remove(image);
        }
        for thread_id in &deleted_threads {
            self.pages.remove_thread(thread_id);
        }
        for (root, thread_id) in forgotten_checkpoints {
            self.checkpoints.queue(&root.clone(), move || {
                if let Err(error) = checkpoint::forget(&root, &thread_id) {
                    eprintln!("hoshi: kept the checkpoints of thread {thread_id}: {error}");
                }
            });
        }
        Ok(())
    }

    /// Starts what recorded events call for: turns, titles, watches, cleanups.
    fn react(&mut self, events: &[Event]) {
        let mut message: Option<QueuedMessage> = None;
        let mut joining: Option<String> = None;
        let titled: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                Event::ThreadTitleSet { thread_id, title } => {
                    Some((thread_id.clone(), title.clone()))
                }
                _ => None,
            })
            .collect();
        for event in events {
            match event {
                Event::ItemAdded {
                    item:
                        ThreadItem {
                            id,
                            thread_id,
                            content:
                                ItemContent::UserMessage {
                                    text,
                                    attachments,
                                    task_result,
                                    pull_request_update,
                                    ..
                                },
                            ..
                        },
                } => {
                    message = Some(QueuedMessage {
                        id: id.clone(),
                        text: text.clone(),
                        attachments: attachments.clone(),
                        from_user: task_result.is_none() && pull_request_update.is_none(),
                        held: false,
                    });
                    joining = Some(thread_id.clone());
                    if let Some((thread_id, provisional)) = titled.first() {
                        let request = TitleRequest {
                            context: text.clone(),
                            previous_title: None,
                            images: titles::attachment_names(attachments),
                            model: None,
                            instructions: String::new(),
                        };
                        self.name_thread(thread_id, provisional, request);
                    }
                }
                Event::TurnStarted {
                    thread_id, turn_id, ..
                } => {
                    let started = message.take();
                    let (prompt, attachments) = started
                        .map(|message| (message.text, message.attachments))
                        .unwrap_or_default();
                    start_turn(self, thread_id, turn_id, prompt, attachments, None);
                }
                Event::ThreadCreated { thread } if thread.worktree_path.is_none() => {
                    refresh_branches(self.this.clone(), vec![thread.id.clone()]);
                }
                Event::ProjectCreated { project } => {
                    let watched = (project.id.clone(), project.workspace_root.clone());
                    watch_projects(self.this.clone(), vec![watched]);
                }
                Event::ProjectRemoved { project_id } => {
                    if let Some(watcher) = &mut self.watcher {
                        watcher.unwatch(project_id);
                    }
                    self.repositories.remove(project_id);
                    self.root_branches.remove(project_id);
                }
                Event::ThreadDeleted { thread_id } => self.forget_thread(thread_id),
                _ => {}
            }
        }
        if let (Some(message), Some(thread_id)) = (message, joining) {
            self.join_turn(&thread_id, message);
        }
    }

    /// Marks `folders` as ones git changes outside the lock for `thread_id`. Refused while
    /// other git work runs on the thread or in one of the folders.
    fn begin_op(&mut self, thread_id: &str, folders: &[&str]) -> Result<WorkspaceOp, CommandError> {
        let folders: Vec<String> = folders
            .iter()
            .map(|folder| git::normalize(folder))
            .collect();
        let taken = self.thread_ops.contains_key(thread_id)
            || folders
                .iter()
                .any(|folder| self.folder_ops.contains(folder));
        if taken {
            return Err(CommandError::ThreadBusy {
                thread_id: thread_id.to_owned(),
            });
        }
        self.thread_ops.insert(thread_id.to_owned(), Vec::new());
        self.folder_ops.extend(folders.iter().cloned());
        Ok(WorkspaceOp {
            thread_id: thread_id.to_owned(),
            folders,
        })
    }

    /// Ends git work [`State::begin_op`] started: runs `first`, then the messages sent to the
    /// thread meanwhile, then the turns that waited for its folders. Returns how `first` went.
    fn end_op(&mut self, op: WorkspaceOp, first: Option<Command>) -> Result<(), CommandError> {
        let held = self.thread_ops.remove(&op.thread_id).unwrap_or_default();
        for folder in &op.folders {
            self.folder_ops.remove(folder);
        }
        let result = first.map_or(Ok(()), |command| self.run_command(command));
        for command in held {
            if let Err(error) = self.run_command(command) {
                eprintln!("hoshi: failed to send a message held for git work: {error}");
            }
        }
        for folder in &op.folders {
            self.start_parked(folder);
        }
        result
    }

    /// Starts the turns that waited for git to finish with `folder`.
    fn start_parked(&mut self, folder: &str) {
        for turn in self.parked.remove(folder).unwrap_or_default() {
            let current = self.model.thread(&turn.thread_id).is_some_and(|thread| {
                thread.active_turn_id.as_deref() == Some(turn.turn_id.as_str())
            });
            if !current {
                continue;
            }
            start_turn(
                self,
                &turn.thread_id,
                &turn.turn_id,
                turn.prompt,
                turn.attachments,
                Some(turn.runs_on),
            );
        }
    }

    /// Ends a parked turn the user stopped before git was done with its folder.
    fn interrupt_parked(&mut self, thread_id: &str) {
        let mut stopped = Vec::new();
        for turns in self.parked.values_mut() {
            turns.retain(|turn| {
                let parked = turn.thread_id == thread_id;
                if parked {
                    stopped.push(turn.turn_id.clone());
                }
                !parked
            });
        }
        for turn_id in stopped {
            let outcome = TurnOutcome::Interrupted;
            self.receive(
                thread_id,
                &turn_id,
                ProviderOutput::TurnCompleted { outcome },
            );
        }
    }

    /// Drops what the engine keeps for a thread that is gone.
    fn forget_thread(&mut self, thread_id: &str) {
        if let Some(held) = self.thread_ops.get_mut(thread_id) {
            held.clear();
        }
        for turns in self.parked.values_mut() {
            turns.retain(|turn| turn.thread_id != thread_id);
        }
        if let Some(mcp) = &self.mcp {
            mcp.forget(thread_id);
        }
        self.delegations.remove(thread_id);
        self.pull_requests.forget(thread_id);
        self.pull_request_cache.forget(thread_id);
        self.queued.remove(thread_id);
        self.after_compaction.remove(thread_id);
        self.idle_compactions.remove(thread_id);
        self.watches.remove(thread_id);
        self.watch_reads.remove(thread_id);
        self.pending_wakes.remove(thread_id);
        self.titles_to_refine.remove(thread_id);
        self.session_moves.remove(thread_id);
        self.recovery.forget(thread_id);
        if let Some(folder) = self.chat_folder(thread_id) {
            let _ = std::fs::remove_dir_all(folder);
        }
    }

    /// Passes a message sent to a busy thread on to its running turn, or keeps it for a turn
    /// of its own when the turn is ending.
    fn join_turn(&mut self, thread_id: &str, message: QueuedMessage) {
        let attachments = self.turn_attachments(message.attachments.clone());
        let joined = self.turns.get(thread_id).is_some_and(|turn| {
            turn.running
                .steer(&message.id, &message.text, &attachments, message.from_user)
        });
        if joined {
            return;
        }
        let held = self.queue_holds(thread_id);
        self.queued
            .entry(thread_id.to_owned())
            .or_default()
            .push_back(QueuedMessage { held, ..message });
        self.save_recovery(thread_id);
        self.show_queue(thread_id);
        // The turn may have ended between the message's events and this call.
        if !self.turns.contains_key(thread_id) {
            self.start_next_queued(thread_id);
        }
    }

    /// Keeps the message until the thread is compacted, where the user sees it wait.
    fn hold_until_compacted(&mut self, thread_id: &str, message: QueuedMessage) {
        (self.notify)(Notification::HeldMessage(ThreadHeldMessage {
            thread_id: thread_id.to_owned(),
            message: Some(held_message(thread_id, &message)),
        }));
        self.after_compaction.insert(thread_id.to_owned(), message);
        self.save_recovery(thread_id);
    }

    fn take_held(&mut self, thread_id: &str) -> Option<QueuedMessage> {
        let message = self.after_compaction.remove(thread_id)?;
        self.save_recovery(thread_id);
        (self.notify)(Notification::HeldMessage(ThreadHeldMessage {
            thread_id: thread_id.to_owned(),
            message: None,
        }));
        Some(message)
    }

    /// The message the user sent to go once the context was compacted.
    fn send_after_compaction(&mut self, thread_id: &str) {
        let idle = self
            .model
            .thread(thread_id)
            .is_some_and(|thread| thread.active_turn_id.is_none());
        if !idle {
            return;
        }
        let Some(message) = self.take_held(thread_id) else {
            return;
        };
        let message_id = message.id;
        let send = Command::MessageSend {
            thread_id: thread_id.to_owned(),
            message_id: message_id.clone(),
            text: message.text,
            attachments: message.attachments,
        };
        if let Err(error) = self.run_command(send) {
            eprintln!("hoshi: failed to send a message after compacting: {error}");
            return;
        }
        self.release_queue_after(thread_id, Some(&message_id));
    }

    /// When the idle thread compacts before its prompt cache expires; `None` while it does
    /// not, as when its provider doesn't or something is still due to reach its agent.
    fn idle_compaction_window(&self, thread_id: &str) -> Option<Range<u64>> {
        let thread = self.model.thread(thread_id)?;
        let busy = thread.active_turn_id.is_some()
            || thread.archived
            || self.snoozed(thread_id)
            || thread.parent_thread_id.is_some()
            || self.lingering.contains_key(thread_id)
            || self
                .queued
                .get(thread_id)
                .is_some_and(|queued| !queued.is_empty())
            || self.after_compaction.contains_key(thread_id)
            || self.thread_ops.contains_key(thread_id)
            || self
                .pending_wakes
                .get(thread_id)
                .is_some_and(|wakes| !wakes.is_empty())
            || self
                .delegations
                .values()
                .any(|task| task.parent == thread_id)
            || !self.model.pending_requests(thread_id).is_empty();
        let compacts = self
            .adapter_for(&thread.provider)
            .is_some_and(|adapter| adapter.compacts_idle_threads());
        if busy || !compacts {
            return None;
        }
        compaction_before_expiry(self.idle_compaction, self.context_usages.get(thread_id))
    }

    fn schedule_idle_compaction(&mut self, thread_id: &str) {
        match self.idle_compaction_window(thread_id) {
            Some(window) => {
                self.idle_compactions
                    .insert(thread_id.to_owned(), window.start);
            }
            None => {
                self.idle_compactions.remove(thread_id);
            }
        }
    }

    /// Sends `/compact` to the idle threads whose time came, while their prompt cache holds.
    fn compact_idle_threads(&mut self, now: u64) {
        let due: Vec<String> = self
            .idle_compactions
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(thread_id, _)| thread_id.clone())
            .collect();
        for thread_id in due {
            self.idle_compactions.remove(&thread_id);
            let open = self
                .idle_compaction_window(&thread_id)
                .is_some_and(|window| window.contains(&now));
            if !open {
                continue;
            }
            let compact = Command::MessageSend {
                thread_id,
                message_id: delegation::new_id(),
                text: COMPACT_COMMAND.to_owned(),
                attachments: Vec::new(),
            };
            if let Err(error) = self.run_command(compact) {
                eprintln!("hoshi: failed to compact an idle thread: {error}");
            }
        }
    }

    /// Starts the oldest message waiting for its own turn, such as one sent while the user
    /// stopped the running turn, unless it waits for the user.
    fn start_next_queued(&mut self, thread_id: &str) {
        let idle = self
            .model
            .thread(thread_id)
            .is_some_and(|thread| thread.active_turn_id.is_none());
        let Some(queued) = self.queued.get_mut(thread_id).filter(|_| idle) else {
            return;
        };
        let Some(message) = queued
            .iter()
            .position(|message| !message.held)
            .and_then(|next| queued.remove(next))
        else {
            return;
        };
        self.save_recovery(thread_id);
        self.start_queued(thread_id, message);
        self.show_queue(thread_id);
    }

    /// Starts a turn of its own for a message taken out of the idle thread's queue.
    fn start_queued(&mut self, thread_id: &str, message: QueuedMessage) {
        let now = now();
        let kept = self
            .store
            .thread_item(thread_id, &message.id)
            .ok()
            .flatten();
        let events = start_queued_turn(&self.model, thread_id, &message.id, kept.as_ref(), &now);
        if events.is_empty() {
            return;
        }
        if let Err(error) = self.commit(&events, &now) {
            eprintln!("hoshi: failed to start a queued message: {error}");
            return;
        }
        start_turn(
            self,
            thread_id,
            &message.id,
            message.text,
            message.attachments,
            None,
        );
    }

    fn turn_attachments(&self, attachments: Vec<MessageAttachment>) -> Vec<TurnAttachment> {
        attachments
            .into_iter()
            .map(|attachment| {
                let path = self.attachments.path_of(&attachment);
                let extracted_text = match &attachment {
                    MessageAttachment::File { .. } => {
                        path.as_deref().and_then(document_text::sidecar)
                    }
                    _ => None,
                };
                TurnAttachment {
                    path,
                    extracted_text,
                    attachment,
                }
            })
            .collect()
    }

    /// The images and files the thread's earlier messages carried since its conversation
    /// last compacted, leaving out those of the message about to go, `current`.
    fn earlier_media(&self, thread_id: &str, current: &[TurnAttachment]) -> Vec<TurnAttachment> {
        let earlier = match self.store.media_since_compaction(thread_id) {
            Ok(earlier) => earlier,
            Err(error) => {
                eprintln!("hoshi: failed to read the media of thread {thread_id}: {error}");
                return Vec::new();
            }
        };
        let earlier = earlier
            .into_iter()
            .filter(|attachment| !current.iter().any(|sent| sent.attachment == *attachment))
            .collect();
        self.turn_attachments(earlier)
    }

    /// Asks the provider for a title in the background, retrying a first title twice.
    /// It replaces `current` only if the thread still has that title by then.
    fn name_thread(&mut self, thread_id: &str, current: &str, mut request: TitleRequest) {
        let provider = self
            .thread_titles
            .provider
            .as_deref()
            .unwrap_or(DEFAULT_PROVIDER);
        let Some(adapter) = self.adapter_for(provider) else {
            return;
        };
        let forced = request.previous_title.is_some();
        if !self.thread_titles.generate && !forced {
            return;
        }
        request.model.clone_from(&self.thread_titles.model);
        request
            .instructions
            .clone_from(&self.thread_titles.instructions);
        let state = self.this.clone();
        let (thread_id, current) = (thread_id.to_owned(), current.to_owned());
        thread::spawn(move || {
            let attempts = if forced { 1 } else { TITLE_ATTEMPTS };
            let mut delay = TITLE_RETRY_DELAY;
            for attempt in 1..=attempts {
                if let Some(generated) = adapter.generate_title(&request) {
                    let Some(shared) = state.upgrade() else {
                        return;
                    };
                    let mut state = lock(&shared);
                    let events = record_title(&state.model, &thread_id, &current, &generated.title);
                    if let Err(error) = state.commit(&events, &now()) {
                        eprintln!("hoshi: failed to record a thread title: {error}");
                    }
                    if !events.is_empty() && !generated.needs_refinement {
                        state.name_branch(&thread_id, &generated.title);
                    }
                    if generated.needs_refinement && request.previous_title.is_none() {
                        state.titles_to_refine.insert(thread_id);
                    }
                    return;
                }
                if attempt < attempts {
                    thread::sleep(delay);
                    delay *= 2;
                }
            }
        });
    }

    /// Names the branch of a worktree hoshi made after the thread's title, as t3code does.
    /// Only the `hoshi/<id>` it started on, and only before it was ever pushed.
    fn name_branch(&self, thread_id: &str, title: &str) {
        let Some(thread) = self.model.thread(thread_id) else {
            return;
        };
        let placeholder = format!("hoshi/{}", thread_id.chars().take(8).collect::<String>());
        let (Some(path), Some(branch)) = (&thread.worktree_path, &thread.branch) else {
            return;
        };
        if thread.worktree_adopted || thread.worktree_released || *branch != placeholder {
            return;
        }
        let Some(name) = worktree::branch_name(title) else {
            return;
        };
        let (path, state, thread_id) = (path.clone(), self.this.clone(), thread_id.to_owned());
        thread::spawn(move || {
            let pushed = git::branch_config(&path, &placeholder, "remote").is_some();
            if pushed || git::current_branch(&path).as_deref() != Some(placeholder.as_str()) {
                return;
            }
            let Some(free) = (1..=20)
                .map(|n| match n {
                    1 => format!("hoshi/{name}"),
                    _ => format!("hoshi/{name}-{n}"),
                })
                .find(|candidate| git::branch_tip(&path, candidate).is_none())
            else {
                return;
            };
            if let Err(error) = git::rename_branch(&path, &placeholder, &free) {
                eprintln!("hoshi: kept branch {placeholder}: {error}");
                return;
            }
            read_branches(&state, &[thread_id]);
        });
    }

    /// Names the thread from its messages. `forced` regenerates even when the user
    /// turned generated titles off.
    fn rename_from_conversation(&mut self, thread_id: &str, forced: bool) {
        if !forced && !self.thread_titles.generate {
            return;
        }
        let Some(current) = self
            .model
            .thread(thread_id)
            .map(|thread| thread.title.clone())
        else {
            return;
        };
        let items = self.store.thread_items(thread_id).unwrap_or_default();
        let (context, images) = titles::conversation(&items);
        if context.is_empty() {
            return;
        }
        let request = TitleRequest {
            context,
            previous_title: Some(current.clone()),
            images,
            model: None,
            instructions: String::new(),
        };
        self.name_thread(thread_id, &current, request);
    }

    fn keep_provider_version(
        &mut self,
        provider: &str,
        installation: &Installation,
        update: ProviderUpdate,
    ) {
        let version = ProviderVersion {
            installed: installation.installed.clone(),
            latest: installation.latest.clone(),
            update_available: installation.update_available(),
            update_command: installation.updater.as_ref().map(Updater::display),
            update,
        };
        self.provider_versions.insert(provider.to_owned(), version);
        self.notify_provider_version(provider);
    }

    fn set_provider_update(&mut self, provider: &str, update: ProviderUpdate) {
        let version = self
            .provider_versions
            .entry(provider.to_owned())
            .or_default();
        version.update = update;
        self.notify_provider_version(provider);
    }

    fn forget_provider(&mut self, provider: String) {
        self.provider_versions.remove(&provider);
        self.provider_accounts.remove(&provider);
        (self.notify)(Notification::ProviderVersion(ProviderVersionUpdate {
            provider,
            version: None,
        }));
        self.notify_providers();
    }

    /// Also sends the whole status, so a status taken before this change can't undo it.
    fn notify_provider_version(&self, provider: &str) {
        (self.notify)(Notification::ProviderVersion(ProviderVersionUpdate {
            provider: provider.to_owned(),
            version: self.provider_versions.get(provider).cloned(),
        }));
        self.notify_providers();
    }

    fn providers_status(&self) -> ProvidersStatus {
        ProvidersStatus {
            versions: self.provider_versions.clone(),
            accounts: self.provider_accounts.clone(),
            checked_at: self.providers_checked_at.clone(),
            sign_ins: self.provider_sign_ins.clone(),
            logins: self.provider_logins.clone(),
            codex_account: self.codex_account.clone(),
            usage_limits: self.usage_limits.clone(),
            revision: self.providers_revision.fetch_add(1, Ordering::Relaxed),
        }
    }

    fn notify_providers(&self) {
        (self.notify)(Notification::Providers(self.providers_status()));
    }

    fn set_sign_in(&mut self, provider: &str, sign_in: ProviderSignIn) {
        self.provider_sign_ins.insert(provider.to_owned(), sign_in);
        self.notify_providers();
    }

    /// Codex signed in or out: its account as its files now say.
    fn keep_codex_account(&mut self, account: Option<CodexAccount>, status: ProviderAccountStatus) {
        self.codex_account = account;
        if self.adapter_for(CODEX).is_some() {
            self.provider_accounts.insert(CODEX.to_owned(), status);
        }
    }

    fn keep_usage_limits(&mut self, provider: &str, limits: UsageLimits) {
        if let Err(error) = self.store.save_provider_usage_limits(provider, &limits) {
            eprintln!("hoshi: failed to cache the usage limits: {error}");
        }
        if provider == DEFAULT_PROVIDER {
            (self.notify)(Notification::UsageLimits(limits.clone()));
        }
        self.usage_limits.insert(provider.to_owned(), limits);
        self.notify_providers();
    }

    fn record_context_usage(&mut self, thread_id: &str, turn_id: &str, usage: ContextUsage) {
        let active = self
            .model
            .thread(thread_id)
            .is_some_and(|thread| thread.active_turn_id.as_deref() == Some(turn_id));
        if !active {
            return;
        }
        let mut usage = usage.merged_into(self.context_usages.get(thread_id).copied());
        if let Some(ttl) = usage.cache_ttl_secs {
            usage.cache_expires_at = Some(unix_millis() + ttl * 1000);
        }
        self.context_usages.insert(thread_id.to_owned(), usage);
        let update = ThreadContextUsage {
            thread_id: thread_id.to_owned(),
            usage,
        };
        if let Err(error) = self.store.save_context_usage(&update) {
            eprintln!("hoshi: failed to cache a context usage: {error}");
        }
        (self.notify)(Notification::ContextUsage(update));
    }

    /// The agent's session works elsewhere than the thread's folder: in a worktree it
    /// entered, or back in the project root. git confirms which in the background; when
    /// moves overlap, the latest one wins.
    fn follow_session(&mut self, thread_id: &str, turn_id: &str, path: &str) {
        let Some(cwd) = self.model.thread_cwd(thread_id).map(git::normalize) else {
            return;
        };
        let path = git::normalize(path);
        if path == cwd && !self.session_moves.contains_key(thread_id) {
            return;
        }
        let Some(workspace) = self
            .turns
            .get_mut(thread_id)
            .filter(|turn| turn.id == turn_id)
            .and_then(|turn| turn.workspace.as_mut())
        else {
            return;
        };
        workspace.moved = true;
        let workspace = workspace.clone();
        let check = self.worktree_check(thread_id, turn_id, workspace);
        self.session_moves
            .insert(thread_id.to_owned(), path.clone());
        follow_session_move(self.this.clone(), thread_id.to_owned(), path, check);
    }

    /// Records the branch read in `cwd` off the lock. `false` when the thread moved to
    /// another folder meanwhile, so the read says nothing about where it is now.
    fn record_branch_read(&mut self, thread_id: &str, cwd: &str, branch: Option<String>) -> bool {
        if self.model.thread_cwd(thread_id) != Some(cwd) {
            return false;
        }
        // What git does there may have removed the folder this read found no branch in.
        if self.thread_ops.contains_key(thread_id) || self.folder_ops.contains(&git::normalize(cwd))
        {
            return true;
        }
        let events = record_branch(&self.model, thread_id, branch);
        if !events.is_empty() {
            self.pull_requests.check_soon(thread_id);
        }
        if let Err(error) = self.commit(&events, &now()) {
            eprintln!("hoshi: failed to record a branch: {error}");
        }
        true
    }

    fn worktree_check(
        &self,
        thread_id: &str,
        turn_id: &str,
        workspace: TurnWorkspace,
    ) -> WorktreeCheck {
        let claimed = self
            .model
            .threads()
            .filter(|thread| thread.id != thread_id)
            .filter_map(|thread| thread.worktree_path.clone())
            .collect();
        WorktreeCheck {
            turn_id: turn_id.to_owned(),
            workspace,
            claimed,
        }
    }
}

/// What changed in `folder` since the turn arrived there, which becomes the checkpoint a
/// rewind of the turn restores.
fn changes_since_arrival(
    folder: &str,
    thread_id: &str,
    turn_id: &str,
    arrivals: usize,
) -> Result<Vec<DiffFile>, String> {
    let before = checkpoint::reference(thread_id, turn_id, Moment::Before);
    let after = checkpoint::reference(thread_id, turn_id, Moment::After);
    checkpoint::capture(folder, &after)?;
    if arrivals > 0 {
        let arrived = checkpoint::reference(thread_id, turn_id, Moment::Arrived(arrivals));
        checkpoint::rename(folder, &arrived, &before)?;
    }
    checkpoint::changes(folder, &before, &after)
}

/// [`Engine::release_worktree`], also for [`merged_worktrees`]' sweeps.
fn release_worktree(
    shared: &Arc<Mutex<State>>,
    terminals: &Terminals,
    thread_id: &str,
) -> Result<(), CommandError> {
    if terminals.any_open_for(thread_id) {
        return Err(CommandError::TerminalsOpen {
            thread_id: thread_id.to_owned(),
        });
    }
    let release = Command::ThreadWorktreeRelease {
        thread_id: thread_id.to_owned(),
    };
    let (op, root, path, checkpoints) = {
        let mut state = lock(shared);
        let (thread, root) = state.removable(thread_id)?;
        let path = thread
            .worktree_path
            .clone()
            .ok_or_else(|| CommandError::NothingToClean {
                thread_id: thread_id.to_owned(),
            })?;
        if thread.worktree_released {
            return state.decide_and_commit(release);
        }
        state.close_lingering(thread_id);
        let op = state.begin_op(thread_id, &[&path])?;
        (op, root, path, state.checkpoints.clone())
    };
    checkpoints.settle(&path);
    let removed = State::remove_clean_worktree(&root, &path, thread_id);
    let mut state = lock(shared);
    let released = removed.and_then(|()| state.decide_and_commit(release));
    state.end_op(op, None)?;
    released
}

/// Attaches a worktree the turn created, then reads the thread's branch.
fn refresh_after_turn(state: Weak<Mutex<State>>, thread_id: String, check: Option<WorktreeCheck>) {
    thread::spawn(move || {
        if let Some(check) = check {
            attach_created_worktree(&state, &thread_id, &check);
        }
        read_branches(&state, std::slice::from_ref(&thread_id));
        // The turn may have pushed or opened a pull request.
        if let Some(shared) = state.upgrade() {
            lock(&shared).pull_requests.check_soon(&thread_id);
        }
    });
}

/// The worktree the turn created and worked in, with its branch, if git shows one.
fn created_worktree(check: &WorktreeCheck) -> Option<(String, Option<String>)> {
    let after = git::worktrees(&check.workspace.repository)?;
    let claimed: Vec<String> = check
        .claimed
        .iter()
        .map(|path| git::normalize(path))
        .collect();
    let path = worktree::created_by_turn(&check.workspace, &after, &claimed)?;
    let branch = git::current_branch(&path);
    Some((path, branch))
}

/// Moves a thread to the worktree its turn created and worked in. The agent made it, so
/// hoshi never removes it.
fn attach_created_worktree(state: &Weak<Mutex<State>>, thread_id: &str, check: &WorktreeCheck) {
    if check.workspace.moved {
        return;
    }
    let (Some((path, branch)), Some(shared)) = (created_worktree(check), state.upgrade()) else {
        return;
    };
    let mut state = lock(&shared);
    let events = record_worktree(&state.model, thread_id, path.clone(), branch, true);
    if events.is_empty() {
        return;
    }
    if let Err(error) = state.commit(&events, &now()) {
        eprintln!("hoshi: failed to record a worktree: {error}");
        return;
    }
    state.follow_turn(thread_id, &check.turn_id, &path, true);
}

/// Moves the thread where its agent's session went, once git shows it is the project root
/// or one of its worktrees; anywhere else stays the agent's own business. hoshi never
/// removes a worktree it did not create, so the thread adopts it.
fn follow_session_move(
    state: Weak<Mutex<State>>,
    thread_id: String,
    path: String,
    check: WorktreeCheck,
) {
    thread::spawn(move || {
        let root = &check.workspace.repository;
        let in_root = path == git::normalize(root);
        let found =
            in_root || git::worktrees(root).is_some_and(|worktrees| worktrees.contains(&path));
        let branch = if in_root {
            git::current_branch(root)
        } else if found {
            git::current_branch(&path)
        } else {
            None
        };
        let created = !in_root && !check.workspace.worktrees_before.contains(&path);
        let folder = if in_root { root.clone() } else { path.clone() };
        let Some(shared) = state.upgrade() else {
            return;
        };
        let mut state = lock(&shared);
        if state.session_moves.get(&thread_id) != Some(&path) {
            return;
        }
        state.session_moves.remove(&thread_id);
        if !found {
            return;
        }
        let events = if in_root {
            record_worktree_left(&state.model, &thread_id, branch)
        } else {
            record_worktree(&state.model, &thread_id, path, branch, true)
        };
        if events.is_empty() {
            return;
        }
        state.pull_requests.check_soon(&thread_id);
        if let Err(error) = state.commit(&events, &now()) {
            eprintln!("hoshi: failed to follow the agent's session: {error}");
            return;
        }
        state.follow_turn(&thread_id, &check.turn_id, &folder, created);
    });
}

fn in_any(project_ids: &[String], thread: &hoshi_core::Thread) -> bool {
    thread
        .project_id
        .as_ref()
        .is_some_and(|project_id| project_ids.contains(project_id))
}

/// Whether a branch change in the thread's folder is the thread's own: its worktree, or
/// the project root while one of its turns runs there. Idle threads in the root keep the
/// branch they worked on.
fn follows_its_folder(thread: &hoshi_core::Thread) -> bool {
    thread.worktree_path.is_some() || thread.active_turn_id.is_some()
}

/// Reads the branch of the threads in the given projects that follow their folder. A
/// running turn that created a worktree moves to it now, not only when the turn ends.
fn refresh_projects(state: &Weak<Mutex<State>>, project_ids: &[String]) {
    let Some(shared) = state.upgrade() else {
        return;
    };
    let (thread_ids, following, checks) = {
        let state = lock(&shared);
        let in_projects = |thread_id: &str| {
            state
                .model
                .thread(thread_id)
                .is_some_and(|thread| in_any(project_ids, thread))
        };
        let thread_ids: Vec<String> = state
            .model
            .threads()
            .filter(|thread| in_any(project_ids, thread))
            .map(|thread| thread.id.clone())
            .collect();
        let following: Vec<String> = state
            .model
            .threads()
            .filter(|thread| in_any(project_ids, thread) && follows_its_folder(thread))
            .map(|thread| thread.id.clone())
            .collect();
        let checks: Vec<(String, WorktreeCheck)> = state
            .turns
            .iter()
            .filter(|(thread_id, _)| in_projects(thread_id))
            .filter_map(|(thread_id, turn)| {
                let workspace = turn.workspace.clone()?;
                Some((
                    thread_id.clone(),
                    state.worktree_check(thread_id, &turn.id, workspace),
                ))
            })
            .collect();
        (thread_ids, following, checks)
    };
    for (thread_id, check) in &checks {
        attach_created_worktree(state, thread_id, check);
    }
    read_root_branches(state, project_ids);
    read_branches(state, &following);
    let mut state = lock(&shared);
    for thread_id in &thread_ids {
        state.pull_requests.check_soon(thread_id);
    }
}

/// Reads the branch of each project's root folder off the lock, and tells the UI of changes.
fn read_root_branches(state: &Weak<Mutex<State>>, project_ids: &[String]) {
    for project_id in project_ids {
        let Some(shared) = state.upgrade() else {
            return;
        };
        let root = lock(&shared)
            .model
            .project(project_id)
            .map(|project| project.workspace_root.clone());
        let Some(root) = root else { continue };
        let branch = git::current_branch(&root);
        let mut state = lock(&shared);
        if state.model.project(project_id).is_none()
            || state.root_branches.get(project_id) == Some(&branch)
        {
            continue;
        }
        state
            .root_branches
            .insert(project_id.clone(), branch.clone());
        (state.notify)(Notification::ProjectBranch(hoshi_core::ProjectBranch {
            project_id: project_id.clone(),
            branch,
        }));
    }
}

/// Finds each project's `.git` folder off the lock, then starts watching it.
fn watch_projects(state: Weak<Mutex<State>>, projects: Vec<(String, String)>) {
    if projects.is_empty() {
        return;
    }
    thread::spawn(move || {
        let project_ids: Vec<String> = projects.iter().map(|(id, _)| id.clone()).collect();
        read_root_branches(&state, &project_ids);
        for (project_id, root) in projects {
            let Some(git_dir) = git::common_dir(&root) else {
                continue;
            };
            let repository = git::origin_url(&root).and_then(|url| parse_remote(&url));
            let Some(shared) = state.upgrade() else {
                return;
            };
            let mut state = lock(&shared);
            if state.model.project(&project_id).is_none() {
                continue;
            }
            if let Some(watcher) = &mut state.watcher {
                watcher.watch(&project_id, git_dir);
            }
            if let Some(repository) = repository {
                state.repositories.insert(project_id, repository);
            }
        }
    });
}

/// Reads each thread's branch with git, off the lock, and records changes.
fn restore_pull_request_details(state: Weak<Mutex<State>>) {
    thread::spawn(move || {
        let Some(shared) = state.upgrade() else {
            return;
        };
        let mut state = lock(&shared);
        let saved = match state.store.pull_request_details() {
            Ok(saved) => saved,
            Err(error) => {
                eprintln!("hoshi: failed to read the saved pull request details: {error}");
                return;
            }
        };
        state
            .pull_request_cache
            .restore_details(saved.into_iter().map(|cached| {
                let version = Version {
                    number: cached.number,
                    updated_at: cached.version,
                };
                (cached.thread_id, version, cached.detail)
            }));
    });
}

fn refresh_branches(state: Weak<Mutex<State>>, thread_ids: Vec<String>) {
    if thread_ids.is_empty() {
        return;
    }
    thread::spawn(move || read_branches(&state, &thread_ids));
}

/// Ends, now and then, the processes kept between turns that went quiet for too long.
fn end_idle_lingering(state: Weak<Mutex<State>>) {
    thread::spawn(move || {
        loop {
            thread::sleep(IDLE_LINGERING_CHECK);
            let Some(state) = state.upgrade() else {
                return;
            };
            lock(&state).close_idle_lingering(SystemTime::now());
        }
    });
}

fn read_branches(state: &Weak<Mutex<State>>, thread_ids: &[String]) {
    for thread_id in thread_ids {
        let Some(shared) = state.upgrade() else {
            return;
        };
        loop {
            let cwd = lock(&shared).model.thread_cwd(thread_id).map(str::to_owned);
            let Some(cwd) = cwd else { break };
            let branch = git::current_branch(&cwd);
            if lock(&shared).record_branch_read(thread_id, &cwd, branch) {
                break;
            }
        }
    }
}

/// How long a request for the files waits on a read already on its way.
const FILES_WAIT_LIMIT: std::time::Duration = std::time::Duration::from_secs(20);
const FILES_WAIT_STEP: std::time::Duration = std::time::Duration::from_millis(40);

/// Reads a pull request's detail off the lock, caches and saves it, and tells the UI.
fn read_detail(state: Weak<Mutex<State>>, thread_id: String, version: Version) {
    thread::spawn(move || {
        let Some(shared) = state.upgrade() else {
            return;
        };
        let target = lock(&shared).pull_request_target_of(&thread_id);
        let result = target.and_then(|(forge, repository)| {
            forge
                .pull_request_detail(&repository, version.number)
                .inspect_err(|error| {
                    eprintln!(
                        "hoshi: couldn't read pull request #{}: {error}",
                        version.number
                    );
                })
                .ok()
        });
        let mut state = lock(&shared);
        let read = result.clone().map(|detail| (version.clone(), detail));
        state
            .pull_request_cache
            .finish_detail(&thread_id, read, Instant::now());
        if let Some(detail) = &result {
            let cached = CachedPullRequestDetail {
                thread_id: thread_id.clone(),
                number: version.number,
                version: version.updated_at.clone(),
                detail: detail.clone(),
            };
            if let Err(error) = state.store.save_pull_request_detail(&cached) {
                eprintln!("hoshi: failed to save a pull request detail: {error}");
            }
        }
        (state.notify)(Notification::PullRequestDetail(ThreadPullRequestDetail {
            thread_id,
            detail: result,
        }));
    });
}

/// Reads a pull request's files off the lock and caches them with their patches.
fn read_files(
    state: &Weak<Mutex<State>>,
    thread_id: &str,
    version: Version,
) -> Option<Vec<DiffFile>> {
    let shared = state.upgrade()?;
    let target = lock(&shared).pull_request_target_of(thread_id);
    let files = target.and_then(|(forge, repository)| {
        forge
            .pull_request_files(&repository, version.number)
            .inspect_err(|error| {
                eprintln!(
                    "hoshi: couldn't read the files of #{}: {error}",
                    version.number
                );
            })
            .ok()
    });
    let listed = files
        .as_ref()
        .map(|files| files.iter().map(|file| file.file.clone()).collect());
    let read = files.map(|files| (version, files));
    lock(&shared)
        .pull_request_cache
        .finish_files(thread_id, read, Instant::now());
    listed
}

/// Reads due pull requests every few seconds, one at a time, off the lock.
fn sync_pull_requests(state: Weak<Mutex<State>>) {
    let (wake, woken) = mpsc::channel();
    if let Some(shared) = state.upgrade() {
        lock(&shared).sync_wake = Some(wake);
    }
    thread::spawn(move || {
        let mut swept = Instant::now();
        loop {
            if let Err(mpsc::RecvTimeoutError::Disconnected) =
                woken.recv_timeout(pull_requests::TICK)
            {
                return;
            }
            while woken.try_recv().is_ok() {}
            let Some(shared) = state.upgrade() else {
                return;
            };
            if swept.elapsed() >= SECRETS_SWEPT_EVERY {
                swept = Instant::now();
                let secrets = lock(&shared).secrets_dir.clone();
                secret_requests::forget_old(&secrets);
            }
            lock(&shared).compact_idle_threads(unix_millis());
            lock(&shared).resume_limited_threads(unix_millis());
            let jobs = lock(&shared).due_pull_request_jobs();
            for job in jobs {
                let result = job
                    .forge
                    .pull_request_for_branch(&job.repository, &job.branch);
                let watch = {
                    let mut state = lock(&shared);
                    state.record_pull_request(&job, result);
                    state.watch_due(&job.thread_id)
                };
                let Some((pull_request, viewer)) = watch else {
                    continue;
                };
                let viewer = viewer.or_else(|| job.forge.viewer().ok());
                match job
                    .forge
                    .pull_request_detail(&job.repository, pull_request.number)
                {
                    Ok(detail) => lock(&shared).observe_watch(
                        &job.thread_id,
                        job.repository.forge,
                        &pull_request,
                        &detail,
                        viewer,
                    ),
                    Err(error) => eprintln!("hoshi: couldn't read a watched pull request: {error}"),
                }
            }
        }
    });
}

impl State {
    fn wake_sync(&self) {
        if let Some(wake) = &self.sync_wake {
            let _ = wake.send(());
        }
    }

    /// The forge and repository of a thread's project.
    fn pull_request_target_of(&self, thread_id: &str) -> Option<(Arc<dyn Forge>, RepositoryRef)> {
        let thread = self.model.thread(thread_id)?;
        let repository = self.repositories.get(thread.project_id.as_ref()?)?.clone();
        let forge = self.forges.get(&repository.forge)?.clone();
        Some((forge, repository))
    }

    fn due_pull_request_jobs(&self) -> Vec<PullRequestJob> {
        let now = Instant::now();
        self.model
            .threads()
            .filter(|thread| !thread.archived || self.pull_requests.has_open(&thread.id))
            .filter_map(|thread| {
                let branch = thread.branch.clone()?;
                let repository = self.repositories.get(thread.project_id.as_ref()?)?.clone();
                let forge = self.forges.get(&repository.forge)?.clone();
                self.pull_requests
                    .is_due(&thread.id, repository.forge, now)
                    .then(|| PullRequestJob {
                        thread_id: thread.id.clone(),
                        branch,
                        repository,
                        forge,
                    })
            })
            .collect()
    }

    fn record_pull_request(
        &mut self,
        job: &PullRequestJob,
        result: Result<Option<hoshi_core::PullRequest>, hoshi_forge::ForgeError>,
    ) {
        let Some(thread) = self.model.thread(&job.thread_id) else {
            return;
        };
        // The branch moved while the forge answered; the next read covers the new one.
        if thread.branch.as_deref() != Some(job.branch.as_str()) {
            return;
        }
        let working = thread.status == ThreadStatus::Working || thread.watching_pull_request;
        let changed = self.pull_requests.record(
            &job.thread_id,
            job.repository.forge,
            result,
            working,
            Instant::now(),
            Utc::now(),
        );
        let Some(pull_request) = changed else {
            return;
        };
        let update = ThreadPullRequest {
            thread_id: job.thread_id.clone(),
            pull_request,
        };
        if let Err(error) = self.store.save_pull_request(&update) {
            eprintln!("hoshi: failed to cache a pull request: {error}");
        }
        (self.notify)(Notification::PullRequest(update));
    }
}

fn slug(title: &str) -> String {
    let slug: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "project".to_owned()
    } else {
        slug.to_owned()
    }
}

/// Starts a turn on `runs_on`, or on what its thread is set to.
fn start_turn(
    state: &mut State,
    thread_id: &str,
    turn_id: &str,
    prompt: String,
    attachments: Vec<MessageAttachment>,
    runs_on: Option<RunsOn>,
) {
    state.idle_compactions.remove(thread_id);
    let Some(provider) = state
        .model
        .thread(thread_id)
        .map(|thread| thread.provider.clone())
    else {
        return;
    };
    let Some(adapter) = state.adapter_for(&provider) else {
        state.receive(
            thread_id,
            turn_id,
            turn_failed(TurnFailure::ProviderDisabled),
        );
        return;
    };
    let Some(cwd) = state.cwd(thread_id) else {
        let failure = TurnFailure::SpawnFailed {
            detail: format!("thread {thread_id} has no folder to run in"),
        };
        state.receive(thread_id, turn_id, turn_failed(failure));
        return;
    };
    let folder = git::normalize(&cwd);
    if state.folder_ops.contains(&folder) {
        let runs_on = runs_on.or_else(|| state.model.thread(thread_id).map(RunsOn::of));
        let Some(runs_on) = runs_on else {
            return;
        };
        state.parked.entry(folder).or_default().push(ParkedTurn {
            thread_id: thread_id.to_owned(),
            turn_id: turn_id.to_owned(),
            prompt,
            attachments,
            runs_on,
        });
        return;
    }
    // Turns hoshi starts itself, such as a task's answer, come here without the restore a
    // message from the user goes through.
    if let Err(error) = state.restore_worktree(thread_id) {
        let failure = TurnFailure::SpawnFailed {
            detail: error.to_string(),
        };
        state.receive(thread_id, turn_id, turn_failed(failure));
        return;
    }
    let prompt = state.carry_cut_work(thread_id, turn_id, prompt);
    let Some(runs_on) = runs_on.or_else(|| state.model.thread(thread_id).map(RunsOn::of)) else {
        return;
    };
    let prepared = match state.prepare_handoff(
        thread_id,
        turn_id,
        &prompt,
        &attachments,
        runs_on.model.as_deref(),
        adapter.as_ref(),
    ) {
        Ok(prepared) => prepared,
        Err(failure) => {
            state.receive(thread_id, turn_id, turn_failed(failure));
            return;
        }
    };
    let (fork_from, context, handoff) = match prepared {
        Some(prepared) => (prepared.fork_from, prepared.context, Some(prepared.handoff)),
        None => (None, None, None),
    };
    let Some(thread) = state.model.thread(thread_id) else {
        return;
    };
    let attachments = state.turn_attachments(attachments);
    let earlier_media = state.earlier_media(thread_id, &attachments);
    let depth = state.depth(thread_id);
    let mcp = state
        .mcp
        .as_ref()
        .filter(|_| depth < delegation::MAX_DEPTH)
        .and_then(|server| {
            Some(hoshi_providers::McpServer {
                url: server.url.clone(),
                authorization: format!("Bearer {}", server.token_for(thread_id)?),
            })
        });
    let bitbucket = state.opens_pull_requests(thread_id);
    let instructions = mcp.as_ref().map(|_| {
        delegation::instructions(
            depth,
            thread.project_id.is_some(),
            bitbucket,
            state.browser.agent_access,
        )
    });
    let mut request = TurnRequest {
        cwd: cwd.to_owned(),
        prompt,
        attachments,
        earlier_media,
        resume_session_id: thread.session_id.clone(),
        resume_session_at: thread.resume_at.clone(),
        fork_session: false,
        handoff: context,
        effort: state
            .known_models(&provider)
            .map_or(runs_on.effort, |catalog| {
                taken_effort(&catalog, runs_on.model.as_deref(), runs_on.effort)
            }),
        model: runs_on.model,
        fast_mode: runs_on.fast_mode,
        ultracode: runs_on.ultracode,
        access_mode: thread.access_mode,
        plan_mode: thread.plan_mode,
        mcp,
        instructions,
        subagent_effort: state.subagent_effort,
        environment: servers::agent_environment(thread_id),
    };
    if let Some((session, cursor)) = fork_from {
        request.resume_session_id = Some(session);
        request.resume_session_at = Some(cursor);
        request.fork_session = true;
    }
    let delivers_inline = request.handoff.is_some() && !adapter.injects_context();
    let compacting = request.prompt.trim() == COMPACT_COMMAND && request.attachments.is_empty();
    let resumed = request
        .resume_session_id
        .as_deref()
        .filter(|_| !request.fork_session);
    state.note_resumed_session(thread_id, resumed);
    let (workspace, checkpointed) = state.turn_workspace(thread_id, turn_id, &cwd);
    state.close_lingering_if_idle(thread_id, SystemTime::now());
    if let Some(lingering) = state.lingering.remove(thread_id) {
        if lingering.running.continue_with(&request) {
            let turn = Turn {
                id: turn_id.to_owned(),
                process: lingering.process,
                running: lingering.running,
                workspace,
                checkpointed,
                arrivals: 0,
                cursor: None,
                background: lingering.background,
                compacting,
                handoff,
            };
            state.turns.insert(thread_id.to_owned(), turn);
            if delivers_inline {
                state.handoff_delivered(thread_id, turn_id, false);
            }
            return;
        }
        if lingering.runs_background_work() {
            state.lingering.insert(thread_id.to_owned(), lingering);
            state.abandon_handoff(handoff);
            let refused = turn_failed(TurnFailure::BackgroundWorkRunning);
            state.receive(thread_id, turn_id, refused);
            return;
        }
        state.end_lingering(thread_id, lingering);
    }
    state.processes += 1;
    let process = state.processes;
    let weak: Weak<Mutex<State>> = state.this.clone();
    let sink_thread = thread_id.to_owned();
    let sink = Box::new(move |output| {
        if let Some(shared) = weak.upgrade() {
            lock(&shared).receive_from(&sink_thread, process, output);
        }
    });
    match adapter.start_turn(request, sink) {
        Ok(running) => {
            let turn = Turn {
                id: turn_id.to_owned(),
                process,
                running,
                workspace,
                checkpointed,
                arrivals: 0,
                cursor: None,
                background: None,
                compacting,
                handoff,
            };
            state.turns.insert(thread_id.to_owned(), turn);
            if delivers_inline {
                state.handoff_delivered(thread_id, turn_id, false);
            }
        }
        Err(failure) => {
            state.abandon_handoff(handoff);
            state.receive(thread_id, turn_id, turn_failed(failure));
        }
    }
}

impl State {
    /// What a turn starting now in `cwd` reads its changes against: the project's worktrees
    /// before it, and a checkpoint of the folder, queued now.
    fn turn_workspace(
        &self,
        thread_id: &str,
        turn_id: &str,
        cwd: &str,
    ) -> (Option<TurnWorkspace>, Option<String>) {
        let project = self
            .model
            .thread(thread_id)
            .and_then(|thread| thread.project_id.as_deref())
            .and_then(|project_id| self.model.project(project_id));
        let workspace = project.and_then(|project| {
            let repository = project.workspace_root.clone();
            let worktrees_before = git::worktrees(&repository)?;
            Some(TurnWorkspace {
                repository,
                worktrees_before,
                tool_inputs: Vec::new(),
                moved: false,
            })
        });
        let checkpointed = workspace.as_ref().map(|_| cwd.to_owned());
        if let Some(dir) = &checkpointed {
            let (dir, reference) = (
                dir.clone(),
                checkpoint::reference(thread_id, turn_id, Moment::Before),
            );
            self.checkpoints.queue(&dir.clone(), move || {
                if let Err(error) = checkpoint::capture(&dir, &reference) {
                    eprintln!("hoshi: no checkpoint before the turn in {dir}: {error}");
                }
            });
        }
        (workspace, checkpointed)
    }
}

/// Checks the enabled providers' CLIs, or only `only`'s. Skipped while an update or
/// another check runs.
fn check_provider_versions(state: &Mutex<State>, maintenance: &Mutex<()>, only: Option<&str>) {
    let _checking = match maintenance.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return,
    };
    let adapters: Vec<_> = lock(state)
        .adapters
        .iter()
        .filter(|adapter| only.is_none_or(|provider| adapter.provider() == provider))
        .cloned()
        .collect();
    for adapter in adapters {
        let provider = adapter.provider();
        adapter.forget_version();
        let Some(installation) = adapter.installation() else {
            continue;
        };
        let mut state = lock(state);
        if state.adapter_for(provider).is_none() {
            continue;
        }
        if provider == DEFAULT_PROVIDER {
            state.provider_version_checked_at = Some(Instant::now());
        }
        let update = state
            .provider_versions
            .get(provider)
            .map(|known| known.update.clone())
            .unwrap_or_default();
        state.keep_provider_version(provider, &installation, update);
    }
}

/// Turns spawn the CLI the update replaces, so it refuses while any runs.
fn update_provider(state: &Mutex<State>, maintenance: &Mutex<()>, provider: &str) {
    let _updating = maintenance
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (adapter, turns_running) = {
        let state = lock(state);
        (state.adapter_for(provider), !state.turns.is_empty())
    };
    let Some(adapter) = adapter else {
        return;
    };
    let failed = |reason| ProviderUpdate::Failed {
        reason,
        output: None,
    };
    if turns_running {
        lock(state).set_provider_update(provider, failed(ProviderUpdateFailure::TurnsRunning));
        return;
    }
    lock(state).set_provider_update(
        provider,
        ProviderUpdate::Running {
            line: None,
            step: None,
        },
    );
    let Some(before) = adapter.installation() else {
        lock(state).set_provider_update(provider, failed(ProviderUpdateFailure::Unsupported));
        return;
    };
    let Some(updater) = before.updater.clone() else {
        lock(state).keep_provider_version(
            provider,
            &before,
            failed(ProviderUpdateFailure::Unsupported),
        );
        return;
    };
    if before.installed.is_some() && before.latest.is_some() && !before.update_available() {
        lock(state).keep_provider_version(provider, &before, ProviderUpdate::Idle);
        return;
    }
    let mut notified_at: Option<Instant> = None;
    let mut shown_step = None;
    let result = run_update(&updater, |progress| {
        let (line, step) = match progress {
            UpdateProgress::Line(line) => (Some(line), None),
            UpdateProgress::Step(step) => (None, Some(step)),
        };
        let next_step = step.is_some_and(|step| {
            shown_step
                .is_none_or(|shown| std::mem::discriminant(&shown) != std::mem::discriminant(&step))
        });
        if next_step || notified_at.is_none_or(|at| at.elapsed() >= UPDATE_PROGRESS_EVERY) {
            notified_at = Some(Instant::now());
            shown_step = step.or(shown_step);
            lock(state).set_provider_update(provider, ProviderUpdate::Running { line, step });
        }
    });
    adapter.forget_version();
    let after = adapter.installation().unwrap_or(before.clone());
    let was_missing = before.installed.is_none() && after.installed.is_some();
    let account = was_missing.then(|| adapter.account_status());
    let update = match result {
        Err(failure) => ProviderUpdate::Failed {
            reason: failure.reason,
            output: failure.output,
        },
        Ok(()) if after.update_available() || after.installed.is_none() => {
            ProviderUpdate::Unchanged
        }
        Ok(()) => ProviderUpdate::Succeeded,
    };
    let mut state = lock(state);
    state.keep_provider_version(provider, &after, update);
    if let Some(account) = account
        && state.adapter_for(provider).is_some()
    {
        state.provider_accounts.insert(provider.to_owned(), account);
        state.notify_providers();
    }
}

/// Checks again every few hours, for as long as the engine lives.
fn watch_provider_version(state: Weak<Mutex<State>>, maintenance: Arc<Mutex<()>>) {
    thread::spawn(move || {
        loop {
            thread::sleep(PROVIDER_VERSION_CHECK_EVERY);
            let Some(state) = state.upgrade() else {
                return;
            };
            check_provider_versions(&state, &maintenance, None);
        }
    });
}

/// One lock per provider, held while it is asked for its limits.
type UsageProbes = Mutex<HashMap<String, Arc<Mutex<()>>>>;

fn check_usage_limits(
    state: &Mutex<State>,
    probes: &UsageProbes,
    provider: &str,
    force: bool,
) -> Option<UsageLimits> {
    let requested_at = Instant::now();
    let probe = hold(probes).entry(provider.to_owned()).or_default().clone();
    let _probing = hold(&probe);
    let adapter = {
        let state = lock(state);
        let fresh = state
            .usage_checked_at
            .get(provider)
            .is_some_and(|checked_at| {
                if force {
                    *checked_at >= requested_at
                } else {
                    checked_at.elapsed() < USAGE_LIMITS_FRESH_FOR
                }
            });
        if fresh {
            return state.usage_limits.get(provider).cloned();
        }
        state.adapter_for(provider)?
    };
    let limits = adapter.usage_limits()?;
    let mut state = lock(state);
    state
        .usage_checked_at
        .insert(provider.to_owned(), Instant::now());
    state.keep_usage_limits(provider, limits.clone());
    Some(limits)
}

/// `effort` when the model `catalog` runs for `model` takes it, else that model's default, as
/// a turn's history records it.
fn taken_effort(
    catalog: &ProviderModels,
    model: Option<&str>,
    effort: Option<hoshi_core::Effort>,
) -> Option<hoshi_core::Effort> {
    let effort = effort?;
    let model = model.or(catalog.default_model.as_deref());
    match catalog
        .models
        .iter()
        .find(|known| Some(known.slug.as_str()) == model)
    {
        Some(known) if !known.efforts.contains(&effort) => known.default_effort,
        _ => Some(effort),
    }
}

/// How many forks back a fork's history is followed to find what a turn ran on.
const MAX_FORK_DEPTH: usize = 8;

/// `choice` with the model and effort `catalog` runs for its defaults: the catalog's default
/// model, and the model's default effort when it was none or, unless `recorded` as what a
/// turn ran on, one the model doesn't take.
fn name_choice(choice: &mut ModelChoice, catalog: &ProviderModels, recorded: bool) {
    if choice.model.is_none() {
        choice.model.clone_from(&catalog.default_model);
    }
    let Some(model) = catalog
        .models
        .iter()
        .find(|model| Some(&model.slug) == choice.model.as_ref())
    else {
        return;
    };
    let taken = |effort: hoshi_core::Effort| recorded || model.efforts.contains(&effort);
    if !choice.effort.is_some_and(taken) {
        choice.effort = model.default_effort;
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    hold(state)
}

fn hold<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn unix_millis() -> u64 {
    Utc::now().timestamp_millis().max(0) as u64
}

/// GitHub through its CLI's account, Bitbucket Cloud through the API token kept in the
/// system's credential store.
pub fn default_forges() -> Vec<Arc<dyn Forge>> {
    vec![
        Arc::new(GitHub::with_cli_credentials()),
        Arc::new(Bitbucket::new(|| {
            BitbucketCredentials::decode(&secrets::read(SecretKey::Bitbucket)?)
        })),
    ]
}

fn forge_error(error: ForgeError) -> CommandError {
    match error {
        ForgeError::Unauthorized => CommandError::ForgeUnauthorized,
        ForgeError::RateLimited { .. } => CommandError::ForgeRateLimited,
        ForgeError::NotFound => CommandError::ForgeUnavailable,
        ForgeError::Rejected(detail)
        | ForgeError::Conflict(detail)
        | ForgeError::Forbidden {
            message: detail, ..
        } => CommandError::ForgeRejected { detail },
        ForgeError::Network(detail) | ForgeError::Unexpected(detail) => {
            CommandError::ForgeUnreachable { detail }
        }
    }
}

fn internal(error: StoreError) -> CommandError {
    CommandError::Internal {
        detail: error.to_string(),
    }
}

#[cfg(test)]
mod tests;
