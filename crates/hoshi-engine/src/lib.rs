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
mod engine;
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
mod state;
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
    /// The projects whose `origin` was read, whether a forge serves it or not.
    origins_read: HashSet<String>,
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
    /// What messages on their way run on, by message id, whatever is picked for the user's next
    /// one; `None` for where their thread is.
    routes: HashMap<String, Option<Box<ModelChoice>>>,
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
    /// The providers whose CLI is being updated or waits to be; their turns wait in `parked`
    /// under the provider's id.
    updating: HashSet<String>,
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
    /// What was picked for it, sent while another provider's turn ran or the thread compacts.
    /// The transcript takes it once its turn starts there.
    #[serde(default)]
    to: Option<Box<ModelChoice>>,
    /// Where the thread was when a message of the user's that its transcript has was queued, for it to run there
    /// though one queued before moved the thread.
    #[serde(default)]
    sent_on: Option<Box<ModelChoice>>,
}

impl QueuedMessage {
    /// Whether its turn moves `thread` to another provider.
    fn moves(&self, thread: &hoshi_core::Thread) -> bool {
        self.to
            .as_ref()
            .or(self.sent_on.as_ref())
            .is_some_and(|on| on.provider != thread.provider)
    }
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
    /// It runs in a process kept for a delegated task that owed its answer: the run the
    /// agent woke for, which it takes in, reopens the task as a woken turn would.
    owes_answer: bool,
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
            origins_read: HashSet::new(),
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
            routes: HashMap::new(),
            mcp: None,
            delegations: HashMap::new(),
            after_compaction: HashMap::new(),
            idle_compaction: IdleCompaction::default(),
            idle_compactions: HashMap::new(),
            folder_ops: HashSet::new(),
            parked: HashMap::new(),
            updating: HashSet::new(),
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
        state.forget_unused_chat_folders();
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
    state.agent_moved(thread_id, &path);
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
                state.repositories.insert(project_id.clone(), repository);
            }
            state.origins_read.insert(project_id);
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
            lock(&shared).start_waiting_switches();
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
    let waits_for = if state.folder_ops.contains(&folder) {
        Some(folder)
    } else {
        state.updating.contains(&provider).then(|| provider.clone())
    };
    if let Some(waits_for) = waits_for {
        let runs_on = runs_on.or_else(|| state.model.thread(thread_id).map(RunsOn::of));
        let Some(runs_on) = runs_on else {
            return;
        };
        state.parked.entry(waits_for).or_default().push(ParkedTurn {
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
    state.read_origin(thread_id);
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
            let background = if lingering.ended {
                state.clear_background(thread_id, lingering.background.as_ref());
                None
            } else {
                state.resumed_background(lingering.background)
            };
            let turn = Turn {
                id: turn_id.to_owned(),
                process: lingering.process,
                running: lingering.running,
                workspace,
                checkpointed,
                arrivals: 0,
                cursor: None,
                background,
                compacting,
                handoff,
                owes_answer: lingering.owes_answer,
            };
            state.turns.insert(thread_id.to_owned(), turn);
            if delivers_inline {
                state.handoff_delivered(thread_id, turn_id, false);
            }
            return;
        }
        if lingering.runs_background_work() && lingering.running.lingers() {
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
                owes_answer: false,
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

/// How an update ended.
struct UpdateEnd {
    /// `None` when it never got to look at the CLI.
    installation: Option<Installation>,
    update: ProviderUpdate,
    /// Read again for a CLI that was missing.
    account: Option<ProviderAccountStatus>,
}

/// Updates a provider [`State::begin_update`] marked, then lets its turns start again. The mark
/// goes before the end is told, so whoever reads the models then reads the updated CLI's.
fn update_provider(state: &Mutex<State>, maintenance: &Mutex<()>, provider: &str) {
    let end = run_provider_update(state, maintenance, provider);
    let mut state = lock(state);
    state.updating.remove(provider);
    if let Some(end) = end {
        match end.installation {
            Some(installation) => state.keep_provider_version(provider, &installation, end.update),
            None => state.set_provider_update(provider, end.update),
        }
        if let Some(account) = end.account
            && state.adapter_for(provider).is_some()
        {
            state.provider_accounts.insert(provider.to_owned(), account);
            state.notify_providers();
        }
    }
    state.start_parked(provider);
}

/// Turns spawn the CLI the update replaces, so it refuses while any runs.
fn run_provider_update(
    state: &Mutex<State>,
    maintenance: &Mutex<()>,
    provider: &str,
) -> Option<UpdateEnd> {
    let _updating = maintenance
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (adapter, turns_running) = {
        let state = lock(state);
        let busy = !state.turns.is_empty() || state.keeps_agents_busy(provider);
        (state.adapter_for(provider), busy)
    };
    let adapter = adapter?;
    let ended = |installation, update| {
        Some(UpdateEnd {
            installation,
            update,
            account: None,
        })
    };
    let failed = |reason| ProviderUpdate::Failed {
        reason,
        output: None,
    };
    if turns_running {
        return ended(None, failed(ProviderUpdateFailure::TurnsRunning));
    }
    let Some(before) = adapter.installation() else {
        return ended(None, failed(ProviderUpdateFailure::Unsupported));
    };
    let Some(updater) = before.updater.clone() else {
        return ended(Some(before), failed(ProviderUpdateFailure::Unsupported));
    };
    if before.installed.is_some() && before.latest.is_some() && !before.update_available() {
        return ended(Some(before), ProviderUpdate::Idle);
    }
    lock(state).close_lingering_of(provider);
    adapter.end_idle_processes();
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
    Some(UpdateEnd {
        installation: Some(after),
        update,
        account,
    })
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
        if state.updating.contains(provider) {
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
