//! JSON-RPC 2.0 over an agent's stdio, one message per line. [`Connection`] sends requests
//! and routes what comes back.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{ChildStdin, ChildStdout};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// What the agent sent that is not an answer to one of ours.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Incoming {
    Notification {
        method: String,
        params: Value,
    },
    /// The agent waits for [`Connection::reply`] with this `id`.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
}

/// The error an agent answered a request with, or why no answer came.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Value,
}

impl RpcError {
    fn local(message: impl Into<String>) -> Self {
        Self {
            code: 0,
            message: message.into(),
            data: Value::Null,
        }
    }

    /// The agent ended before it answered.
    pub fn gone() -> Self {
        Self::local("the agent exited")
    }

    /// The agent's message, with its details when it gave some.
    pub fn detail(&self) -> String {
        match self.data["details"].as_str() {
            Some(details) if !details.is_empty() => format!("{}: {details}", self.message),
            _ => self.message.clone(),
        }
    }
}

type Pending = Arc<Mutex<HashMap<u64, Sender<Result<Value, RpcError>>>>>;

/// One agent process's JSON-RPC channel.
#[derive(Clone)]
pub(crate) struct Connection {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    next_id: Arc<AtomicU64>,
    pending: Pending,
}

impl Connection {
    /// Reads `stdout` on a background thread, handing `incoming` everything that is not an
    /// answer. `ended` runs once the agent stops writing, after the waiting requests failed.
    pub fn start(
        stdin: ChildStdin,
        stdout: ChildStdout,
        mut incoming: impl FnMut(Incoming) + Send + 'static,
        ended: impl FnOnce() + Send + 'static,
    ) -> Self {
        let connection = Self {
            stdin: Arc::new(Mutex::new(Some(stdin))),
            next_id: Arc::new(AtomicU64::new(1)),
            pending: Pending::default(),
        };
        let pending = connection.pending.clone();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if let Some(answer) = route(&message, &pending) {
                    incoming(answer);
                }
            }
            for (_, waiting) in lock(&pending).drain() {
                let _ = waiting.send(Err(RpcError::local("the agent exited")));
            }
            ended();
        });
        connection
    }

    fn write(&self, message: &Value) -> bool {
        let mut stdin = lock(&self.stdin);
        let Some(pipe) = stdin.as_mut() else {
            return false;
        };
        let written = pipe
            .write_all(format!("{message}\n").as_bytes())
            .and_then(|()| pipe.flush());
        if written.is_err() {
            *stdin = None;
        }
        written.is_ok()
    }

    /// Sends a request; its answer arrives on the receiver.
    pub fn request(&self, method: &str, params: Value) -> Receiver<Result<Value, RpcError>> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = mpsc::channel();
        lock(&self.pending).insert(id, sender.clone());
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if !self.write(&message) {
            lock(&self.pending).remove(&id);
            let _ = sender.send(Err(RpcError::local("the agent no longer reads")));
        }
        receiver
    }

    /// Sends a request and waits for its answer, up to `timeout`.
    pub fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, RpcError> {
        self.request(method, params)
            .recv_timeout(timeout)
            .unwrap_or_else(|_| Err(RpcError::local(format!("no answer to {method}"))))
    }

    pub fn notify(&self, method: &str, params: Value) {
        self.write(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    pub fn reply(&self, id: &Value, result: Value) {
        self.write(&json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    }

    pub fn reply_error(&self, id: &Value, code: i64, message: &str) {
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message },
        }));
    }

    /// Closes stdin, which ends an agent that reads it.
    pub fn close(&self) {
        *lock(&self.stdin) = None;
    }
}

/// Settles an answer to one of our requests; anything else comes back.
fn route(message: &Value, pending: &Pending) -> Option<Incoming> {
    let method = message["method"].as_str();
    match (method, message.get("id")) {
        (None, Some(id)) => {
            let waiting = id.as_u64().and_then(|id| lock(pending).remove(&id))?;
            let answer = match message.get("error") {
                Some(error) => Err(RpcError {
                    code: error["code"].as_i64().unwrap_or_default(),
                    message: error["message"].as_str().unwrap_or_default().to_owned(),
                    data: error["data"].clone(),
                }),
                None => Ok(message["result"].clone()),
            };
            let _ = waiting.send(answer);
            None
        }
        (Some(method), Some(id)) => Some(Incoming::Request {
            id: id.clone(),
            method: method.to_owned(),
            params: message["params"].clone(),
        }),
        (Some(method), None) => Some(Incoming::Notification {
            method: method.to_owned(),
            params: message["params"].clone(),
        }),
        (None, None) => None,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Keeps the last `max_bytes` an agent wrote to stderr, for when it dies.
pub(crate) fn stderr_tail(
    stderr: Option<impl Read + Send + 'static>,
    max_bytes: usize,
) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(mut stderr) = stderr {
            let _ = stderr.read_to_end(&mut buffer);
        }
        let start = buffer.len().saturating_sub(max_bytes);
        String::from_utf8_lossy(&buffer[start..]).trim().to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_settle_their_request_and_the_rest_comes_back() {
        let pending = Pending::default();
        let (sender, receiver) = mpsc::channel();
        lock(&pending).insert(7, sender);

        let answer = route(
            &json!({ "jsonrpc": "2.0", "id": 7, "result": { "ok": true } }),
            &pending,
        );
        let request = route(
            &json!({ "jsonrpc": "2.0", "id": 0, "method": "session/request_permission", "params": {} }),
            &pending,
        );
        let failed = route(
            &json!({ "jsonrpc": "2.0", "id": 8, "error": { "code": -32603, "message": "boom" } }),
            &pending,
        );

        assert_eq!(answer, None);
        assert_eq!(receiver.recv().unwrap(), Ok(json!({ "ok": true })));
        assert!(
            matches!(request, Some(Incoming::Request { method, .. }) if method == "session/request_permission")
        );
        assert_eq!(failed, None);
    }
}
