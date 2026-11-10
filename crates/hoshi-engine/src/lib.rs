//! Runtime loop: takes commands, lets the core decide events, persists them
//! through the store, notifies the UI, then runs the resulting effects
//! (provider turns, git). Provider output comes back through the same path.

mod attachments;
mod background;
mod browser;
mod checkpoint;
mod delegation;
mod diff;
mod editors;
mod folders;
mod git;
mod git_actions;
mod mcp;
use delegation::Delegation;
use mcp::{McpServer, ToolFilter, ToolHandler};
mod paths;
mod project_icon;
mod pull_request_cache;
mod pull_request_tool;
mod pull_request_watch;
mod pull_requests;
mod refreshing;
mod secret_requests;
mod secrets;
mod setup_files;
mod source_control;
mod terminal;
mod thread_tools;
mod titles;
mod watch;
mod worktree;
mod worktree_setup;

use attachments::Attachments;
use background::Lingering;
use browser::tools::Browsing;
use checkpoint::{Checkpointer, Moment};
use chrono::{SecondsFormat, Utc};
use hoshi_core::ProviderAccountStatus;
use hoshi_core::{BankedResetFailure, BankedResetSpend};
use hoshi_core::{
    BrowserFrame, BrowserInput, BrowserPage, GitActionProgress, ProviderUpdate,
    ProviderUpdateFailure, ProviderVersion, TranscriptPage,
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
    start_queued_turn, turn_failed,
};
use hoshi_forge::{
    Bitbucket, BitbucketCredentials, Forge, ForgeError, GitHub, RepositoryRef,
    parse_pull_request_url, parse_remote, parse_repository_name,
};
use hoshi_providers::{
    Installation, ProviderAdapter, RunningTurn, TitleRequest, TurnAttachment, TurnRequest, Updater,
    run_update,
};
use hoshi_store::{CachedPullRequestDetail, Store, StoreError};
use pull_request_cache::{PullRequestCache, Version};
use pull_request_watch::Wake;
use pull_requests::PullRequestSync;
use refreshing::Refreshing;
use secrets::SecretKey;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError, Weak, mpsc};
use std::thread;
use std::time::{Duration, Instant};
pub use terminal::TerminalSink;
use terminal::{Launch, Terminals};
use watch::GitWatcher;
use worktree::TurnWorkspace;
use worktree_setup::{SetupLaunch, WorktreeSetups};

pub enum Notification {
    Event(EventEnvelope),
    Stream(StreamDelta),
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
    /// `None` once the provider is disabled.
    ProviderVersion(Option<ProviderVersion>),
    /// The step a running git action reached.
    GitActionProgress(GitActionProgress),
    /// What a thread's browser page painted, while the panel shows it.
    BrowserFrame(BrowserFrame),
    /// Where a thread's browser page is, and whether the agent just acted on it.
    BrowserPage(BrowserPage),
    /// A worktree's setup command started, wrote output, or ended.
    WorktreeSetup(hoshi_core::WorktreeSetupRun),
}

pub type Notify = Arc<dyn Fn(Notification) + Send + Sync>;

/// Shared by every thread that talks to the engine: Tauri commands and the
/// reader threads of running turns.
pub struct Engine {
    state: Arc<Mutex<State>>,
    terminals: Arc<Terminals>,
    /// The headless browser agents open pages in; apart from the state, as pages are slow.
    browsing: Arc<Mutex<Browsing>>,
    /// Per working folder.
    paths: Refreshing<Vec<paths::Indexed>>,
    /// Per working folder, which may add its own commands and skills.
    slash_commands: Refreshing<Vec<SlashCommand>>,
    /// Held while the provider is asked for its limits, so one ask serves everyone waiting.
    usage_probe: Arc<Mutex<()>>,
    /// Held while a banked reset is spent: the grant and the request id a retry reuses until
    /// the provider answers.
    reset_spend: Mutex<Option<(String, String)>>,
    /// Held while the provider's CLI is checked against its releases or updated.
    maintenance: Arc<Mutex<()>>,
}

const LINKED_PULL_REQUEST_FRESH_FOR: Duration = Duration::from_secs(60);
const PATHS_FRESH_FOR: Duration = Duration::from_secs(10);
const SLASH_COMMANDS_FRESH_FOR: Duration = Duration::from_secs(5 * 60);
const USAGE_LIMITS_FRESH_FOR: Duration = Duration::from_secs(60);
const PROVIDER_VERSION_CHECK_EVERY: Duration = Duration::from_secs(60 * 60);
const PROVIDER_VERSION_FRESH_FOR: Duration = Duration::from_secs(10 * 60);
const UPDATE_PROGRESS_EVERY: Duration = Duration::from_millis(250);
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
    /// `None` when the provider is disabled.
    adapter: Option<Arc<dyn ProviderAdapter>>,
    notify: Notify,
    /// New worktrees go under here, one folder per project.
    worktrees_dir: PathBuf,
    /// Each chat works in its own folder under here, named by its id.
    chats_dir: PathBuf,
    attachments: Attachments,
    /// Lets background work (branch refreshes) reach the state without keeping it alive.
    this: Weak<Mutex<State>>,
    /// The projects' setup commands running in their new worktrees.
    setups: WorktreeSetups,
    /// `None` when the platform watcher failed to start; branches then refresh after turns only.
    watcher: Option<GitWatcher>,
    forges: HashMap<ForgeKind, Arc<dyn Forge>>,
    /// The forge repository of each project whose `origin` a forge serves.
    repositories: HashMap<String, RepositoryRef>,
    pull_requests: PullRequestSync,
    /// The files of the pull request last read for each thread, with their patches, so the
    /// UI can ask for one patch at a time.
    /// What the side panel last read of each thread's pull request.
    pull_request_cache: PullRequestCache,
    /// Pull requests read for links in the transcript, with when they were read.
    linked_pull_requests: HashMap<(RepositoryRef, u32), (Instant, PullRequest)>,
    context_usages: HashMap<String, ContextUsage>,
    thread_titles: ThreadTitleSettings,
    subagent_effort: SubagentEffort,
    /// Threads whose generated title missed the subject; named again after their turn.
    titles_to_refine: HashSet<String>,
    /// `None` until the provider reports or is asked for them; kept across restarts.
    usage_limits: Option<UsageLimits>,
    /// When the provider last answered for its limits in this run.
    usage_checked_at: Option<Instant>,
    /// `None` until the enabled provider's CLI was checked.
    provider_version: Option<ProviderVersion>,
    provider_version_checked_at: Option<Instant>,
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
}

/// A message waiting for the thread's running turn to end.
struct QueuedMessage {
    id: String,
    text: String,
    attachments: Vec<MessageAttachment>,
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
}

/// What a background check for a worktree the turn created needs from the state.
struct WorktreeCheck {
    turn_id: String,
    workspace: TurnWorkspace,
    /// Worktrees of other threads.
    claimed: Vec<String>,
}

impl Engine {
    /// Keeps everything under `data_dir`: the database, worktrees, chats and attachments.
    pub fn open(data_dir: &Path, notify: Notify) -> Result<Self, StoreError> {
        Self::from_store(
            Store::open(&data_dir.join("state.sqlite"))?,
            data_dir,
            notify,
        )
    }

