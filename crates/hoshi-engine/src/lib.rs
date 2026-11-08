//! Runtime loop: takes commands, lets the core decide events, persists them
//! through the store, notifies the UI, then runs the resulting effects
//! (provider turns, git). Provider output comes back through the same path.

mod attachments;
mod diff;
mod editors;
mod folders;
mod git;
mod paths;
mod project_icon;
mod pull_request_cache;
mod pull_requests;
mod refreshing;
mod secrets;
mod terminal;
mod titles;
mod watch;
mod worktree;

use attachments::Attachments;
use chrono::{SecondsFormat, Utc};
use hoshi_core::{
    ChangesSummary, Command, CommandError, ContextUsage, DiffFile, DiffScope, Editor, Event,
    EventEnvelope, FileDiff, FolderListing, ForgeAccount, ForgeKind, ItemContent,
    MessageAttachment, ProviderModels, ProviderOutput, PullRequest, PullRequestAction,
    PullRequestDetail, PullRequestLabel, ReadModel, RemoteRepository, SlashCommand, Snapshot,
    StreamDelta, StreamKind, TerminalAttach, ThreadContextUsage, ThreadItem, ThreadPullRequest,
    ThreadPullRequestDetail, ThreadStatus, ThreadTitleSettings, ThreadWorkspace, TurnFailure,
    TurnOutcome, UsageLimits, WorkspaceEntry, decide, record_branch, record_output, record_title,
    record_worktree, record_worktree_left, turn_failed,
};
use hoshi_core::{ProviderUpdate, ProviderUpdateFailure, ProviderVersion};
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
use pull_requests::PullRequestSync;
use refreshing::Refreshing;
use secrets::SecretKey;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError, Weak, mpsc};
use std::thread;
use std::time::{Duration, Instant};
pub use terminal::TerminalSink;
use terminal::{Launch, Terminals};
use watch::GitWatcher;
use worktree::TurnWorkspace;

pub enum Notification {
    Event(EventEnvelope),
    Stream(StreamDelta),
    /// Read from a forge; cached, not part of the event log.
    PullRequest(ThreadPullRequest),
    /// A fresh read of a pull request's detail, after the panel showed the cached one.
    PullRequestDetail(ThreadPullRequestDetail),
    /// Cached, not part of the event log: it changes on every model call.
    ContextUsage(ThreadContextUsage),
    /// The provider plan's limits, as a running turn last reported them.
    UsageLimits(UsageLimits),
    /// `None` once the provider is disabled.
    ProviderVersion(Option<ProviderVersion>),
}

pub type Notify = Arc<dyn Fn(Notification) + Send + Sync>;

/// Shared by every thread that talks to the engine: Tauri commands and the
/// reader threads of running turns.
pub struct Engine {
    state: Arc<Mutex<State>>,
    terminals: Terminals,
    /// Per working folder.
    paths: Refreshing<Vec<paths::Indexed>>,
    /// Per working folder, which may add its own commands and skills.
    slash_commands: Refreshing<Vec<SlashCommand>>,
    /// Held while the provider is asked for its limits, so one ask serves everyone waiting.
    usage_probe: Arc<Mutex<()>>,
    /// Held while the provider's CLI is checked against its releases or updated.
    maintenance: Arc<Mutex<()>>,
}

const LINKED_PULL_REQUEST_FRESH_FOR: Duration = Duration::from_secs(60);
const PATHS_FRESH_FOR: Duration = Duration::from_secs(10);
const SLASH_COMMANDS_FRESH_FOR: Duration = Duration::from_secs(5 * 60);
const USAGE_LIMITS_FRESH_FOR: Duration = Duration::from_secs(60);
const PROVIDER_VERSION_CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const UPDATE_PROGRESS_EVERY: Duration = Duration::from_millis(250);
const PATH_SEARCH_LIMIT: usize = 50;
/// A first title is tried this many times, waiting longer before each retry.
const TITLE_ATTEMPTS: u32 = 3;
const TITLE_RETRY_DELAY: Duration = Duration::from_secs(2);

