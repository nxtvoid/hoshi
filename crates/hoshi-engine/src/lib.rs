//! Runtime loop: takes commands, lets the core decide events, persists them
//! through the store, notifies the UI, then runs the resulting effects
//! (provider turns, git). Provider output comes back through the same path.

mod git;

use chrono::{SecondsFormat, Utc};
use hoshi_core::{
    Command, CommandError, Event, EventEnvelope, ItemContent, ProviderOutput, ReadModel, Snapshot,
    StreamDelta, ThreadItem, ThreadWorkspace, TurnFailure, TurnOutcome, decide, record_branch,
    record_output, turn_failed,
};
use hoshi_providers::{ProviderAdapter, RunningTurn, TurnRequest};
use hoshi_store::{Store, StoreError};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::thread;

pub enum Notification {
    Event(EventEnvelope),
    Stream(StreamDelta),
}

pub type Notify = Arc<dyn Fn(Notification) + Send + Sync>;

/// Shared by every thread that talks to the engine: Tauri commands and the
/// reader threads of running turns.
pub struct Engine {
    state: Arc<Mutex<State>>,
}

struct State {
    store: Store,
    model: ReadModel,
    sequence: u64,
    /// Running turns by thread id, with the turn they belong to.
    turns: HashMap<String, (String, Box<dyn RunningTurn>)>,
    /// `None` when the provider is disabled.
    adapter: Option<Arc<dyn ProviderAdapter>>,
    notify: Notify,
    /// New worktrees go under here, one folder per project.
    worktrees_dir: PathBuf,
    /// Lets background work (branch refreshes) reach the state without keeping it alive.
    this: Weak<Mutex<State>>,
}

impl Engine {
    pub fn open(
        database: &Path,
        worktrees_dir: PathBuf,
        notify: Notify,
    ) -> Result<Self, StoreError> {
        Self::from_store(Store::open(database)?, worktrees_dir, notify)
    }

    /// Turns that were running when the app last closed are recorded as
    /// interrupted, and every thread's branch is read again.
    pub fn from_store(
        store: Store,
        worktrees_dir: PathBuf,
        notify: Notify,
    ) -> Result<Self, StoreError> {
        let snapshot = store.snapshot()?;
        let mut state = State {
            model: ReadModel::from_snapshot(&snapshot),
            sequence: snapshot.sequence,
            store,
            turns: HashMap::new(),
            adapter: None,
            notify,
            worktrees_dir,
            this: Weak::new(),
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
        Ok(Self { state: shared })
    }

    pub fn set_adapter(&self, adapter: Option<Arc<dyn ProviderAdapter>>) {
        lock(&self.state).adapter = adapter;
    }

    pub fn snapshot(&self) -> Snapshot {
        let state = lock(&self.state);
        Snapshot {
            sequence: state.sequence,
            projects: state.model.projects().cloned().collect(),
            threads: state.model.threads().cloned().collect(),
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
        let now = now();
        let events = decide(&state.model, command, &now)?;
        state.commit(&events, &now)?;

        let mut prompt = None;
        for event in &events {
            match event {
                Event::ItemAdded {
                    item:
                        ThreadItem {
                            content: ItemContent::UserMessage { text },
                            ..
                        },
                } => prompt = Some(text.clone()),
                Event::TurnStarted {
                    thread_id, turn_id, ..
                } => {
                    let prompt = prompt.take().unwrap_or_default();
                    start_turn(&self.state, &mut state, thread_id, turn_id, prompt);
                }
                Event::ThreadCreated { thread } if thread.worktree_path.is_none() => {
                    refresh_branches(state.this.clone(), vec![thread.id.clone()]);
                }
                _ => {}
            }
        }
        if let Some(thread_id) = interrupt
            && let Some((_, turn)) = state.turns.get(&thread_id)
        {
            turn.interrupt();
        }
        // A deleted thread (or a removed project's threads) must not keep an agent running.
        let orphaned: Vec<_> = state
            .turns
            .keys()
            .filter(|thread_id| state.model.thread(thread_id).is_none())
            .cloned()
            .collect();
        for thread_id in orphaned {
            if let Some((_, turn)) = state.turns.remove(&thread_id) {
                turn.interrupt();
            }
        }
        for (repository, path) in freed_worktrees {
            if let Err(error) = git::remove_worktree(&repository, &path) {
                eprintln!("hoshi: kept worktree {path}: {error}");
            }
        }
        Ok(())
    }
}

impl State {
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
            Command::ThreadCreate {
                thread_id,
                project_id,
                title,
                workspace: ThreadWorkspace::NewWorktree,
                model,
                effort,
            } => {
                let workspace = self.create_worktree(&thread_id, &project_id)?;
                Ok(Command::ThreadCreate {
                    thread_id,
                    project_id,
                    title,
                    workspace,
                    model,
                    effort,
                })
            }
            other => Ok(other),
        }
    }

    fn create_worktree(
        &self,
        thread_id: &str,
        project_id: &str,
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
        let branch = format!("hoshi/{short_id}");
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
        git::add_worktree(root, &path, &branch).map_err(failed)?;
        Ok(ThreadWorkspace::Worktree {
            path: path.to_string_lossy().into_owned(),
            branch,
        })
    }