    /// Turns that were running when the app last closed are recorded as
    /// interrupted, and every thread's branch is read again. Files go under `data_dir`.
    pub fn from_store(store: Store, data_dir: &Path, notify: Notify) -> Result<Self, StoreError> {
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
        let usage_limits = store.usage_limits().unwrap_or_else(|error| {
            eprintln!("hoshi: failed to read the cached usage limits: {error}");
            None
        });
        let terminals = Arc::new(Terminals::default());
        let setups = WorktreeSetups::new(terminals.clone(), notify.clone());
        let mut state = State {
            setups,
            model: ReadModel::from_snapshot(&snapshot),
            sequence: snapshot.sequence,
            store,
            turns: HashMap::new(),
            lingering: HashMap::new(),
            processes: 0,
            adapter: None,
            notify,
            worktrees_dir: data_dir.join("worktrees"),
            chats_dir: data_dir.join("chats"),
            attachments: Attachments::new(data_dir.join("attachments")),
            this: Weak::new(),
            watcher: None,
            sync_wake: None,
            forges: HashMap::new(),
            repositories: HashMap::new(),
            pull_requests,
            pull_request_cache: PullRequestCache::default(),
            linked_pull_requests: HashMap::new(),
            context_usages,
            usage_limits,
            usage_checked_at: None,
            provider_version: None,
            provider_version_checked_at: None,
            session_moves: HashMap::new(),
            queued: HashMap::new(),
            mcp: None,
            delegations: HashMap::new(),
            after_compaction: HashMap::new(),
            folder_ops: HashSet::new(),
            parked: HashMap::new(),
            thread_ops: HashMap::new(),
            checkpoints: Arc::new(Checkpointer::new()),
            secrets_dir: data_dir.join("secrets"),
            secret_waits: HashMap::new(),
            watches,
            watch_reads: HashMap::new(),
            pending_wakes: HashMap::new(),
            viewers: HashMap::new(),
            thread_titles: ThreadTitleSettings::default(),
            subagent_effort: SubagentEffort::default(),
            titles_to_refine: HashSet::new(),
        };
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
            .map(|thread| thread.id.clone())
            .collect();
        let shared = Arc::new_cyclic(|this| {
            state.this = this.clone();
            Mutex::new(state)
        });
        restore_pull_request_details(Arc::downgrade(&shared));
        refresh_branches(Arc::downgrade(&shared), thread_ids);
        let weak = Arc::downgrade(&shared);
        let browsing = Arc::new(Mutex::new(Browsing::new(
            data_dir.join("browser"),
            lock(&shared).notify.clone(),
        )));
        let pages = browsing.clone();
        let tools: Arc<ToolHandler> = Arc::new(move |caller, tool, arguments| {
            if tool.starts_with("preview_") {
                let mut pages = pages
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                return pages.call(caller, tool, arguments);
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
        Ok(Self {
            state: shared,
            terminals,
            browsing,
            paths: Refreshing::new(PATHS_FRESH_FOR),
            slash_commands: Refreshing::new(SLASH_COMMANDS_FRESH_FOR),
            usage_probe: Arc::default(),
            reset_spend: Mutex::default(),
            maintenance,
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
        read_branches(&Arc::downgrade(&self.state), &[thread_id.to_owned()]);
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
        if self.terminals.any_open_for(thread_id) {
            return Err(CommandError::TerminalsOpen {
                thread_id: thread_id.to_owned(),
            });
        }
        let release = Command::ThreadWorktreeRelease {
            thread_id: thread_id.to_owned(),
        };
        let (op, root, path, checkpoints) = {
            let mut state = lock(&self.state);
            let (thread, root) = state.removable(thread_id)?;
            let path =
                thread
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
        let mut state = lock(&self.state);
        let released = removed.and_then(|()| state.decide_and_commit(release));
        state.end_op(op, None)?;
        released
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

    /// Who each forge is signed in as. Asks the forges, so it can take a moment.
    pub fn forge_accounts(&self) -> Vec<ForgeAccount> {
        let forges: Vec<_> = lock(&self.state).forges.values().cloned().collect();
        forges
            .into_iter()
            .map(|forge| ForgeAccount {
                forge: forge.kind(),
                login: forge.viewer().ok(),
            })
            .collect()
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

    /// Also reads the plan's limits in the background, so they are ready when asked for.
    /// Also reads the plan's limits and checks the CLI against its releases in the background.
    pub fn set_adapter(&self, adapter: Option<Arc<dyn ProviderAdapter>>) {
        let enabled = adapter.is_some();
        {
            let mut state = lock(&self.state);
            state.adapter = adapter;
            if !enabled {
                state.provider_version = None;
                (state.notify)(Notification::ProviderVersion(None));
            }
        }
        self.slash_commands.clear();
        if enabled {
            let state = Arc::downgrade(&self.state);
            let probe = self.usage_probe.clone();
            thread::spawn(move || {
                if let Some(state) = state.upgrade() {
                    check_usage_limits(&state, &probe, false);
                }
            });
            let state = Arc::downgrade(&self.state);
            let maintenance = self.maintenance.clone();
            thread::spawn(move || {
                if let Some(state) = state.upgrade() {
                    check_provider_version(&state, &maintenance);
                }
            });
        }
    }

    /// The enabled provider's CLI as last checked, without checking it.
    pub fn provider_version(&self) -> Option<ProviderVersion> {
        lock(&self.state).provider_version.clone()
    }

    /// Checks the CLI against its latest release. While an update or another check
    /// runs, answers with what is known.
    pub fn check_provider_version(&self) -> Option<ProviderVersion> {
        check_provider_version(&self.state, &self.maintenance)
    }

    /// Whether the enabled provider's CLI is installed and signed in. Runs the CLI.
    pub fn provider_account_status(&self) -> ProviderAccountStatus {
        let adapter = lock(&self.state).adapter.clone();
        adapter.map_or(ProviderAccountStatus::Unknown, |adapter| {
            adapter.account_status()
        })
    }

    /// Updates the provider's CLI in the background; its progress arrives as
    /// `Notification::ProviderVersion`.
    pub fn update_provider(&self) {
        let state = Arc::downgrade(&self.state);
        let maintenance = self.maintenance.clone();
        thread::spawn(move || {
            if let Some(state) = state.upgrade() {
                update_provider(&state, &maintenance);
            }
        });
    }

    pub fn set_thread_titles(&self, settings: ThreadTitleSettings) {
        lock(&self.state).thread_titles = settings;
    }

    /// How agents choose the model and effort of their subagents, from their next turn on.
    pub fn set_subagent_effort(&self, policy: SubagentEffort) {
        lock(&self.state).subagent_effort = policy;
    }

    /// Names the thread again from its whole conversation, in the background. The
    /// title only changes if the user did not rename it meanwhile.
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
        let Some(adapter) = lock(&self.state).adapter.clone() else {
            return Ok(None);
        };
        let root = PathBuf::from(&cwd);
        let commands = self
            .slash_commands
            .get(&cwd, move || adapter.slash_commands(&root));
        Ok(commands.map(|commands| commands.as_ref().clone()))
    }

    /// The enabled provider's models. Can run the provider's CLI.
    pub fn models(&self) -> Option<ProviderModels> {
        let adapter = lock(&self.state).adapter.clone()?;
        Some(adapter.models())
    }

    /// The limits last read or reported, possibly in an earlier run, without asking the provider.
    pub fn known_usage_limits(&self) -> Option<UsageLimits> {
        lock(&self.state).usage_limits.clone()
    }

    /// The plan's limits, asking the provider unless it answered within the last minute.
    /// `force` always waits for an answer given after the call. `None` when it can't say.
    pub fn usage_limits(&self, force: bool) -> Option<UsageLimits> {
        check_usage_limits(&self.state, &self.usage_probe, force)
    }

    /// Spends the next banked reset, then reads the limits again. Reaches the network.
    pub fn spend_banked_reset(&self) -> BankedResetSpend {
        let mut pending = self
            .reset_spend
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (adapter, grant) = {
            let state = lock(&self.state);
            let grant = state
                .usage_limits
                .as_ref()
                .and_then(|limits| limits.banked_resets.as_ref())
                .filter(|banked| banked.count > 0)
                .and_then(|banked| banked.next_grant_id.clone());
            (state.adapter.clone(), grant)
        };
        let (Some(adapter), Some(grant)) = (adapter, grant) else {
            return BankedResetSpend::NoneLeft;
        };
        let request_id = match pending.take() {
            Some((pending_grant, request_id)) if pending_grant == grant => request_id,
            _ => delegation::new_id(),
        };
        let spend = adapter.spend_banked_reset(&grant, &request_id);
        if !spend.settled() {
            *pending = Some((grant, request_id));
            return spend;
        }
        if matches!(spend, BankedResetSpend::Failed { .. }) {
            return spend;
        }
        let refreshed = check_usage_limits(&self.state, &self.usage_probe, true);
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

    /// Cancels a delegated task: it stops if it runs, and its answer never goes back.
    pub fn cancel_task(&self, thread_id: &str) {
        lock(&self.state).cancel_task(thread_id);
    }

    pub fn snapshot(&self) -> Snapshot {
        let state = lock(&self.state);
        Snapshot {
            sequence: state.sequence,
            projects: state.model.projects().cloned().collect(),
            threads: state.model.threads().cloned().collect(),
            pending_requests: state.model.all_pending_requests(),
            pending_tasks: state.delegations.keys().cloned().collect(),
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
        state.run_command(command)?;
        let model = &state.model;
        self.terminals
            .retain_threads(|thread_id| model.thread(thread_id).is_some());
        drop(state);
        if !deleted.is_empty() {
            let mut browsing = self.browsing();
            for thread_id in &deleted {
                browsing.forget(thread_id);
            }
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

    fn browsing(&self) -> std::sync::MutexGuard<'_, Browsing> {
        self.browsing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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

    pub fn browser_close(&self, thread_id: &str) {
        self.browsing().forget(thread_id);
    }

    /// Closes every page and the headless browser, as when the app quits.
    /// Ends the agent processes kept between turns, and the commands running in them, as
    /// when the app quits.
    pub fn close_lingering_agents(&self) {
        lock(&self.state).close_all_lingering();
    }

    pub fn close_browser(&self) {
        self.browsing().close();
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
            return state.run_command(send);
        }
        state.after_compaction.insert(
            thread_id.to_owned(),
            QueuedMessage {
                id: message_id.to_owned(),
                text,
                attachments,
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
            state.after_compaction.remove(thread_id);
        }
        compacted
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

    /// An image the user removed before sending it.
    pub fn discard_image(&self, image: &MessageAttachment) {
        lock(&self.state).attachments.remove(image);
    }

    /// A sent image as a data URL; `None` when its file is gone.
    pub fn image_data_url(&self, id: &str, mime_type: &str) -> Option<String> {
        lock(&self.state).attachments.data_url(id, mime_type)
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
}

impl State {
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
                self.restore_released_worktree(thread_id)?;
                Ok(command)
            }
            Command::ThreadCreate {
                thread_id,
                project_id,
                title,
                workspace:
                    workspace @ (ThreadWorkspace::NewWorktree | ThreadWorkspace::BranchWorktree { .. }),
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
        let root = self.project_root(thread)?;
        Ok((thread, root))
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
    /// The images in the transcripts of the threads `command` deletes.
    fn images_freed_by(&self, command: &Command) -> Vec<MessageAttachment> {
        self.threads_deleted_by(command)
            .into_iter()
            .flat_map(|thread| self.store.thread_items(&thread.id).unwrap_or_default())
            .flat_map(|item| match item.content {
                ItemContent::UserMessage { attachments, .. } => attachments,
                _ => Vec::new(),
            })
            .filter(|attachment| matches!(attachment, MessageAttachment::Image { .. }))
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
        self.threads_deleted_by(command)
            .into_iter()
            .filter(|thread| !thread.worktree_adopted)
            .filter_map(|thread| {
                let path = thread.worktree_path.clone()?;
                let project = self.model.project(thread.project_id.as_deref()?)?;
                Some((project.workspace_root.clone(), path))
            })
            .collect()
    }

    fn commit(&mut self, events: &[Event], now: &str) -> Result<(), CommandError> {
        if events.is_empty() {
            return Ok(());
        }
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
        Ok(())
    }

    fn receive(&mut self, thread_id: &str, turn_id: &str, output: ProviderOutput) {
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
        let active = self
            .model
            .thread(thread_id)
            .is_some_and(|thread| thread.active_turn_id.as_deref() == Some(turn_id));
        if active {
            (self.notify)(Notification::Stream(StreamDelta {
                thread_id: thread_id.to_owned(),
                item_id,
                kind,
                delta,
            }));
        }
    }

    fn record(&mut self, thread_id: &str, turn_id: &str, output: ProviderOutput) {
        match output {
            ProviderOutput::ContextUsage(usage) => {
                self.record_context_usage(thread_id, turn_id, usage);
                return;
            }
            ProviderOutput::UsageLimits(update) => {
                let mut limits = self.usage_limits.clone().unwrap_or_default();
                limits.merge(update);
                self.keep_usage_limits(limits);
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
        if completed {
            let ended_turn = match self.turns.get(thread_id) {
                Some(turn) if turn.id == turn_id => self.turns.remove(thread_id),
                _ => None,
            };
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
                self.deliver_task_result(thread_id, outcome);
            }
            self.start_next_queued(thread_id);
            self.send_after_compaction(thread_id);
            self.deliver_pending_wakes(thread_id);
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
            let running = match self.turns.get(&thread_id) {
                Some(turn) => Some(&turn.running),
                None => self
                    .lingering
                    .get(&thread_id)
                    .map(|lingering| &lingering.running),
            };
            if let Some(running) = running {
                running.stop_background_task(&task_id);
            }
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
                                    text, attachments, ..
                                },
                            ..
                        },
                } => {
                    message = Some(QueuedMessage {
                        id: id.clone(),
                        text: text.clone(),
                        attachments: attachments.clone(),
                    });
                    joining = Some(thread_id.clone());
                    if let Some((thread_id, provisional)) = titled.first() {
                        let request = TitleRequest {
                            context: text.clone(),
                            previous_title: None,
                            images: titles::image_names(attachments),
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
                    start_turn(self, thread_id, turn_id, prompt, attachments);
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
        self.watches.remove(thread_id);
        self.watch_reads.remove(thread_id);
        self.pending_wakes.remove(thread_id);
        self.titles_to_refine.remove(thread_id);
        self.session_moves.remove(thread_id);
        if let Some(folder) = self.chat_folder(thread_id) {
            let _ = std::fs::remove_dir_all(folder);
        }
    }

    /// Passes a message sent to a busy thread on to its running turn, or keeps it for a turn
    /// of its own when the turn is ending.
    fn join_turn(&mut self, thread_id: &str, message: QueuedMessage) {
        let attachments = self.turn_attachments(message.attachments.clone());
        let joined = self
            .turns
            .get(thread_id)
            .is_some_and(|turn| turn.running.steer(&message.id, &message.text, &attachments));
        if joined {
            return;
        }
        self.queued
            .entry(thread_id.to_owned())
            .or_default()
            .push_back(message);
        // The turn may have ended between the message's events and this call.
        if !self.turns.contains_key(thread_id) {
            self.start_next_queued(thread_id);
        }
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
        let Some(message) = self.after_compaction.remove(thread_id) else {
            return;
        };
        let send = Command::MessageSend {
            thread_id: thread_id.to_owned(),
            message_id: message.id,
            text: message.text,
            attachments: message.attachments,
        };
        if let Err(error) = self.run_command(send) {
            eprintln!("hoshi: failed to send a message after compacting: {error}");
        }
    }

    /// Starts the oldest message waiting for its own turn, such as one sent while the user
    /// stopped the running turn.
    fn start_next_queued(&mut self, thread_id: &str) {
        let Some(message) = self.queued.get_mut(thread_id).and_then(VecDeque::pop_front) else {
            return;
        };
        let now = now();
        let events = start_queued_turn(&self.model, thread_id, &message.id, &now);
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
        );
    }

    fn turn_attachments(&self, attachments: Vec<MessageAttachment>) -> Vec<TurnAttachment> {
        attachments
            .into_iter()
            .map(|attachment| TurnAttachment {
                path: match &attachment {
                    MessageAttachment::Image { id, mime_type, .. } => {
                        self.attachments.path(id, mime_type)
                    }
                    _ => None,
                },
                attachment,
            })
            .collect()
    }

    /// Asks the provider for a title in the background, retrying a first title twice.
    /// It replaces `current` only if the thread still has that title by then.
    fn name_thread(&mut self, thread_id: &str, current: &str, mut request: TitleRequest) {
        let Some(adapter) = self.adapter.clone() else {
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

    fn keep_provider_version(&mut self, installation: &Installation, update: ProviderUpdate) {
        self.provider_version = Some(ProviderVersion {
            installed: installation.installed.clone(),
            latest: installation.latest.clone(),
            update_available: installation.update_available(),
            update_command: installation.updater.as_ref().map(Updater::display),
            update,
        });
        (self.notify)(Notification::ProviderVersion(self.provider_version.clone()));
    }

    fn set_provider_update(&mut self, update: ProviderUpdate) {
        let version = self.provider_version.get_or_insert_default();
        version.update = update;
        (self.notify)(Notification::ProviderVersion(self.provider_version.clone()));
    }

    fn keep_usage_limits(&mut self, limits: UsageLimits) {
        if let Err(error) = self.store.save_usage_limits(&limits) {
            eprintln!("hoshi: failed to cache the usage limits: {error}");
        }
        (self.notify)(Notification::UsageLimits(limits.clone()));
        self.usage_limits = Some(limits);
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
            let now = Utc::now().timestamp_millis().max(0) as u64;
            usage.cache_expires_at = Some(now + ttl * 1000);
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

/// Reads the branch of every thread in the given projects. A running turn that
/// created a worktree moves to it now, not only when the turn ends.
fn refresh_projects(state: &Weak<Mutex<State>>, project_ids: &[String]) {
    let Some(shared) = state.upgrade() else {
        return;
    };
    let (thread_ids, checks) = {
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
        (thread_ids, checks)
    };
    for (thread_id, check) in &checks {
        attach_created_worktree(state, thread_id, check);
    }
    read_branches(state, &thread_ids);
    let mut state = lock(&shared);
    for thread_id in &thread_ids {
        state.pull_requests.check_soon(thread_id);
    }
}

/// Finds each project's `.git` folder off the lock, then starts watching it.
fn watch_projects(state: Weak<Mutex<State>>, projects: Vec<(String, String)>) {
    if projects.is_empty() {
        return;
    }
    thread::spawn(move || {
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

fn read_branches(state: &Weak<Mutex<State>>, thread_ids: &[String]) {
    for thread_id in thread_ids {
        let Some(shared) = state.upgrade() else {
            return;
        };
        let cwd = lock(&shared).model.thread_cwd(thread_id).map(str::to_owned);
        let Some(cwd) = cwd else { continue };
        let branch = git::current_branch(&cwd);
        let mut state = lock(&shared);
        // What git does there may have removed the folder this read found no branch in.
        if state.thread_ops.contains_key(thread_id)
            || state.folder_ops.contains(&git::normalize(&cwd))
        {
            continue;
        }
        let events = record_branch(&state.model, thread_id, branch);
        if !events.is_empty() {
            state.pull_requests.check_soon(thread_id);
        }
        if let Err(error) = state.commit(&events, &now()) {
            eprintln!("hoshi: failed to record a branch: {error}");
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

fn start_turn(
    state: &mut State,
    thread_id: &str,
    turn_id: &str,
    prompt: String,
    attachments: Vec<MessageAttachment>,
) {
    let Some(adapter) = state.adapter.clone() else {
        state.receive(
            thread_id,
            turn_id,
            turn_failed(TurnFailure::ProviderDisabled),
        );
        return;
    };
    if state.model.thread(thread_id).is_none() {
        return;
    }
    let Some(cwd) = state.cwd(thread_id) else {
        let failure = TurnFailure::SpawnFailed {
            detail: format!("thread {thread_id} has no folder to run in"),
        };
        state.receive(thread_id, turn_id, turn_failed(failure));
        return;
    };
    let folder = git::normalize(&cwd);
    if state.folder_ops.contains(&folder) {
        state.parked.entry(folder).or_default().push(ParkedTurn {
            thread_id: thread_id.to_owned(),
            turn_id: turn_id.to_owned(),
            prompt,
            attachments,
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
    let Some(thread) = state.model.thread(thread_id) else {
        return;
    };
    let attachments = state.turn_attachments(attachments);
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
    let instructions = mcp
        .as_ref()
        .map(|_| delegation::instructions(depth, thread.project_id.is_some(), bitbucket));
    let request = TurnRequest {
        cwd: cwd.to_owned(),
        prompt,
        attachments,
        resume_session_id: thread.session_id.clone(),
        resume_session_at: thread.resume_at.clone(),
        model: thread.model.clone(),
        effort: thread.effort,
        fast_mode: thread.fast_mode,
        ultracode: thread.ultracode,
        access_mode: thread.access_mode,
        plan_mode: thread.plan_mode,
        mcp,
        instructions,
        subagent_effort: state.subagent_effort,
    };
    let (workspace, checkpointed) = state.turn_workspace(thread_id, turn_id, &cwd);
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
            };
            state.turns.insert(thread_id.to_owned(), turn);
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
            };
            state.turns.insert(thread_id.to_owned(), turn);
        }
        Err(failure) => state.receive(thread_id, turn_id, turn_failed(failure)),
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

fn check_provider_version(
    state: &Mutex<State>,
    maintenance: &Mutex<()>,
) -> Option<ProviderVersion> {
    let _checking = match maintenance.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return lock(state).provider_version.clone(),
    };
    let adapter = lock(state).adapter.clone()?;
    adapter.forget_version();
    let installation = adapter.installation()?;
    let mut state = lock(state);
    state.provider_version_checked_at = Some(Instant::now());
    let update = state
        .provider_version
        .as_ref()
        .map(|known| known.update.clone())
        .unwrap_or_default();
    state.keep_provider_version(&installation, update);
    state.provider_version.clone()
}

/// Turns spawn the CLI the update replaces, so it refuses while any runs.
fn update_provider(state: &Mutex<State>, maintenance: &Mutex<()>) {
    let _updating = maintenance
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (adapter, turns_running) = {
        let state = lock(state);
        (state.adapter.clone(), !state.turns.is_empty())
    };
    let Some(adapter) = adapter else {
        return;
    };
    let failed = |reason| ProviderUpdate::Failed {
        reason,
        output: None,
    };
    if turns_running {
        lock(state).set_provider_update(failed(ProviderUpdateFailure::TurnsRunning));
        return;
    }
    lock(state).set_provider_update(ProviderUpdate::Running { line: None });
    let Some(before) = adapter.installation() else {
        lock(state).set_provider_update(failed(ProviderUpdateFailure::Unsupported));
        return;
    };
    let Some(updater) = before.updater.clone() else {
        lock(state).keep_provider_version(&before, failed(ProviderUpdateFailure::Unsupported));
        return;
    };
    if before.latest.is_some() && !before.update_available() {
        lock(state).keep_provider_version(&before, ProviderUpdate::Idle);
        return;
    }
    let mut notified_at: Option<Instant> = None;
    let result = run_update(&updater, |line| {
        if notified_at.is_none_or(|at| at.elapsed() >= UPDATE_PROGRESS_EVERY) {
            notified_at = Some(Instant::now());
            lock(state).set_provider_update(ProviderUpdate::Running { line: Some(line) });
        }
    });
    adapter.forget_version();
    let after = adapter.installation().unwrap_or(before);
    let update = match result {
        Err(failure) => ProviderUpdate::Failed {
            reason: failure.reason,
            output: failure.output,
        },
        Ok(()) if after.update_available() => ProviderUpdate::Unchanged,
        Ok(()) => ProviderUpdate::Succeeded,
    };
    lock(state).keep_provider_version(&after, update);
}

/// Checks again every few hours, for as long as the engine lives.
fn watch_provider_version(state: Weak<Mutex<State>>, maintenance: Arc<Mutex<()>>) {
    thread::spawn(move || {
        loop {
            thread::sleep(PROVIDER_VERSION_CHECK_EVERY);
            let Some(state) = state.upgrade() else {
                return;
            };
            check_provider_version(&state, &maintenance);
        }
    });
}

fn check_usage_limits(state: &Mutex<State>, probe: &Mutex<()>, force: bool) -> Option<UsageLimits> {
    let requested_at = Instant::now();
    let _probing = probe
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let adapter = {
        let state = lock(state);
        let fresh = state.usage_checked_at.is_some_and(|checked_at| {
            if force {
                checked_at >= requested_at
            } else {
                checked_at.elapsed() < USAGE_LIMITS_FRESH_FOR
            }
        });
        if fresh {
            return state.usage_limits.clone();
        }
        state.adapter.clone()?
    };
    let limits = adapter.usage_limits()?;
    let mut state = lock(state);
    state.usage_checked_at = Some(Instant::now());
    state.keep_usage_limits(limits.clone());
    Some(limits)
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
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
mod tests {
    use super::*;
    use hoshi_core::{
        AccessMode, ApprovalDecision, BackgroundTask, Effort, GitAction, GitActionRequest,
        RequestResponse, RequestStatus, ThreadStatus, ToolDetail,
    };
    use hoshi_providers::OutputSink;
    use std::sync::mpsc::{self, Receiver, Sender};

    #[derive(Debug, Clone, PartialEq)]
    struct Request {
        cwd: String,
        prompt: String,
        resume: Option<String>,
        resume_at: Option<String>,
        model: Option<String>,
        attachments: Vec<TurnAttachment>,
        instructions: Option<String>,
        subagent_effort: SubagentEffort,
    }

    /// Records each request and replays a script; a turn whose script does not
    /// end it waits for an interrupt.
    struct FakeAdapter {
        requests: Mutex<Vec<Request>>,
        script: Vec<ProviderOutput>,
        /// What a title request answers; `None` declines.
        title: Option<String>,
        /// What the user answered the turns' requests, by request id.
        responses: Arc<Mutex<Vec<(String, RequestResponse)>>>,
        /// Messages that joined a running turn, by id; `None` refuses them all.
        steers: Option<Arc<Mutex<Vec<String>>>>,
        /// Ends each waiting turn, in the order they started.
        finishers: Mutex<Vec<Sender<TurnOutcome>>>,
    }

    struct FakeTurn {
        stop: Sender<TurnOutcome>,
        responses: Arc<Mutex<Vec<(String, RequestResponse)>>>,
        steers: Option<Arc<Mutex<Vec<String>>>>,
    }

    impl RunningTurn for FakeTurn {
        fn interrupt(&self) {
            let _ = self.stop.send(TurnOutcome::Interrupted);
        }

        fn stop_background_task(&self, _task_id: &str) {
            let _ = self.stop.send(TurnOutcome::Completed);
        }

        fn steer(&self, message_id: &str, _prompt: &str, _attachments: &[TurnAttachment]) -> bool {
            let Some(steers) = &self.steers else {
                return false;
            };
            steers.lock().unwrap().push(message_id.to_owned());
            true
        }

        fn respond(&self, request_id: &str, response: &RequestResponse) {
            self.responses
                .lock()
                .unwrap()
                .push((request_id.to_owned(), response.clone()));
        }
    }

    impl ProviderAdapter for FakeAdapter {
        fn models(&self) -> hoshi_core::ProviderModels {
            hoshi_core::ProviderModels::default()
        }

        fn generate_title(
            &self,
            _request: &TitleRequest,
        ) -> Option<hoshi_providers::GeneratedTitle> {
            Some(hoshi_providers::GeneratedTitle {
                title: self.title.clone()?,
                needs_refinement: false,
            })
        }

        fn start_turn(
            &self,
            request: TurnRequest,
            mut sink: OutputSink,
        ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
            let mut requests = self.requests.lock().unwrap();
            let turn = requests.len();
            requests.push(Request {
                cwd: request.cwd,
                prompt: request.prompt,
                resume: request.resume_session_id,
                resume_at: request.resume_session_at,
                model: request.model,
                attachments: request.attachments,
                instructions: request.instructions,
                subagent_effort: request.subagent_effort,
            });
            let script: Vec<_> = self
                .script
                .iter()
                .cloned()
                .map(|output| match output {
                    ProviderOutput::AssistantMessage { item_id, text } => {
                        let item_id = format!("{turn}:{item_id}");
                        ProviderOutput::AssistantMessage { item_id, text }
                    }
                    other => other,
                })
                .collect();
            let (stop, stopped) = mpsc::channel();
            self.finishers.lock().unwrap().push(stop.clone());
            thread::spawn(move || {
                let ends = script
                    .iter()
                    .any(|output| matches!(output, ProviderOutput::TurnCompleted { .. }));
                script.into_iter().for_each(&mut sink);
                if !ends {
                    let outcome = stopped.recv().unwrap_or(TurnOutcome::Interrupted);
                    sink(ProviderOutput::TurnCompleted { outcome });
                }
            });
            Ok(Box::new(FakeTurn {
                stop,
                responses: self.responses.clone(),
                steers: self.steers.clone(),
            }))
        }
    }

    struct Harness {
        engine: Engine,
        adapter: Arc<FakeAdapter>,
        events: Receiver<Event>,
        root: String,
        data_dir: PathBuf,
    }

    struct LimitsAdapter {
        asks: Mutex<u8>,
    }

    impl ProviderAdapter for LimitsAdapter {
        fn models(&self) -> hoshi_core::ProviderModels {
            hoshi_core::ProviderModels::default()
        }

        fn usage_limits(&self) -> Option<UsageLimits> {
            thread::sleep(Duration::from_millis(50));
            let mut asks = self.asks.lock().unwrap();
            *asks += 1;
            Some(UsageLimits {
                windows: vec![hoshi_core::UsageWindow {
                    kind: hoshi_core::UsageWindowKind::Session,
                    model: None,
                    used_percent: *asks,
                    window_minutes: 300,
                    resets_at: None,
                }],
                banked_resets: None,
                account: None,
            })
        }

        fn start_turn(
            &self,
            _request: TurnRequest,
            _sink: OutputSink,
        ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
            Err(TurnFailure::ProviderDisabled)
        }
    }

    /// Banks one reset of `grant_a` and answers each spend with the next of `answers`.
    struct ResetAdapter {
        answers: Mutex<Vec<BankedResetSpend>>,
        spends: Mutex<Vec<(String, String)>>,
    }

    impl ProviderAdapter for ResetAdapter {
        fn models(&self) -> hoshi_core::ProviderModels {
            hoshi_core::ProviderModels::default()
        }

        fn usage_limits(&self) -> Option<UsageLimits> {
            Some(UsageLimits {
                windows: Vec::new(),
                banked_resets: Some(hoshi_core::BankedResets {
                    count: 1,
                    next_grant_id: Some("grant_a".into()),
                    expires_at: Vec::new(),
                }),
                account: None,
            })
        }

        fn spend_banked_reset(&self, grant_id: &str, request_id: &str) -> BankedResetSpend {
            self.spends
                .lock()
                .unwrap()
                .push((grant_id.to_owned(), request_id.to_owned()));
            self.answers.lock().unwrap().remove(0)
        }

        fn start_turn(
            &self,
            _request: TurnRequest,
            _sink: OutputSink,
        ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
            Err(TurnFailure::ProviderDisabled)
        }
    }

    /// Reports 1.0.0 against a 1.1.0 release until its updater ran.
    /// Its update writes `marker`, which the installed version reads.
    struct OutdatedAdapter {
        marker: PathBuf,
    }

    impl ProviderAdapter for OutdatedAdapter {
        fn models(&self) -> hoshi_core::ProviderModels {
            hoshi_core::ProviderModels::default()
        }

        fn installation(&self) -> Option<Installation> {
            let updated = self.marker.is_file();
            let marker = self.marker.display();
            let (program, args) = if cfg!(windows) {
                (
                    "cmd",
                    vec!["/C".to_owned(), format!("echo updated> {marker}")],
                )
            } else {
                (
                    "sh",
                    vec!["-c".to_owned(), format!("echo updated > '{marker}'")],
                )
            };
            Some(Installation {
                installed: Some(if updated { "1.1.0" } else { "1.0.0" }.into()),
                latest: Some("1.1.0".into()),
                updater: Some(Updater {
                    program: program.into(),
                    args,
                }),
            })
        }

        fn start_turn(
            &self,
            _request: TurnRequest,
            _sink: OutputSink,
        ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
            Err(TurnFailure::ProviderDisabled)
        }
    }

    #[test]
    fn an_outdated_cli_is_updated_and_checked_again() {
        let (sender, versions) = mpsc::channel();
        let notify: Notify = Arc::new(move |notification| {
            if let Notification::ProviderVersion(Some(version)) = notification {
                let _ = sender.send(version);
            }
        });
        let dir = scratch_dir("provider-update");
        let engine = Engine::from_store(Store::open_in_memory().unwrap(), &dir, notify).unwrap();
        engine.set_adapter(Some(Arc::new(OutdatedAdapter {
            marker: dir.join("updated"),
        })));

        let checked = engine.check_provider_version().unwrap();
        assert!(checked.update_available);
        assert!(checked.update_command.is_some());

        engine.update_provider();
        let finished = versions
            .iter()
            .find(|version| {
                !matches!(
                    version.update,
                    ProviderUpdate::Idle | ProviderUpdate::Running { .. }
                )
            })
            .unwrap();

        assert_eq!(finished.update, ProviderUpdate::Succeeded);
        assert_eq!(finished.installed.as_deref(), Some("1.1.0"));
        assert!(!finished.update_available);
    }

    #[test]
    fn usage_limits_are_read_once_while_fresh_and_kept_across_restarts() {
        let data_dir = scratch_dir("usage-limits");
        let notify: Notify = Arc::new(|_| {});
        let engine = Engine::open(&data_dir, notify.clone()).unwrap();
        let adapter = Arc::new(LimitsAdapter {
            asks: Mutex::new(0),
        });

        engine.set_adapter(Some(adapter.clone()));
        let first = engine.usage_limits(false).unwrap();
        assert_eq!(*adapter.asks.lock().unwrap(), 1);
        assert_eq!(engine.usage_limits(false), Some(first));

        let forced = engine.usage_limits(true).unwrap();
        assert_eq!(*adapter.asks.lock().unwrap(), 2);
        drop(engine);

        let restarted = Engine::open(&data_dir, notify).unwrap();
        assert_eq!(restarted.known_usage_limits(), Some(forced));
    }

    #[test]
    fn an_unanswered_spend_is_retried_with_the_same_request() {
        let data_dir = scratch_dir("banked-resets");
        let engine = Engine::open(&data_dir, Arc::new(|_| {})).unwrap();
        let unconfirmed = BankedResetSpend::Failed {
            reason: BankedResetFailure::Unconfirmed,
        };
        let adapter = Arc::new(ResetAdapter {
            answers: Mutex::new(vec![
                unconfirmed,
                BankedResetSpend::Reset,
                BankedResetSpend::NothingToReset,
            ]),
            spends: Mutex::default(),
        });

        assert_eq!(engine.spend_banked_reset(), BankedResetSpend::NoneLeft);
        engine.set_adapter(Some(adapter.clone()));
        engine.usage_limits(true).unwrap();
        assert_eq!(engine.spend_banked_reset(), unconfirmed);
        assert_eq!(engine.spend_banked_reset(), BankedResetSpend::Reset);
        assert_eq!(
            engine.spend_banked_reset(),
            BankedResetSpend::NothingToReset
        );

        let spends = adapter.spends.lock().unwrap();
        assert!(spends.iter().all(|(grant, _)| grant == "grant_a"));
        assert_eq!(spends[0].1, spends[1].1);
        assert_ne!(spends[1].1, spends[2].1);
    }

    #[test]
    fn secret_files_past_their_day_go_when_the_engine_starts() {
        let data_dir = scratch_dir("old-secrets");
        let secrets = data_dir.join("secrets");
        std::fs::create_dir_all(&secrets).unwrap();
        let (old, fresh) = (secrets.join("old"), secrets.join("fresh"));
        std::fs::write(&old, "token-1").unwrap();
        std::fs::write(&fresh, "token-2").unwrap();
        let file = std::fs::File::options().write(true).open(&old).unwrap();
        file.set_modified(std::time::SystemTime::now() - Duration::from_secs(25 * 60 * 60))
            .unwrap();
        drop(file);

        let _engine = Engine::from_store(
            Store::open_in_memory().unwrap(),
            &data_dir,
            Arc::new(|_| {}),
        )
        .unwrap();

        assert!(!old.exists());
        assert!(fresh.exists());
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hoshi-engine-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    impl Harness {
        /// An engine with project `p` on `root` and a local thread `t`.
        fn new(name: &str, root: PathBuf, store: Store, script: Vec<ProviderOutput>) -> Self {
            let (sender, events) = mpsc::channel();
            let notify: Notify = Arc::new(move |notification| {
                if let Notification::Event(envelope) = notification {
                    let _ = sender.send(envelope.event);
                }
            });
            let data_dir = scratch_dir(&format!("{name}-data"));
            let engine = Engine::from_store(store, &data_dir, notify).unwrap();
            let adapter = Arc::new(FakeAdapter {
                requests: Mutex::new(Vec::new()),
                script,
                title: None,
                responses: Arc::default(),
                steers: None,
                finishers: Mutex::default(),
            });
            engine.set_adapter(Some(adapter.clone()));
            let root = dunce::canonicalize(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            engine
                .dispatch(Command::ProjectCreate {
                    project_id: "p".into(),
                    title: "Repo".into(),
                    workspace_root: root.clone(),
                })
                .unwrap();
            let harness = Self {
                engine,
                adapter,
                events,
                root,
                data_dir,
            };
            harness.create_thread("t", ThreadWorkspace::Local).unwrap();
            harness
        }

        fn in_memory(name: &str, script: Vec<ProviderOutput>) -> Self {
            Self::new(
                name,
                scratch_dir(name),
                Store::open_in_memory().unwrap(),
                script,
            )
        }

        fn create_thread(&self, id: &str, workspace: ThreadWorkspace) -> Result<(), CommandError> {
            self.create_thread_in(id, Some("p"), workspace)
        }

        fn create_thread_in(
            &self,
            id: &str,
            project_id: Option<&str>,
            workspace: ThreadWorkspace,
        ) -> Result<(), CommandError> {
            self.engine.dispatch(Command::ThreadCreate {
                thread_id: id.into(),
                project_id: project_id.map(str::to_owned),
                title: String::new(),
                workspace,
                model: Some("haiku".into()),
                effort: Some(Effort::Low),
                fast_mode: false,
                ultracode: false,
                access_mode: AccessMode::AcceptEdits,
                plan_mode: false,
            })
        }

        fn send(&self, thread_id: &str, message_id: &str, text: &str) {
            self.engine
                .dispatch(Command::MessageSend {
                    thread_id: thread_id.into(),
                    message_id: message_id.into(),
                    text: text.into(),
                    attachments: Vec::new(),
                })
                .unwrap();
        }

        fn wait_for<T>(&self, find: impl FnMut(Event) -> Option<T>) -> T {
            self.events.iter().find_map(find).unwrap()
        }

        fn wait_for_turn_end(&self) -> TurnOutcome {
            self.wait_for(|event| match event {
                Event::TurnCompleted { outcome, .. } => Some(outcome),
                _ => None,
            })
        }

        fn thread(&self, thread_id: &str) -> hoshi_core::Thread {
            self.engine
                .snapshot()
                .threads
                .into_iter()
                .find(|thread| thread.id == thread_id)
                .unwrap()
        }
    }

    #[test]
    fn a_turn_s_background_commands_are_known_until_it_ends_and_can_be_stopped() {
        let harness = Harness::in_memory("background-commands", Vec::new());
        harness.send("t", "m1", "run it in the background");
        let turn_id = harness.wait_for(|event| match event {
            Event::TurnStarted { turn_id, .. } => Some(turn_id),
            _ => None,
        });
        let task = BackgroundTask {
            task_id: "b1".into(),
            tool_call_id: Some("toolu_1".into()),
            description: "Start the dev server".into(),
            started_at: 1,
        };
        lock(&harness.engine.state).receive(
            "t",
            &turn_id,
            ProviderOutput::BackgroundTasks {
                tasks: vec![task.clone()],
                waiting: true,
            },
        );
        assert_eq!(
            harness.engine.snapshot().background_tasks,
            [ThreadBackgroundTasks {
                thread_id: "t".into(),
                tasks: vec![task],
                waiting: true,
            }]
        );

        harness
            .engine
            .dispatch(Command::BackgroundTaskStop {
                thread_id: "t".into(),
                task_id: "b1".into(),
            })
            .unwrap();

        assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Completed);
        assert!(harness.engine.snapshot().background_tasks.is_empty());
    }

    fn completed_turn() -> Vec<ProviderOutput> {
        vec![
            ProviderOutput::SessionStarted {
                session_id: "s1".into(),
            },
            ProviderOutput::AssistantMessage {
                item_id: "a".into(),
                text: "done".into(),
            },
            ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Completed,
            },
        ]
    }

    #[test]
    fn a_turn_runs_in_the_project_root_and_the_next_one_resumes_its_session() {
        let harness = Harness::in_memory("resume", completed_turn());

        harness.send("t", "m1", "first");
        assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Completed);
        harness.send("t", "m2", "second");
        harness.wait_for_turn_end();

        let request = |prompt: &str, resume: Option<&str>| Request {
            cwd: harness.root.clone(),
            prompt: prompt.into(),
            resume: resume.map(Into::into),
            resume_at: None,
            model: Some("haiku".into()),
            attachments: Vec::new(),
            instructions: Some(delegation::instructions(0, true, false)),
            subagent_effort: SubagentEffort::AgentDecides,
        };
        assert_eq!(
            *harness.adapter.requests.lock().unwrap(),
            [request("first", None), request("second", Some("s1"))]
        );
        assert_eq!(harness.thread("t").status, ThreadStatus::Idle);
        assert_eq!(harness.engine.transcript("t", None).unwrap().items.len(), 4);
    }

    #[test]
    fn a_chat_runs_in_its_own_folder_which_goes_away_with_it() {
        let harness = Harness::in_memory("chat", completed_turn());
        harness
            .create_thread_in("chat-1", None, ThreadWorkspace::Local)
            .unwrap();

        harness.send("chat-1", "m1", "hello");
        assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Completed);

        let folder = harness.data_dir.join("chats").join("chat-1");
        let request = harness.adapter.requests.lock().unwrap()[0].clone();
        assert_eq!(PathBuf::from(&request.cwd), folder);
        assert!(folder.is_dir());
        let instructions = request.instructions.unwrap();
        assert!(instructions.contains("delegate_task"));
        assert!(!instructions.contains("watch_pull_request"));
        {
            let mut state = lock(&harness.engine.state);
            assert!(!state.offers_tool("chat-1", "watch_pull_request"));
            let isolated = state.call_tool(
                "chat-1",
                "delegate_task",
                &serde_json::json!({ "task": "look", "isolation": "worktree" }),
            );
            assert!(isolated.is_err());
        }
        assert_eq!(
            harness.create_thread_in("chat-2", None, ThreadWorkspace::NewWorktree),
            Err(CommandError::ChatNotLocal {
                thread_id: "chat-2".into()
            })
        );
        assert_eq!(
            harness.engine.search_paths(None, None, "").unwrap(),
            Vec::new()
        );

        harness
            .engine
            .dispatch(Command::ThreadDelete {
                thread_id: "chat-1".into(),
            })
            .unwrap();
        assert!(!folder.exists());
    }

    #[test]
    fn a_sent_image_reaches_the_provider_and_goes_away_with_its_thread() {
        let harness = Harness::in_memory("image", completed_turn());
        let image = harness
            .engine
            .save_image("shot.png", "image/png", "cG5n")
            .unwrap();

        harness
            .engine
            .dispatch(Command::MessageSend {
                thread_id: "t".into(),
                message_id: "m1".into(),
                text: "look".into(),
                attachments: vec![image.clone()],
            })
            .unwrap();
        harness.wait_for_turn_end();
        let sent = harness.adapter.requests.lock().unwrap()[0].attachments[0].clone();
        let path = sent.path.expect("the image's file");

        assert_eq!(sent.attachment, image);
        assert!(path.exists());
        harness
            .engine
            .dispatch(Command::ThreadDelete {
                thread_id: "t".into(),
            })
            .unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn removing_a_project_forgets_what_the_engine_kept_for_its_threads() {
        let harness = Harness::in_memory("project-removed", completed_turn());
        {
            let mut state = lock(&harness.engine.state);
            let message = || QueuedMessage {
                id: "m1".into(),
                text: "later".into(),
                attachments: Vec::new(),
            };
            state
                .queued
                .entry("t".into())
                .or_default()
                .push_back(message());
            state.after_compaction.insert("t".into(), message());
            state.watches.insert("t".into(), Default::default());
            state.pending_wakes.insert(
                "t".into(),
                vec![Wake {
                    update: hoshi_core::PullRequestUpdate::default(),
                    text: "news".into(),
                }],
            );
        }

        harness
            .engine
            .dispatch(Command::ProjectRemove {
                project_id: "p".into(),
            })
            .unwrap();

        let state = lock(&harness.engine.state);
        assert!(state.model.thread("t").is_none());
        assert!(state.queued.is_empty());
        assert!(state.after_compaction.is_empty());
        assert!(state.watches.is_empty());
        assert!(state.pending_wakes.is_empty());
    }

    fn titled_harness(name: &str) -> Harness {
        let harness = Harness::in_memory(name, completed_turn());
        harness.engine.set_adapter(Some(Arc::new(FakeAdapter {
            requests: Mutex::new(Vec::new()),
            script: completed_turn(),
            title: Some("Empty TSX Snippet".into()),
            responses: Arc::default(),
            steers: None,
            finishers: Mutex::default(),
        })));
        harness
    }

    #[test]
    fn a_first_message_gets_a_generated_title() {
        let harness = titled_harness("generated-title");

        harness.send("t", "m1", "```tsx\n```");

        let title = harness.wait_for(|event| match event {
            Event::ThreadTitleSet { title, .. } if !title.is_empty() => Some(title),
            _ => None,
        });
        assert_eq!(title, "Empty TSX Snippet");
    }

    #[test]
    fn titles_can_be_turned_off_but_still_regenerated_on_request() {
        let harness = titled_harness("titles-off");
        harness.engine.set_thread_titles(ThreadTitleSettings {
            generate: false,
            ..ThreadTitleSettings::default()
        });
        harness.send("t", "m1", "fix the login bug");
        harness.wait_for_turn_end();
        assert_eq!(harness.thread("t").title, "fix the login bug");

        harness.engine.regenerate_title("t").unwrap();

        let title = harness.wait_for(|event| match event {
            Event::ThreadTitleSet { title, .. } => Some(title),
            _ => None,
        });
        assert_eq!(title, "Empty TSX Snippet");
    }

    #[test]
    fn interrupting_stops_the_turn() {
        let harness = Harness::in_memory("interrupt", Vec::new());
        harness.send("t", "m1", "long task");

        harness
            .engine
            .dispatch(Command::TurnInterrupt {
                thread_id: "t".into(),
            })
            .unwrap();

        assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Interrupted);
        assert_eq!(harness.thread("t").status, ThreadStatus::Idle);
    }

    #[test]
    fn a_disabled_provider_fails_the_turn() {
        let harness = Harness::in_memory("disabled", completed_turn());
        harness.engine.set_adapter(None);

        harness.send("t", "m1", "hi");

        assert_eq!(
            harness.wait_for_turn_end(),
            TurnOutcome::Failed {
                reason: TurnFailure::ProviderDisabled
            }
        );
        assert_eq!(harness.thread("t").status, ThreadStatus::Error);
    }

    #[test]
    fn a_chat_whose_folder_cannot_be_made_fails_its_turn() {
        let harness = Harness::in_memory("chat-no-folder", completed_turn());
        std::fs::write(harness.data_dir.join("chats"), "not a folder").unwrap();
        harness
            .create_thread_in("chat-1", None, ThreadWorkspace::Local)
            .unwrap();

        harness.send("chat-1", "m1", "hello");

        assert!(matches!(
            harness.wait_for_turn_end(),
            TurnOutcome::Failed {
                reason: TurnFailure::SpawnFailed { .. }
            }
        ));
        assert_eq!(harness.thread("chat-1").status, ThreadStatus::Error);
        assert!(harness.adapter.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn a_turn_left_running_by_a_crash_is_closed_on_startup() {
        let dir = scratch_dir("crash");
        let database = dir.join("state.sqlite");
        let harness = Harness::new("crash", dir, Store::open(&database).unwrap(), Vec::new());
        harness.send("t", "m1", "long task");
        drop(harness);

        let engine = Engine::from_store(
            Store::open(&database).unwrap(),
            &scratch_dir("crash-reopened"),
            Arc::new(|_| {}),
        )
        .unwrap();

        let thread = &engine.snapshot().threads[0];
        assert_eq!(thread.status, ThreadStatus::Idle);
        assert_eq!(thread.active_turn_id, None);
    }

    fn approval_requested() -> Vec<ProviderOutput> {
        vec![ProviderOutput::RequestOpened {
            request_id: "r1".into(),
            content: ItemContent::Approval {
                tool_call_id: None,
                tool_name: "Bash".into(),
                input: "{}".into(),
                reason: None,
                can_accept_for_session: true,
                status: RequestStatus::Pending,
                decision: None,
                feedback: None,
            },
        }]
    }

    #[test]
    fn an_answer_reaches_the_turn_that_asked() {
        let harness = Harness::in_memory("answer", approval_requested());
        harness.send("t", "m1", "run the tests");
        harness.wait_for(|event| match event {
            Event::ThreadStatusSet { status, .. } => Some(status),
            _ => None,
        });
        assert_eq!(harness.thread("t").status, ThreadStatus::WaitingForInput);

        let response = RequestResponse::Decision {
            decision: ApprovalDecision::AcceptForSession,
            feedback: None,
        };
        harness
            .engine
            .dispatch(Command::RequestRespond {
                thread_id: "t".into(),
                request_id: "r1".into(),
                response: response.clone(),
            })
            .unwrap();

        assert_eq!(
            *harness.adapter.responses.lock().unwrap(),
            vec![("r1".to_owned(), response)]
        );
        assert_eq!(harness.thread("t").status, ThreadStatus::Working);
        assert!(harness.engine.snapshot().pending_requests.is_empty());
    }

    #[test]
    fn a_request_left_open_by_a_crash_is_cancelled_on_startup() {
        let dir = scratch_dir("crash-request");
        let database = dir.join("state.sqlite");
        let harness = Harness::new(
            "crash-request",
            dir,
            Store::open(&database).unwrap(),
            approval_requested(),
        );
        harness.send("t", "m1", "run the tests");
        harness.wait_for(|event| match event {
            Event::ThreadStatusSet { .. } => Some(()),
            _ => None,
        });
        drop(harness);

        let engine = Engine::from_store(
            Store::open(&database).unwrap(),
            &scratch_dir("crash-request-reopened"),
            Arc::new(|_| {}),
        )
        .unwrap();

        assert!(engine.snapshot().pending_requests.is_empty());
        let items = engine.transcript("t", None).unwrap().items;
        let request = items.iter().find(|item| item.id == "r1").unwrap();
        assert_eq!(
            request.content.request_status(),
            Some(RequestStatus::Cancelled)
        );
        assert_eq!(engine.snapshot().threads[0].status, ThreadStatus::Idle);
    }

    #[test]
    fn a_question_left_open_by_a_crash_is_answered_in_a_message() {
        let dir = scratch_dir("crash-question");
        let database = dir.join("state.sqlite");
        let asked = vec![ProviderOutput::RequestOpened {
            request_id: "q1".into(),
            content: ItemContent::Questions {
                questions: vec![hoshi_core::Question {
                    header: "Color".into(),
                    question: "Which color?".into(),
                    options: Vec::new(),
                    multi_select: false,
                }],
                status: RequestStatus::Pending,
                answers: None,
            },
        }];
        let harness = Harness::new(
            "crash-question",
            dir,
            Store::open(&database).unwrap(),
            asked,
        );
        harness.send("t", "m1", "paint it");
        harness.wait_for(|event| match event {
            Event::ThreadStatusSet { .. } => Some(()),
            _ => None,
        });
        drop(harness);

        let (sender, events) = mpsc::channel();
        let notify: Notify = Arc::new(move |notification| {
            if let Notification::Event(envelope) = notification {
                let _ = sender.send(envelope.event);
            }
        });
        let data_dir = scratch_dir("crash-question-reopened");
        let engine =
            Engine::from_store(Store::open(&database).unwrap(), &data_dir, notify).unwrap();
        let again = adapter(None);
        engine.set_adapter(Some(again.clone()));
        assert_eq!(engine.snapshot().pending_requests.len(), 1);
        assert_eq!(engine.snapshot().threads[0].status, ThreadStatus::Idle);

        engine
            .dispatch(Command::RequestRespond {
                thread_id: "t".into(),
                request_id: "q1".into(),
                response: RequestResponse::Answers {
                    answers: vec!["Blue".into()],
                },
            })
            .unwrap();
        events
            .iter()
            .find(|event| matches!(event, Event::TurnStarted { .. }))
            .unwrap();

        assert!(engine.snapshot().pending_requests.is_empty());
        assert_eq!(
            again.requests.lock().unwrap()[0].prompt,
            "Which color?
Blue"
        );
    }

    #[test]
    fn a_whole_transcript_leaves_diffs_to_their_item() {
        let change = ToolDetail::FileChange {
            path: "/repo/a.txt".into(),
            additions: 1,
            deletions: 0,
            diff: Some("@@ -0,0 +1,1 @@\n+a".into()),
        };
        let script = vec![
            ProviderOutput::ToolCallStarted {
                item_id: "edit".into(),
                name: "Write".into(),
                input: "{}".into(),
            },
            ProviderOutput::ToolCallCompleted {
                item_id: "edit".into(),
                output: "written".into(),
                is_error: false,
                detail: Some(change.clone()),
            },
            ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Completed,
            },
        ];
        let harness = Harness::in_memory("diffs", script);
        harness.send("t", "m1", "write a");
        harness.wait_for_turn_end();

        let detail_of = |item: &ThreadItem| match &item.content {
            ItemContent::ToolCall { detail, .. } => detail.clone(),
            _ => None,
        };
        let items = harness.engine.transcript("t", None).unwrap().items;
        let listed = items.iter().find(|item| item.id == "edit").unwrap();
        assert_eq!(detail_of(listed), Some(change.summary()));
        let whole = harness.engine.thread_item("t", "edit").unwrap().unwrap();
        assert_eq!(detail_of(&whole), Some(change));
    }

    fn adapter(steers: Option<Arc<Mutex<Vec<String>>>>) -> Arc<FakeAdapter> {
        Arc::new(FakeAdapter {
            requests: Mutex::new(Vec::new()),
            script: Vec::new(),
            title: None,
            responses: Arc::default(),
            steers,
            finishers: Mutex::default(),
        })
    }

    fn finish(adapter: &FakeAdapter, turn: usize, outcome: TurnOutcome) {
        let finisher = adapter.finishers.lock().unwrap()[turn].clone();
        finisher.send(outcome).unwrap();
    }

    #[test]
    fn a_message_to_a_working_thread_joins_its_turn() {
        let harness = Harness::in_memory("steer", Vec::new());
        let steers = Arc::<Mutex<Vec<String>>>::default();
        let adapter = adapter(Some(steers.clone()));
        harness.engine.set_adapter(Some(adapter.clone()));
        harness.send("t", "m1", "long task");
        harness.send("t", "m2", "also this");

        assert_eq!(*steers.lock().unwrap(), ["m2"]);
        assert_eq!(adapter.requests.lock().unwrap().len(), 1);
        finish(&adapter, 0, TurnOutcome::Completed);
        harness.wait_for_turn_end();
        assert_eq!(adapter.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_message_too_late_for_its_turn_starts_the_next_one() {
        let harness = Harness::in_memory("late-message", Vec::new());
        let adapter = adapter(None);
        harness.engine.set_adapter(Some(adapter.clone()));
        harness.send("t", "m1", "long task");
        harness.send("t", "m2", "also this");
        assert_eq!(adapter.requests.lock().unwrap().len(), 1);

        finish(&adapter, 0, TurnOutcome::Completed);
        harness.wait_for(|event| match event {
            Event::TurnStarted { turn_id, .. } if turn_id == "m2" => Some(()),
            _ => None,
        });

        assert_eq!(harness.thread("t").active_turn_id.as_deref(), Some("m2"));
        let requests = adapter.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].prompt, "also this");
    }

    #[test]
    fn a_message_sent_while_a_turn_stops_starts_the_next_one() {
        let harness = Harness::in_memory("stopped-queue", Vec::new());
        let adapter = adapter(None);
        harness.engine.set_adapter(Some(adapter.clone()));
        harness.send("t", "m1", "long task");
        harness.send("t", "m2", "also this");

        harness
            .engine
            .dispatch(Command::TurnInterrupt {
                thread_id: "t".into(),
            })
            .unwrap();
        harness.wait_for(|event| match event {
            Event::TurnStarted { turn_id, .. } if turn_id == "m2" => Some(()),
            _ => None,
        });

        assert_eq!(harness.thread("t").active_turn_id.as_deref(), Some("m2"));
        assert_eq!(adapter.requests.lock().unwrap().len(), 2);
    }

    fn delegate(harness: &Harness, arguments: serde_json::Value) -> serde_json::Value {
        lock(&harness.engine.state)
            .call_tool("t", "delegate_task", &arguments)
            .unwrap()
    }

    fn child_turn_ends(harness: &Harness, adapter: &FakeAdapter, turn: usize) {
        finish(adapter, turn, TurnOutcome::Completed);
        harness.wait_for_turn_end();
        // Waits for the engine to finish reacting.
        harness.thread("t");
    }

    #[test]
    fn a_delegated_task_runs_in_a_child_and_answers_into_the_running_turn() {
        let harness = Harness::in_memory("delegate", Vec::new());
        let steers = Arc::<Mutex<Vec<String>>>::default();
        let adapter = adapter(Some(steers.clone()));
        harness.engine.set_adapter(Some(adapter.clone()));
        harness.send("t", "m1", "split the work");

        let task = delegate(
            &harness,
            serde_json::json!({ "task": "count the files", "title": "Count" }),
        );

        let child = task["taskId"].as_str().unwrap().to_owned();
        let thread = harness.thread(&child);
        assert_eq!(thread.parent_thread_id.as_deref(), Some("t"));
        assert_eq!(thread.title, "Count");
        assert_eq!(
            adapter.requests.lock().unwrap()[1].prompt,
            "count the files"
        );
        assert!(task["next"].is_string());

        child_turn_ends(&harness, &adapter, 1);

        assert_eq!(steers.lock().unwrap().len(), 1);
        let items = harness.engine.transcript("t", None).unwrap().items;
        assert!(items.iter().any(|item| matches!(
            &item.content,
            ItemContent::UserMessage { task_result: Some(from), text, .. }
                if *from == child && text.contains("[completed]")
        )));
        assert!(harness.thread(&child).task_answered);
        assert!(lock(&harness.engine.state).pending_delegations().is_empty());
    }

    #[test]
    fn a_delegated_task_wakes_its_parent_once_the_parent_rested() {
        let harness = Harness::in_memory("delegate-wake", Vec::new());
        let adapter = adapter(None);
        harness.engine.set_adapter(Some(adapter.clone()));
        harness.send("t", "m1", "split the work");
        delegate(&harness, serde_json::json!({ "task": "count the files" }));
        finish(&adapter, 0, TurnOutcome::Completed);
        harness.wait_for_turn_end();

        child_turn_ends(&harness, &adapter, 1);

        let requests = adapter.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[2].prompt.starts_with("Delegated task"));
        assert!(harness.thread("t").active_turn_id.is_some());
    }

    #[test]
    fn a_cancelled_task_never_answers_and_only_its_parent_reaches_it() {
        let harness = Harness::in_memory("delegate-cancel", Vec::new());
        let adapter = adapter(None);
        harness.engine.set_adapter(Some(adapter.clone()));
        harness.send("t", "m1", "split the work");
        let task = delegate(&harness, serde_json::json!({ "task": "count the files" }));
        let task_id = serde_json::json!({ "taskId": task["taskId"] });

        let stranger = lock(&harness.engine.state).call_tool("p", "task_status", &task_id);
        assert!(stranger.is_err());
        let running = lock(&harness.engine.state)
            .call_tool("t", "task_status", &task_id)
            .unwrap();
        assert_eq!(running["status"], "running");

        let cancelled = lock(&harness.engine.state)
            .call_tool("t", "task_cancel", &task_id)
            .unwrap();
        assert_eq!(cancelled["status"], "cancel_requested");
        harness.wait_for_turn_end();
        finish(&adapter, 0, TurnOutcome::Completed);
        harness.wait_for_turn_end();

        assert_eq!(adapter.requests.lock().unwrap().len(), 2);
        assert_eq!(harness.thread("t").active_turn_id, None);
        let child = task["taskId"].as_str().unwrap();
        assert!(harness.thread(child).task_cancelled);
        assert!(lock(&harness.engine.state).pending_delegations().is_empty());
    }

    #[test]
    fn the_user_cancels_a_task_waiting_on_its_plan_and_it_never_answers() {
        let harness = Harness::in_memory("delegate-plan-cancel", Vec::new());
        let adapter = adapter(None);
        harness.engine.set_adapter(Some(adapter.clone()));
        let child = planning_task(&harness, &adapter);

        harness.engine.cancel_task(&child);
        harness.send(&child, "m5", "one more thing");
        child_turn_ends(&harness, &adapter, 2);

        assert!(harness.thread(&child).task_cancelled);
        assert!(harness.engine.snapshot().pending_tasks.is_empty());
        assert_eq!(adapter.requests.lock().unwrap().len(), 3);
    }

    #[test]
    fn a_task_delegates_once_more_with_the_model_effort_and_mode_it_was_given() {
        let harness = Harness::in_memory("delegate-nested", Vec::new());
        let adapter = adapter(None);
        harness.engine.set_adapter(Some(adapter.clone()));
        harness.send("t", "m1", "split the work");

        let task = delegate(
            &harness,
            serde_json::json!({ "task": "plan it", "model": "opus", "effort": "high", "plan": true }),
        );
        let child = task["taskId"].as_str().unwrap().to_owned();
        let thread = harness.thread(&child);
        assert_eq!(thread.model.as_deref(), Some("opus"));
        assert_eq!(thread.effort, Some(Effort::High));
        assert!(thread.plan_mode);

        let deeper = lock(&harness.engine.state)
            .call_tool(
                &child,
                "delegate_task",
                &serde_json::json!({ "task": "count" }),
            )
            .unwrap();
        let grandchild = deeper["taskId"].as_str().unwrap().to_owned();
        {
            let state = lock(&harness.engine.state);
            assert!(state.offers_tool("t", "watch_pull_request"));
            assert!(!state.offers_tool(&child, "watch_pull_request"));
            assert!(!state.offers_tool(&child, "unwatch_pull_request"));
        }
        assert_eq!(
            harness.thread(&grandchild).parent_thread_id.as_deref(),
            Some(child.as_str())
        );
        let too_deep = lock(&harness.engine.state).call_tool(
            &grandchild,
            "delegate_task",
            &serde_json::json!({ "task": "more" }),
        );
        assert!(too_deep.is_err());
        let wrong = lock(&harness.engine.state).call_tool(
            "t",
            "delegate_task",
            &serde_json::json!({ "task": "x", "effort": "huge" }),
        );
        assert!(wrong.is_err());

        let requests = adapter.requests.lock().unwrap();
        let told = |turn: usize| requests[turn].instructions.clone();
        assert!(told(0).is_some_and(|told| !told.contains(delegation::TASK_INSTRUCTIONS)));
        assert!(told(1).is_some_and(|told| told.contains(delegation::TASK_INSTRUCTIONS)));
        assert_eq!(told(2), None);
    }

    #[test]
    fn turns_carry_how_the_agent_chooses_its_subagents_effort() {
        let harness = Harness::in_memory("subagent-effort", Vec::new());
        let adapter = adapter(None);
        harness.engine.set_adapter(Some(adapter.clone()));
        harness.send("t", "m1", "look it up");
        finish(&adapter, 0, TurnOutcome::Completed);
        harness.wait_for_turn_end();
        harness.engine.set_subagent_effort(SubagentEffort::Thread);
        harness.send("t", "m2", "again");

        let requests = adapter.requests.lock().unwrap();
        assert_eq!(requests[0].subagent_effort, SubagentEffort::AgentDecides);
        assert_eq!(requests[1].subagent_effort, SubagentEffort::Thread);
    }

    /// A task whose plan the user sent back to planning, its turn over, and its parent at rest.
    fn planning_task(harness: &Harness, adapter: &FakeAdapter) -> String {
        harness.send("t", "m1", "split the work");
        let task = delegate(
            harness,
            serde_json::json!({ "task": "plan the README", "plan": true }),
        );
        let child = task["taskId"].as_str().unwrap().to_owned();
        finish(adapter, 0, TurnOutcome::Completed);
        harness.wait_for_turn_end();
        let declined = Event::ItemAdded {
            item: ThreadItem {
                id: "plan1".into(),
                thread_id: child.clone(),
                created_at: "2026-11-08T10:00:00Z".into(),
                content: ItemContent::ProposedPlan {
                    plan: "1. Sort the sections".into(),
                    status: RequestStatus::Resolved,
                    decision: Some(ApprovalDecision::Decline),
                    feedback: None,
                },
            },
        };
        lock(&harness.engine.state)
            .commit(&[declined], "2026-11-08T10:00:00Z")
            .unwrap();
        child_turn_ends(harness, adapter, 1);
        child
    }

    #[test]
    fn a_task_still_planning_tells_its_status_and_answers_as_is_when_the_user_says() {
        let harness = Harness::in_memory("delegate-plan-as-is", Vec::new());
        let adapter = adapter(None);
        harness.engine.set_adapter(Some(adapter.clone()));
        let child = planning_task(&harness, &adapter);

        let status = lock(&harness.engine.state)
            .call_tool("t", "task_status", &serde_json::json!({ "taskId": child }))
            .unwrap();
        assert_eq!(status["status"], "waiting_for_user");
        assert!(harness.engine.snapshot().pending_tasks.contains(&child));

        harness.engine.send_task_answer(&child);
        harness.thread("t");

        let requests = adapter.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[2].prompt.contains("1. Sort the sections"));
        assert!(harness.engine.snapshot().pending_tasks.is_empty());
    }

    #[test]
    fn a_task_whose_plan_the_user_sent_back_answers_only_once_it_ends_otherwise() {
        let harness = Harness::in_memory("delegate-plan", Vec::new());
        let adapter = adapter(None);
        harness.engine.set_adapter(Some(adapter.clone()));
        harness.send("t", "m1", "split the work");
        let task = delegate(
            &harness,
            serde_json::json!({ "task": "plan the README", "plan": true }),
        );
        let child = task["taskId"].as_str().unwrap().to_owned();
        finish(&adapter, 0, TurnOutcome::Completed);
        harness.wait_for_turn_end();
        let declined = Event::ItemAdded {
            item: ThreadItem {
                id: "plan1".into(),
                thread_id: child.clone(),
                created_at: "2026-11-08T10:00:00Z".into(),
                content: ItemContent::ProposedPlan {
                    plan: "1. Sort the sections".into(),
                    status: RequestStatus::Resolved,
                    decision: Some(ApprovalDecision::Decline),
                    feedback: None,
                },
            },
        };
        lock(&harness.engine.state)
            .commit(&[declined], "2026-11-08T10:00:00Z")
            .unwrap();

        child_turn_ends(&harness, &adapter, 1);
        assert_eq!(adapter.requests.lock().unwrap().len(), 2);

        harness.send(&child, "m5", "keep it as it is");
        child_turn_ends(&harness, &adapter, 2);

        let requests = adapter.requests.lock().unwrap();
        assert_eq!(requests.len(), 4);
        assert!(requests[3].prompt.starts_with("Delegated task"));
        assert!(requests[3].prompt.contains("1. Sort the sections"));
    }

    #[test]
    fn a_task_a_restart_cut_short_answers_once_the_user_has_it_finish() {
        let store_dir = scratch_dir("delegate-restart-store");
        let database = store_dir.join("state.sqlite");
        let first = adapter(None);
        let child = {
            let harness = Harness::new(
                "delegate-restart",
                scratch_dir("delegate-restart"),
                Store::open(&database).unwrap(),
                Vec::new(),
            );
            harness.engine.set_adapter(Some(first.clone()));
            harness.send("t", "m1", "split the work");
            let task = delegate(&harness, serde_json::json!({ "task": "count the files" }));
            finish(&first, 0, TurnOutcome::Completed);
            harness.wait_for_turn_end();
            task["taskId"].as_str().unwrap().to_owned()
        };

        let (sender, events) = mpsc::channel();
        let notify: Notify = Arc::new(move |notification| {
            if let Notification::Event(envelope) = notification {
                let _ = sender.send(envelope.event);
            }
        });
        let data_dir = scratch_dir("delegate-restart-again");
        let engine =
            Engine::from_store(Store::open(&database).unwrap(), &data_dir, notify).unwrap();
        let again = adapter(None);
        engine.set_adapter(Some(again.clone()));
        let harness = Harness {
            engine,
            adapter: again.clone(),
            events,
            root: String::new(),
            data_dir,
        };
        assert_eq!(harness.thread(&child).active_turn_id, None);

        harness.send(&child, "m9", "go on");
        finish(&again, 0, TurnOutcome::Completed);
        harness.wait_for(|event| match event {
            Event::TurnCompleted { turn_id, .. } if turn_id == "m9" => Some(()),
            _ => None,
        });
        harness.thread("t");

        let requests = again.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].prompt.starts_with("Delegated task"));
        drop(first);
    }

    #[test]
    fn rejects_a_folder_that_does_not_exist() {
        let harness = Harness::in_memory("missing", Vec::new());
        let missing = std::env::temp_dir().join("hoshi-definitely-missing-folder");

        let error = harness
            .engine
            .dispatch(Command::ProjectCreate {
                project_id: "q".into(),
                title: "q".into(),
                workspace_root: missing.to_string_lossy().into_owned(),
            })
            .unwrap_err();

        assert!(matches!(error, CommandError::WorkspaceRootNotFound { .. }));
    }

    #[test]
    fn a_new_worktree_runs_its_project_s_setup_before_the_held_message() {
        let repo = git::tests::temp_repository("engine-setup");
        let harness = Harness::new(
            "setup",
            repo,
            Store::open_in_memory().unwrap(),
            completed_turn(),
        );
        let marker = if cfg!(windows) {
            "Set-Content -Path ready.txt -Value ok"
        } else {
            "echo ok > ready.txt"
        };
        harness
            .engine
            .dispatch(Command::ProjectWorktreeSetupSet {
                project_id: "p".into(),
                setup: Some(hoshi_core::WorktreeSetup {
                    files: Vec::new(),
                    command: format!("  {marker}  "),
                    wait: true,
                }),
            })
            .unwrap();
        harness
            .create_thread("s1234567-rest", ThreadWorkspace::NewWorktree)
            .unwrap();
        harness.send("s1234567-rest", "m1", "go");

        harness.wait_for_turn_end();
        let worktree = harness.thread("s1234567-rest").worktree_path.unwrap();
        assert!(Path::new(&worktree).join("ready.txt").is_file());
        let runs = harness.engine.worktree_setups();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, hoshi_core::WorktreeSetupStatus::Succeeded);
        assert_eq!(runs[0].held_messages, 0);
        assert_eq!(harness.adapter.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_failed_setup_keeps_the_message_until_the_user_sends_it_anyway() {
        let repo = git::tests::temp_repository("engine-setup-failed");
        let harness = Harness::new(
            "setup-failed",
            repo,
            Store::open_in_memory().unwrap(),
            completed_turn(),
        );
        harness
            .engine
            .dispatch(Command::ProjectWorktreeSetupSet {
                project_id: "p".into(),
                setup: Some(hoshi_core::WorktreeSetup {
                    files: Vec::new(),
                    command: "exit 3".into(),
                    wait: true,
                }),
            })
            .unwrap();
        harness
            .create_thread("f1234567-rest", ThreadWorkspace::NewWorktree)
            .unwrap();
        harness.send("f1234567-rest", "m1", "go");
        let deadline = Instant::now() + Duration::from_secs(30);
        let failed = loop {
            let runs = harness.engine.worktree_setups();
            if runs[0].status != hoshi_core::WorktreeSetupStatus::Running {
                break runs[0].clone();
            }
            assert!(Instant::now() < deadline, "the setup never ended");
            thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(failed.status, hoshi_core::WorktreeSetupStatus::Failed);
        assert_eq!(failed.exit_code, Some(3));
        assert_eq!(failed.held_messages, 1);
        assert!(harness.adapter.requests.lock().unwrap().is_empty());

        harness.engine.skip_worktree_setup("f1234567-rest").unwrap();
        harness.wait_for_turn_end();
        assert_eq!(harness.adapter.requests.lock().unwrap().len(), 1);
    }

    fn wait_for_setup_end(harness: &Harness) -> hoshi_core::WorktreeSetupRun {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let runs = harness.engine.worktree_setups();
            if let Some(run) = runs
                .into_iter()
                .find(|run| run.status != hoshi_core::WorktreeSetupStatus::Running)
            {
                return run;
            }
            assert!(Instant::now() < deadline, "the setup never ended");
            thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn a_new_worktree_gets_the_project_s_files_before_its_command_runs() {
        let repo = git::tests::temp_repository("engine-setup-files");
        std::fs::write(repo.join(".env"), "KEY=1").unwrap();
        std::fs::create_dir_all(repo.join("secrets")).unwrap();
        std::fs::write(repo.join("secrets/a.env"), "a").unwrap();
        std::fs::write(repo.join("secrets/b.env"), "b").unwrap();
        std::fs::write(repo.join("secrets/notes.txt"), "n").unwrap();
        let harness = Harness::new(
            "setup-files",
            repo,
            Store::open_in_memory().unwrap(),
            completed_turn(),
        );
        let check = if cfg!(windows) {
            "if (Test-Path secrets/b.env) { Copy-Item .env ready.txt } else { exit 4 }"
        } else {
            "test -f secrets/b.env && cp .env ready.txt"
        };
        harness
            .engine
            .dispatch(Command::ProjectWorktreeSetupSet {
                project_id: "p".into(),
                setup: Some(hoshi_core::WorktreeSetup {
                    files: vec![
                        ".env".into(),
                        "secrets/*.env".into(),
                        "local.properties".into(),
                    ],
                    command: check.into(),
                    wait: true,
                }),
            })
            .unwrap();
        harness
            .create_thread("c1234567-rest", ThreadWorkspace::NewWorktree)
            .unwrap();
        harness.send("c1234567-rest", "m1", "go");

        harness.wait_for_turn_end();
        let worktree = PathBuf::from(harness.thread("c1234567-rest").worktree_path.unwrap());
        assert_eq!(
            std::fs::read_to_string(worktree.join("ready.txt"))
                .unwrap()
                .trim(),
            "KEY=1"
        );
        assert!(worktree.join("secrets/a.env").is_file());
        assert!(!worktree.join("secrets/notes.txt").exists());
        let run = wait_for_setup_end(&harness);
        assert_eq!(run.status, hoshi_core::WorktreeSetupStatus::Succeeded);
        assert_eq!(run.copied_files, 3);
        assert_eq!(run.missing_files, ["local.properties"]);
        assert!(run.failed_files.is_empty());
        assert!(run.terminal_id.is_some());
    }

    #[test]
    fn a_setup_of_files_only_releases_the_held_message_once_copied() {
        let repo = git::tests::temp_repository("engine-setup-files-only");
        std::fs::write(repo.join(".env"), "KEY=1").unwrap();
        let harness = Harness::new(
            "setup-files-only",
            repo,
            Store::open_in_memory().unwrap(),
            completed_turn(),
        );
        harness
            .engine
            .dispatch(Command::ProjectWorktreeSetupSet {
                project_id: "p".into(),
                setup: Some(hoshi_core::WorktreeSetup {
                    files: vec!["missing/*.env".into(), ".env".into()],
                    command: " ".into(),
                    wait: true,
                }),
            })
            .unwrap();
        harness
            .create_thread("o1234567-rest", ThreadWorkspace::NewWorktree)
            .unwrap();
        harness.send("o1234567-rest", "m1", "go");

        harness.wait_for_turn_end();
        let worktree = PathBuf::from(harness.thread("o1234567-rest").worktree_path.unwrap());
        assert!(worktree.join(".env").is_file());
        let run = wait_for_setup_end(&harness);
        assert_eq!(run.status, hoshi_core::WorktreeSetupStatus::Succeeded);
        assert_eq!(run.terminal_id, None);
        assert_eq!(run.exit_code, None);
        assert_eq!(run.copied_files, 1);
        assert_eq!(run.missing_files, ["missing/*.env"]);
        assert_eq!(run.held_messages, 0);
        assert_eq!(harness.adapter.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_setup_refuses_files_outside_the_project() {
        let repo = git::tests::temp_repository("engine-setup-escape");
        let harness = Harness::new(
            "setup-escape",
            repo,
            Store::open_in_memory().unwrap(),
            completed_turn(),
        );

        let error = harness
            .engine
            .dispatch(Command::ProjectWorktreeSetupSet {
                project_id: "p".into(),
                setup: Some(hoshi_core::WorktreeSetup {
                    files: vec![".env".into(), "../outside/.env".into()],
                    command: String::new(),
                    wait: false,
                }),
            })
            .unwrap_err();

        assert_eq!(
            error,
            CommandError::SetupFileInvalid {
                path: "../outside/.env".into()
            }
        );
        assert_eq!(harness.engine.snapshot().projects[0].worktree_setup, None);
    }

    #[test]
    fn a_new_worktree_s_branch_keeps_its_base_and_takes_the_title_s_name() {
        let repo = git::tests::temp_repository("engine-branch-name");
        let harness = Harness::new(
            "branch-name",
            repo,
            Store::open_in_memory().unwrap(),
            completed_turn(),
        );
        harness.engine.set_adapter(Some(Arc::new(FakeAdapter {
            requests: Mutex::new(Vec::new()),
            script: completed_turn(),
            title: Some("Fix the login bug".into()),
            responses: Arc::default(),
            steers: None,
            finishers: Mutex::default(),
        })));
        harness
            .create_thread("n1234567-rest", ThreadWorkspace::NewWorktree)
            .unwrap();
        harness
            .create_thread("p1234567-rest", ThreadWorkspace::NewWorktree)
            .unwrap();
        let pushed = harness.thread("p1234567-rest");
        git::set_branch_config(&harness.root, "hoshi/p1234567", "remote", "origin").unwrap();
        assert_eq!(
            git::branch_config(&harness.root, "hoshi/n1234567", git::BASE_CONFIG).as_deref(),
            Some("main")
        );

        harness.send("n1234567-rest", "m1", "the login fails");
        let renamed = harness.wait_for(|event| match event {
            Event::ThreadBranchSet { thread_id, branch } if thread_id == "n1234567-rest" => branch,
            _ => None,
        });
        harness.send("p1234567-rest", "m2", "the login fails");
        harness.wait_for(|event| match event {
            Event::ThreadTitleSet { thread_id, title }
                if thread_id == "p1234567-rest" && title == "Fix the login bug" =>
            {
                Some(())
            }
            _ => None,
        });

        assert_eq!(renamed, "hoshi/fix-the-login-bug");
        let worktree = harness.thread("n1234567-rest").worktree_path.unwrap();
        assert_eq!(
            git::branch_config(&worktree, "hoshi/fix-the-login-bug", git::BASE_CONFIG).as_deref(),
            Some("main")
        );
        let pushed_path = pushed.worktree_path.unwrap();
        thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(
            git::current_branch(&pushed_path).as_deref(),
            Some("hoshi/p1234567")
        );
        for path in [worktree, pushed_path] {
            let _ = git::remove_worktree(&harness.root, &path);
        }
    }

    #[test]
    fn a_message_sent_after_compacting_waits_for_the_compaction_turn() {
        let mut script = completed_turn();
        script.insert(
            1,
            ProviderOutput::ContextUsage(ContextUsage {
                used_tokens: 250_000,
                max_tokens: Some(1_000_000),
                cache_ttl_secs: Some(3600),
                cache_expires_at: None,
            }),
        );
        let harness = Harness::in_memory("compact", script);

        harness
            .engine
            .compact_then_send("t", "m1", "carry on".into(), Vec::new())
            .unwrap();
        harness.wait_for_turn_end();
        harness.wait_for_turn_end();

        let prompts: Vec<_> = harness
            .adapter
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.prompt.clone())
            .collect();
        assert_eq!(prompts, ["/compact", "carry on"]);
        let usage = lock(&harness.engine.state).context_usages["t"];
        let now = Utc::now().timestamp_millis() as u64;
        let expires = usage.cache_expires_at.unwrap();
        assert!(expires > now + 3_500_000 && expires <= now + 3_600_000);
    }

    #[test]
    fn a_message_after_a_compaction_that_failed_at_once_is_still_sent() {
        let harness = Harness::in_memory("compact-failed", completed_turn());
        harness.engine.set_adapter(None);

        harness
            .engine
            .compact_then_send("t", "m1", "carry on".into(), Vec::new())
            .unwrap();

        let ids: Vec<_> = harness
            .engine
            .transcript("t", None)
            .unwrap()
            .items
            .into_iter()
            .map(|item| item.id)
            .collect();
        assert!(ids.contains(&"m1".to_owned()));
        assert!(lock(&harness.engine.state).after_compaction.is_empty());
    }

    #[test]
    fn a_watched_pull_request_wakes_its_thread_on_a_failing_check() {
        let harness = Harness::in_memory("pr-watch", completed_turn());
        let pull_request = hoshi_core::PullRequest {
            forge: ForgeKind::Github,
            number: 7,
            title: "Work".into(),
            url: "https://github.com/o/r/pull/7".into(),
            author: Some("me".into()),
            head_branch: "hoshi/work".into(),
            head_sha: Some("abc1234".into()),
            base_branch: "main".into(),
            state: hoshi_core::PullRequestState::Open,
            checks: None,
            unresolved_comments: 0,
            additions: 0,
            deletions: 0,
            updated_at: "2026-11-07T12:00:00Z".into(),
        };
        let detail = |state: hoshi_core::ChecksState| hoshi_core::PullRequestDetail {
            supports: hoshi_core::PullRequestSupport::all(),
            forge_id: "id".into(),
            repository: "o/r".into(),
            mergeability: hoshi_core::Mergeability::Mergeable,
            merge_methods: Vec::new(),
            can_edit: true,
            body: String::new(),
            changed_files: 1,
            reviewers: Vec::new(),
            labels: Vec::new(),
            checks: vec![hoshi_core::PullRequestCheck {
                name: "lint".into(),
                state,
                url: None,
            }],
            review_threads: Vec::new(),
            timeline: Vec::new(),
        };
        let call =
            |tool: &str| lock(&harness.engine.state).call_tool("t", tool, &serde_json::json!({}));
        assert!(call("watch_pull_request").is_err());
        lock(&harness.engine.state).pull_requests.record(
            "t",
            ForgeKind::Github,
            Ok(Some(pull_request.clone())),
            false,
            Instant::now(),
            chrono::Utc::now(),
        );

        let watched = call("watch_pull_request").unwrap();
        assert_eq!(watched["number"], 7);
        assert!(harness.thread("t").watching_pull_request);
        let observe = |state: hoshi_core::ChecksState| {
            lock(&harness.engine.state).observe_watch(
                "t",
                ForgeKind::Github,
                &pull_request,
                &detail(state),
                Some("me".into()),
            );
        };
        observe(hoshi_core::ChecksState::Pending);
        observe(hoshi_core::ChecksState::Failing);

        let update = harness.wait_for(|event| match event {
            Event::ItemAdded { item } => match item.content {
                ItemContent::UserMessage {
                    pull_request_update: Some(update),
                    text,
                    ..
                } => Some((update, text)),
                _ => None,
            },
            _ => None,
        });
        assert_eq!(update.0.failed_checks, ["lint"]);
        assert!(update.1.contains("Checks failed on abc1234"));
        harness.wait_for_turn_end();
        let unwatched = call("unwatch_pull_request").unwrap();
        assert_eq!(unwatched["wasWatching"], true);
        assert!(!harness.thread("t").watching_pull_request);
    }

    #[test]
    fn a_comment_left_while_the_app_was_closed_wakes_the_watching_thread() {
        let dir = scratch_dir("pr-watch-restart");
        let database = dir.join("state.sqlite");
        let harness = Harness::new(
            "pr-watch-restart",
            dir,
            Store::open(&database).unwrap(),
            completed_turn(),
        );
        let pull_request = hoshi_core::PullRequest {
            forge: ForgeKind::Github,
            number: 7,
            title: "Work".into(),
            url: "https://github.com/o/r/pull/7".into(),
            author: Some("me".into()),
            head_branch: "hoshi/work".into(),
            head_sha: Some("abc1234".into()),
            base_branch: "main".into(),
            state: hoshi_core::PullRequestState::Open,
            checks: None,
            unresolved_comments: 0,
            additions: 0,
            deletions: 0,
            updated_at: "2026-11-07T12:00:00Z".into(),
        };
        lock(&harness.engine.state).pull_requests.record(
            "t",
            ForgeKind::Github,
            Ok(Some(pull_request.clone())),
            false,
            Instant::now(),
            chrono::Utc::now(),
        );
        lock(&harness.engine.state)
            .call_tool("t", "watch_pull_request", &serde_json::json!({}))
            .unwrap();
        drop(harness);

        let (sender, events) = mpsc::channel();
        let notify: Notify = Arc::new(move |notification| {
            if let Notification::Event(envelope) = notification {
                let _ = sender.send(envelope.event);
            }
        });
        let engine = Engine::from_store(
            Store::open(&database).unwrap(),
            &scratch_dir("pr-watch-restarted"),
            notify,
        )
        .unwrap();
        let commented_at =
            (Utc::now() + chrono::Duration::minutes(1)).to_rfc3339_opts(SecondsFormat::Secs, true);
        let detail = hoshi_core::PullRequestDetail {
            supports: hoshi_core::PullRequestSupport::all(),
            forge_id: "id".into(),
            repository: "o/r".into(),
            mergeability: hoshi_core::Mergeability::Mergeable,
            merge_methods: Vec::new(),
            can_edit: true,
            body: String::new(),
            changed_files: 1,
            reviewers: Vec::new(),
            labels: Vec::new(),
            checks: Vec::new(),
            review_threads: Vec::new(),
            timeline: vec![hoshi_core::TimelineItem::Comment {
                author: Some("reviewer".into()),
                body: "Please rename this.".into(),
                created_at: commented_at,
                url: "https://github.com/o/r/pull/7#c1".into(),
            }],
        };

        lock(&engine.state).observe_watch(
            "t",
            ForgeKind::Github,
            &pull_request,
            &detail,
            Some("me".into()),
        );

        let update = events
            .try_iter()
            .find_map(|event| match event {
                Event::ItemAdded { item } => match item.content {
                    ItemContent::UserMessage {
                        pull_request_update: Some(update),
                        ..
                    } => Some(update),
                    _ => None,
                },
                _ => None,
            })
            .expect("a wake for the comment");
        assert_eq!(update.comments, 1);
    }

    #[test]
    fn a_check_that_failed_while_the_app_was_closed_wakes_the_watching_thread() {
        let dir = scratch_dir("pr-watch-checks-restart");
        let database = dir.join("state.sqlite");
        let harness = Harness::new(
            "pr-watch-checks-restart",
            dir,
            Store::open(&database).unwrap(),
            completed_turn(),
        );
        let pull_request = hoshi_core::PullRequest {
            forge: ForgeKind::Github,
            number: 7,
            title: "Work".into(),
            url: "https://github.com/o/r/pull/7".into(),
            author: Some("me".into()),
            head_branch: "hoshi/work".into(),
            head_sha: Some("abc1234".into()),
            base_branch: "main".into(),
            state: hoshi_core::PullRequestState::Open,
            checks: None,
            unresolved_comments: 0,
            additions: 0,
            deletions: 0,
            updated_at: "2026-11-07T12:00:00Z".into(),
        };
        let detail = |state: hoshi_core::ChecksState| hoshi_core::PullRequestDetail {
            supports: hoshi_core::PullRequestSupport::all(),
            forge_id: "id".into(),
            repository: "o/r".into(),
            mergeability: hoshi_core::Mergeability::Mergeable,
            merge_methods: Vec::new(),
            can_edit: true,
            body: String::new(),
            changed_files: 1,
            reviewers: Vec::new(),
            labels: Vec::new(),
            checks: vec![hoshi_core::PullRequestCheck {
                name: "lint".into(),
                state,
                url: None,
            }],
            review_threads: Vec::new(),
            timeline: Vec::new(),
        };
        lock(&harness.engine.state).pull_requests.record(
            "t",
            ForgeKind::Github,
            Ok(Some(pull_request.clone())),
            false,
            Instant::now(),
            chrono::Utc::now(),
        );
        lock(&harness.engine.state)
            .call_tool("t", "watch_pull_request", &serde_json::json!({}))
            .unwrap();
        lock(&harness.engine.state).observe_watch(
            "t",
            ForgeKind::Github,
            &pull_request,
            &detail(hoshi_core::ChecksState::Pending),
            Some("me".into()),
        );
        drop(harness);

        let (sender, events) = mpsc::channel();
        let notify: Notify = Arc::new(move |notification| {
            if let Notification::Event(envelope) = notification {
                let _ = sender.send(envelope.event);
            }
        });
        let engine = Engine::from_store(
            Store::open(&database).unwrap(),
            &scratch_dir("pr-watch-checks-restarted"),
            notify,
        )
        .unwrap();
        lock(&engine.state).observe_watch(
            "t",
            ForgeKind::Github,
            &pull_request,
            &detail(hoshi_core::ChecksState::Failing),
            Some("me".into()),
        );

        let update = events
            .try_iter()
            .find_map(|event| match event {
                Event::ItemAdded { item } => match item.content {
                    ItemContent::UserMessage {
                        pull_request_update: Some(update),
                        ..
                    } => Some(update),
                    _ => None,
                },
                _ => None,
            })
            .expect("a wake for the failed check");
        assert_eq!(update.failed_checks, ["lint"]);
    }

    #[test]
    fn an_agent_finds_messages_waits_for_and_reads_another_thread() {
        let harness = Harness::in_memory("thread-tools", completed_turn());
        harness.create_thread("o", ThreadWorkspace::Local).unwrap();
        let call = |tool: &str, arguments: serde_json::Value| {
            lock(&harness.engine.state).call_tool("t", tool, &arguments)
        };

        let listed = call("thread_list", serde_json::json!({})).unwrap();
        let ids: Vec<_> = listed["threads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|thread| thread["threadId"].as_str().unwrap().to_owned())
            .collect();
        assert!(ids.contains(&"o".to_owned()));
        assert!(
            call(
                "thread_send",
                serde_json::json!({ "threadId": "t", "message": "hi" })
            )
            .is_err()
        );
        let sent = call(
            "thread_send",
            serde_json::json!({ "threadId": "o", "message": "check the tests" }),
        )
        .unwrap();
        assert_eq!(sent["delivery"], "started");
        let state = Arc::downgrade(&harness.engine.state);
        let waited = thread_tools::wait_for_thread(
            &state,
            "t",
            &serde_json::json!({ "threadId": "o", "timeoutMs": 10_000 }),
        )
        .unwrap();
        let read = call("thread_read", serde_json::json!({ "threadId": "o" })).unwrap();

        assert_eq!(waited.get("timedOut"), None);
        assert_eq!(waited["lastAnswer"], "done");
        assert_eq!(read.get("archived"), None);
        assert_eq!(read.get("earlierMessages"), None);
        let roles: Vec<_> = read["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| {
                (
                    message["role"].as_str().unwrap(),
                    message["text"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(roles, [("user", "check the tests"), ("assistant", "done")]);
    }

    #[test]
    fn a_secret_the_user_saves_reaches_the_agent_as_a_file_only() {
        let harness = Harness::in_memory("secret", Vec::new());
        harness.send("t", "m1", "deploy it");
        let ask = |label: &'static str| {
            let state = Arc::downgrade(&harness.engine.state);
            thread::spawn(move || {
                let arguments = serde_json::json!({ "label": label, "reason": "To deploy." });
                secret_requests::request_secret(&state, "t", &arguments)
            })
        };
        let opened = |harness: &Harness| {
            harness.wait_for(|event| match event {
                Event::ItemAdded { item } => match item.content {
                    ItemContent::SecretRequest {
                        status: RequestStatus::Pending,
                        ..
                    } => Some(item.id),
                    _ => None,
                },
                _ => None,
            })
        };

        let saving = ask("Deploy token");
        let request_id = opened(&harness);
        assert_eq!(harness.thread("t").status, ThreadStatus::WaitingForInput);
        harness
            .engine
            .answer_secret_request("t", &request_id, Some("s3cret"))
            .unwrap();
        let answer = saving.join().unwrap().unwrap();
        let declining = ask("Other token");
        let declined_id = opened(&harness);
        harness
            .engine
            .answer_secret_request("t", &declined_id, None)
            .unwrap();
        let declined = declining.join().unwrap().unwrap();

        assert_eq!(answer["status"], "saved");
        let file = answer["file"].as_str().unwrap();
        assert_eq!(std::fs::read_to_string(file).unwrap(), "s3cret");
        assert_eq!(declined["status"], "declined");
        let items = harness.engine.transcript("t", None).unwrap().items;
        let recorded = serde_json::to_string(&items).unwrap();
        assert!(!recorded.contains("s3cret"));
        assert!(items.iter().any(|item| matches!(
            item.content,
            ItemContent::SecretRequest {
                saved: Some(true),
                status: RequestStatus::Resolved,
                ..
            }
        )));
        assert_eq!(harness.thread("t").status, ThreadStatus::Working);
    }

    #[test]
    fn a_git_action_commits_on_a_new_branch_and_pushes_it() {
        let origin = git::tests::temp_repository("engine-git-origin");
        let origin_dir = origin.to_string_lossy().into_owned();
        git::stdout(
            &origin_dir,
            &["config", "receive.denyCurrentBranch", "ignore"],
        )
        .unwrap();
        let repo = git::tests::temp_repository("engine-git-action");
        let harness = Harness::new(
            "git-action",
            repo.clone(),
            Store::open_in_memory().unwrap(),
            completed_turn(),
        );
        git::stdout(&harness.root, &["remote", "add", "origin", &origin_dir]).unwrap();
        git::stdout(&harness.root, &["fetch", "origin"]).unwrap();
        std::fs::write(repo.join("login.txt"), "fixed\n").unwrap();
        let request = |message: Option<&str>| GitActionRequest {
            action: GitAction::CommitPush,
            message: message.map(Into::into),
            paths: None,
            new_branch: true,
            draft: false,
        };

        assert_eq!(
            harness
                .engine
                .run_git_action("t", &request(None))
                .unwrap_err(),
            CommandError::CommitMessageUnavailable {
                thread_id: "t".into()
            }
        );
        let before = harness.engine.git_status("t").unwrap();
        assert!(before.has_changes && before.has_remote);
        let message = "fix: stop the login loop\n\n- keep it";
        let result = harness
            .engine
            .run_git_action("t", &request(Some(message)))
            .unwrap();

        assert_eq!(result.subject.as_deref(), Some("fix: stop the login loop"));
        let branch = "hoshi/fix-stop-the-login-loop";
        assert_eq!(result.pushed.as_deref(), Some(branch));
        let after = harness.engine.git_status("t").unwrap();
        assert!(!after.has_changes && after.has_upstream);
        assert_eq!(after.branch.as_deref(), Some(branch));
        assert!(git::branch_tip(&origin_dir, branch).is_some());
        let _ = std::fs::remove_dir_all(&origin);
    }

    /// Bitbucket as far as opening pull requests goes.
    #[derive(Default)]
    struct FakeBitbucket {
        opened: Mutex<Vec<hoshi_forge::NewPullRequest>>,
    }

    impl FakeBitbucket {
        fn pull_request(opened: &hoshi_forge::NewPullRequest) -> PullRequest {
            PullRequest {
                forge: ForgeKind::Bitbucket,
                number: 12,
                title: opened.title.clone(),
                url: "https://bitbucket.org/acme/shop/pull-requests/12".into(),
                author: Some("ana".into()),
                head_branch: opened.head.clone(),
                head_sha: None,
                base_branch: opened.base.clone(),
                state: hoshi_core::PullRequestState::Open,
                checks: None,
                unresolved_comments: 0,
                additions: 0,
                deletions: 0,
                updated_at: "2026-11-09T12:00:00Z".into(),
            }
        }
    }

    impl Forge for FakeBitbucket {
        fn kind(&self) -> ForgeKind {
            ForgeKind::Bitbucket
        }
        fn viewer(&self) -> Result<String, ForgeError> {
            Ok("ana".into())
        }
        fn pull_request_for_branch(
            &self,
            _: &RepositoryRef,
            branch: &str,
        ) -> Result<Option<PullRequest>, ForgeError> {
            Ok(self
                .opened
                .lock()
                .unwrap()
                .iter()
                .find(|opened| opened.head == branch)
                .map(Self::pull_request))
        }
        fn pull_request(&self, _: &RepositoryRef, _: u32) -> Result<PullRequest, ForgeError> {
            Err(ForgeError::NotFound)
        }
        fn search_pull_requests(
            &self,
            _: &RepositoryRef,
            _: &str,
        ) -> Result<Vec<PullRequest>, ForgeError> {
            Ok(Vec::new())
        }
        fn pull_request_detail(
            &self,
            _: &RepositoryRef,
            _: u32,
        ) -> Result<PullRequestDetail, ForgeError> {
            Err(ForgeError::NotFound)
        }
        fn pull_request_files(
            &self,
            _: &RepositoryRef,
            _: u32,
        ) -> Result<Vec<hoshi_forge::PullRequestFile>, ForgeError> {
            Ok(Vec::new())
        }
        fn act(
            &self,
            _: &RepositoryRef,
            _: u32,
            _: &str,
            _: &PullRequestAction,
        ) -> Result<(), ForgeError> {
            Err(ForgeError::NotFound)
        }
        fn create_pull_request(
            &self,
            _: &RepositoryRef,
            pull_request: &hoshi_forge::NewPullRequest,
        ) -> Result<PullRequest, ForgeError> {
            self.opened.lock().unwrap().push(pull_request.clone());
            Ok(Self::pull_request(pull_request))
        }
        fn reviewer_candidates(
            &self,
            _: &RepositoryRef,
            _: &str,
        ) -> Result<Vec<String>, ForgeError> {
            Ok(Vec::new())
        }
        fn labels(&self, _: &RepositoryRef) -> Result<Vec<PullRequestLabel>, ForgeError> {
            Ok(Vec::new())
        }
        fn repository(&self, _: &RepositoryRef) -> Result<RemoteRepository, ForgeError> {
            Err(ForgeError::NotFound)
        }
    }

    #[test]
    fn an_agent_on_bitbucket_pushes_its_branch_and_opens_its_pull_request() {
        let origin = git::tests::temp_repository("engine-pr-origin");
        let origin_dir = origin.to_string_lossy().into_owned();
        git_in(
            &origin_dir,
            &["config", "receive.denyCurrentBranch", "ignore"],
        );
        let repo = git::tests::temp_repository("engine-pr-tool");
        let harness = Harness::new(
            "pr-tool",
            repo,
            Store::open_in_memory().unwrap(),
            completed_turn(),
        );
        git_in(&harness.root, &["remote", "add", "origin", &origin_dir]);
        git_in(&harness.root, &["fetch", "origin"]);
        git_in(&harness.root, &["switch", "-c", "fix/login"]);
        git_in(
            &harness.root,
            &["commit", "--allow-empty", "-m", "fix: login"],
        );
        let state = Arc::downgrade(&harness.engine.state);
        let open = |arguments: serde_json::Value| {
            pull_request_tool::create_pull_request(&state, "t", &arguments)
        };
        let arguments = serde_json::json!({ "title": "Fix the login", "description": "Done." });

        assert!(
            open(arguments.clone())
                .unwrap_err()
                .contains("isn't on Bitbucket")
        );
        let forge = Arc::new(FakeBitbucket::default());
        harness.engine.set_forges(vec![forge.clone()]);
        lock(&harness.engine.state).repositories.insert(
            "p".into(),
            RepositoryRef {
                forge: ForgeKind::Bitbucket,
                host: "bitbucket.org".into(),
                owner: "acme".into(),
                name: "shop".into(),
            },
        );
        assert!(lock(&harness.engine.state).opens_pull_requests("t"));

        let opened = open(arguments.clone()).unwrap();

        assert_eq!(opened["number"], 12);
        assert_eq!(opened["state"], "open");
        assert_eq!(opened["base"], "main");
        assert_eq!(opened["pushed"], true);
        assert!(git::branch_tip(&origin_dir, "fix/login").is_some());
        let sent = forge.opened.lock().unwrap()[0].clone();
        assert_eq!(
            (sent.head.as_str(), sent.body.as_str()),
            ("fix/login", "Done.")
        );
        assert_eq!(
            lock(&harness.engine.state)
                .pull_requests
                .get("t")
                .map(|pull_request| pull_request.number),
            Some(12)
        );
        let again = open(arguments).unwrap_err();
        assert!(
            again.contains("already open") && again.contains("pull-requests/12"),
            "{again}"
        );
        let _ = std::fs::remove_dir_all(&origin);
    }

    #[test]
    fn a_worktree_thread_runs_in_its_own_worktree_until_deleted() {
        let repo = git::tests::temp_repository("engine-worktree");
        let store = Store::open_in_memory().unwrap();
        let harness = Harness::new("worktree", repo, store, completed_turn());

        harness
            .create_thread("w1234567-rest", ThreadWorkspace::NewWorktree)
            .unwrap();
        let thread = harness.thread("w1234567-rest");
        let worktree = thread.worktree_path.clone().unwrap();
        harness.send("w1234567-rest", "m1", "work");
        harness.wait_for_turn_end();

        assert_eq!(thread.branch.as_deref(), Some("hoshi/w1234567"));
        assert_eq!(
            git::current_branch(&worktree).as_deref(),
            Some("hoshi/w1234567")
        );
        assert_eq!(harness.adapter.requests.lock().unwrap()[0].cwd, worktree);
        harness
            .engine
            .dispatch(Command::ThreadDelete {
                thread_id: "w1234567-rest".into(),
            })
            .unwrap();
        wait_until(|| !Path::new(&worktree).exists());
    }

    #[test]
    fn a_thread_can_start_on_a_branch_only_origin_has() {
        let origin = git::tests::temp_repository("engine-origin");
        let origin_str = origin.to_string_lossy().into_owned();
        git_in(&origin_str, &["switch", "-c", "feature"]);
        git_in(
            &origin_str,
            &["commit", "--allow-empty", "-m", "feature work"],
        );
        let tip = git_in(&origin_str, &["rev-parse", "HEAD"]);
        git_in(&origin_str, &["switch", "main"]);
        let repo = git::tests::temp_repository("engine-from-branch");
        let harness = Harness::new(
            "from-branch",
            repo,
            Store::open_in_memory().unwrap(),
            Vec::new(),
        );
        git_in(&harness.root, &["remote", "add", "origin", &origin_str]);
        git_in(&harness.root, &["branch", "mine"]);

        harness
            .create_thread(
                "f1234567-rest",
                ThreadWorkspace::BranchWorktree {
                    branch: "feature".into(),
                },
            )
            .unwrap();
        harness
            .create_thread(
                "g1234567-rest",
                ThreadWorkspace::BranchWorktree {
                    branch: "mine".into(),
                },
            )
            .unwrap();

        let feature = harness.thread("f1234567-rest");
        let worktree = feature.worktree_path.unwrap();
        assert_eq!(feature.branch.as_deref(), Some("feature"));
        assert_eq!(git_in(&worktree, &["rev-parse", "HEAD"]), tip);
        assert_eq!(
            git_in(&worktree, &["rev-parse", "--abbrev-ref", "@{upstream}"]),
            "origin/feature"
        );
        let mine = harness.thread("g1234567-rest");
        assert_eq!(
            git::current_branch(&mine.worktree_path.unwrap()).as_deref(),
            Some("mine")
        );
        assert!(matches!(
            harness.create_thread(
                "h1234567-rest",
                ThreadWorkspace::BranchWorktree {
                    branch: "nowhere".into(),
                },
            ),
            Err(CommandError::WorktreeFailed { .. })
        ));
    }

    fn git_in(dir: &str, args: &[&str]) -> String {
        git::stdout(dir, args).unwrap().trim().to_owned()
    }

    /// The thread's pull request, as the sync would have read it.
    fn set_pull_request(
        harness: &Harness,
        thread_id: &str,
        state: hoshi_core::PullRequestState,
        head_sha: Option<String>,
    ) {
        let branch = harness.thread(thread_id).branch.unwrap();
        let pull_request = hoshi_core::PullRequest {
            forge: ForgeKind::Github,
            number: 7,
            title: "Work".into(),
            url: "https://github.com/o/r/pull/7".into(),
            author: None,
            head_branch: branch,
            head_sha,
            base_branch: "main".into(),
            state,
            checks: None,
            unresolved_comments: 0,
            additions: 0,
            deletions: 0,
            updated_at: "2026-11-07T12:00:00Z".into(),
        };
        lock(&harness.engine.state).pull_requests.record(
            thread_id,
            ForgeKind::Github,
            Ok(Some(pull_request)),
            false,
            Instant::now(),
            chrono::Utc::now(),
        );
    }

    fn commit_in(dir: &str, file: &str) -> String {
        std::fs::write(Path::new(dir).join(file), "work").unwrap();
        git_in(dir, &["add", file]);
        git_in(dir, &["commit", "-m", file]);
        git_in(dir, &["rev-parse", "HEAD"])
    }

    #[test]
    fn a_released_worktree_comes_back_with_the_next_message() {
        let repo = git::tests::temp_repository("engine-release");
        let harness = Harness::new(
            "release",
            repo,
            Store::open_in_memory().unwrap(),
            completed_turn(),
        );
        harness
            .create_thread("r1234567-rest", ThreadWorkspace::NewWorktree)
            .unwrap();
        let worktree = harness.thread("r1234567-rest").worktree_path.unwrap();

        harness.engine.release_worktree("r1234567-rest").unwrap();

        let thread = harness.thread("r1234567-rest");
        assert!(!Path::new(&worktree).exists());
        assert!(thread.worktree_released);
        assert_eq!(thread.branch.as_deref(), Some("hoshi/r1234567"));
        assert!(git::branch_tip(&harness.root, "hoshi/r1234567").is_some());

        harness.send("r1234567-rest", "m1", "go on");
        harness.wait_for_turn_end();

        assert!(Path::new(&worktree).is_dir());
        assert!(!harness.thread("r1234567-rest").worktree_released);
        assert_eq!(harness.adapter.requests.lock().unwrap()[0].cwd, worktree);
    }

    #[test]
    fn a_worktree_with_uncommitted_files_is_kept() {
        let repo = git::tests::temp_repository("engine-dirty");
        let harness = Harness::new("dirty", repo, Store::open_in_memory().unwrap(), Vec::new());
        harness
            .create_thread("d1234567-rest", ThreadWorkspace::NewWorktree)
            .unwrap();
        let worktree = harness.thread("d1234567-rest").worktree_path.unwrap();
        std::fs::write(Path::new(&worktree).join("notes.txt"), "draft").unwrap();

        let refused = harness.engine.release_worktree("d1234567-rest");

        assert_eq!(
            refused,
            Err(CommandError::WorktreeDirty {
                thread_id: "d1234567-rest".into()
            })
        );
        assert!(Path::new(&worktree).join("notes.txt").exists());
        assert!(!harness.thread("d1234567-rest").worktree_released);
    }

    #[test]
    fn a_lock_left_by_a_finished_agent_is_lifted_and_a_live_one_kept() {
        let repo = git::tests::temp_repository("engine-locked");
        let harness = Harness::new("locked", repo, Store::open_in_memory().unwrap(), Vec::new());
        for id in ["l1234567-rest", "k1234567-rest"] {
            harness
                .create_thread(id, ThreadWorkspace::NewWorktree)
                .unwrap();
        }
        let stale = harness.thread("l1234567-rest").worktree_path.unwrap();
        let kept = harness.thread("k1234567-rest").worktree_path.unwrap();
        // No process has this pid: the agent is gone.
        let gone = "claude session work (pid 4294967294)";
        git_in(
            &harness.root,
            &["worktree", "lock", "--reason", gone, &stale],
        );
        git_in(
            &harness.root,
            &["worktree", "lock", "--reason", "mine", &kept],
        );

        harness.engine.release_worktree("l1234567-rest").unwrap();
        let refused = harness.engine.release_worktree("k1234567-rest");

        assert!(!Path::new(&stale).exists());
        assert_eq!(
            refused,
            Err(CommandError::WorktreeLocked {
                thread_id: "k1234567-rest".into(),
                reason: "mine".into()
            })
        );
        assert!(Path::new(&kept).is_dir());
    }

    #[test]
    fn cleaning_up_waits_for_a_merge_and_keeps_local_only_commits() {
        let repo = git::tests::temp_repository("engine-cleanup-refused");
        let harness = Harness::new(
            "cleanup-refused",
            repo,
            Store::open_in_memory().unwrap(),
            Vec::new(),
        );
        harness
            .create_thread("c1234567-rest", ThreadWorkspace::NewWorktree)
            .unwrap();
        let worktree = harness.thread("c1234567-rest").worktree_path.unwrap();
        let pushed = commit_in(&worktree, "a.txt");
        set_pull_request(
            &harness,
            "c1234567-rest",
            hoshi_core::PullRequestState::Open,
            Some(pushed.clone()),
        );
        assert_eq!(
            harness.engine.clean_up_thread("c1234567-rest"),
            Err(CommandError::PullRequestNotMerged {
                thread_id: "c1234567-rest".into()
            })
        );

        commit_in(&worktree, "after-merge.txt");
        set_pull_request(
            &harness,
            "c1234567-rest",
            hoshi_core::PullRequestState::Merged,
            Some(pushed),
        );

        assert_eq!(
            harness.engine.clean_up_thread("c1234567-rest"),
            Err(CommandError::UnpushedCommits {
                thread_id: "c1234567-rest".into()
            })
        );
        assert!(Path::new(&worktree).is_dir());
        assert!(git::branch_tip(&harness.root, "hoshi/c1234567").is_some());
    }

    #[test]
    fn cleaning_up_a_merged_thread_removes_only_local_things() {
        let repo = git::tests::temp_repository("engine-cleanup");
        let harness = Harness::new(
            "cleanup",
            repo,
            Store::open_in_memory().unwrap(),
            Vec::new(),
        );
        harness
            .create_thread("m1234567-rest", ThreadWorkspace::NewWorktree)
            .unwrap();
        let worktree = harness.thread("m1234567-rest").worktree_path.unwrap();
        let head = commit_in(&worktree, "a.txt");
        set_pull_request(
            &harness,
            "m1234567-rest",
            hoshi_core::PullRequestState::Merged,
            Some(head),
        );

        harness.engine.clean_up_thread("m1234567-rest").unwrap();

        let thread = harness.thread("m1234567-rest");
        assert!(!Path::new(&worktree).exists());
        assert!(git::branch_tip(&harness.root, "hoshi/m1234567").is_none());
        assert!(thread.archived);
        assert_eq!(thread.worktree_path, None);
        assert_eq!(thread.branch, None);
        assert_eq!(thread.session_id, None);
        assert!(git::branch_tip(&harness.root, "main").is_some());
    }

    #[test]
    fn cleaning_up_a_root_thread_goes_back_to_the_base_branch() {
        let repo = git::tests::temp_repository("engine-cleanup-root");
        let harness = Harness::new(
            "cleanup-root",
            repo,
            Store::open_in_memory().unwrap(),
            Vec::new(),
        );
        harness
            .create_thread("r1234567-rest", ThreadWorkspace::Local)
            .unwrap();
        git_in(&harness.root, &["branch", "feature"]);
        harness
            .engine
            .switch_branch("r1234567-rest", "feature")
            .unwrap();
        let head = commit_in(&harness.root, "a.txt");
        set_pull_request(
            &harness,
            "r1234567-rest",
            hoshi_core::PullRequestState::Merged,
            Some(head),
        );
        std::fs::write(Path::new(&harness.root).join("draft.txt"), "draft").unwrap();

        assert_eq!(
            harness.engine.clean_up_thread("r1234567-rest"),
            Err(CommandError::WorktreeDirty {
                thread_id: "r1234567-rest".into()
            })
        );
        assert_eq!(
            git::current_branch(&harness.root).as_deref(),
            Some("feature")
        );

        std::fs::remove_file(Path::new(&harness.root).join("draft.txt")).unwrap();
        harness.engine.clean_up_thread("r1234567-rest").unwrap();

        let thread = harness.thread("r1234567-rest");
        assert_eq!(git::current_branch(&harness.root).as_deref(), Some("main"));
        assert!(git::branch_tip(&harness.root, "feature").is_none());
        assert!(thread.archived);
        assert_eq!(thread.session_id, None);
    }

    fn bash(command: &str) -> ProviderOutput {
        ProviderOutput::ToolCallStarted {
            item_id: command.into(),
            name: "Bash".into(),
            input: serde_json::json!({ "command": command }).to_string(),
        }
    }

    fn interrupt(harness: &Harness) {
        harness
            .engine
            .dispatch(Command::TurnInterrupt {
                thread_id: "t".into(),
            })
            .unwrap();
    }

    fn turn_changes(harness: &Harness) -> Vec<DiffFile> {
        harness.wait_for(|event| match event {
            Event::ItemAdded { item } => match item.content {
                ItemContent::TurnChanges { files: Some(files) } => Some(files),
                _ => None,
            },
            _ => None,
        })
    }

    fn paths_of(files: &[DiffFile]) -> Vec<&str> {
        files.iter().map(|file| file.path.as_str()).collect()
    }

    /// A folder beside the project's, named after it as agents name the worktrees they add.
    fn sibling(repo: &Path, suffix: &str) -> PathBuf {
        let name = repo.file_name().unwrap().to_string_lossy();
        let path = repo.with_file_name(format!("{name}-{suffix}"));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn a_worktree_the_agent_adds_beside_the_project_and_works_in_becomes_the_thread_folder() {
        let repo = git::tests::temp_repository("engine-agent-worktree");
        commit_in(&repo.to_string_lossy(), "README.md");
        let worktree = sibling(&repo, "1234");
        let name = worktree.file_name().unwrap().to_string_lossy().into_owned();
        let script = vec![
            bash(&format!("git worktree add ../{name} -b fix/1234 main")),
            bash(&format!("cd ../{name} && echo fixed > fix.txt")),
        ];
        let store = Store::open_in_memory().unwrap();
        let harness = Harness::new("agent-worktree", repo.clone(), store, script);
        std::fs::write(repo.join("draft.txt"), "mine\n").unwrap();
        harness.send("t", "m1", "add a worktree for ticket 1234 and fix it there");
        let before = checkpoint::reference("t", "m1", Moment::Before);
        wait_until(|| checkpoint::exists(&harness.root, &before));

        git::add_worktree(&harness.root, &worktree, "fix/1234").unwrap();
        std::fs::write(worktree.join("fix.txt"), "fixed\n").unwrap();
        std::fs::write(repo.join("draft.txt"), "mine, edited by hand\n").unwrap();
        interrupt(&harness);

        let expected = git::normalize(&worktree.to_string_lossy());
        let attached = harness.wait_for(|event| match event {
            Event::ThreadWorktreeAttached {
                worktree_path,
                branch,
                ..
            } => Some((worktree_path, branch)),
            _ => None,
        });
        assert_eq!(attached, (expected.clone(), Some("fix/1234".into())));
        let files = turn_changes(&harness);
        assert_eq!(paths_of(&files), ["fix.txt"]);
        assert_eq!(harness.engine.turn_diff_files("t", "m1").unwrap(), files);
        assert!(harness.thread("t").worktree_adopted);

        harness.engine.rewind_thread("t", "m1", true).unwrap();
        assert!(!worktree.join("fix.txt").exists());
        assert!(repo.join("draft.txt").exists());

        harness.send("t", "m2", "continue");
        assert_eq!(harness.adapter.requests.lock().unwrap()[1].cwd, expected);
        interrupt(&harness);
        harness.wait_for_turn_end();
        harness
            .engine
            .dispatch(Command::ThreadDelete {
                thread_id: "t".into(),
            })
            .unwrap();
        assert!(worktree.is_dir());
        let _ = git::remove_worktree(&harness.root, &expected);
    }

    #[test]
    fn a_worktree_the_agent_only_adds_leaves_the_thread_where_it_was() {
        let repo = git::tests::temp_repository("engine-unused-worktree");
        let worktree = sibling(&repo, "later");
        let name = worktree.file_name().unwrap().to_string_lossy().into_owned();
        let script = vec![bash(&format!("git worktree add ../{name} -b chore/later"))];
        let store = Store::open_in_memory().unwrap();
        let harness = Harness::new("unused-worktree", repo.clone(), store, script);
        harness.send("t", "m1", "prepare a worktree for later");
        let before = checkpoint::reference("t", "m1", Moment::Before);
        wait_until(|| checkpoint::exists(&harness.root, &before));

        git::add_worktree(&harness.root, &worktree, "chore/later").unwrap();
        std::fs::write(repo.join("notes.txt"), "plan\n").unwrap();
        interrupt(&harness);

        assert_eq!(paths_of(&turn_changes(&harness)), ["notes.txt"]);
        assert_eq!(harness.thread("t").worktree_path, None);
        assert!(worktree.is_dir());
        let _ = git::remove_worktree(&harness.root, &worktree.to_string_lossy());
    }

    #[test]
    fn a_turn_that_moves_to_a_worktree_lists_what_it_changed_there() {
        let repo = git::tests::temp_repository("engine-moved-changes");
        commit_in(&repo.to_string_lossy(), "README.md");
        let worktree = sibling(&repo, "moved");
        git::add_worktree(&repo.to_string_lossy(), &worktree, "feat/moved").unwrap();
        let path = git::normalize(&worktree.to_string_lossy());
        let store = Store::open_in_memory().unwrap();
        let harness = Harness::new("moved-changes", repo.clone(), store, Vec::new());
        harness.send("t", "m1", "work in the other worktree");
        let before = checkpoint::reference("t", "m1", Moment::Before);
        wait_until(|| checkpoint::exists(&harness.root, &before));
        std::fs::write(repo.join("stray.txt"), "root\n").unwrap();

        let entered = ProviderOutput::WorkingDirectory { path: path.clone() };
        lock(&harness.engine.state).receive("t", "m1", entered);
        let arrived = checkpoint::reference("t", "m1", Moment::Arrived(1));
        wait_until(|| checkpoint::exists(&path, &arrived));
        std::fs::write(worktree.join("work.txt"), "done\n").unwrap();
        interrupt(&harness);

        let files = turn_changes(&harness);
        assert_eq!(paths_of(&files), ["work.txt"]);
        assert_eq!(
            harness.thread("t").worktree_path.as_deref(),
            Some(path.as_str())
        );
        harness.engine.rewind_thread("t", "m1", true).unwrap();
        assert!(!worktree.join("work.txt").exists());
        assert!(repo.join("stray.txt").exists());
        let _ = git::remove_worktree(&harness.root, &path);
    }

    #[test]
    fn files_are_never_reverted_in_another_folder_than_the_turn_s() {
        let repo = git::tests::temp_repository("engine-rewind-elsewhere");
        let worktree = sibling(&repo, "elsewhere");
        git::add_worktree(&repo.to_string_lossy(), &worktree, "feat/elsewhere").unwrap();
        let path = git::normalize(&worktree.to_string_lossy());
        let store = Store::open_in_memory().unwrap();
        let harness = Harness::new("rewind-elsewhere", repo.clone(), store, Vec::new());
        harness.send("t", "m1", "change the root");
        let before = checkpoint::reference("t", "m1", Moment::Before);
        wait_until(|| checkpoint::exists(&harness.root, &before));
        std::fs::write(repo.join("root.txt"), "root\n").unwrap();
        interrupt(&harness);
        assert_eq!(paths_of(&turn_changes(&harness)), ["root.txt"]);

        harness.send("t", "m2", "now go to the worktree");
        let entered = ProviderOutput::WorkingDirectory { path: path.clone() };
        lock(&harness.engine.state).receive("t", "m2", entered);
        wait_until(|| harness.thread("t").worktree_path.is_some());
        std::fs::write(worktree.join("kept.txt"), "kept\n").unwrap();
        interrupt(&harness);
        harness.wait_for_turn_end();

        assert_eq!(
            harness.engine.rewind_thread("t", "m1", true).unwrap_err(),
            CommandError::NoCheckpoint {
                thread_id: "t".into(),
                message_id: "m1".into()
            }
        );
        assert!(worktree.join("kept.txt").exists());
        assert!(repo.join("root.txt").exists());
        let _ = git::remove_worktree(&harness.root, &path);
    }

    #[test]
    fn a_finished_turn_lists_the_files_it_changed_and_shows_their_diff() {
        let repo = git::tests::temp_repository("engine-turn-changes");
        let store = Store::open_in_memory().unwrap();
        let harness = Harness::new("turn-changes", repo.clone(), store, Vec::new());
        harness.send("t", "m1", "add a file");
        let before = checkpoint::reference("t", "m1", Moment::Before);
        wait_until(|| git::stdout(&harness.root, &["rev-parse", "--verify", &before]).is_ok());

        std::fs::write(repo.join("notes.txt"), "one\ntwo\n").unwrap();
        harness
            .engine
            .dispatch(Command::TurnInterrupt {
                thread_id: "t".into(),
            })
            .unwrap();

        let files = harness.wait_for(|event| match event {
            Event::ItemAdded { item } => match item.content {
                ItemContent::TurnChanges { files: Some(files) } => Some(files),
                _ => None,
            },
            _ => None,
        });
        let summary: Vec<_> = files
            .iter()
            .map(|file| (file.path.as_str(), file.additions))
            .collect();
        assert_eq!(summary, [("notes.txt", Some(2))]);
        assert_eq!(harness.engine.turn_diff_files("t", "m1").unwrap(), files);
        let diff = harness.engine.turn_file_diff("t", "m1", &files[0]).unwrap();
        assert!(diff.text.contains("+two"));
    }

    #[test]
    fn rewinding_drops_the_message_and_resumes_the_conversation_before_it() {
        let mut script = completed_turn();
        script.insert(
            2,
            ProviderOutput::ConversationCursor {
                cursor: "u-answer".into(),
            },
        );
        let harness = Harness::in_memory("rewind", script);
        harness.send("t", "m1", "one");
        harness.wait_for_turn_end();
        harness.send("t", "m2", "two");
        harness.wait_for_turn_end();

        assert_eq!(
            harness.engine.rewind_thread("t", "m2", true).unwrap_err(),
            CommandError::NoCheckpoint {
                thread_id: "t".into(),
                message_id: "m2".into()
            }
        );
        harness.engine.rewind_thread("t", "m2", false).unwrap();

        let ids: Vec<_> = harness
            .engine
            .transcript("t", None)
            .unwrap()
            .items
            .into_iter()
            .map(|item| item.id)
            .collect();
        assert!(ids.contains(&"m1".to_owned()));
        assert!(!ids.contains(&"m2".to_owned()));
        harness.send("t", "m3", "two again");
        harness.wait_for_turn_end();
        let requests = harness.adapter.requests.lock().unwrap();
        assert_eq!(requests[1].resume_at, None);
        assert_eq!(requests[2].resume.as_deref(), Some("s1"));
        assert_eq!(requests[2].resume_at.as_deref(), Some("u-answer"));
        drop(requests);
        assert_eq!(harness.thread("t").resume_at, None);
        assert_eq!(
            harness.engine.rewind_thread("t", "m1b", false).unwrap_err(),
            CommandError::CannotRewind {
                thread_id: "t".into(),
                message_id: "m1b".into()
            }
        );
    }

    fn waiting_adapter() -> Arc<FakeAdapter> {
        Arc::new(FakeAdapter {
            requests: Mutex::new(Vec::new()),
            script: Vec::new(),
            title: None,
            responses: Arc::default(),
            steers: None,
            finishers: Mutex::default(),
        })
    }

    #[test]
    fn files_are_not_reverted_while_another_thread_works_in_the_same_folder() {
        let harness = Harness::in_memory("rewind-shared", completed_turn());
        harness.create_thread("o", ThreadWorkspace::Local).unwrap();
        harness.send("t", "m1", "one");
        harness.wait_for_turn_end();
        harness.engine.set_adapter(Some(waiting_adapter()));
        harness.send("o", "m2", "long task");

        assert_eq!(
            harness.engine.rewind_thread("t", "m1", true).unwrap_err(),
            CommandError::ThreadBusy {
                thread_id: "o".into()
            }
        );
        harness.engine.rewind_thread("t", "m1", false).unwrap();
        harness
            .engine
            .dispatch(Command::TurnInterrupt {
                thread_id: "o".into(),
            })
            .unwrap();
        harness.wait_for_turn_end();
    }

    #[test]
    fn a_message_to_a_thread_git_works_on_waits_for_that_work() {
        let harness = Harness::in_memory("op-held", completed_turn());
        let cwd = lock(&harness.engine.state).cwd("t").unwrap();
        let op = lock(&harness.engine.state).begin_op("t", &[&cwd]).unwrap();
        assert_eq!(
            lock(&harness.engine.state)
                .begin_op("t", &[&cwd])
                .err()
                .unwrap(),
            CommandError::ThreadBusy {
                thread_id: "t".into()
            }
        );

        harness.send("t", "m1", "go");
        assert!(harness.adapter.requests.lock().unwrap().is_empty());
        assert_eq!(harness.thread("t").status, ThreadStatus::Idle);

        lock(&harness.engine.state).end_op(op, None).unwrap();
        harness.wait_for_turn_end();
        assert_eq!(harness.adapter.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_turn_in_a_folder_git_changes_starts_once_git_is_done() {
        let harness = Harness::in_memory("op-parked", completed_turn());
        harness.create_thread("o", ThreadWorkspace::Local).unwrap();
        let cwd = lock(&harness.engine.state).cwd("t").unwrap();
        let op = lock(&harness.engine.state).begin_op("o", &[&cwd]).unwrap();

        harness.send("t", "m1", "go");
        assert!(harness.adapter.requests.lock().unwrap().is_empty());
        assert_eq!(harness.thread("t").status, ThreadStatus::Working);

        lock(&harness.engine.state).end_op(op, None).unwrap();
        assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Completed);
        assert_eq!(harness.adapter.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_turn_waiting_for_git_can_be_stopped() {
        let harness = Harness::in_memory("op-parked-stop", completed_turn());
        harness.create_thread("o", ThreadWorkspace::Local).unwrap();
        let cwd = lock(&harness.engine.state).cwd("t").unwrap();
        let op = lock(&harness.engine.state).begin_op("o", &[&cwd]).unwrap();
        harness.send("t", "m1", "go");

        harness
            .engine
            .dispatch(Command::TurnInterrupt {
                thread_id: "t".into(),
            })
            .unwrap();
        assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Interrupted);
        lock(&harness.engine.state).end_op(op, None).unwrap();

        assert!(harness.adapter.requests.lock().unwrap().is_empty());
        assert_eq!(harness.thread("t").status, ThreadStatus::Idle);
    }

    #[test]
    fn creating_a_worktree_leaves_the_engine_free_for_other_threads() {
        let repo = git::tests::temp_repository("engine-slow-checkout");
        let hooks = repo.join(".git").join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        std::fs::write(hooks.join("post-checkout"), "#!/bin/sh\nsleep 2\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let hook = hooks.join("post-checkout");
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let harness = Harness::new(
            "slow-checkout",
            repo,
            Store::open_in_memory().unwrap(),
            completed_turn(),
        );

        let engine = &harness.engine;
        let (created_in, snapshot_in) = thread::scope(|scope| {
            let creating = scope.spawn(|| {
                let started = Instant::now();
                engine
                    .dispatch(Command::ThreadCreate {
                        thread_id: "s1234567-rest".into(),
                        project_id: Some("p".into()),
                        title: String::new(),
                        workspace: ThreadWorkspace::NewWorktree,
                        model: None,
                        effort: None,
                        fast_mode: false,
                        ultracode: false,
                        access_mode: AccessMode::AcceptEdits,
                        plan_mode: false,
                    })
                    .unwrap();
                started.elapsed()
            });
            thread::sleep(Duration::from_millis(500));
            let started = Instant::now();
            engine.snapshot();
            let snapshot_in = started.elapsed();
            (creating.join().unwrap(), snapshot_in)
        });

        assert!(created_in >= Duration::from_millis(1500), "{created_in:?}");
        assert!(snapshot_in < Duration::from_millis(500), "{snapshot_in:?}");
        assert!(harness.thread("s1234567-rest").worktree_path.is_some());
    }

    #[test]
    fn a_task_in_its_own_worktree_is_created_off_the_lock() {
        let repo = git::tests::temp_repository("engine-slow-task");
        let hooks = repo.join(".git").join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        std::fs::write(hooks.join("post-checkout"), "#!/bin/sh\nsleep 2\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let hook = hooks.join("post-checkout");
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let harness = Harness::new(
            "slow-task",
            repo,
            Store::open_in_memory().unwrap(),
            Vec::new(),
        );
        let adapter = adapter(None);
        harness.engine.set_adapter(Some(adapter.clone()));
        harness.send("t", "m1", "split the work");
        let state = Arc::downgrade(&harness.engine.state);
        let engine = &harness.engine;

        let (task, snapshot_in) = thread::scope(|scope| {
            let delegating = scope.spawn(|| {
                let arguments =
                    serde_json::json!({ "task": "count the files", "isolation": "worktree" });
                delegation::delegate_task(&state, "t", &arguments).unwrap()
            });
            thread::sleep(Duration::from_millis(500));
            let started = Instant::now();
            engine.snapshot();
            let snapshot_in = started.elapsed();
            (delegating.join().unwrap(), snapshot_in)
        });

        assert!(snapshot_in < Duration::from_millis(500), "{snapshot_in:?}");
        let child = harness.thread(task["taskId"].as_str().unwrap());
        assert_eq!(child.parent_thread_id.as_deref(), Some("t"));
        let worktree = child.worktree_path.unwrap();
        assert!(Path::new(&worktree).is_dir());
        let requests = adapter.requests.lock().unwrap();
        assert_eq!(requests[1].cwd, worktree);
        assert_eq!(requests[1].prompt, "count the files");
    }

    #[test]
    fn a_wake_for_a_released_thread_creates_its_worktree_again() {
        let repo = git::tests::temp_repository("engine-wake-released");
        let harness = Harness::new(
            "wake-released",
            repo,
            Store::open_in_memory().unwrap(),
            completed_turn(),
        );
        harness
            .create_thread("r1234567-rest", ThreadWorkspace::NewWorktree)
            .unwrap();
        let worktree = harness.thread("r1234567-rest").worktree_path.unwrap();
        harness.engine.release_worktree("r1234567-rest").unwrap();
        assert!(!Path::new(&worktree).exists());
        let pull_request = hoshi_core::PullRequest {
            forge: ForgeKind::Github,
            number: 7,
            title: "Work".into(),
            url: "https://github.com/o/r/pull/7".into(),
            author: Some("me".into()),
            head_branch: "hoshi/r1234567".into(),
            head_sha: Some("abc1234".into()),
            base_branch: "main".into(),
            state: hoshi_core::PullRequestState::Open,
            checks: None,
            unresolved_comments: 0,
            additions: 0,
            deletions: 0,
            updated_at: "2026-11-07T12:00:00Z".into(),
        };
        lock(&harness.engine.state).pull_requests.record(
            "r1234567-rest",
            ForgeKind::Github,
            Ok(Some(pull_request.clone())),
            false,
            Instant::now(),
            chrono::Utc::now(),
        );
        lock(&harness.engine.state)
            .call_tool(
                "r1234567-rest",
                "watch_pull_request",
                &serde_json::json!({}),
            )
            .unwrap();
        let detail = hoshi_core::PullRequestDetail {
            supports: hoshi_core::PullRequestSupport::all(),
            forge_id: "id".into(),
            repository: "o/r".into(),
            mergeability: hoshi_core::Mergeability::Mergeable,
            merge_methods: Vec::new(),
            can_edit: true,
            body: String::new(),
            changed_files: 1,
            reviewers: Vec::new(),
            labels: Vec::new(),
            checks: vec![hoshi_core::PullRequestCheck {
                name: "lint".into(),
                state: hoshi_core::ChecksState::Failing,
                url: None,
            }],
            review_threads: Vec::new(),
            timeline: Vec::new(),
        };

        lock(&harness.engine.state).observe_watch(
            "r1234567-rest",
            ForgeKind::Github,
            &pull_request,
            &detail,
            Some("me".into()),
        );

        assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Completed);
        assert!(Path::new(&worktree).is_dir());
        let requests = harness.adapter.requests.lock().unwrap();
        assert_eq!(requests.last().unwrap().cwd, worktree);
    }

    fn wait_until(mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out");
            thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn an_existing_worktree_the_agent_enters_is_adopted_and_never_removed() {
        let repo = git::tests::temp_repository("engine-adopt-worktree");
        let worktree = repo.with_extension("adopt-wt");
        let _ = std::fs::remove_dir_all(&worktree);
        git::add_worktree(&repo.to_string_lossy(), &worktree, "feat/mine").unwrap();
        let path = git::normalize(&worktree.to_string_lossy());
        let script = vec![
            ProviderOutput::WorkingDirectory { path: path.clone() },
            ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Completed,
            },
        ];
        let harness = Harness::new(
            "adopt-worktree",
            repo,
            Store::open_in_memory().unwrap(),
            script,
        );

        harness.send("t", "m1", "work in my worktree");
        harness.wait_for_turn_end();
        wait_until(|| harness.thread("t").worktree_path.is_some());

        let thread = harness.thread("t");
        assert_eq!(thread.worktree_path.as_deref(), Some(path.as_str()));
        assert_eq!(thread.branch.as_deref(), Some("feat/mine"));
        assert!(thread.worktree_adopted);
        assert_eq!(
            harness.engine.release_worktree("t").unwrap_err(),
            CommandError::WorktreeAdopted {
                thread_id: "t".into()
            }
        );
        harness
            .engine
            .dispatch(Command::ThreadDelete {
                thread_id: "t".into(),
            })
            .unwrap();
        assert!(worktree.is_dir());
        let _ = git::remove_worktree(&harness.root, &path);
    }

    #[test]
    fn leaving_the_worktree_brings_the_thread_back_to_the_project_root() {
        let repo = git::tests::temp_repository("engine-leave-worktree");
        let worktree = repo.with_extension("leave-wt");
        let _ = std::fs::remove_dir_all(&worktree);
        git::add_worktree(&repo.to_string_lossy(), &worktree, "feat/leave").unwrap();
        let path = git::normalize(&worktree.to_string_lossy());
        let root = git::normalize(&repo.to_string_lossy());
        let script = vec![
            ProviderOutput::WorkingDirectory { path: path.clone() },
            ProviderOutput::WorkingDirectory { path: root },
            ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Completed,
            },
        ];
        let harness = Harness::new(
            "leave-worktree",
            repo,
            Store::open_in_memory().unwrap(),
            script,
        );

        harness.send("t", "m1", "go in and out");
        harness.wait_for_turn_end();
        wait_until(|| lock(&harness.engine.state).session_moves.is_empty());
        // The root's branch is read in the background once the turn ends.
        wait_until(|| harness.thread("t").branch.is_some());

        let thread = harness.thread("t");
        assert_eq!(thread.worktree_path, None);
        assert_eq!(thread.branch.as_deref(), Some("main"));
        assert!(!thread.worktree_adopted);
        let _ = git::remove_worktree(&harness.root, &path);
    }

    #[test]
    fn a_worktree_created_mid_turn_is_attached_before_the_turn_ends() {
        let repo = git::tests::temp_repository("engine-live-worktree");
        let worktree = repo.with_extension("live-wt");
        let _ = std::fs::remove_dir_all(&worktree);
        let folder = worktree.to_string_lossy().into_owned();
        let script = vec![
            bash(&format!("git worktree add {folder} -b chore/deps")),
            bash(&format!("cd {folder} && cargo update")),
        ];
        let store = Store::open_in_memory().unwrap();
        let harness = Harness::new("live-worktree", repo, store, script);
        harness.send("t", "m1", "make a worktree and keep going");
        // Lets the watcher register the new project's `.git` folder.
        thread::sleep(std::time::Duration::from_millis(500));

        git::add_worktree(&harness.root, &worktree, "chore/deps").unwrap();

        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        let attached = std::iter::from_fn(|| {
            let left = deadline.saturating_duration_since(Instant::now());
            harness.events.recv_timeout(left).ok()
        })
        .find_map(|event| match event {
            Event::ThreadWorktreeAttached { branch, .. } => Some(branch),
            _ => None,
        });
        assert_eq!(attached, Some(Some("chore/deps".into())));
        assert_eq!(harness.thread("t").status, ThreadStatus::Working);
        let _ = git::remove_worktree(&harness.root, &worktree.to_string_lossy());
    }

    #[test]
    fn a_local_thread_records_the_branch_of_the_project_root() {
        let repo = git::tests::temp_repository("engine-branch");
        let store = Store::open_in_memory().unwrap();
        let harness = Harness::new("branch", repo, store, Vec::new());

        let branch = harness.wait_for(|event| match event {
            Event::ThreadBranchSet { branch, .. } => Some(branch),
            _ => None,
        });

        assert_eq!(branch.as_deref(), Some("main"));
    }

    #[test]
    fn a_worktree_needs_a_git_repository() {
        let harness = Harness::in_memory("no-git", Vec::new());

        let error = harness
            .create_thread("w", ThreadWorkspace::NewWorktree)
            .unwrap_err();

        assert!(matches!(error, CommandError::NotAGitRepository { .. }));
    }

    /// One agent process the test speaks for, kept after its turns while it says so.
    #[derive(Default)]
    struct KeptProcess {
        sink: Mutex<Option<OutputSink>>,
        spawned: Mutex<usize>,
        lingers: Mutex<bool>,
        /// Whether a next turn may run in it.
        takes_turns: Mutex<bool>,
        continued: Mutex<Vec<String>>,
        stopped: Mutex<Vec<String>>,
        closed: Mutex<bool>,
    }

    impl KeptProcess {
        fn report(&self, output: ProviderOutput) {
            let mut sink = self.sink.lock().unwrap().take().unwrap();
            sink(output);
            *self.sink.lock().unwrap() = Some(sink);
        }
    }

    struct KeptAdapter(Arc<KeptProcess>);

    struct KeptTurn(Arc<KeptProcess>);

    impl RunningTurn for KeptTurn {
        fn interrupt(&self) {}

        fn respond(&self, _request_id: &str, _response: &RequestResponse) {}

        fn stop_background_task(&self, task_id: &str) {
            self.0.stopped.lock().unwrap().push(task_id.to_owned());
        }

        fn await_background_tasks(
            &self,
            task_ids: &[String],
            _awaited: bool,
        ) -> Option<Vec<String>> {
            Some(
                task_ids
                    .iter()
                    .filter(|task_id| *task_id == "b1")
                    .cloned()
                    .collect(),
            )
        }

        fn lingers(&self) -> bool {
            *self.0.lingers.lock().unwrap()
        }

        fn continue_with(&self, request: &TurnRequest) -> bool {
            if !*self.0.takes_turns.lock().unwrap() {
                return false;
            }
            self.0
                .continued
                .lock()
                .unwrap()
                .push(request.prompt.clone());
            true
        }

        fn close(&self) {
            *self.0.closed.lock().unwrap() = true;
        }
    }

    impl ProviderAdapter for KeptAdapter {
        fn models(&self) -> hoshi_core::ProviderModels {
            hoshi_core::ProviderModels::default()
        }

        fn start_turn(
            &self,
            _request: TurnRequest,
            sink: OutputSink,
        ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
            *self.0.spawned.lock().unwrap() += 1;
            *self.0.sink.lock().unwrap() = Some(sink);
            Ok(Box::new(KeptTurn(self.0.clone())))
        }
    }

    fn dev_server() -> BackgroundTask {
        BackgroundTask {
            task_id: "b1".into(),
            tool_call_id: Some("toolu_1".into()),
            description: "Start the dev server".into(),
            started_at: 1,
        }
    }

    /// Thread `t`'s first turn starts the dev server and completes, leaving it running.
    fn leave_dev_server_running(name: &str) -> (Harness, Arc<KeptProcess>) {
        let harness = Harness::in_memory(name, Vec::new());
        let process = Arc::new(KeptProcess {
            takes_turns: Mutex::new(true),
            ..KeptProcess::default()
        });
        harness
            .engine
            .set_adapter(Some(Arc::new(KeptAdapter(process.clone()))));
        harness.send("t", "m1", "start the dev server");
        harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));
        process.report(ProviderOutput::SessionStarted {
            session_id: "s1".into(),
        });
        process.report(ProviderOutput::BackgroundTasks {
            tasks: vec![dev_server()],
            waiting: false,
        });
        *process.lingers.lock().unwrap() = true;
        process.report(ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        });
        assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Completed);
        (harness, process)
    }

    fn running_in_background(harness: &Harness) -> Vec<ThreadBackgroundTasks> {
        harness.engine.snapshot().background_tasks
    }

    #[test]
    fn a_turn_that_leaves_a_command_running_ends_and_its_process_takes_the_next_message() {
        let (harness, process) = leave_dev_server_running("kept-process");

        assert_eq!(harness.thread("t").status, ThreadStatus::Idle);
        assert_eq!(
            running_in_background(&harness),
            [ThreadBackgroundTasks {
                thread_id: "t".into(),
                tasks: vec![dev_server()],
                waiting: false,
            }]
        );

        harness.send("t", "m2", "is it up?");
        let started = harness.wait_for(|event| match event {
            Event::TurnStarted { turn_id, .. } => Some(turn_id),
            _ => None,
        });
        assert_eq!(started, "m2");
        assert_eq!(*process.spawned.lock().unwrap(), 1);
        assert_eq!(*process.continued.lock().unwrap(), ["is it up?"]);

        process.report(ProviderOutput::AssistantMessage {
            item_id: "a2".into(),
            text: "It is.".into(),
        });
        process.report(ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        });
        assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Completed);
        assert_eq!(running_in_background(&harness).len(), 1);
    }

    #[test]
    fn a_command_left_running_that_ends_wakes_the_agent_in_a_turn_of_its_own() {
        let (harness, process) = leave_dev_server_running("kept-process-wake");

        process.report(ProviderOutput::BackgroundTasks {
            tasks: Vec::new(),
            waiting: false,
        });
        process.report(ProviderOutput::Woke);
        let woken = harness.wait_for(|event| match event {
            Event::TurnStarted { turn_id, .. } => Some(turn_id),
            _ => None,
        });
        assert_ne!(woken, "m1");
        assert_eq!(harness.thread("t").status, ThreadStatus::Working);
        process.report(ProviderOutput::BackgroundTaskEnded {
            task_id: "b1".into(),
            tool_use_id: Some("toolu_1".into()),
            failed: true,
        });
        *process.lingers.lock().unwrap() = false;
        process.report(ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        });
        assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Completed);

        let items = harness.engine.transcript("t", None).unwrap().items;
        assert!(
            items
                .iter()
                .any(|item| item.id == format!("{woken}:ended:b1"))
        );
        assert!(running_in_background(&harness).is_empty());
        assert!(lock(&harness.engine.state).lingering.is_empty());
    }

    #[test]
    fn a_process_kept_between_turns_is_forgotten_once_it_ends() {
        let (harness, process) = leave_dev_server_running("kept-process-ends");

        process.report(ProviderOutput::ProcessEnded);

        assert!(running_in_background(&harness).is_empty());
        assert!(lock(&harness.engine.state).lingering.is_empty());
        harness.send("t", "m2", "again");
        harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));
        assert_eq!(*process.spawned.lock().unwrap(), 2);
    }

    #[test]
    fn a_command_left_running_can_be_stopped_between_turns() {
        let (harness, process) = leave_dev_server_running("kept-process-stop");

        harness
            .engine
            .dispatch(Command::BackgroundTaskStop {
                thread_id: "t".into(),
                task_id: "b1".into(),
            })
            .unwrap();

        assert_eq!(*process.stopped.lock().unwrap(), ["b1"]);
    }

    #[test]
    fn a_next_message_the_kept_process_cannot_take_starts_a_fresh_one() {
        let (harness, process) = leave_dev_server_running("kept-process-refuses");
        *process.takes_turns.lock().unwrap() = false;

        harness.send("t", "m2", "now with another model");
        harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));

        assert!(*process.closed.lock().unwrap());
        assert_eq!(*process.spawned.lock().unwrap(), 2);
        assert!(running_in_background(&harness).is_empty());
    }

    #[test]
    fn archiving_or_deleting_a_thread_ends_the_process_its_turn_kept() {
        let (harness, process) = leave_dev_server_running("kept-process-archive");
        harness
            .engine
            .dispatch(Command::ThreadArchive {
                thread_id: "t".into(),
                archived: true,
            })
            .unwrap();
        assert!(*process.closed.lock().unwrap());
        assert!(running_in_background(&harness).is_empty());

        let (harness, process) = leave_dev_server_running("kept-process-delete");
        harness
            .engine
            .dispatch(Command::ThreadDelete {
                thread_id: "t".into(),
            })
            .unwrap();
        assert!(*process.closed.lock().unwrap());
        assert!(lock(&harness.engine.state).lingering.is_empty());
    }

    #[test]
    fn the_agent_awaits_its_background_commands_while_its_turn_runs() {
        let (harness, _process) = leave_dev_server_running("kept-process-await");
        let call = |harness: &Harness| {
            lock(&harness.engine.state).call_tool(
                "t",
                background::TOOL,
                &serde_json::json!({ "backgroundTaskIds": ["b1", "b9"] }),
            )
        };
        assert!(call(&harness).is_err(), "no turn runs");

        harness.send("t", "m2", "run the tests");
        harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));
        let answer = call(&harness).unwrap();

        assert_eq!(answer["backgroundTaskIds"], serde_json::json!(["b1"]));
        assert_eq!(answer["notRunning"], serde_json::json!(["b9"]));
        assert!(answer["next"].is_string());
    }
}
