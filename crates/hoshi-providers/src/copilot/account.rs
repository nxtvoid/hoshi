//! Whether Copilot is signed in, as its headless server's `auth.getStatus` says. It weighs
//! every source the CLI signs in with (`copilot login`, the GitHub CLI's login, a token in the
//! environment) and checks the token, so it answers what a turn would find.

use crate::process::kill_tree;
use hoshi_core::ProviderAccountStatus;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::Duration;

const STATUS_TIMEOUT: Duration = Duration::from_secs(20);
const STATUS_REQUEST: u64 = 1;

/// Who Copilot runs as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Account {
    pub status: ProviderAccountStatus,
    /// The GitHub login, when signed in.
    pub login: Option<String>,
}

/// Asks `command`, the CLI, in headless mode. `None` when it gave no answer.
pub(crate) fn read(mut command: Command) -> Option<Account> {
    command
        .args(["--headless", "--stdio", "--no-auto-update"])
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let pid = child.id();
    let child = Mutex::new(child);
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(answer(&mut BufReader::new(stdout), STATUS_REQUEST));
    });
    let request = json!({
        "jsonrpc": "2.0",
        "id": STATUS_REQUEST,
        "method": "auth.getStatus",
        "params": {},
    });
    let written = stdin
        .write_all(&frame(&request))
        .and_then(|()| stdin.flush());
    let status = written
        .ok()
        .and_then(|()| receiver.recv_timeout(STATUS_TIMEOUT).ok().flatten());
    drop(stdin);
    kill_tree(pid, &child);
    let _ = child.lock().map(|mut child| child.wait());
    account(&status?)
}

/// The account an `auth.getStatus` result describes.
fn account(status: &Value) -> Option<Account> {
    let signed_in = status["isAuthenticated"].as_bool()?;
    Some(Account {
        status: if signed_in {
            ProviderAccountStatus::Ready
        } else {
            ProviderAccountStatus::SignedOut
        },
        login: status["login"]
            .as_str()
            .filter(|login| signed_in && !login.is_empty())
            .map(str::to_owned),
    })
}

/// A message as the headless server reads them: a `Content-Length` header, then the JSON.
fn frame(message: &Value) -> Vec<u8> {
    let body = message.to_string();
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

/// The result of request `id`, skipping whatever else the server sends first.
fn answer(reader: &mut impl BufRead, id: u64) -> Option<Value> {
    loop {
        let message = next_message(reader)?;
        if message.get("method").is_none() && message["id"].as_u64() == Some(id) {
            return message.get("result").cloned();
        }
    }
}

fn next_message(reader: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            if length.is_some() {
                break;
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; length?];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn a_signed_in_status_names_the_login() {
        let status = json!({
            "isAuthenticated": true,
            "authType": "gh-cli",
            "host": "https://github.com",
            "statusMessage": "octocat (via gh)",
            "login": "octocat",
        });
        assert_eq!(
            account(&status),
            Some(Account {
                status: ProviderAccountStatus::Ready,
                login: Some("octocat".into()),
            })
        );
    }

    #[test]
    fn a_rejected_token_reads_as_signed_out() {
        let status = json!({ "isAuthenticated": false, "statusMessage": "Not authenticated" });
        assert_eq!(
            account(&status),
            Some(Account {
                status: ProviderAccountStatus::SignedOut,
                login: None,
            })
        );
        assert_eq!(account(&json!({ "error": "unknown" })), None);
    }

    #[test]
    fn the_answer_is_found_among_the_servers_messages() {
        let status = json!({ "jsonrpc": "2.0", "id": 1, "result": { "isAuthenticated": true } });
        let other = json!({ "jsonrpc": "2.0", "id": 2, "result": { "version": "1.0.112" } });
        let event = json!({ "jsonrpc": "2.0", "method": "session.event", "params": {} });
        let mut stream = Vec::new();
        for message in [&event, &other, &status] {
            stream.extend(frame(message));
        }

        let answer = answer(&mut Cursor::new(stream), 1);

        assert_eq!(answer, Some(json!({ "isAuthenticated": true })));
    }

    #[test]
    fn a_server_that_ends_first_gives_no_answer() {
        let mut stream = frame(&json!({ "jsonrpc": "2.0", "id": 2, "result": {} }));
        stream.extend(b"Content-Length: 40\r\n\r\n{\"jsonrpc\"");
        assert_eq!(answer(&mut Cursor::new(stream), 1), None);
    }
}
