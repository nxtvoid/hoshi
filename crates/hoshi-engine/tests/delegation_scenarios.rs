//! Real Claude Code agents delegate tasks through hoshi's MCP server while everything the UI
//! would receive is recorded, for `apps/desktop/tests/lib/task-run-replays.test.ts` to replay
//! against the UI's own logic. Each run spends tokens, so they only run on request:
//! `cargo test -p hoshi-engine --test delegation_scenarios -- --ignored --test-threads=3`
//!
//! `HOSHI_SCENARIO_MODEL` picks the model, `HOSHI_SCENARIO_DIR` where the recordings go.

use hoshi_core::{AccessMode, Command, Effort, Event, ItemContent, ThreadStatus, ThreadWorkspace};
use hoshi_engine::{Engine, Notification, Notify};
use hoshi_providers::ClaudeCodeAdapter;
use hoshi_store::Store;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const ROOT_THREAD: &str = "t";
const TIMEOUT: Duration = Duration::from_secs(300);
/// Nothing ran for this long: the run is over.
const QUIET: Duration = Duration::from_secs(6);
/// Longer strings are cut: the replay reads how runs unfold, not what they said.
const KEEP_CHARS: usize = 2_000;

/// What the user does after the agent's first turn ended.
enum Later {
    Say(&'static str),
    StopTasks,
}

/// When the user does it.
enum When {
    AfterFirstTurn(Duration),
    /// As the first task's answer reaches the agent, which then works on its reply.
    OnFirstAnswer,
}

struct Scenario {
    name: &'static str,
    prompt: &'static str,
    later: Vec<(When, Later)>,
    /// What the agent's last answer has to tell, as it only can once the tasks' answers
    /// reached it.
    expects: &'static [&'static str],
    /// How many tasks, at least, have to work in a worktree of their own.
    worktrees: usize,
}

fn git(folder: &Path, arguments: &[&str]) {
    let done = std::process::Command::new("git")
        .current_dir(folder)
        .args(["-c", "user.name=nxtvoid"])
        .args(["-c", "user.email=nxtvoid@users.noreply.github.com"])
        .args(arguments)
        .output()
        .unwrap();
    assert!(done.status.success(), "git {arguments:?}: {done:?}");
}

