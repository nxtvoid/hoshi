//! Runtime loop: takes commands, lets the core decide events, persists them
//! through the store, notifies the UI, then runs the resulting effects
//! (provider turns). Provider output comes back through the same path.

use chrono::{SecondsFormat, Utc};
use hoshi_core::{
    Command, CommandError, Event, EventEnvelope, ItemContent, ProviderOutput, ReadModel, Snapshot,
    StreamDelta, ThreadItem, TurnFailure, TurnOutcome, decide, record_output, turn_failed,
};
use hoshi_providers::{ProviderAdapter, RunningTurn, TurnRequest};
use hoshi_store::{Store, StoreError};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

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
}

impl Engine {
    pub fn open(database: &Path, notify: Notify) -> Result<Self, StoreError> {
        Self::from_store(Store::open(database)?, notify)
    }

    /// Turns that were running when the app last closed are recorded as interrupted.
    pub fn from_store(store: Store, notify: Notify) -> Result<Self, StoreError> {
        let snapshot = store.snapshot()?;
        let mut state = State {
            model: ReadModel::from_snapshot(&snapshot),
            sequence: snapshot.sequence,
            store,
            turns: HashMap::new(),
            adapter: None,
            notify,
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
        Ok(Self {
            state: Arc::new(Mutex::new(state)),
        })
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
        let command = normalize(command)?;
        let interrupt = match &command {
            Command::TurnInterrupt { thread_id } => Some(thread_id.clone()),
            _ => None,
        };
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
        Ok(())
    }
}

impl State {
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
        if completed
            && self
                .turns
                .get(thread_id)
                .is_some_and(|(turn, _)| turn == turn_id)
        {
            self.turns.remove(thread_id);
        }
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

/// Resolves a new project's folder to its canonical path, so the same folder
/// typed two ways is still one project.
fn normalize(command: Command) -> Result<Command, CommandError> {
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
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoshi_core::{ThreadStatus, TurnOutcome};
    use hoshi_providers::OutputSink;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread;

    /// Records each request and replays a script; a turn whose script is
    /// empty waits for an interrupt.
    struct FakeAdapter {
        requests: Mutex<Vec<(String, String, Option<String>)>>,
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
            requests.push((request.cwd, request.prompt, request.resume_session_id));
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
        events: Receiver<Event>,
        root: String,
    }

    impl Harness {
        fn new(script: Vec<ProviderOutput>) -> (Self, Arc<FakeAdapter>) {
            let (sender, events) = mpsc::channel();
            let notify: Notify = Arc::new(move |notification| {
                if let Notification::Event(envelope) = notification {
                    let _ = sender.send(envelope.event);
                }
            });
            let engine = Engine::from_store(Store::open_in_memory().unwrap(), notify).unwrap();
            let adapter = Arc::new(FakeAdapter {
                requests: Mutex::new(Vec::new()),
                script,
            });
            engine.set_adapter(Some(adapter.clone()));
            let root = std::env::temp_dir();
            engine
                .dispatch(Command::ProjectCreate {
                    project_id: "p".into(),
                    title: "p".into(),
                    workspace_root: root.to_string_lossy().into_owned(),
                })
                .unwrap();
            engine
                .dispatch(Command::ThreadCreate {
                    thread_id: "t".into(),
                    project_id: "p".into(),
                    title: String::new(),
                })
                .unwrap();
            let root = dunce::canonicalize(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            (
                Self {
                    engine,
                    events,
                    root,
                },
                adapter,
            )
        }

        fn send(&self, message_id: &str, text: &str) {
            self.engine
                .dispatch(Command::MessageSend {
                    thread_id: "t".into(),
                    message_id: message_id.into(),
                    text: text.into(),
                })
                .unwrap();
        }

        fn wait_for_turn_end(&self) -> TurnOutcome {
            self.events
                .iter()
                .find_map(|event| match event {
                    Event::TurnCompleted { outcome, .. } => Some(outcome),
                    _ => None,
                })
                .unwrap()
        }

        fn thread_status(&self) -> ThreadStatus {
            self.engine.snapshot().threads[0].status
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
        let (harness, adapter) = Harness::new(completed_turn());

        harness.send("m1", "first");
        assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Completed);
        harness.send("m2", "second");
        harness.wait_for_turn_end();

        let requests = adapter.requests.lock().unwrap().clone();
        assert_eq!(
            requests,
            [
                (harness.root.clone(), "first".into(), None),
                (harness.root.clone(), "second".into(), Some("s1".into())),
            ]
        );
        assert_eq!(harness.thread_status(), ThreadStatus::Idle);
        assert_eq!(harness.engine.thread_items("t").unwrap().len(), 4);
    }

    #[test]
    fn interrupting_stops_the_turn() {
        let (harness, _) = Harness::new(Vec::new());
        harness.send("m1", "long task");

        harness
            .engine
            .dispatch(Command::TurnInterrupt {
                thread_id: "t".into(),
            })
            .unwrap();

        assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Interrupted);
        assert_eq!(harness.thread_status(), ThreadStatus::Idle);
    }

    #[test]
    fn a_disabled_provider_fails_the_turn() {
        let (harness, _) = Harness::new(completed_turn());
        harness.engine.set_adapter(None);

        harness.send("m1", "hi");

        assert_eq!(
            harness.wait_for_turn_end(),
            TurnOutcome::Failed {
                reason: TurnFailure::ProviderDisabled
            }
        );
        assert_eq!(harness.thread_status(), ThreadStatus::Error);
    }

    #[test]
    fn a_turn_left_running_by_a_crash_is_closed_on_startup() {
        let (harness, _) = Harness::new(Vec::new());
        harness.send("m1", "long task");
        // Simulates a crash: the process is gone, only the stored state remains.
        let state = Arc::into_inner(harness.engine.state).unwrap();
        let store = state.into_inner().unwrap().store;

        let engine = Engine::from_store(store, Arc::new(|_| {})).unwrap();

        let thread = &engine.snapshot().threads[0];
        assert_eq!(thread.status, ThreadStatus::Idle);
        assert_eq!(thread.active_turn_id, None);
    }

    #[test]
    fn rejects_a_folder_that_does_not_exist() {
        let (harness, _) = Harness::new(Vec::new());
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
}
