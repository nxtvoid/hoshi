use super::*;
use crate::{Notification, Notify};
use hoshi_core::{
    AccessMode, BackgroundTask, Event, ProviderModels, ProviderOutput, RequestResponse,
    ThreadWorkspace,
};
use hoshi_providers::{OutputSink, RunningTurn, TurnAttachment, TurnRequest};
use hoshi_store::Store;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

const CLAUDE: &str = hoshi_core::DEFAULT_PROVIDER;
const CODEX: &str = "codex";

/// Answers every turn at once: a session, `Done.` as `<provider>-<turn>:answer`, and a
/// cursor after it.
struct Scripted {
    name: &'static str,
    forks: bool,
    injects: bool,
    window: Option<u64>,
    /// Leaves a command running in the background, in a process kept after the turn.
    background: bool,
    requests: Mutex<Vec<TurnRequest>>,
}

impl Scripted {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            forks: true,
            injects: name == CODEX,
            window: None,
            background: false,
            requests: Mutex::default(),
        }
    }

    fn request(&self, turn: usize) -> TurnRequest {
        self.requests.lock().unwrap()[turn].clone()
    }
}

struct Done {
    lingers: bool,
}

impl RunningTurn for Done {
    fn interrupt(&self) {}

    fn respond(&self, _request_id: &str, _response: &RequestResponse) {}

    fn lingers(&self) -> bool {
        self.lingers
    }
}

impl ProviderAdapter for Scripted {
    fn provider(&self) -> &'static str {
        self.name
    }

    fn models(&self) -> ProviderModels {
        ProviderModels::default()
    }

    fn forks_sessions(&self) -> bool {
        self.forks
    }

    fn injects_context(&self) -> bool {
        self.injects
    }

    fn context_window(&self, _model: Option<&str>) -> Option<u64> {
        self.window
    }

    fn start_turn(
        &self,
        request: TurnRequest,
        mut sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
        let mut requests = self.requests.lock().unwrap();
        let turn = requests.len() + 1;
        let name = self.name;
        let session = match (&request.resume_session_id, request.fork_session) {
            (Some(session), false) => session.clone(),
            _ => format!("{name}-{turn}"),
        };
        let injects = self.injects && request.handoff.is_some();
        let background = self.background;
        requests.push(request);
        std::thread::spawn(move || {
            sink(ProviderOutput::SessionStarted {
                session_id: session,
            });
            if injects {
                sink(ProviderOutput::ContextHandoffDelivered { injected: true });
            }
            sink(ProviderOutput::AssistantMessage {
                item_id: format!("{name}-{turn}:answer"),
                text: "Done.".into(),
            });
            sink(ProviderOutput::ConversationCursor {
                cursor: format!("{name}-{turn}:cursor"),
            });
            if background {
                sink(ProviderOutput::BackgroundTasks {
                    tasks: vec![BackgroundTask {
                        task_id: "server".into(),
                        tool_call_id: None,
                        description: "dev server".into(),
                        started_at: 0,
                    }],
                    waiting: false,
                });
            }
            sink(ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Completed,
            });
        });
        Ok(Box::new(Done {
            lingers: background,
        }))
    }
}

