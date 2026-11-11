//! Whether real Claude Code agents use hoshi's tools when they should, and leave them alone
//! when they shouldn't, with the instructions and tool descriptions of this checkout. Each
//! case reproduces a real mistake (`bad`), guards a right choice (`guard`), or is ordinary
//! work no hoshi tool is for (`control`). Runs spend tokens, so they only run on request:
//! `cargo test -p hoshi-engine --test tool_use_evals -- --ignored --test-threads=4`
//!
//! Model output varies, so each case runs `HOSHI_EVAL_SAMPLES` times (2 by default, a third
//! when they disagree) and fails when half its samples or more do. `HOSHI_EVAL_MODEL` picks
//! the model, `sonnet` by default, and `HOSHI_EVAL_EFFORT` its effort, `low` by default. Change a case only if it still fails on the text it was
//! written against.

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
const TIMEOUT: Duration = Duration::from_secs(300);
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

struct Case {
    name: &'static str,
    kind: Kind,
    /// The files of the project the agent works in.
    files: &'static [(&'static str, &'static str)],
    prompt: &'static str,
    calls: Vec<Call>,
    must_not: Vec<Call>,
    /// Calls that have to come in this order, each after the one before.
    order: &'static [&'static str],
    /// Text the last answer has to hold, in any case.
    answer: &'static [&'static str],
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
            effort: Some(
                std::env::var("HOSHI_EVAL_EFFORT")
                    .ok()
                    .and_then(|effort| serde_json::from_value(serde_json::json!(effort)).ok())
                    .unwrap_or(Effort::Low),
            ),
            fast_mode: false,
            ultracode: false,
            access_mode: AccessMode::FullAccess,
            plan_mode: false,
        })
        .unwrap();
    engine
        .dispatch(Command::MessageSend {
            thread_id: THREAD.into(),
            message_id: "00000001-1111-4111-8111-111111111111".into(),
            text: case.prompt.into(),
            attachments: Vec::new(),
        })
        .unwrap();

    let started = Instant::now();
    let mut quiet_since = None::<Instant>;
    let rested = loop {
        thread::sleep(Duration::from_millis(250));
        let snapshot = engine.snapshot();
        let quiet = ended.load(Ordering::Acquire)
            && snapshot.background_tasks.is_empty()
            && snapshot
                .threads
                .iter()
                .all(|thread| thread.status == ThreadStatus::Idle);
        quiet_since = quiet.then(|| quiet_since.unwrap_or_else(Instant::now));
        if quiet_since.is_some_and(|since| since.elapsed() >= QUIET) {
            break true;
        }
        if started.elapsed() >= TIMEOUT {
            break false;
        }
    };
    let mut calls = Vec::new();
    let mut answer = String::new();
    for item in engine.transcript(THREAD, None).unwrap().items {
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
}
