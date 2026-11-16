//! A Chrome DevTools Protocol connection over the browser's WebSocket: calls that wait for
//! their answer, and events for whoever listens.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;
use std::time::Duration;
use tungstenite::{Message, WebSocket};

const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the socket thread waits for the browser before sending what is queued.
const POLL: Duration = Duration::from_millis(4);

type Reply = mpsc::Sender<Result<Value, String>>;

/// Something the browser reported on its own.
#[derive(Debug, Clone)]
pub struct CdpEvent {
    pub method: String,
    pub params: Value,
    /// The page session it came from; `None` for the browser's own.
    pub session_id: Option<String>,
}

pub struct Connection {
    outgoing: mpsc::Sender<String>,
    pending: Arc<Mutex<HashMap<u64, Reply>>>,
    listeners: Arc<Mutex<Vec<mpsc::Sender<CdpEvent>>>>,
    next_id: AtomicU64,
    /// Until the socket closes, as when the browser quits or crashes.
    open: Arc<AtomicBool>,
}

impl Connection {
    /// Connects to `url`, the `ws://` address the browser printed when it started.
    pub fn connect(url: &str) -> Result<Self, String> {
        let address = url
            .strip_prefix("ws://")
            .and_then(|rest| rest.split('/').next())
            .ok_or_else(|| format!("not a DevTools address: {url}"))?;
        let stream = TcpStream::connect(address).map_err(|error| error.to_string())?;
        let (socket, _) = tungstenite::client(url, stream).map_err(|error| error.to_string())?;
        socket
            .get_ref()
            .set_read_timeout(Some(POLL))
            .map_err(|error| error.to_string())?;
        let (outgoing, queued) = mpsc::channel::<String>();
        let pending = Arc::<Mutex<HashMap<u64, Reply>>>::default();
        let listeners = Arc::<Mutex<Vec<mpsc::Sender<CdpEvent>>>>::default();
        let open = Arc::new(AtomicBool::new(true));
        let (waiting, listening, running) = (pending.clone(), listeners.clone(), open.clone());
        thread::spawn(move || {
            run(socket, &queued, &waiting, &listening);
            drop(queued);
            running.store(false, Ordering::Release);
            for (_, reply) in lock(&waiting).drain() {
                let _ = reply.send(Err("the browser closed".into()));
            }
        });
        Ok(Self {
            outgoing,
            pending,
            listeners,
            next_id: AtomicU64::new(1),
            open,
        })
    }

    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    /// Calls `method` on the browser, or on a page when `session` names its session.
    pub fn call(
        &self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value, String> {
        self.call_within(method, params, session, CALL_TIMEOUT)
    }

    pub fn call_within(
        &self,
        method: &str,
        params: Value,
        session: Option<&str>,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        let (reply, answer) = mpsc::channel();
        lock(&self.pending).insert(id, reply);
        if self.outgoing.send(message.to_string()).is_err() {
            lock(&self.pending).remove(&id);
            return Err("the browser closed".into());
        }
        match answer.recv_timeout(timeout) {
            Ok(result) => result,
            Err(_) => {
                lock(&self.pending).remove(&id);
                Err(format!("the browser didn't answer {method} in time"))
            }
        }
    }

    /// Every event the browser reports from now on.
    pub fn subscribe(&self) -> mpsc::Receiver<CdpEvent> {
        let (sender, receiver) = mpsc::channel();
        lock(&self.listeners).push(sender);
        receiver
    }
}

#[cfg(test)]
impl Connection {
    /// A connection to no browser, whose events the test reports itself.
    pub fn offline() -> Self {
        let (outgoing, _) = mpsc::channel();
        Self {
            outgoing,
            pending: Arc::default(),
            listeners: Arc::default(),
            next_id: AtomicU64::new(1),
            open: Arc::default(),
        }
    }

    /// A connection to a browser that takes every call and never answers.
    pub fn unanswered() -> (Self, mpsc::Receiver<String>) {
        let (outgoing, sent) = mpsc::channel();
        let connection = Self {
            outgoing,
            pending: Arc::default(),
            listeners: Arc::default(),
            next_id: AtomicU64::new(1),
            open: Arc::new(AtomicBool::new(true)),
        };
        (connection, sent)
    }

    /// Hands `event` to every listener still there; returns how many took it.
    pub fn report(&self, event: &CdpEvent) -> usize {
        let mut listeners = lock(&self.listeners);
        listeners.retain(|listener| listener.send(event.clone()).is_ok());
        listeners.len()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Owns the socket: sends what is queued, reads what the browser says, until either closes.
fn run(
    mut socket: WebSocket<TcpStream>,
    queued: &mpsc::Receiver<String>,
    pending: &Mutex<HashMap<u64, Reply>>,
    listeners: &Mutex<Vec<mpsc::Sender<CdpEvent>>>,
) {
    'serve: loop {
        loop {
            match queued.try_recv() {
                Ok(text) => {
                    if socket.send(Message::text(text)).is_err() {
                        break 'serve;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => break 'serve,
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => dispatch(text.as_str(), pending, listeners),
            Ok(Message::Close(frame)) => {
                if std::env::var_os("HOSHI_BROWSER_LOG").is_some() {
                    eprintln!("DEBUG cdp: close frame {frame:?}");
                }
                break;
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => {
                if std::env::var_os("HOSHI_BROWSER_LOG").is_some() {
                    eprintln!("DEBUG cdp: read error {error:?}");
                }
                break;
            }
        }
    }
}

fn dispatch(
    text: &str,
    pending: &Mutex<HashMap<u64, Reply>>,
    listeners: &Mutex<Vec<mpsc::Sender<CdpEvent>>>,
) {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return;
    };
    if let Some(id) = value["id"].as_u64() {
        if let Some(reply) = lock(pending).remove(&id) {
            let result = match value.get("error") {
                Some(error) => Err(error["message"]
                    .as_str()
                    .unwrap_or("the browser refused")
                    .to_owned()),
                None => Ok(value["result"].clone()),
            };
            let _ = reply.send(result);
        }
        return;
    }
    let Some(method) = value["method"].as_str() else {
        return;
    };
    let event = CdpEvent {
        method: method.to_owned(),
        params: value["params"].clone(),
        session_id: value["sessionId"].as_str().map(str::to_owned),
    };
    lock(listeners).retain(|listener| listener.send(event.clone()).is_ok());
}

#[cfg(test)]
mod tests {
    use super::Connection;
    use serde_json::json;
    use std::time::{Duration, Instant};

    #[test]
    fn a_call_the_browser_never_answers_gives_up_in_time() {
        let (connection, sent) = Connection::unanswered();
        let started = Instant::now();

        let result = connection.call_within(
            "Page.navigate",
            json!({}),
            Some("page"),
            Duration::from_millis(200),
        );

        assert_eq!(
            result,
            Err("the browser didn't answer Page.navigate in time".into())
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(sent.try_recv().unwrap().contains("Page.navigate"));
        assert!(connection.pending.lock().unwrap().is_empty());
    }

    #[test]
    fn a_closed_browser_fails_calls_at_once() {
        let connection = Connection::offline();
        let started = Instant::now();
        let result = connection.call("Page.navigate", json!({}), None);
        assert_eq!(result, Err("the browser closed".into()));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