struct Setup {
    engine: Engine,
    events: mpsc::Receiver<Event>,
    claude: Arc<Scripted>,
    codex: Arc<Scripted>,
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hoshi-handoffs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Project `p` on `root`, with thread `t` on Claude Code in `workspace`.
fn setup(name: &str, root: &Path, workspace: ThreadWorkspace, claude: Scripted) -> Setup {
    let (sender, events) = mpsc::channel();
    let notify: Notify = Arc::new(move |notification| {
        if let Notification::Event(envelope) = notification {
            let _ = sender.send(envelope.event);
        }
    });
    let data = scratch(&format!("{name}-data"));
    let engine = Engine::from_store(Store::open_in_memory().unwrap(), &data, notify).unwrap();
    let (claude, codex) = (Arc::new(claude), Arc::new(Scripted::new(CODEX)));
    engine.set_adapters(vec![claude.clone(), codex.clone()]);
    let root = dunce::canonicalize(root).unwrap();
    engine
        .dispatch(Command::ProjectCreate {
            project_id: "p".into(),
            title: "Repo".into(),
            workspace_root: root.to_string_lossy().into_owned(),
        })
        .unwrap();
    engine
        .dispatch(Command::ThreadCreate {
            thread_id: "t".into(),
            project_id: Some("p".into()),
            title: "Login".into(),
            workspace,
            provider: CLAUDE.into(),
            model: None,
            effort: None,
            fast_mode: false,
            ultracode: false,
            access_mode: AccessMode::AcceptEdits,
            plan_mode: false,
        })
        .unwrap();
    Setup {
        engine,
        events,
        claude,
        codex,
    }
}

impl Setup {
    fn send(&self, thread_id: &str, message_id: &str, text: &str) {
        self.engine
            .dispatch(Command::MessageSend {
                thread_id: thread_id.into(),
                message_id: message_id.into(),
                text: text.into(),
                attachments: Vec::new(),
            })
            .unwrap();
        self.events
                .iter()
                .find(|event| {
                    matches!(event, Event::TurnCompleted { thread_id: done, .. } if done == thread_id)
                })
                .unwrap();
    }