fn record(scenario: Scenario) {
    let root = std::env::temp_dir().join(format!(
        "hoshi-scenario-{}-{}",
        scenario.name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("notes.txt"), "Remember the milk.\n").unwrap();
    std::fs::write(project.join("todo.md"), "- [ ] Ship it\n").unwrap();
    std::fs::write(
        project.join("README.md"),
        "# Milk\n\nReminds you to buy milk.\n",
    )
    .unwrap();
    git(&project, &["init", "--quiet", "--initial-branch=main"]);
    git(&project, &["add", "."]);
    git(&project, &["commit", "--quiet", "-m", "chore: start"]);

    let started = Instant::now();
    let steps = Arc::new(Mutex::new(Vec::<Value>::new()));
    let first_turn = Arc::new(Mutex::new(None::<Instant>));
    let first_answer = Arc::new(Mutex::new(false));
    let notify: Notify = {
        let (steps, first_turn, first_answer) =
            (steps.clone(), first_turn.clone(), first_answer.clone());
        Arc::new(move |notification| {
            let at = started.elapsed().as_millis() as u64;
            let step = match notification {
                Notification::Event(envelope) => {
                    match &envelope.event {
                        Event::TurnCompleted { thread_id, .. } if thread_id == ROOT_THREAD => {
                            first_turn.lock().unwrap().get_or_insert_with(Instant::now);
                        }
                        Event::ItemAdded { item }
                            if item.thread_id == ROOT_THREAD
                                && matches!(
                                    item.content,
                                    ItemContent::UserMessage {
                                        task_result: Some(_),
                                        ..
                                    }
                                ) =>
                        {
                            *first_answer.lock().unwrap() = true;
                        }
                        _ => {}
                    }
                    json!({ "at": at, "event": envelope })
                }
                Notification::BackgroundTasks(tasks) => {
                    json!({ "at": at, "backgroundTasks": tasks })
                }
                _ => return,
            };
            steps.lock().unwrap().push(step);
        })
    };
    let engine =
        Engine::from_store(Store::open_in_memory().unwrap(), &root.join("data"), notify).unwrap();
    let model = std::env::var("HOSHI_SCENARIO_MODEL").unwrap_or_else(|_| "sonnet".into());
    engine.set_adapter(Some(Arc::new(ClaudeCodeAdapter::new(None, Some(model)))));
    engine
        .dispatch(Command::ProjectCreate {
            project_id: "p".into(),
            title: "Project".into(),
            workspace_root: project.to_string_lossy().into_owned(),
        })
        .unwrap();
    engine
        .dispatch(Command::ThreadCreate {
            thread_id: ROOT_THREAD.into(),
            project_id: Some("p".into()),
            title: "Parent".into(),
            workspace: ThreadWorkspace::Local,
            model: None,
            effort: Some(Effort::Low),
            fast_mode: false,
            ultracode: false,
            access_mode: AccessMode::FullAccess,
            plan_mode: false,
        })
        .unwrap();
    let say = |number: usize, text: &str| {
        engine
            .dispatch(Command::MessageSend {
                thread_id: ROOT_THREAD.into(),
                message_id: format!("{number:08}-1111-4111-8111-111111111111"),
                text: text.into(),
                attachments: Vec::new(),
            })
            .unwrap();
    };
    say(1, scenario.prompt);

    let mut later = scenario.later.into_iter().peekable();
    let mut said = 1;
    let mut quiet_since = None::<Instant>;
    let timed_out = loop {
        thread::sleep(Duration::from_millis(250));
        let first = *first_turn.lock().unwrap();
        let answered = *first_answer.lock().unwrap();
        while later.peek().is_some_and(|(when, _)| match when {
            When::AfterFirstTurn(after) => first.is_some_and(|first| first.elapsed() >= *after),
            When::OnFirstAnswer => answered,
        }) {
            match later.next().unwrap().1 {
                Later::Say(text) => {
                    said += 1;
                    say(said, text);
                }
                Later::StopTasks => {
                    let snapshot = engine.snapshot();
                    for task in snapshot.threads.iter().filter(|thread| {
                        thread.parent_thread_id.as_deref() == Some(ROOT_THREAD)
                            && (snapshot.pending_tasks.contains(&thread.id)
                                || snapshot
                                    .background_tasks
                                    .iter()
                                    .any(|background| background.thread_id == thread.id))
                    }) {
                        engine.cancel_task(&task.id);
                    }
                }
            }
        }
        let snapshot = engine.snapshot();
        let quiet = first.is_some()
            && later.peek().is_none()
            && snapshot.pending_tasks.is_empty()
            && snapshot.background_tasks.is_empty()
            && snapshot
                .threads
                .iter()
                .all(|thread| thread.status == ThreadStatus::Idle);
        quiet_since = quiet.then(|| quiet_since.unwrap_or_else(Instant::now));
        if quiet_since.is_some_and(|since| since.elapsed() >= QUIET) {
            break false;
        }
        if started.elapsed() >= TIMEOUT {
            break true;
        }
    };
    let answer = engine
        .transcript(ROOT_THREAD, None)
        .unwrap()
        .items
        .into_iter()
        .rev()
        .find_map(|item| match item.content {
            ItemContent::AssistantMessage { text } => Some(text.to_lowercase()),
            _ => None,
        })
        .unwrap_or_default();
    let worktrees = engine
        .snapshot()
        .threads
        .iter()
        .filter(|thread| {
            thread.parent_thread_id.as_deref() == Some(ROOT_THREAD)
                && thread.worktree_path.is_some()
        })
        .count();
    drop(engine);

    let mut recording = json!({
        "name": scenario.name,
        "prompt": scenario.prompt,
        "timedOut": timed_out,
        "steps": *steps.lock().unwrap(),
    });
    scrub(&mut recording, &hidden_paths(&root));
    let dir = std::env::var("HOSHI_SCENARIO_DIR").map_or_else(
        |_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../apps/desktop/tests/fixtures/task-runs")
        },
        PathBuf::from,
    );
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{}.json", scenario.name)),
        serde_json::to_string(&recording).unwrap(),
    )
    .unwrap();
    let _ = std::fs::remove_dir_all(&root);
    assert!(!timed_out, "{} never came to rest", scenario.name);
    assert!(
        worktrees >= scenario.worktrees,
        "{}: {worktrees} tasks got a worktree of their own, not {}",
        scenario.name,
        scenario.worktrees
    );
    for expected in scenario.expects {
        assert!(
            answer.contains(&expected.to_lowercase()),
            "{}: the last answer doesn't tell {expected:?}: {answer}",
            scenario.name
        );
    }
}