struct State {
    /// Wakes the pull request sync before its next tick.
    sync_wake: Option<mpsc::Sender<()>>,
    store: Store,
    model: ReadModel,
    sequence: u64,
    /// Running turns by thread id.
    turns: HashMap<String, Turn>,
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
    /// Threads whose generated title missed the subject; named again after their turn.
    titles_to_refine: HashSet<String>,
    /// `None` until the provider reports or is asked for them; kept across restarts.
    usage_limits: Option<UsageLimits>,
    /// When the provider last answered for its limits in this run.
    usage_checked_at: Option<Instant>,
    /// `None` until the enabled provider's CLI was checked.
    provider_version: Option<ProviderVersion>,
    /// Where each thread's agent last moved its session, until git confirmed it.
    session_moves: HashMap<String, String>,
}

/// A thread whose pull request is due to be read.
struct PullRequestJob {
    thread_id: String,
    branch: String,
    repository: RepositoryRef,
    forge: Arc<dyn Forge>,
}

struct Turn {
    id: String,
    running: Box<dyn RunningTurn>,
    /// `None` outside a git repository.
    workspace: Option<TurnWorkspace>,
}

/// What a background check for a worktree the turn created needs from the state.
struct WorktreeCheck {
    workspace: TurnWorkspace,
    /// Worktrees of other threads.
    claimed: Vec<String>,
    /// No other turn of the project was running at the time of the check.
    alone: bool,
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
        let saved_details = store.pull_request_details()?.into_iter().map(|cached| {
            let version = Version {
                number: cached.number,
                updated_at: cached.version,
            };
            (cached.thread_id, version, cached.detail)
        });
        let pull_request_cache = PullRequestCache::with_details(saved_details);
        let context_usages = snapshot
            .context_usages
            .iter()
            .map(|cached| (cached.thread_id.clone(), cached.usage))
            .collect();
        let usage_limits = store.usage_limits().unwrap_or_else(|error| {
            eprintln!("hoshi: failed to read the cached usage limits: {error}");
            None
        });
        let mut state = State {
            model: ReadModel::from_snapshot(&snapshot),
            sequence: snapshot.sequence,
            store,
            turns: HashMap::new(),
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
            pull_request_cache,
            linked_pull_requests: HashMap::new(),
            context_usages,
            usage_limits,
            usage_checked_at: None,
            provider_version: None,
            session_moves: HashMap::new(),
            thread_titles: ThreadTitleSettings::default(),
            titles_to_refine: HashSet::new(),
        };
        let stale: Vec<_> = state
            .model
            .threads()
            .filter_map(|thread| Some((thread.id.clone(), thread.active_turn_id.clone()?)))
            .collect();
        for (thread_id, turn_id) in stale {
            let outcome = TurnOutcome::Interrupted;
            state.receive(
                &thread_id,
                &turn_id,
                ProviderOutput::TurnCompleted { outcome },
            );
        }
        let thread_ids = state
            .model
            .threads()
            .map(|thread| thread.id.clone())
            .collect();
        let shared = Arc::new_cyclic(|this| {
            state.this = this.clone();
            Mutex::new(state)
        });
        refresh_branches(Arc::downgrade(&shared), thread_ids);
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
            terminals: Terminals::default(),
            paths: Refreshing::new(PATHS_FRESH_FOR),
            slash_commands: Refreshing::new(SLASH_COMMANDS_FRESH_FOR),
            usage_probe: Arc::default(),
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
        let working = lock(&self.state)
            .model
            .thread(thread_id)
            .is_some_and(|thread| thread.status == ThreadStatus::Working);
        if working {
            return Err(CommandError::ThreadBusy {
                thread_id: thread_id.to_owned(),
            });
        }
        let cwd = self.repository_cwd(thread_id)?;
        git::switch(&cwd, branch).map_err(|detail| CommandError::GitFailed { detail })?;
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