    fn pick(&self, thread_id: &str, provider: &str) -> Result<(), CommandError> {
        self.engine.dispatch(Command::ThreadModelSet {
            thread_id: thread_id.into(),
            provider: provider.into(),
            model: None,
            effort: None,
            fast_mode: false,
            ultracode: false,
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

    fn handoff_item(&self, thread_id: &str, turn_id: &str) -> ItemContent {
        self.engine
            .thread_item(thread_id, &format!("{turn_id}:handoff"))
            .unwrap()
            .unwrap()
            .content
    }
}

fn local(name: &str, claude: Scripted) -> Setup {
    setup(name, &scratch(name), ThreadWorkspace::Local, claude)
}

fn delivery(content: &ItemContent) -> HandoffDelivery {
    match content {
        ItemContent::ContextHandoff { delivery, .. } => *delivery,
        other => panic!("not a handoff: {other:?}"),
    }
}

fn handoff_text(request: &TurnRequest) -> String {
    request.handoff.as_ref().unwrap().render()
}

#[test]
fn another_provider_gets_the_conversation_and_a_switch_back_only_what_it_missed() {
    let setup = local("switch", Scripted::new(CLAUDE));
    setup.send("t", "m1", "Remember APPLE.");

    setup.pick("t", CODEX).unwrap();
    assert_eq!(setup.thread("t").provider, CLAUDE);
    setup.send("t", "m2", "Go on.");

    let to_codex = setup.codex.request(0);
    assert_eq!(to_codex.resume_session_id, None);
    assert_eq!(to_codex.prompt, "Go on.");
    let text = handoff_text(&to_codex);
    assert!(
        text.starts_with("Context handoff (full_thread_summary):"),
        "{text}"
    );
    assert!(text.contains("item=m1;") && text.contains("Remember APPLE."));
    assert!(text.contains("item=claudeCode-1:answer;"));
    assert_eq!(
        delivery(&setup.handoff_item("t", "m2")),
        HandoffDelivery::Injected
    );
    assert!(setup.thread("t").pending_handoffs.is_empty());

    setup.pick("t", CLAUDE).unwrap();
    setup.send("t", "m3", "And now?");

    let back = setup.claude.request(1);
    assert_eq!(back.resume_session_id.as_deref(), Some("claudeCode-1"));
    let text = handoff_text(&back);
    assert!(
        text.starts_with("Context handoff (delta_since_target_last_seen):"),
        "{text}"
    );
    assert!(text.contains("item=m2;") && text.contains("item=codex-1:answer;"));
    assert!(!text.contains("item=m1;"));
    assert_eq!(
        delivery(&setup.handoff_item("t", "m3")),
        HandoffDelivery::Inline
    );
    let thread = setup.thread("t");
    assert_eq!(thread.provider, CLAUDE);
    assert_eq!(thread.provider_sessions[0].session_id, "codex-1");
}

#[test]
fn a_handoff_that_leaves_no_room_fails_the_turn_and_waits_for_the_next() {
    let setup = local("too-large", Scripted::new(CLAUDE));
    setup.send("t", "m1", "Remember APPLE.");
    setup.pick("t", CODEX).unwrap();
    setup.engine.set_adapters(vec![
        setup.claude.clone(),
        Arc::new(Scripted {
            window: Some(16_100),
            ..Scripted::new(CODEX)
        }),
    ]);

    setup.send("t", "m2", "Go on.");

    assert_eq!(
        delivery(&setup.handoff_item("t", "m2")),
        HandoffDelivery::Failed
    );
    let failed = setup.engine.thread_item("t", "m2:failed").unwrap().unwrap();
    assert_eq!(
        failed.content,
        ItemContent::TurnFailed {
            reason: TurnFailure::ContextHandoffTooLarge
        }
    );
    assert_eq!(setup.thread("t").pending_handoffs.len(), 1);
}

/// An agent whose turns report only what the test has them report.
struct Held {
    name: &'static str,
    restarts: bool,
    interrupted: Arc<AtomicBool>,
    sinks: Mutex<Vec<OutputSink>>,
}

struct HeldTurn(Arc<AtomicBool>);

impl RunningTurn for HeldTurn {
    fn interrupt(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    fn respond(&self, _request_id: &str, _response: &RequestResponse) {}

    fn steer(
        &self,
        _message_id: &str,
        _text: &str,
        _attachments: &[TurnAttachment],
        _now: bool,
    ) -> bool {
        true
    }

    fn lingers(&self) -> bool {
        true
    }
}

impl ProviderAdapter for Held {
    fn provider(&self) -> &'static str {
        self.name
    }

    fn models(&self) -> ProviderModels {
        ProviderModels::default()
    }

    fn restarts_to_switch(&self) -> bool {
        self.restarts
    }

    fn start_turn(
        &self,
        _request: TurnRequest,
        sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
        self.sinks.lock().unwrap().push(sink);
        Ok(Box::new(HeldTurn(self.interrupted.clone())))
    }
}

impl Held {
    fn report(&self, output: ProviderOutput) {
        (self.sinks.lock().unwrap()[0])(output);
    }

    fn says(&self, item_id: &str) {
        self.report(ProviderOutput::AssistantMessage {
            item_id: item_id.into(),
            text: "Still on it.".into(),
        });
    }

    fn background(&self, tasks: &[&str]) {
        self.report(ProviderOutput::BackgroundTasks {
            tasks: tasks
                .iter()
                .map(|task| BackgroundTask {
                    task_id: (*task).into(),
                    tool_call_id: None,
                    description: "dev server".into(),
                    started_at: 0,
                })
                .collect(),
            waiting: false,
        });
    }

    fn ends(&self, outcome: TurnOutcome) {
        self.report(ProviderOutput::TurnCompleted { outcome });
    }
}

/// Thread `t` with its first turn still running, on an agent of which `restarts` tells how its
/// turns give way.
fn working(name: &str, restarts: bool) -> (Setup, Arc<Held>) {
    let setup = local(name, Scripted::new(CLAUDE));
    let held = Arc::new(Held {
        name: CLAUDE,
        restarts,
        interrupted: Arc::default(),
        sinks: Mutex::default(),
    });
    setup
        .engine
        .set_adapters(vec![held.clone(), setup.codex.clone()]);
    setup.dispatch("m1", "Remember APPLE.").unwrap();
    (setup, held)
}

impl Setup {
    fn dispatch(&self, message_id: &str, text: &str) -> Result<(), CommandError> {
        self.engine.dispatch(Command::MessageSend {
            thread_id: "t".into(),
            message_id: message_id.into(),
            text: text.into(),
            attachments: Vec::new(),
        })
    }

    fn ended(&self, turns: usize) {
        let ended = self
            .events
            .iter()
            .filter(|event| matches!(event, Event::TurnCompleted { .. }));
        assert_eq!(ended.take(turns).count(), turns);
    }

    fn waiting(&self) -> Vec<String> {
        self.engine
            .thread_queue("t")
            .messages
            .into_iter()
            .map(|message| message.message_id)
            .collect()
    }
}

#[test]
fn a_message_for_another_provider_waits_for_the_turn_and_takes_all_it_said() {
    let (setup, held) = working("switch-waits", false);
    setup.pick("t", CODEX).unwrap();
    setup.dispatch("m2", "Go on.").unwrap();

    assert_eq!(setup.waiting(), ["m2"]);
    assert!(setup.engine.thread_item("t", "m2").unwrap().is_none());
    assert!(!held.interrupted.load(Ordering::SeqCst));
    assert_eq!(setup.thread("t").provider, CLAUDE);

    held.says("late");
    held.ends(TurnOutcome::Completed);
    setup.ended(2);

    assert!(setup.waiting().is_empty());
    let thread = setup.thread("t");
    assert_eq!(thread.provider, CODEX);
    assert_eq!(thread.pending_model, None);
    let text = handoff_text(&setup.codex.request(0));
    assert!(
        text.contains("item=m1;") && text.contains("item=late;"),
        "{text}"
    );
    assert!(!text.contains("item=m2;"), "{text}");
}

#[test]
fn a_message_goes_where_it_was_sent_though_another_provider_is_picked_since() {
    let (setup, held) = working("switch-kept", false);
    setup.pick("t", CODEX).unwrap();
    setup.dispatch("m2", "Go on.").unwrap();
    setup.pick("t", CLAUDE).unwrap();
    setup.dispatch("m3", "And this, Claude.").unwrap();
    assert_eq!(setup.waiting(), ["m2"], "m3 joins the running turn");

    held.ends(TurnOutcome::Completed);
    setup.ended(2);

    let thread = setup.thread("t");
    assert_eq!(thread.provider, CODEX);
    assert_eq!(
        thread.pending_model.unwrap().provider,
        CLAUDE,
        "the next message goes back"
    );
}

#[test]
fn a_turn_that_restarts_to_switch_is_stopped_for_the_message() {
    let (setup, held) = working("switch-restarts", true);
    setup.pick("t", CODEX).unwrap();
    assert!(
        !held.interrupted.load(Ordering::SeqCst),
        "a pick alone waits"
    );
    setup.dispatch("m2", "Go on.").unwrap();
    assert!(held.interrupted.load(Ordering::SeqCst));

    held.ends(TurnOutcome::Interrupted);
    setup.ended(2);

    assert_eq!(setup.thread("t").provider, CODEX);
    assert!(setup.waiting().is_empty());
}

#[test]
fn a_message_for_another_provider_waits_for_what_the_turn_left_running() {
    let (setup, held) = working("switch-background", true);
    held.background(&["server"]);
    setup.pick("t", CODEX).unwrap();
    setup.dispatch("m2", "Go on.").unwrap();
    assert!(
        !held.interrupted.load(Ordering::SeqCst),
        "stopping it would not free the thread"
    );
    held.ends(TurnOutcome::Completed);
    setup.ended(1);

    assert_eq!(setup.waiting(), ["m2"]);
    assert_eq!(
        setup.engine.send_queued_now("t", "m2"),
        Err(CommandError::BackgroundWorkRunning {
            thread_id: "t".into()
        })
    );
    assert_eq!(setup.waiting(), ["m2"]);

    held.background(&[]);
    lock(&setup.engine.state).start_waiting_switches();
    assert_eq!(setup.waiting(), ["m2"], "its agent may still wake");
    lock(&setup.engine.state)
        .lingering
        .get_mut("t")
        .unwrap()
        .heard_at -= std::time::Duration::from_secs(60);
    lock(&setup.engine.state).start_waiting_switches();
    setup.ended(1);

    assert_eq!(setup.thread("t").provider, CODEX);
}

#[test]
fn a_message_taken_out_of_the_queue_never_reaches_the_transcript() {
    let (setup, held) = working("switch-removed", false);
    setup.pick("t", CODEX).unwrap();
    setup.dispatch("m2", "Go on.").unwrap();
    setup.engine.remove_queued_message("t", "m2").unwrap();
    held.ends(TurnOutcome::Completed);
    setup.ended(1);

    assert!(setup.engine.thread_item("t", "m2").unwrap().is_none());
    assert_eq!(setup.thread("t").provider, CLAUDE);
}

#[test]
fn a_kept_process_with_background_work_holds_off_a_message_for_another_provider() {
    let setup = local(
        "background",
        Scripted {
            background: true,
            ..Scripted::new(CLAUDE)
        },
    );
    setup.send("t", "m1", "Start the server.");
    setup.pick("t", CODEX).unwrap();

    assert_eq!(
        setup.dispatch("m2", "Go on."),
        Err(CommandError::BackgroundWorkRunning {
            thread_id: "t".into()
        })
    );
    let thread = setup.thread("t");
    assert_eq!(thread.provider, CLAUDE);
    assert_eq!(thread.pending_model.unwrap().provider, CODEX);
}

#[test]
fn a_fork_resumes_a_copy_of_its_source_session_from_its_turn() {
    let setup = local("native-fork", Scripted::new(CLAUDE));
    setup.send("t", "m1", "Remember APPLE.");
    setup.send("t", "m2", "Remember BANANA.");
    let point = ForkPoint::Turn {
        item_id: "claudeCode-1:answer".into(),
    };

    setup.engine.fork_thread("t", "f", &point).unwrap();
    let history = setup.engine.fork_history("f", None).unwrap();
    setup.send("f", "f1", "Which words?");

    let ids: Vec<&str> = history.items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(ids, ["m1", "claudeCode-1:answer"]);
    let forked = setup.claude.request(2);
    assert_eq!(forked.resume_session_id.as_deref(), Some("claudeCode-1"));
    assert_eq!(
        forked.resume_session_at.as_deref(),
        Some("claudeCode-1:cursor")
    );
    assert!(forked.fork_session);
    assert!(forked.handoff.is_none());
    let fork = setup.thread("f");
    assert_eq!(fork.title, "Login fork");
    assert_eq!(fork.session_id.as_deref(), Some("claudeCode-3"));
    assert!(fork.pending_handoffs.is_empty());
    assert_eq!(
        setup.thread("t").session_id.as_deref(),
        Some("claudeCode-1")
    );
}

#[test]
fn a_fork_knows_what_its_turn_ran_on() {
    let setup = local("fork-model", Scripted::new(CLAUDE));
    let effort_of = |thread_id: &str, effort| hoshi_core::Command::ThreadModelSet {
        thread_id: thread_id.into(),
        provider: CLAUDE.into(),
        model: Some("opus".into()),
        effort: Some(effort),
        fast_mode: false,
        ultracode: false,
    };
    let effort = |effort| effort_of("t", effort);
    setup
        .engine
        .dispatch(effort(hoshi_core::Effort::Medium))
        .unwrap();
    setup.send("t", "m1", "Remember APPLE.");
    setup
        .engine
        .dispatch(effort(hoshi_core::Effort::High))
        .unwrap();
    setup.send("t", "m2", "Remember BANANA.");

    setup
        .engine
        .fork_thread(
            "t",
            "f",
            &ForkPoint::Turn {
                item_id: "m1".into(),
            },
        )
        .unwrap();

    let last = |setup: &Setup| setup.thread("f").last_turn_model.map(|model| model.effort);
    assert_eq!(last(&setup), Some(Some(hoshi_core::Effort::Medium)));

    let fork_effort = effort_of("f", hoshi_core::Effort::High);
    setup.engine.dispatch(fork_effort).unwrap();
    setup.send("f", "f1", "Which words?");
    assert_eq!(last(&setup), Some(Some(hoshi_core::Effort::High)));
    setup.engine.rewind_thread("f", "f1", false).unwrap();
    assert_eq!(
        last(&setup),
        Some(Some(hoshi_core::Effort::Medium)),
        "before its first own turn, a fork ran what its source did"
    );
}

#[test]
fn a_fork_on_another_provider_gets_its_source_history_as_a_transcript() {
    let setup = local("portable-fork", Scripted::new(CLAUDE));
    setup.send("t", "m1", "Remember APPLE.");
    setup.send("t", "m2", "Remember BANANA.");
    setup
        .engine
        .fork_thread(
            "t",
            "f",
            &ForkPoint::Turn {
                item_id: "m1".into(),
            },
        )
        .unwrap();

    setup.pick("f", CODEX).unwrap();
    setup.send("f", "f1", "Which words?");

    let request = setup.codex.request(0);
    assert_eq!(request.resume_session_id, None);
    assert!(!request.fork_session);
    let text = handoff_text(&request);
    assert!(text.contains("Thread: t. Covered turns: 1-1."), "{text}");
    assert!(text.contains("Remember APPLE.") && !text.contains("BANANA"));
    assert!(setup.thread("f").pending_handoffs.is_empty());
}

#[test]
fn a_merge_back_reaches_the_parent_s_next_turn_as_the_fork_s_delta() {
    let setup = local("merge-back", Scripted::new(CLAUDE));
    setup.send("t", "m1", "Plan the fix.");
    setup
        .engine
        .fork_thread("t", "f", &ForkPoint::LatestStable)
        .unwrap();
    setup.send("f", "f1", "Try the other approach.");

    setup
        .engine
        .merge_back("f", &ForkPoint::LatestStable)
        .unwrap();
    setup.send("t", "m2", "What did the fork find?");

    let request = setup.claude.request(2);
    assert_eq!(request.resume_session_id.as_deref(), Some("claudeCode-1"));
    let text = handoff_text(&request);
    let expected = [
        "Context handoff (merge_back / fork_delta_summary):",
        "Merge-back context from forked conversation.",
        "Source thread: f",
        "Target thread: t",
        "Covered fork turns: 1-1",
        "",
        "Fork delta:",
        "- User: Try the other approach.",
        "- Assistant: Done.",
    ]
    .join("\n");
    assert!(text.starts_with(&expected), "{text}");
    assert!(setup.thread("t").pending_handoffs.is_empty());
}

fn wait_until(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_worktree_a_fork_shares_stays_until_neither_thread_uses_it() {
    let repo = crate::git::tests::temp_repository("handoffs-shared-worktree");
    let setup = setup(
        "shared-worktree",
        &repo,
        ThreadWorkspace::NewWorktree,
        Scripted::new(CLAUDE),
    );
    setup.send("t", "m1", "Work.");
    let worktree = setup.thread("t").worktree_path.unwrap();
    setup
        .engine
        .fork_thread("t", "f", &ForkPoint::LatestStable)
        .unwrap();
    assert_eq!(
        setup.thread("f").worktree_path.as_deref(),
        Some(worktree.as_str())
    );

    assert_eq!(
        setup.engine.release_worktree("t"),
        Err(CommandError::WorktreeShared {
            thread_id: "t".into(),
            other_thread_id: "f".into()
        })
    );
    let delete = |thread_id: &str| Command::ThreadDelete {
        thread_id: thread_id.into(),
    };
    assert!(
        lock(&setup.engine.state)
            .worktrees_freed_by(&delete("t"))
            .is_empty()
    );
    setup.engine.dispatch(delete("t")).unwrap();
    let freed = lock(&setup.engine.state).worktrees_freed_by(&delete("f"));
    assert_eq!(freed.len(), 1);
    assert_eq!(freed[0].1, worktree);

    setup
        .engine
        .dispatch(Command::ThreadDelete {
            thread_id: "f".into(),
        })
        .unwrap();
    wait_until(|| !Path::new(&worktree).exists());
}
