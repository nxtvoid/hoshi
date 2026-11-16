//! Thread titles from a short, tool-less Claude Code run on a small model.

use super::models::manifest;
use crate::process::hide_console;
use crate::titles::{parse, prompt};
use crate::{GeneratedTitle, TitleRequest};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(90);
/// The quick model titles use unless the user picked another.
const DEFAULT_MODEL: &str = "haiku";

/// What the model must answer with.
fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": { "type": "string" },
            "needsRefinement": { "type": "boolean" },
        },
        "required": ["title", "needsRefinement"],
        "additionalProperties": false,
    })
}

/// Runs the CLI outside any project, without tools, hooks or a saved session.
pub(crate) fn generate(binary: &str, request: &TitleRequest) -> Option<GeneratedTitle> {
    let model = request.model.as_deref().unwrap_or(DEFAULT_MODEL);
    parse(&run_quick(
        binary,
        model,
        &schema(),
        &prompt(request),
        TIMEOUT,
    )?)
}

/// One tool-less run of `model` at low effort that answers `prompt` as `schema` says;
/// what the CLI printed, or `None` when it failed or took longer than `timeout`.
pub(crate) fn run_quick(
    binary: &str,
    model: &str,
    schema: &Value,
    prompt: &str,
    timeout: Duration,
) -> Option<String> {
    let model = manifest().resolve(Some(model), Some(hoshi_core::Effort::Low), false, false);
    let mut command = Command::new(binary);
    command
        .current_dir(std::env::temp_dir())
        .args(["-p", "--output-format", "json"])
        .args(["--json-schema", &schema.to_string()])
        .args(["--model", &model.slug])
        .args([
            "--settings",
            &json!({ "disableAllHooks": true }).to_string(),
        ])
        .args(["--tools", ""])
        .arg("--disable-slash-commands")
        .arg("--no-session-persistence")
        .args(["--permission-mode", "dontAsk"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(effort) = &model.effort {
        command.args(["--effort", effort]);
    }
    hide_console(&mut command);
    let mut child = command.spawn().ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(prompt.as_bytes());
    }
    let mut stdout = child.stdout.take()?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut output = String::new();
        let _ = stdout.read_to_string(&mut output);
        let _ = sender.send(output);
    });
    let output = receiver.recv_timeout(timeout).ok();
    let _ = child.kill();
    let _ = child.wait();
    output
}