    /// Checks the token against Bitbucket, then keeps it in the system's credential store.
    pub fn connect_bitbucket(
        &self,
        email: &str,
        token: &str,
    ) -> Result<ForgeAccount, CommandError> {
        let credentials = BitbucketCredentials {
            email: email.trim().to_owned(),
            token: token.trim().to_owned(),
        };
        let login = Bitbucket::with_credentials(credentials.clone())
            .viewer()
            .map_err(forge_error)?;
        secrets::write(SecretKey::Bitbucket, &credentials.encode())
            .map_err(|detail| CommandError::Internal { detail })?;
        self.credentials_changed(ForgeKind::Bitbucket);
        Ok(ForgeAccount {
            forge: ForgeKind::Bitbucket,
            login: Some(login),
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
        let mut state = lock(&self.state);
        let (thread, root) = state.removable(thread_id)?;
        let path = thread
            .worktree_path
            .clone()
            .ok_or_else(|| CommandError::NothingToClean {
                thread_id: thread_id.to_owned(),
            })?;
        if !thread.worktree_released {
            State::remove_clean_worktree(&root, &path, thread_id)?;
        }
        state.decide_and_commit(Command::ThreadWorktreeRelease {
            thread_id: thread_id.to_owned(),
        })
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

    /// The cleanup itself; returns the root threads whose branch changed under them.
    fn clean_up_locked(&self, thread_id: &str) -> Result<Vec<String>, CommandError> {
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
        if let Some(path) = &worktree
            && Path::new(path).is_dir()
            && !git::is_clean(path).map_err(|detail| CommandError::GitFailed { detail })?
        {
            return Err(CommandError::WorktreeDirty {
                thread_id: thread_id.to_owned(),
            });
        }
        // Only the branch the pull request merged; one the thread moved to since stays.
        let branch = thread
            .branch
            .clone()
            .filter(|branch| *branch == pull_request.head_branch)
            .and_then(|branch| Some((git::branch_tip(&root, &branch)?, branch)));
        if let Some((tip, _)) = &branch {
            let held_by_pull_request = pull_request.head_sha.as_deref().is_some_and(|head| {
                head.starts_with(tip.as_str())
                    || tip.starts_with(head)
                    || git::is_ancestor(&root, tip, head)
            });
            if !held_by_pull_request && !git::on_a_remote(&root, tip) {
                return Err(CommandError::UnpushedCommits {
                    thread_id: thread_id.to_owned(),
                });
            }
        }
        let in_root = thread.worktree_path.is_none()
            && branch.is_some()
            && git::current_branch(&root).as_deref() == Some(pull_request.head_branch.as_str());
        let sharing_root = if in_root {
            state.root_threads_sharing(&thread)?
        } else {
            Vec::new()
        };
        if in_root {
            if !git::is_clean(&root).map_err(|detail| CommandError::GitFailed { detail })? {
                return Err(CommandError::WorktreeDirty {
                    thread_id: thread_id.to_owned(),
                });
            }
            git::switch(&root, &pull_request.base_branch)
                .map_err(|detail| CommandError::GitFailed { detail })?;
        }
        if let Some(path) = &worktree {
            State::remove_clean_worktree(&root, path, thread_id)?;
        }
        if let Some((_, name)) = &branch
            && let Err(detail) = git::delete_branch(&root, name)
        {
            if worktree.is_some() {
                state.decide_and_commit(Command::ThreadWorktreeRelease {
                    thread_id: thread_id.to_owned(),
                })?;
            }
            return Err(CommandError::GitFailed { detail });
        }
        state.decide_and_commit(Command::ThreadCleanUp {
            thread_id: thread_id.to_owned(),
        })?;
        Ok(sharing_root)
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

    pub fn snapshot(&self) -> Snapshot {
        let state = lock(&self.state);
        Snapshot {
            sequence: state.sequence,
            projects: state.model.projects().cloned().collect(),
            threads: state.model.threads().cloned().collect(),
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
        }
    }

    pub fn thread_items(&self, thread_id: &str) -> Result<Vec<ThreadItem>, CommandError> {
        lock(&self.state)
            .store
            .thread_items(thread_id)
            .map_err(internal)
    }

    pub fn dispatch(&self, command: Command) -> Result<(), CommandError> {
        let mut state = lock(&self.state);
        let command = state.prepare(command)?;
        let interrupt = match &command {
            Command::TurnInterrupt { thread_id } => Some(thread_id.clone()),
            _ => None,
        };
        let freed_worktrees = state.worktrees_freed_by(&command);
        let freed_images = state.images_freed_by(&command);
        let now = now();
        let events = decide(&state.model, command, &now)?;
        state.commit(&events, &now)?;

        let mut message = None;
        let titled: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                Event::ThreadTitleSet { thread_id, title } => {
                    Some((thread_id.clone(), title.clone()))
                }
                _ => None,
            })
            .collect();
        for event in &events {
            match event {
                Event::ItemAdded {
                    item:
                        ThreadItem {
                            content: ItemContent::UserMessage { text, attachments },
                            ..
                        },
                } => {
                    message = Some((text.clone(), attachments.clone()));
                    if let Some((thread_id, provisional)) = titled.first() {
                        let request = TitleRequest {
                            context: text.clone(),
                            previous_title: None,
                            images: titles::image_names(attachments),
                            model: None,
                            instructions: String::new(),
                        };
                        state.name_thread(thread_id, provisional, request);
                    }
                }
                Event::TurnStarted {
                    thread_id, turn_id, ..
                } => {
                    let (prompt, attachments) = message.take().unwrap_or_default();
                    start_turn(
                        &self.state,
                        &mut state,
                        thread_id,
                        turn_id,
                        prompt,
                        attachments,
                    );
                }
                Event::ThreadCreated { thread } if thread.worktree_path.is_none() => {
                    refresh_branches(state.this.clone(), vec![thread.id.clone()]);
                }
                Event::ProjectCreated { project } => {
                    let watched = (project.id.clone(), project.workspace_root.clone());
                    watch_projects(state.this.clone(), vec![watched]);
                }
                Event::ProjectRemoved { project_id } => {
                    if let Some(watcher) = &mut state.watcher {
                        watcher.unwatch(project_id);
                    }
                    state.repositories.remove(project_id);
                }
                Event::ThreadDeleted { thread_id } => {
                    state.pull_requests.forget(thread_id);
                    state.pull_request_cache.forget(thread_id);
                    if let Some(folder) = state.chat_folder(thread_id) {
                        let _ = std::fs::remove_dir_all(folder);
                    }
                }
                _ => {}
            }
        }
        if let Some(thread_id) = interrupt
            && let Some(turn) = state.turns.get(&thread_id)
        {
            turn.running.interrupt();
        }
        // A deleted thread (or a removed project's threads) must not keep an agent running.
        let orphaned: Vec<_> = state
            .turns
            .keys()
            .filter(|thread_id| state.model.thread(thread_id).is_none())
            .cloned()
            .collect();
        for thread_id in orphaned {
            if let Some(turn) = state.turns.remove(&thread_id) {
                turn.running.interrupt();
            }
        }
        let State {
            model,
            context_usages,
            ..
        } = &mut *state;
        context_usages.retain(|thread_id, _| model.thread(thread_id).is_some());
        self.terminals
            .retain_threads(|thread_id| model.thread(thread_id).is_some());
        for (repository, path) in freed_worktrees {
            if let Err(error) = git::remove_worktree(&repository, &path) {
                eprintln!("hoshi: kept worktree {path}: {error}");
            }
        }
        for image in &freed_images {
            state.attachments.remove(image);
        }
        Ok(())
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
        };
        self.terminals.open(terminal_id, launch, size, sink)
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
    fn restore_released_worktree(&self, thread_id: &str) -> Result<(), CommandError> {
        let Some(thread) = self.model.thread(thread_id) else {
            return Ok(());
        };
        let (true, Some(path)) = (thread.worktree_released, &thread.worktree_path) else {
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
        let project =
            self.model
                .project(project_id)
                .ok_or_else(|| CommandError::ProjectNotFound {
                    project_id: project_id.to_owned(),
                })?;
        let root = &project.workspace_root;
        if !git::is_repository(root) {
            return Err(CommandError::NotAGitRepository { path: root.clone() });
        }
        let short_id: String = thread_id.chars().take(8).collect();
        let branch = existing.map_or_else(|| format!("hoshi/{short_id}"), str::to_owned);
        let folder = format!(
            "{}-{}",
            slug(&project.title),
            project_id.chars().take(8).collect::<String>()
        );
        let path = self.worktrees_dir.join(folder).join(&short_id);
        let failed = |detail: String| CommandError::WorktreeFailed { detail };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| failed(error.to_string()))?;
        }
        match existing {
            Some(_) => git::add_branch_worktree(root, &path, &branch),
            None => git::add_worktree(root, &path, &branch),
        }
        .map_err(failed)?;
        Ok(ThreadWorkspace::Worktree {
            path: path.to_string_lossy().into_owned(),
            branch,
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
        Ok(())
    }

    fn receive(&mut self, thread_id: &str, turn_id: &str, output: ProviderOutput) {
        let (item_id, kind, delta) = match output {
            ProviderOutput::TextDelta { item_id, delta } => (item_id, StreamKind::Text, delta),
            ProviderOutput::ReasoningDelta { item_id, delta } => {
                (item_id, StreamKind::Reasoning, delta)
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
            _ => {}
        }
        if let ProviderOutput::ToolCallStarted { input, .. } = &output
            && let Some(turn) = self.turns.get_mut(thread_id)
            && turn.id == turn_id
            && let Some(workspace) = &mut turn.workspace
        {
            workspace.record_tool_input(input);
        }
        let completed = matches!(output, ProviderOutput::TurnCompleted { .. });
        let now = now();
        let events = record_output(&self.model, thread_id, turn_id, output, &now);
        if let Err(error) = self.commit(&events, &now) {
            eprintln!("hoshi: failed to record provider output: {error}");
        }
        if completed {
            let workspace = match self.turns.get(thread_id) {
                Some(turn) if turn.id == turn_id => {
                    self.turns.remove(thread_id).and_then(|turn| turn.workspace)
                }
                _ => None,
            };
            if self.titles_to_refine.remove(thread_id) {
                self.rename_from_conversation(thread_id, false);
            }
            // The agent may have created a worktree or switched branches.
            let check = workspace.map(|workspace| self.worktree_check(thread_id, workspace));
            refresh_after_turn(self.this.clone(), thread_id.to_owned(), check);
        }
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
        let usage = usage.merged_into(self.context_usages.get(thread_id).copied());
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
        let check = self.worktree_check(thread_id, workspace);
        self.session_moves
            .insert(thread_id.to_owned(), path.clone());
        follow_session_move(self.this.clone(), thread_id.to_owned(), path, check);
    }

    fn worktree_check(&self, thread_id: &str, workspace: TurnWorkspace) -> WorktreeCheck {
        let project_id = self
            .model
            .thread(thread_id)
            .and_then(|thread| thread.project_id.as_deref());
        let claimed = self
            .model
            .threads()
            .filter(|thread| thread.id != thread_id)
            .filter_map(|thread| thread.worktree_path.clone())
            .collect();
        let alone = !self.turns.keys().any(|other| {
            other != thread_id
                && self
                    .model
                    .thread(other)
                    .and_then(|thread| thread.project_id.as_deref())
                    == project_id
        });
        WorktreeCheck {
            workspace,
            claimed,
            alone,
        }
    }
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

/// Moves a thread to the worktree its turn created, if git shows one.
fn attach_created_worktree(state: &Weak<Mutex<State>>, thread_id: &str, check: &WorktreeCheck) {
    if check.workspace.moved {
        return;
    }
    let Some(after) = git::worktrees(&check.workspace.repository) else {
        return;
    };
    let claimed: Vec<String> = check
        .claimed
        .iter()
        .map(|path| git::normalize(path))
        .collect();
    let created = worktree::created_by_turn(&check.workspace, &after, &claimed, check.alone);
    let (Some(path), Some(shared)) = (created, state.upgrade()) else {
        return;
    };
    let branch = git::current_branch(&path);
    let mut state = lock(&shared);
    let events = record_worktree(&state.model, thread_id, path, branch, false);
    if let Err(error) = state.commit(&events, &now()) {
        eprintln!("hoshi: failed to record a worktree: {error}");
    }
}

/// Moves the thread where its agent's session went, once git shows it is the project root
/// or one of its worktrees; anywhere else stays the agent's own business. A worktree that
/// existed before the turn, or that another thread owns, is adopted: hoshi never removes it.
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
        let adopted = check.workspace.worktrees_before.contains(&path)
            || check
                .claimed
                .iter()
                .any(|claimed| git::normalize(claimed) == path);
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
            record_worktree(&state.model, &thread_id, path, branch, adopted)
        };
        if events.is_empty() {
            return;
        }
        state.pull_requests.check_soon(&thread_id);
        if let Err(error) = state.commit(&events, &now()) {
            eprintln!("hoshi: failed to follow the agent's session: {error}");
        }
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
                    state.worktree_check(thread_id, workspace),
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
            let jobs = lock(&shared).due_pull_request_jobs();
            for job in jobs {
                let result = job
                    .forge
                    .pull_request_for_branch(&job.repository, &job.branch);
                lock(&shared).record_pull_request(&job, result);
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
        let working = thread.status == ThreadStatus::Working;
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
    shared: &Arc<Mutex<State>>,
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
    let (Some(cwd), Some(thread)) = (state.cwd(thread_id), state.model.thread(thread_id)) else {
        return;
    };
    let project = thread
        .project_id
        .as_deref()
        .and_then(|project_id| state.model.project(project_id));
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
    let attachments = attachments
        .into_iter()
        .map(|attachment| TurnAttachment {
            path: match &attachment {
                MessageAttachment::Image { id, mime_type, .. } => {
                    state.attachments.path(id, mime_type)
                }
                _ => None,
            },
            attachment,
        })
        .collect();
    let request = TurnRequest {
        cwd: cwd.to_owned(),
        prompt,
        attachments,
        resume_session_id: thread.session_id.clone(),
        model: thread.model.clone(),
        effort: thread.effort,
        fast_mode: thread.fast_mode,
        ultracode: thread.ultracode,
        access_mode: thread.access_mode,
        plan_mode: thread.plan_mode,
    };
    let weak: Weak<Mutex<State>> = Arc::downgrade(shared);
    let (sink_thread, sink_turn) = (thread_id.to_owned(), turn_id.to_owned());
    let sink = Box::new(move |output| {
        if let Some(shared) = weak.upgrade() {
            lock(&shared).receive(&sink_thread, &sink_turn, output);
        }
    });
    match adapter.start_turn(request, sink) {
        Ok(running) => {
            let turn = Turn {
                id: turn_id.to_owned(),
                running,
                workspace,
            };
            state.turns.insert(thread_id.to_owned(), turn);
        }
        Err(failure) => state.receive(thread_id, turn_id, turn_failed(failure)),
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
    let installation = adapter.installation()?;
    let mut state = lock(state);
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
        ForgeError::Rejected(detail) => CommandError::ForgeRejected { detail },
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
    use hoshi_core::{AccessMode, Effort, ThreadStatus};
    use hoshi_providers::OutputSink;
    use std::sync::mpsc::{self, Receiver, Sender};

    #[derive(Debug, Clone, PartialEq)]
    struct Request {
        cwd: String,
        prompt: String,
        resume: Option<String>,
        model: Option<String>,
        attachments: Vec<TurnAttachment>,
    }

    /// Records each request and replays a script; a turn whose script is
    /// empty waits for an interrupt.
    struct FakeAdapter {
        requests: Mutex<Vec<Request>>,
        script: Vec<ProviderOutput>,
        /// What a title request answers; `None` declines.
        title: Option<String>,
    }

    struct FakeTurn(Sender<()>);

    impl RunningTurn for FakeTurn {
        fn interrupt(&self) {
            let _ = self.0.send(());
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
                model: request.model,
                attachments: request.attachments,
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
            thread::spawn(move || {
                if script.is_empty() {
                    let _ = stopped.recv();
                    let outcome = TurnOutcome::Interrupted;
                    sink(ProviderOutput::TurnCompleted { outcome });
                }
                script.into_iter().for_each(&mut sink);
            });
            Ok(Box::new(FakeTurn(stop)))
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

    /// Reports 1.0.0 against a 1.1.0 release until its updater ran.
    struct OutdatedAdapter {
        updated: Mutex<bool>,
    }

    impl ProviderAdapter for OutdatedAdapter {
        fn models(&self) -> hoshi_core::ProviderModels {
            hoshi_core::ProviderModels::default()
        }

        fn installation(&self) -> Option<Installation> {
            let updated = *self.updated.lock().unwrap();
            let (program, args) = if cfg!(windows) {
                ("cmd", vec!["/C".to_owned(), "echo updated".to_owned()])
            } else {
                ("sh", vec!["-c".to_owned(), "echo updated".to_owned()])
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

        fn forget_version(&self) {
            *self.updated.lock().unwrap() = true;
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
        let engine = Engine::from_store(
            Store::open_in_memory().unwrap(),
            &scratch_dir("provider-update"),
            notify,
        )
        .unwrap();
        engine.set_adapter(Some(Arc::new(OutdatedAdapter {
            updated: Mutex::new(false),
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
            model: Some("haiku".into()),
            attachments: Vec::new(),
        };
        assert_eq!(
            *harness.adapter.requests.lock().unwrap(),
            [request("first", None), request("second", Some("s1"))]
        );
        assert_eq!(harness.thread("t").status, ThreadStatus::Idle);
        assert_eq!(harness.engine.thread_items("t").unwrap().len(), 4);
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
        let cwd = harness.adapter.requests.lock().unwrap()[0].cwd.clone();
        assert_eq!(PathBuf::from(cwd), folder);
        assert!(folder.is_dir());
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

    fn titled_harness(name: &str) -> Harness {
        let harness = Harness::in_memory(name, completed_turn());
        harness.engine.set_adapter(Some(Arc::new(FakeAdapter {
            requests: Mutex::new(Vec::new()),
            script: completed_turn(),
            title: Some("Empty TSX Snippet".into()),
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
        assert!(!Path::new(&worktree).exists());
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

    #[test]
    fn a_worktree_the_agent_creates_becomes_the_thread_workspace() {
        let repo = git::tests::temp_repository("engine-agent-worktree");
        let worktree = repo.with_extension("agent-wt");
        let _ = std::fs::remove_dir_all(&worktree);
        let store = Store::open_in_memory().unwrap();
        let harness = Harness::new("agent-worktree", repo, store, Vec::new());
        harness.send("t", "m1", "make a worktree");

        git::add_worktree(&harness.root, &worktree, "feat/csv").unwrap();
        harness
            .engine
            .dispatch(Command::TurnInterrupt {
                thread_id: "t".into(),
            })
            .unwrap();

        let attached = harness.wait_for(|event| match event {
            Event::ThreadWorktreeAttached {
                worktree_path,
                branch,
                ..
            } => Some((worktree_path, branch)),
            _ => None,
        });
        let expected = git::normalize(&worktree.to_string_lossy());
        assert_eq!(attached, (expected.clone(), Some("feat/csv".into())));
        harness.send("t", "m2", "continue");
        assert_eq!(harness.adapter.requests.lock().unwrap()[1].cwd, expected);
        let _ = git::remove_worktree(&harness.root, &expected);
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
        let store = Store::open_in_memory().unwrap();
        let harness = Harness::new("live-worktree", repo, store, Vec::new());
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
}
