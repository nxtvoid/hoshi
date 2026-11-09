//! One-off control requests: the CLI starts without a prompt, answers, and is
//! stopped, which costs no tokens.

use crate::process::hide_console;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(30);

/// The CLI's answer to `request`, asked from `cwd`. `None` when it can't answer in time.
pub(crate) fn ask(
    binary: &str,
    cwd: &Path,
    request_id: &'static str,
    request: Value,
) -> Option<Value> {
    ask_within(binary, cwd, request_id, request, TIMEOUT)
}

/// Like `ask`, giving up after `timeout`.
pub(crate) fn ask_within(
    binary: &str,
    cwd: &Path,
    request_id: &'static str,
    request: Value,
    timeout: Duration,
) -> Option<Value> {
    let mut command = Command::new(binary);
    command
        .current_dir(cwd)
        .args(["-p", "--input-format", "stream-json"])
        .args(["--output-format", "stream-json", "--verbose"])
        .arg("--no-session-persistence")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    hide_console(&mut command);
    let mut child = command.spawn().ok()?;
    let message = json!({
        "type": "control_request",
        "request_id": request_id,
        "request": request,
    });
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{message}");
    }
    let stdout = child.stdout.take()?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let answer = BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
            .find_map(|line| response(&line, request_id));
        let _ = sender.send(answer);
    });
    let answer = receiver.recv_timeout(timeout).ok().flatten();
    let _ = child.kill();
    let _ = child.wait();
    answer
}

/// The successful answer to `request_id` in a line of output; other lines are `None`.
pub(crate) fn response(line: &str, request_id: &str) -> Option<Value> {
    let mut value: Value = serde_json::from_str(line).ok()?;
    let response = &value["response"];
    if value["type"] != "control_response"
        || response["request_id"] != request_id
        || response["subtype"] != "success"
    {
        return None;
    }
    Some(value["response"]["response"].take())
}
