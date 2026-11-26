//! One-off control requests: the CLI starts without a prompt, answers, and exits, which
//! costs no tokens.

use crate::process::hide_console;
use serde_json::{Value, json};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(30);
const EXIT_GRACE: Duration = Duration::from_secs(2);
const KILL_GRACE: Duration = Duration::from_secs(5);
const EXIT_POLL: Duration = Duration::from_millis(100);

/// The CLI's answer to `request`, asked from `cwd`. `None` when it can't answer in time.
pub(crate) fn ask(
    binary: &Path,
    cwd: &Path,
    request_id: &'static str,
    request: Value,
) -> Option<Value> {
    ask_within(binary, cwd, request_id, request, TIMEOUT)
}

/// Like `ask`, giving up after `timeout`.
pub(crate) fn ask_within(
    binary: &Path,
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
        .arg("--no-session-persistence");
    ask_command(command, request_id, request, timeout)
}

fn ask_command(
    mut command: Command,
    request_id: &'static str,
    request: Value,
    timeout: Duration,
) -> Option<Value> {
    command
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
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::new();
        let mut answer = None;
        while answer.is_none() && matches!(reader.read_until(b'\n', &mut line), Ok(1..)) {
            answer = response(&String::from_utf8_lossy(&line), request_id);
            line.clear();
        }
        let _ = sender.send(answer);
        let _ = io::copy(&mut reader, &mut io::sink());
    });
    let answer = receiver.recv_timeout(timeout).ok().flatten();
    thread::spawn(move || let_exit(child));
    answer
}

/// Ends a CLI whose stdin is closed as the Agent SDK does: it gets time to exit on its own,
/// then is asked to, and only then killed. Killed at once, it could die mid-way through
/// refreshing its sign-in, after the server spent the old token and before the new one was
/// saved, which signs every Claude Code out.
pub(crate) fn let_exit(child: Child) {
    end_within(child, EXIT_GRACE, KILL_GRACE);
}

fn end_within(mut child: Child, exit_grace: Duration, kill_grace: Duration) {
    if exits_within(&mut child, exit_grace) {
        return;
    }
    #[cfg(unix)]
    if let Some(pid) = i32::try_from(child.id())
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    {
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
    }
    if !exits_within(&mut child, kill_grace) {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn exits_within(child: &mut Child, grace: Duration) -> bool {
    let deadline = Instant::now() + grace;
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return true,
            Ok(None) if Instant::now() >= deadline => return false,
            Ok(None) => thread::sleep(EXIT_POLL),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_processes::tests::long_running;

    #[test]
    fn a_cli_that_exits_on_its_own_is_not_signalled() {
        let mut command = if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/c", "exit"]);
            command
        } else {
            Command::new("true")
        };
        let started = Instant::now();

        end_within(command.spawn().unwrap(), Duration::from_secs(5), KILL_GRACE);

        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_cli_that_keeps_running_is_asked_to_end_then_killed() {
        let exit_grace = Duration::from_millis(200);
        let kill_grace = Duration::from_millis(300);
        let started = Instant::now();

        end_within(long_running().spawn().unwrap(), exit_grace, kill_grace);

        let elapsed = started.elapsed();
        assert!(elapsed >= exit_grace, "{elapsed:?}");
        if cfg!(unix) {
            assert!(
                elapsed < exit_grace + kill_grace,
                "TERM ends it: {elapsed:?}"
            );
        }
    }

    /// A CLI that answers, then writes more than a pipe holds before it exits.
    fn chatty_cli(marker: &Path) -> Command {
        let answer = json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": "hoshi-test", "response": { "ok": true } },
        });
        let mut command = if cfg!(windows) {
            let mut command = Command::new("powershell");
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::Out.WriteLine($env:ANSWER); [Console]::Out.Flush(); \
                 Start-Sleep -Seconds 1; $line = 'x' * 100; \
                 for ($i = 0; $i -lt 3000; $i++) { [Console]::Out.WriteLine($line) }; \
                 [Console]::Out.Flush(); New-Item -ItemType File -Path $env:MARKER | Out-Null",
            ]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args([
                "-c",
                "read line; printf '%s\\n' \"$ANSWER\"; sleep 1; \
                 i=0; while [ $i -lt 3000 ]; do printf '%0100d\\n' 0; i=$((i+1)); done; \
                 touch \"$MARKER\"",
            ]);
            command
        };
        command
            .env("ANSWER", answer.to_string())
            .env("MARKER", marker);
        command
    }

    #[test]
    fn the_answer_comes_back_at_once_and_the_cli_still_finishes_writing() {
        let marker = std::env::temp_dir().join(format!("hoshi-control-{}", uuid::Uuid::new_v4()));

        let answer = ask_command(chatty_cli(&marker), "hoshi-test", json!({}), TIMEOUT);

        assert_eq!(answer, Some(json!({ "ok": true })));
        assert!(!marker.exists(), "answered before the CLI ended");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !marker.exists() && Instant::now() < deadline {
            thread::sleep(EXIT_POLL);
        }
        assert!(marker.exists(), "the CLI was cut off before it ended");
        let _ = std::fs::remove_file(marker);
    }
}
