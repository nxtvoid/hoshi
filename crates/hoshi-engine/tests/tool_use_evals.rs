//! Whether real Claude Code agents use hoshi's tools when they should, and leave them alone
//! when they shouldn't, with the instructions and tool descriptions of this checkout. Each
//! case reproduces a real mistake (`bad`), guards a right choice (`guard`), or is ordinary
//! work no hoshi tool is for (`control`). Runs spend tokens, so they only run on request:
//! `cargo test -p hoshi-engine --test tool_use_evals -- --ignored --test-threads=4`
//!
//! Model output varies, so each case runs `HOSHI_EVAL_SAMPLES` times (2 by default, a third
//! when they disagree) and fails when half its samples or more do. `HOSHI_EVAL_MODEL` picks
//! the model, `sonnet` by default, and `HOSHI_EVAL_EFFORT` its effort, `low` by default.
//! Change a case only if it still fails on the text it was written against.

use hoshi_core::{AccessMode, Command, Effort, Event, ItemContent, ThreadStatus, ThreadWorkspace};
use hoshi_engine::{Engine, Notification, Notify};
use hoshi_providers::ClaudeCodeAdapter;
use hoshi_store::Store;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const THREAD: &str = "t";
const TIMEOUT: Duration = Duration::from_secs(600);
const QUIET: Duration = Duration::from_secs(4);

enum Kind {
    Bad,
    Guard,
    Control,
}

/// A tool call a check looks for: the tool, by the end of its name, and text its input holds.
struct Call {
    tool: &'static str,
    input: &'static [&'static str],
}

fn call(tool: &'static str) -> Call {
    Call { tool, input: &[] }
}

/// When the user writes again.
enum After {
    /// The agent's turn ended; its tasks may still run.
    TurnEnd,
    /// Everything came to rest: tasks answered and the agent replied to them.
    Rest,
}

struct Case {
    name: &'static str,
    kind: Kind,
    /// The files of the project the agent works in.
    files: &'static [(&'static str, &'static str)],
    prompt: &'static str,
    /// What the user writes next, and when; the checks look only at what follows the last.
    then: Vec<(After, &'static str)>,
    calls: Vec<Call>,
    must_not: Vec<Call>,
    /// Calls that have to come in this order, each after the one before.
    order: &'static [&'static str],
    /// Text the last answer has to hold, in any case.
    answer: &'static [&'static str],
    /// Tools called at most this many times.
    at_most: &'static [(&'static str, usize)],
    /// The agent's effort when the case needs one, over `HOSHI_EVAL_EFFORT`.
    effort: Option<Effort>,
}

/// The tool calls of one run: name and input.
type Calls = Vec<(String, String)>;