/// The folders of this machine a recording must not name, each as the ways it gets written,
/// with what to show instead.
fn hidden_paths(root: &Path) -> Vec<(String, String)> {
    let mut hidden = Vec::new();
    let mut hide = |path: &Path, shown: &str| {
        let path = path.to_string_lossy().into_owned();
        hidden.push((path.replace('\\', "\\\\"), shown.to_owned()));
        hidden.push((path.replace('\\', "/"), shown.to_owned()));
        if let Some((drive, rest)) = path.split_once(":\\") {
            let posix = format!("/{}/{}", drive.to_lowercase(), rest.replace('\\', "/"));
            hidden.push((posix, shown.to_owned()));
        }
        hidden.push((path, shown.to_owned()));
    };
    hide(root, "C:/work");
    hide(&std::env::temp_dir(), "C:/temp");
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        hide(Path::new(&home), "C:/Users/nxtvoid");
    }
    hidden
}

fn scrub(value: &mut Value, hidden: &[(String, String)]) {
    match value {
        Value::String(text) => {
            for (path, shown) in hidden {
                *text = replace_ignoring_case(text, path, shown);
            }
            if text.chars().nth(KEEP_CHARS).is_some() {
                *text = text.chars().take(KEEP_CHARS).chain(['…']).collect();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| scrub(item, hidden)),
        Value::Object(fields) => fields.values_mut().for_each(|field| scrub(field, hidden)),
        _ => {}
    }
}

fn replace_ignoring_case(text: &str, found: &str, shown: &str) -> String {
    let (lower, found) = (text.to_ascii_lowercase(), found.to_ascii_lowercase());
    if found.is_empty() || !lower.contains(&found) {
        return text.to_owned();
    }
    let mut replaced = String::with_capacity(text.len());
    let mut rest = 0;
    while let Some(at) = lower[rest..].find(&found) {
        replaced.push_str(&text[rest..rest + at]);
        replaced.push_str(shown);
        rest += at + found.len();
    }
    replaced.push_str(&text[rest..]);
    replaced
}

fn after(seconds: u64, later: Later) -> (When, Later) {
    (When::AfterFirstTurn(Duration::from_secs(seconds)), later)
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn variants_each_in_a_branch_of_its_own() {
    record(Scenario {
        name: "variants-each-in-a-branch-of-its-own",
        expects: &[],
        worktrees: 2,
        prompt: "Quiero probar dos enfoques distintos para mejorar el README sin que se pisen entre sí: uno que lo acorte a lo esencial y otro que le agregue una sección de instalación paso a paso. Cada uno en su propia rama, y al final dime cuál te parece mejor y por qué.",
        later: Vec::new(),
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn the_user_writes_as_an_answer_arrives() {
    record(Scenario {
        name: "the-user-writes-as-an-answer-arrives",
        expects: &[],
        worktrees: 0,
        prompt: "Necesito tres cosas y no quiero quedarme esperando: que alguien revise qué archivos de esta carpeta pesan más de 1 MB, que otro me diga cuántas líneas tiene cada archivo de texto, y que un tercero espere 20 segundos y me confirme que sigue vivo. Sigue tú disponible mientras tanto y avísame cuando estén las tres.",
        later: vec![(When::OnFirstAnswer, Later::Say("¿cómo estás?"))],
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn one_quick_task() {
    record(Scenario {
        name: "one-quick-task",
        expects: &["PINEAPPLE"],
        worktrees: 0,
        prompt: "Delegate one task with the hoshi delegate_task tool: \"Answer with exactly the word PINEAPPLE.\" When its answer arrives, tell me the word.",
        later: Vec::new(),
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn two_tasks_that_answer_together() {
    record(Scenario {
        name: "two-tasks-that-answer-together",
        expects: &["ALPHA", "BETA"],
        worktrees: 0,
        prompt: "Delegate 2 tasks in parallel with delegate_task, both on the same model and effort: one answers with exactly the word ALPHA, the other with exactly the word BETA. When both have answered, tell me both words.",
        later: Vec::new(),
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn three_tasks_in_parallel() {
    record(Scenario {
        name: "three-tasks-in-parallel",
        expects: &["notes.txt"],
        worktrees: 0,
        prompt: "Delega 3 tareas en paralelo con delegate_task, en hilos aparte: (a) que espere 10 segundos y diga \"A lista\"; (b) que espere 25 segundos, pero que lance el sleep en segundo plano, termine su turno sin esperarlo y avise cuando acabe; (c) que liste los archivos de esta carpeta. Cuando terminen todas, hazme un resumen.",
        later: Vec::new(),
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn a_task_with_a_task_of_its_own() {
    record(Scenario {
        name: "a-task-with-a-task-of-its-own",
        expects: &["sub lista"],
        worktrees: 0,
        prompt: "Delega con delegate_task, en hilos aparte, 2 tareas en paralelo: (a) que lea el archivo no-existe.txt y me diga qué contiene; (b) que delegue a su vez una subtarea que espere 10 segundos en segundo plano y diga \"sub lista\", y que (b) me avise cuando su subtarea responda. Cuando terminen las dos, hazme un resumen.",
        later: Vec::new(),
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn the_user_asks_while_tasks_run() {
    record(Scenario {
        name: "the-user-asks-while-tasks-run",
        expects: &["A ready", "B ready"],
        worktrees: 0,
        prompt: "Delegate 2 tasks in parallel with delegate_task: (a) wait 25 seconds, then answer \"A ready\"; (b) wait 35 seconds, then answer \"B ready\". When both have answered, summarize.",
        later: vec![after(8, Later::Say("How is it going so far?"))],
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn the_user_stops_a_task() {
    record(Scenario {
        name: "the-user-stops-a-task",
        expects: &["A ready"],
        worktrees: 0,
        prompt: "Delegate 2 tasks in parallel with delegate_task: (a) answer \"A ready\" right away; (b) wait 120 seconds, then answer \"B ready\". When both have answered, summarize.",
        later: vec![after(15, Later::StopTasks)],
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn a_task_leaves_a_command_running() {
    record(Scenario {
        name: "a-task-leaves-a-command-running",
        expects: &[],
        worktrees: 0,
        prompt: "Delegate one task with delegate_task: \"Start the command `ping -n 200 127.0.0.1` in the background, leave it running without waiting for it, and answer 'server up'.\" Tell me what it answers.",
        later: vec![after(25, Later::StopTasks)],
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn one_task_after_another() {
    record(Scenario {
        name: "one-task-after-another",
        expects: &["FIRST", "SECOND"],
        worktrees: 0,
        prompt: "Delegate a task with delegate_task that answers with the word FIRST. Only once its answer has arrived, delegate a second task that answers with the word SECOND. When that one answers too, tell me both words.",
        later: Vec::new(),
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn everyday_wording() {
    record(Scenario {
        name: "everyday-wording",
        expects: &["notes.txt", "todo.md", "ship it"],
        worktrees: 0,
        prompt: "Necesito dos cosas a la vez y no quiero esperar mirando: que alguien cuente las líneas de notes.txt y que otro revise todo.md y me diga qué falta. Avísame cuando estén las dos.",
        later: Vec::new(),
    });
}