    /// Worktrees (with their repository) that a deletion leaves without a thread.
    fn worktrees_freed_by(&self, command: &Command) -> Vec<(String, String)> {
        let threads: Vec<_> = match command {
            Command::ThreadDelete { thread_id } => {
                self.model.thread(thread_id).into_iter().collect()
            }
            Command::ProjectRemove { project_id } => self
                .model
                .threads()
                .filter(|thread| &thread.project_id == project_id)
                .collect(),
            _ => Vec::new(),
        };
        threads
            .into_iter()
            .filter_map(|thread| {
                let path = thread.worktree_path.clone()?;
                let project = self.model.project(&thread.project_id)?;
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
        if let ProviderOutput::TextDelta { item_id, delta } = output {
            let active = self
                .model
                .thread(thread_id)
                .is_some_and(|thread| thread.active_turn_id.as_deref() == Some(turn_id));
            if active {
                (self.notify)(Notification::Stream(StreamDelta {
                    thread_id: thread_id.to_owned(),
                    item_id,
                    delta,
                }));
            }
            return;
        }
        let completed = matches!(output, ProviderOutput::TurnCompleted { .. });
        let now = now();
        let events = record_output(&self.model, thread_id, turn_id, output, &now);
        if let Err(error) = self.commit(&events, &now) {
            eprintln!("hoshi: failed to record provider output: {error}");
        }
        if completed {
            if self
                .turns
                .get(thread_id)
                .is_some_and(|(turn, _)| turn == turn_id)
            {
                self.turns.remove(thread_id);
            }
            // The agent may have switched branches.
            refresh_branches(self.this.clone(), vec![thread_id.to_owned()]);
        }
    }
}

/// Reads each thread's branch with git, off the lock, and records changes.
fn refresh_branches(state: Weak<Mutex<State>>, thread_ids: Vec<String>) {
    if thread_ids.is_empty() {
        return;
    }
    thread::spawn(move || {
        for thread_id in thread_ids {
            let Some(shared) = state.upgrade() else {
                return;
            };
            let cwd = lock(&shared)
                .model
                .thread_cwd(&thread_id)
                .map(str::to_owned);
            let Some(cwd) = cwd else { continue };
            let branch = git::current_branch(&cwd);
            let mut state = lock(&shared);
            let events = record_branch(&state.model, &thread_id, branch);
            if let Err(error) = state.commit(&events, &now()) {
                eprintln!("hoshi: failed to record a branch: {error}");
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

fn start_turn(
    shared: &Arc<Mutex<State>>,
    state: &mut State,
    thread_id: &str,
    turn_id: &str,
    prompt: String,
) {
    let Some(adapter) = state.adapter.clone() else {
        state.receive(
            thread_id,
            turn_id,
            turn_failed(TurnFailure::ProviderDisabled),
        );
        return;
    };
    let (Some(cwd), Some(thread)) = (
        state.model.thread_cwd(thread_id),
        state.model.thread(thread_id),
    ) else {
        return;
    };
    let request = TurnRequest {
        cwd: cwd.to_owned(),
        prompt,
        resume_session_id: thread.session_id.clone(),
        model: thread.model.clone(),
        effort: thread.effort,
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
            state
                .turns
                .insert(thread_id.to_owned(), (turn_id.to_owned(), running));
        }
        Err(failure) => state.receive(thread_id, turn_id, turn_failed(failure)),
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn internal(error: StoreError) -> CommandError {
    CommandError::Internal {
        detail: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoshi_core::{Effort, ThreadStatus};
    use hoshi_providers::OutputSink;
    use std::sync::mpsc::{self, Receiver, Sender};

    #[derive(Debug, Clone, PartialEq)]
    struct Request {
        cwd: String,
        prompt: String,
        resume: Option<String>,
        model: Option<String>,
    }

    /// Records each request and replays a script; a turn whose script is
    /// empty waits for an interrupt.
    struct FakeAdapter {
        requests: Mutex<Vec<Request>>,
        script: Vec<ProviderOutput>,
    }

    struct FakeTurn(Sender<()>);

    impl RunningTurn for FakeTurn {
        fn interrupt(&self) {
            let _ = self.0.send(());
        }
    }

    impl ProviderAdapter for FakeAdapter {
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
            let engine =
                Engine::from_store(store, scratch_dir(&format!("{name}-wt")), notify).unwrap();
            let adapter = Arc::new(FakeAdapter {
                requests: Mutex::new(Vec::new()),
                script,
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
            self.engine.dispatch(Command::ThreadCreate {
                thread_id: id.into(),
                project_id: "p".into(),
                title: String::new(),
                workspace,
                model: Some("haiku".into()),
                effort: Some(Effort::Low),
            })
        }

        fn send(&self, thread_id: &str, message_id: &str, text: &str) {
            self.engine
                .dispatch(Command::MessageSend {
                    thread_id: thread_id.into(),
                    message_id: message_id.into(),
                    text: text.into(),
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
        };
        assert_eq!(
            *harness.adapter.requests.lock().unwrap(),
            [request("first", None), request("second", Some("s1"))]
        );
        assert_eq!(harness.thread("t").status, ThreadStatus::Idle);
        assert_eq!(harness.engine.thread_items("t").unwrap().len(), 4);
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
            scratch_dir("crash-reopened"),
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