/// What one run did: its tool calls and its last answer.
struct Run {
    calls: Calls,
    answer: String,
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

fn matches(found: &(String, String), wanted: &Call) -> bool {
    let input = found.1.to_lowercase();
    found.0.ends_with(wanted.tool)
        && wanted
            .input
            .iter()
            .all(|text| input.contains(&text.to_lowercase()))
}

fn describe(wanted: &Call) -> String {
    if wanted.input.is_empty() {
        wanted.tool.to_owned()
    } else {
        format!("{} with {:?}", wanted.tool, wanted.input)
    }
}

/// What a run did wrong; empty when it passed.
fn check(case: &Case, calls: &Calls, answer: &str) -> Vec<String> {
    let mut failures = Vec::new();
    for wanted in &case.calls {
        if !calls.iter().any(|found| matches(found, wanted)) {
            failures.push(format!("never called {}", describe(wanted)));
        }
    }
    for unwanted in &case.must_not {
        if calls.iter().any(|found| matches(found, unwanted)) {
            failures.push(format!("called {}", describe(unwanted)));
        }
    }
    for (tool, most) in case.at_most {
        let count = calls
            .iter()
            .filter(|(name, _)| name.ends_with(tool))
            .count();
        if count > *most {
            failures.push(format!("called {tool} {count} times, past {most}"));
        }
    }
    for text in case.answer {
        if !answer.to_lowercase().contains(&text.to_lowercase()) {
            failures.push(format!("the answer doesn't hold {text:?}"));
        }
    }
    let mut from = 0;
    for tool in case.order {
        match calls[from..]
            .iter()
            .position(|(name, _)| name.ends_with(tool))
        {
            Some(at) => from += at + 1,
            None => {
                failures.push(format!("not in the order {:?}", case.order));
                break;
            }
        }
    }
    failures
}

/// One run of the case: the agent's tool calls once its turn came to rest.
fn sample(case: &Case, number: usize) -> Result<Run, String> {
    let root = std::env::temp_dir().join(format!(
        "hoshi-eval-{}-{number}-{}",
        case.name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("project");
    for (path, text) in case.files {
        let file = project.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    std::fs::create_dir_all(&project).unwrap();
    git(&project, &["init", "--quiet", "--initial-branch=main"]);
    git(&project, &["add", "."]);
    git(
        &project,
        &["commit", "--quiet", "--allow-empty", "-m", "chore: start"],
    );

    let ended = Arc::new(AtomicBool::new(false));
    let notify: Notify = {
        let ended = ended.clone();
        Arc::new(move |notification| {
            if let Notification::Event(envelope) = notification
                && let Event::TurnCompleted { thread_id, .. } = &envelope.event
                && thread_id == THREAD
            {
                ended.store(true, Ordering::Release);
            }
        })
    };
    let engine =
        Engine::from_store(Store::open_in_memory().unwrap(), &root.join("data"), notify).unwrap();
    let model = std::env::var("HOSHI_EVAL_MODEL").unwrap_or_else(|_| "sonnet".into());
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
            thread_id: THREAD.into(),
            project_id: Some("p".into()),
            title: "Eval".into(),
            workspace: ThreadWorkspace::Local,
            model: None,
            effort: Some(case.effort.unwrap_or_else(|| {
                std::env::var("HOSHI_EVAL_EFFORT")
                    .ok()
                    .and_then(|effort| serde_json::from_value(serde_json::json!(effort)).ok())
                    .unwrap_or(Effort::Low)
            })),
            fast_mode: false,
            ultracode: false,
            access_mode: AccessMode::FullAccess,
            plan_mode: false,
        })
        .unwrap();
    let send = |number: usize, text: &str| {
        ended.store(false, Ordering::Release);
        engine
            .dispatch(Command::MessageSend {
                thread_id: THREAD.into(),
                message_id: format!("{number:08}-1111-4111-8111-111111111111"),
                text: text.into(),
                attachments: Vec::new(),
            })
            .unwrap();
    };
    let started = Instant::now();
    let wait = |until: &After| {
        let mut quiet_since = None::<Instant>;
        loop {
            thread::sleep(Duration::from_millis(250));
            let snapshot = engine.snapshot();
            let idle = |id: &str| {
                snapshot
                    .threads
                    .iter()
                    .filter(|thread| thread.id == id || matches!(until, After::Rest))
                    .all(|thread| thread.status == ThreadStatus::Idle)
            };
            let quiet = ended.load(Ordering::Acquire)
                && idle(THREAD)
                && (matches!(until, After::TurnEnd)
                    || (snapshot.background_tasks.is_empty() && snapshot.pending_tasks.is_empty()));
            quiet_since = quiet.then(|| quiet_since.unwrap_or_else(Instant::now));
            if quiet_since.is_some_and(|since| since.elapsed() >= QUIET) {
                return true;
            }
            if started.elapsed() >= TIMEOUT {
                return false;
            }
        }
    };
    send(1, case.prompt);
    let mut rested = true;
    let mut from = 0;
    for (number, (after, text)) in case.then.iter().enumerate() {
        rested = rested && wait(after);
        from = engine.transcript(THREAD, None).unwrap().items.len();
        send(number + 2, text);
    }
    rested = rested && wait(&After::Rest);
    let mut calls = Vec::new();
    let mut answer = String::new();
    for item in engine
        .transcript(THREAD, None)
        .unwrap()
        .items
        .into_iter()
        .skip(from)
    {
        match item.content {
            ItemContent::ToolCall { name, input, .. } => calls.push((name, input)),
            ItemContent::AssistantMessage { text } => answer = text,
            _ => {}
        }
    }
    drop(engine);
    let _ = std::fs::remove_dir_all(&root);
    if rested {
        Ok(Run { calls, answer })
    } else {
        Err("never came to rest".into())
    }
}

fn run(case: Case) {
    let samples: usize = std::env::var("HOSHI_EVAL_SAMPLES")
        .ok()
        .and_then(|samples| samples.parse().ok())
        .unwrap_or(2);
    let mut results = Vec::new();
    let mut number = 0;
    while number < samples
        || (samples == 2
            && number == 2
            && results
                .iter()
                .filter(|failures: &&Vec<String>| failures.is_empty())
                .count()
                == 1)
    {
        number += 1;
        let failures = match sample(&case, number) {
            Ok(run) => {
                let failures = check(&case, &run.calls, &run.answer);
                let names: Vec<&str> = run.calls.iter().map(|(name, _)| name.as_str()).collect();
                println!("{} #{number}: {names:?}", case.name);
                if !failures.is_empty() {
                    for (name, input) in &run.calls {
                        let input: String = input.chars().take(300).collect();
                        println!("  {name} {input}");
                    }
                    let answer: String = run.answer.chars().take(600).collect();
                    println!("  answered: {answer}");
                }
                failures
            }
            Err(error) => vec![error],
        };
        results.push(failures);
    }
    let failed: Vec<&Vec<String>> = results
        .iter()
        .filter(|failures| !failures.is_empty())
        .collect();
    let kind = match case.kind {
        Kind::Bad => "bad",
        Kind::Guard => "guard",
        Kind::Control => "control",
    };
    println!(
        "{} ({kind}): {}/{} passed",
        case.name,
        results.len() - failed.len(),
        results.len()
    );
    assert!(
        failed.len() * 2 < results.len(),
        "{} failed {}/{}: {failed:?}",
        case.name,
        failed.len(),
        results.len()
    );
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn inline_visual_is_previewed_then_rendered() {
    run(Case {
        name: "inline-visual-preview-then-render",
        kind: Kind::Guard,
        files: &[],
        prompt: "Show me this as a chart right here:\n\n| month | revenue |\n|---|---|\n| jan | 120 |\n| feb | 135 |\n| mar | 128 |\n| apr | 160 |",
        calls: vec![call("html_preview"), call("html_render")],
        must_not: vec![call("preview_open")],
        order: &["html_preview", "html_render"],
        answer: &[],
        then: Vec::new(),
        at_most: &[],
        effort: None,
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn a_visualizer_is_rendered_not_served() {
    run(Case {
        name: "visualizer-served-and-opened-in-preview",
        kind: Kind::Bad,
        files: &[(
            "data/sales.csv",
            "month,revenue\njan,120\nfeb,135\nmar,128\napr,160\nmay,172\njun,190\n",
        )],
        prompt: "Build me a quick visualizer for data/sales.csv so I can see how revenue moves.",
        calls: vec![call("html_render")],
        must_not: vec![
            call("preview_open"),
            Call {
                tool: "Bash",
                input: &["http.server"],
            },
            Call {
                tool: "Bash",
                input: &["npx serve"],
            },
        ],
        order: &[],
        answer: &[],
        then: Vec::new(),
        at_most: &[],
        effort: None,
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn a_page_on_disk_is_passed_by_path() {
    run(Case {
        name: "revise-page-resends-html",
        kind: Kind::Bad,
        files: &[(
            "report.html",
            "<!doctype html><html><head><title>Report</title></head><body><h1 style=\"color:var(--foreground)\">Quarterly report</h1><p>Revenue grew 12%.</p></body></html>",
        )],
        prompt: "Make the heading in report.html use the accent color, then show me the page here.",
        calls: vec![Call {
            tool: "html_render",
            input: &["\"path\""],
        }],
        must_not: Vec::new(),
        order: &[],
        answer: &[],
        then: Vec::new(),
        at_most: &[],
        effort: None,
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn an_image_to_post_is_a_png_file() {
    run(Case {
        name: "png-to-post-is-a-file",
        kind: Kind::Guard,
        files: &[],
        prompt: "Make me a PNG card announcing \"Acme 1.0 is out\" that I can post on social media. Save it in this folder and show it to me.",
        calls: Vec::new(),
        must_not: vec![call("html_render")],
        order: &[],
        answer: &["![", ".png)"],
        then: Vec::new(),
        at_most: &[],
        effort: None,
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn refactoring_a_chart_component_is_ordinary_work() {
    run(Case {
        name: "control-refactor-chart-component",
        kind: Kind::Control,
        files: &[(
            "src/Chart.ts",
            "export function drawChart(values: number[]) {\n  const max = Math.max(...values)\n  const ticks = [0, max / 2, max].map((tick) => `${tick}`)\n  const bars = values.map((value) => Math.round((value / max) * 100))\n  return { ticks, bars }\n}\n",
        )],
        prompt: "In src/Chart.ts, pull the axis tick code into its own function.",
        calls: Vec::new(),
        must_not: vec![
            call("html_preview"),
            call("html_render"),
            call("preview_open"),
        ],
        order: &[],
        answer: &[],
        then: Vec::new(),
        at_most: &[],
        effort: None,
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn explaining_preview_code_is_ordinary_work() {
    run(Case {
        name: "control-explain-preview-code",
        kind: Kind::Control,
        files: &[(
            "src/preview.ts",
            "/** Opens a preview tab for the page and keeps it in sync with the editor. */\nexport function openPreview(url: string, tabs: Map<string, string>) {\n  const id = `tab-${tabs.size + 1}`\n  tabs.set(id, url)\n  return id\n}\n",
        )],
        prompt: "What does openPreview in src/preview.ts do?",
        calls: Vec::new(),
        must_not: vec![
            call("preview_open"),
            call("html_preview"),
            call("html_render"),
        ],
        order: &[],
        answer: &[],
        then: Vec::new(),
        at_most: &[],
        effort: None,
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn a_table_question_without_a_trend_stays_text() {
    run(Case {
        name: "control-lookup-in-a-table",
        kind: Kind::Control,
        files: &[],
        prompt: "Which month had revenue of exactly 160?

| month | revenue |
|---|---|
| jan | 120 |
| feb | 135 |
| mar | 128 |
| apr | 160 |
| may | 172 |
| jun | 190 |",
        calls: Vec::new(),
        must_not: vec![call("html_render")],
        order: &[],
        answer: &[],
        then: Vec::new(),
        at_most: &[],
        effort: None,
    });
}

const MATH: (&str, &str) = (
    "src/math.ts",
    "export function add(a: number, b: number) {\n  return a - b\n}\n\nexport function average(values: number[]) {\n  return values.reduce((sum, value) => sum + value, 0) / values.length\n}\n",
);
const STRINGS: (&str, &str) = (
    "src/strings.ts",
    "export function capitalize(text: string) {\n  return text[0].toUpperCase() + text.slice(1)\n}\n\nexport function words(text: string) {\n  return text.split(' ').length\n}\n",
);
const DOCS: [(&str, &str); 3] = [
    (
        "docs/intro.md",
        "# Intro\n\nAcme keeps a list of the things you need to buy.\n",
    ),
    (
        "docs/usage.md",
        "# Usage\n\nAdd an item with `acme add milk` and list them with `acme list`.\n",
    ),
    (
        "docs/faq.md",
        "# FAQ\n\nIt stores the list in a plain text file in your home folder.\n",
    ),
];

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn a_new_review_round_is_a_new_task() {
    run(Case {
        name: "review-round-uses-thread-send",
        kind: Kind::Bad,
        files: &[MATH],
        prompt: "Delegate a review of src/math.ts to a task and tell me what it finds. Don't fix anything yet.",
        then: vec![(
            After::Rest,
            "I fixed what it found. Get another review round of src/math.ts.",
        )],
        calls: vec![call("delegate_task")],
        must_not: vec![call("thread_send")],
        order: &[],
        answer: &[],
        at_most: &[],
        effort: None,
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn stopping_every_task_cancels_each() {
    run(Case {
        name: "stop-all-delegates-uses-task-cancel",
        kind: Kind::Guard,
        files: &DOCS,
        prompt: "Start two background tasks and end your turn: one waits 90 seconds (sleep 90), then writes the word count of each file in docs/ to docs/COUNT.md; the other waits 90 seconds, then writes a one-line summary of each file in docs/ to docs/SUMMARY.md.",
        then: vec![(After::TurnEnd, "Stop all the delegates.")],
        calls: vec![call("task_cancel")],
        must_not: vec![call("thread_send")],
        order: &[],
        answer: &[],
        at_most: &[],
        effort: None,
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn running_tasks_are_not_polled() {
    run(Case {
        name: "polling-after-async-delegates",
        kind: Kind::Bad,
        files: &DOCS,
        prompt: "Kick off two background tasks: one counts the lines of each file in docs/, the other finds the longest file in docs/. Tell me both results once they're in.",
        then: Vec::new(),
        calls: vec![call("delegate_task")],
        must_not: vec![call("thread_wait"), call("Monitor"), call("ScheduleWakeup")],
        order: &[],
        answer: &[],
        at_most: &[("task_status", 1)],
        effort: None,
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn a_background_agent_is_a_task() {
    run(Case {
        name: "tt-background-agent",
        // At low effort the agent takes its Agent tool at hand over loading delegate_task.
        kind: Kind::Bad,
        files: &DOCS,
        prompt: "Spin up an agent in the background to check which files in docs/ lack a trailing newline. Meanwhile, what does docs/usage.md say?",
        then: Vec::new(),
        calls: vec![call("delegate_task")],
        must_not: vec![Call {
            tool: "Agent",
            input: &["\"run_in_background\":true"],
        }],
        order: &[],
        answer: &[],
        at_most: &[],
        effort: Some(Effort::High),
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn subagents_asked_for_stay_native() {
    run(Case {
        name: "native-agents-for-parallel-audit",
        kind: Kind::Guard,
        files: &[MATH, STRINGS],
        prompt: "Use subagents to audit src/math.ts and src/strings.ts for bugs in parallel, then give me one combined list. Don't fix anything.",
        then: Vec::new(),
        calls: vec![call("Agent")],
        must_not: vec![call("delegate_task")],
        order: &[],
        answer: &[],
        at_most: &[],
        effort: None,
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn tests_in_the_background_are_a_command() {
    run(Case {
        name: "control-background-test-run",
        kind: Kind::Control,
        files: &[
            MATH,
            (
                "test.js",
                "setTimeout(() => console.log('3 tests passed'), 3000)\n",
            ),
        ],
        prompt: "Run the tests in the background (node test.js) while you rename add to sum in src/math.ts.",
        then: Vec::new(),
        calls: Vec::new(),
        must_not: vec![call("delegate_task"), call("Agent")],
        order: &[],
        answer: &[],
        at_most: &[],
        effort: None,
    });
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn a_stop_button_bug_is_code_work() {
    run(Case {
        name: "control-stop-task-button",
        kind: Kind::Control,
        files: &[(
            "src/TaskRow.tsx",
            "export function TaskRow({ task, stop }: { task: { id: string; title: string }; stop: (id?: string) => void }) {\n  return (\n    <div>\n      {task.title}\n      <button onClick={() => stop()}>Stop</button>\n    </div>\n  )\n}\n",
        )],
        prompt: "The Stop button in src/TaskRow.tsx doesn't cancel the running task. Fix it.",
        then: Vec::new(),
        calls: Vec::new(),
        must_not: vec![call("task_cancel"), call("delegate_task")],
        order: &[],
        answer: &[],
        at_most: &[],
        effort: None,
    });
}

#[test]
fn checks_read_calls_by_name_input_and_order() {
    let case = Case {
        name: "x",
        kind: Kind::Guard,
        files: &[],
        prompt: "",
        calls: vec![Call {
            tool: "html_render",
            input: &["\"path\""],
        }],
        must_not: vec![Call {
            tool: "Bash",
            input: &["http.server"],
        }],
        order: &["html_preview", "html_render"],
        answer: &["![shot]"],
        then: Vec::new(),
        at_most: &[("html_preview", 1)],
        effort: None,
    };
    let calls = |list: &[(&str, &str)]| -> Calls {
        list.iter()
            .map(|(name, input)| (name.to_string(), input.to_string()))
            .collect()
    };
    let good = calls(&[
        ("mcp__hoshi__html_preview", "{}"),
        ("mcp__hoshi__html_render", "{\"path\":\"/a.html\"}"),
    ]);
    assert!(check(&case, &good, "Here: ![shot](/a.png)").is_empty());
    let bad = calls(&[
        ("mcp__hoshi__html_render", "{\"html\":\"<p>\"}"),
        ("Bash", "{\"command\":\"python -m http.server\"}"),
    ]);
    assert_eq!(check(&case, &bad, "done").len(), 4);
    let twice = calls(&[
        ("mcp__hoshi__html_preview", "{}"),
        ("mcp__hoshi__html_preview", "{}"),
        ("mcp__hoshi__html_render", "{\"path\":\"/a.html\"}"),
    ]);
    assert_eq!(check(&case, &twice, "![shot]").len(), 1);
}
