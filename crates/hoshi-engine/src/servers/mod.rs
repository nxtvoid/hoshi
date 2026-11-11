//! The localhost servers each thread's agent and terminals left listening. Their processes
//! inherit the thread's id from the agent or shell that started them, even once detached,
//! so a listener's owner is read from its process's environment.

mod procfs;
#[cfg(windows)]
mod windows;

use crate::{Notification, Notify};
use hoshi_core::{CommandError, LocalServer, ServerOrigin, ThreadServers};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

pub(crate) const THREAD_VARIABLE: &str = "HOSHI_THREAD_ID";
pub(crate) const TERMINAL_VARIABLE: &str = "HOSHI_TERMINAL_ID";
const SCAN_EVERY: Duration = Duration::from_secs(3);
const STOP_GRACE: Duration = Duration::from_secs(3);
const STOP_POLL: Duration = Duration::from_millis(250);
/// How much earlier than its command a server may seem to start, as the two clocks tell it.
const START_SLACK_MS: u64 = 2_000;

/// What the agent of `thread_id` starts with. A hoshi run from a terminal of another hoshi
/// would otherwise pass that terminal's id on to its agents.
pub(crate) fn agent_environment(thread_id: &str) -> Vec<(String, String)> {
    vec![
        (THREAD_VARIABLE.to_owned(), thread_id.to_owned()),
        (TERMINAL_VARIABLE.to_owned(), String::new()),
    ]
}

/// What a terminal's shell starts with.
pub(crate) fn terminal_environment(
    thread_id: &str,
    terminal_id: &str,
) -> [(&'static str, String); 2] {
    [
        (THREAD_VARIABLE, thread_id.to_owned()),
        (TERMINAL_VARIABLE, terminal_id.to_owned()),
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Owner {
    pub thread_id: String,
    pub origin: ServerOrigin,
}

/// The thread a process belongs to, from its `NAME=value` environment entries.
pub(crate) fn owner_from_environment<I, S>(entries: I) -> Option<Owner>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut thread_id = None;
    let mut terminal = false;
    for entry in entries {
        let Some((name, value)) = entry.as_ref().split_once('=') else {
            continue;
        };
        let value = value.trim();
        if name == THREAD_VARIABLE && !value.is_empty() {
            thread_id = Some(value.to_owned());
        } else if name == TERMINAL_VARIABLE && !value.is_empty() {
            terminal = true;
        }
    }
    thread_id.map(|thread_id| Owner {
        thread_id,
        origin: if terminal {
            ServerOrigin::Terminal
        } else {
            ServerOrigin::Agent
        },
    })
}

/// A loopback or wildcard TCP socket in the listening state; `pid` is `None` when no process
/// of a thread holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Listener {
    pub port: u16,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Process {
    pub owner: Option<Owner>,
    pub name: Option<String>,
    /// Unix milliseconds.
    pub started_at: Option<u64>,
    pub parent: Option<u32>,
}

/// One read of the listeners and the processes holding them.
#[derive(Debug, Default)]
pub(crate) struct Seen {
    pub listeners: Vec<Listener>,
    pub processes: HashMap<u32, Process>,
}

/// Why a server could not be stopped.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StopFailure {
    Refused,
    Failed(String),
}

/// Each thread's servers, by port. A port also held by a process of no thread or of
/// another one stays out, as do hoshi's own and its direct children's: the agents and shells.
pub(crate) fn attribute(seen: &Seen, own_pid: u32) -> HashMap<String, Vec<LocalServer>> {
    let mut ports: BTreeMap<u16, Vec<Option<u32>>> = BTreeMap::new();
    for listener in &seen.listeners {
        ports.entry(listener.port).or_default().push(listener.pid);
    }
    let mut threads: HashMap<String, Vec<LocalServer>> = HashMap::new();
    for (port, pids) in ports {
        let mut held: Option<(u32, &Process, &Owner)> = None;
        let shared = pids.iter().any(|pid| {
            let Some(pid) = *pid else { return true };
            let Some(process) = seen.processes.get(&pid) else {
                return true;
            };
            let Some(owner) = &process.owner else {
                return true;
            };
            if pid == own_pid || process.parent == Some(own_pid) {
                return true;
            }
            match held {
                Some((_, _, first)) => first.thread_id != owner.thread_id,
                None => {
                    held = Some((pid, process, owner));
                    false
                }
            }
        });
        let Some((pid, process, owner)) = held.filter(|_| !shared) else {
            continue;
        };
        threads
            .entry(owner.thread_id.clone())
            .or_default()
            .push(LocalServer {
                port,
                pid,
                process_name: process.name.clone(),
                origin: owner.origin,
                started_at: process.started_at,
            });
    }
    for servers in threads.values_mut() {
        without_helper_ports(servers);
    }
    threads
}

/// The first port the system hands a socket that asked for none in particular.
#[cfg(target_os = "linux")]
const ASSIGNED_PORTS_FROM: u16 = 32_768;
#[cfg(not(target_os = "linux"))]
const ASSIGNED_PORTS_FROM: u16 = 49_152;

/// Leaves out the ports a server opened for its own use: where a process holds a port it
/// chose, the ones the system assigned it are its helpers', as a dev server's workers.
fn without_helper_ports(servers: &mut Vec<LocalServer>) {
    let chose: Vec<u32> = servers
        .iter()
        .filter(|server| server.port < ASSIGNED_PORTS_FROM)
        .map(|server| server.pid)
        .collect();
    servers.retain(|server| server.port < ASSIGNED_PORTS_FROM || !chose.contains(&server.pid));
}

/// `pid`'s parent, its parent's parent and so on.
pub(crate) fn ancestors(parents: &HashMap<u32, u32>, pid: u32) -> Vec<u32> {
    let mut found = Vec::new();
    let mut current = pid;
    while let Some(&parent) = parents.get(&current) {
        if parent == 0 || parent == current || found.contains(&parent) {
            break;
        }
        found.push(parent);
        current = parent;
    }
    found
}

/// `pid` and every process under it.
pub(crate) fn descendants(parents: &HashMap<u32, u32>, pid: u32) -> Vec<u32> {
    let mut found = vec![pid];
    let mut next = 0;
    while next < found.len() {
        let current = found[next];
        next += 1;
        for (&child, &parent) in parents {
            if parent == current && child != current && !found.contains(&child) {
                found.push(child);
            }
        }
    }
    found
}

/// Ending `pid` and what it started spares hoshi, the agents and the shells: it is none of
/// them, nor one of hoshi's ancestors, which would take hoshi down with it.
pub(crate) fn may_stop_tree(parents: &HashMap<u32, u32>, pid: u32, own_pid: u32) -> bool {
    pid > 1
        && pid != own_pid
        && parents.get(&pid) != Some(&own_pid)
        && !ancestors(parents, own_pid).contains(&pid)
}

trait Platform: Send {
    /// `None` when the listeners could not be read.
    fn scan(&mut self) -> Option<Seen>;
    /// Forgets what it kept between scans, as pids may be reused while nobody looks.
    fn reset(&mut self);
    fn stop(&mut self, pid: u32, own_pid: u32) -> Result<(), StopFailure>;
}

#[cfg(target_os = "linux")]
fn platform() -> Box<dyn Platform> {
    Box::new(procfs::ProcFs::new("/proc"))
}

#[cfg(windows)]
fn platform() -> Box<dyn Platform> {
    Box::new(windows::Windows::new())
}

#[cfg(not(any(target_os = "linux", windows)))]
fn platform() -> Box<dyn Platform> {
    struct Unsupported;
    impl Platform for Unsupported {
        fn scan(&mut self) -> Option<Seen> {
            Some(Seen::default())
        }
        fn reset(&mut self) {}
        fn stop(&mut self, _pid: u32, _own_pid: u32) -> Result<(), StopFailure> {
            Err(StopFailure::Refused)
        }
    }
    Box::new(Unsupported)
}

struct Scanner {
    platform: Box<dyn Platform>,
    own_pid: u32,
}

impl Scanner {
    fn scan(&mut self) -> HashMap<String, Vec<LocalServer>> {
        self.platform
            .scan()
            .map(|seen| attribute(&seen, self.own_pid))
            .unwrap_or_default()
    }
}

#[derive(Default)]
struct Watching {
    /// How many views show each thread's servers.
    viewers: HashMap<String, usize>,
    /// What each watched thread was last sent.
    sent: HashMap<String, Vec<LocalServer>>,
    ticking: bool,
}

struct Shared {
    notify: Notify,
    watching: Mutex<Watching>,
    scanner: Mutex<Scanner>,
}

/// Scans while a view watches some thread's servers, and tells it when they change.
#[derive(Clone)]
pub struct Servers {
    shared: Arc<Shared>,
}

impl Servers {
    pub fn new(notify: Notify) -> Self {
        Self::with_platform(notify, platform(), std::process::id())
    }

    fn with_platform(notify: Notify, platform: Box<dyn Platform>, own_pid: u32) -> Self {
        Self {
            shared: Arc::new(Shared {
                notify,
                watching: Mutex::default(),
                scanner: Mutex::new(Scanner { platform, own_pid }),
            }),
        }
    }

    /// The thread's servers now; changes follow as notifications until `unwatch`.
    pub fn watch(&self, thread_id: &str) -> Vec<LocalServer> {
        let start = {
            let mut watching = guard(&self.shared.watching);
            *watching.viewers.entry(thread_id.to_owned()).or_default() += 1;
            !std::mem::replace(&mut watching.ticking, true)
        };
        let servers = guard(&self.shared.scanner)
            .scan()
            .remove(thread_id)
            .unwrap_or_default();
        {
            let mut watching = guard(&self.shared.watching);
            if watching.viewers.contains_key(thread_id) {
                watching.sent.insert(thread_id.to_owned(), servers.clone());
            }
        }
        if start {
            let shared = self.shared.clone();
            thread::spawn(move || tick(&shared));
        }
        servers
    }

    pub fn unwatch(&self, thread_id: &str) {
        let mut watching = guard(&self.shared.watching);
        let Some(viewers) = watching.viewers.get_mut(thread_id) else {
            return;
        };
        *viewers -= 1;
        if *viewers == 0 {
            watching.viewers.remove(thread_id);
            watching.sent.remove(thread_id);
        }
    }

    /// Ends the servers the thread's agent started at `since` or later, in Unix
    /// milliseconds: what a command it was just told to stop left listening.
    pub fn stop_agent_servers_since(&self, thread_id: &str, since: u64) {
        {
            let mut scanner = guard(&self.shared.scanner);
            let servers = scanner.scan().remove(thread_id).unwrap_or_default();
            let own_pid = scanner.own_pid;
            let mut ended = Vec::new();
            for server in servers {
                let later = server
                    .started_at
                    .is_some_and(|started| started + START_SLACK_MS >= since);
                if server.origin == ServerOrigin::Agent && later && !ended.contains(&server.pid) {
                    let _ = scanner.platform.stop(server.pid, own_pid);
                    ended.push(server.pid);
                }
            }
        }
        publish(&self.shared);
    }

    /// Ends the process holding `port` and what it started, if it still is the thread's.
    pub fn stop(&self, thread_id: &str, pid: u32, port: u16) -> Result<(), CommandError> {
        let result = {
            let mut scanner = guard(&self.shared.scanner);
            let listed = scanner.scan().get(thread_id).is_some_and(|servers| {
                servers
                    .iter()
                    .any(|server| server.pid == pid && server.port == port)
            });
            if !listed {
                return Err(CommandError::ServerNotFound {
                    thread_id: thread_id.to_owned(),
                    port,
                });
            }
            let own_pid = scanner.own_pid;
            scanner.platform.stop(pid, own_pid)
        };
        publish(&self.shared);
        result.map_err(|failure| match failure {
            StopFailure::Refused => CommandError::ServerStopRefused { port },
            StopFailure::Failed(detail) => CommandError::ServerStopFailed { detail },
        })
    }
}

fn tick(shared: &Shared) {
    loop {
        thread::sleep(SCAN_EVERY);
        {
            let mut watching = guard(&shared.watching);
            if watching.viewers.is_empty() {
                watching.ticking = false;
                drop(watching);
                guard(&shared.scanner).platform.reset();
                return;
            }
        }
        publish(shared);
    }
}

/// Scans and tells each watched thread whose servers changed.
fn publish(shared: &Shared) {
    let mut found = guard(&shared.scanner).scan();
    let changed: Vec<ThreadServers> = {
        let mut watching = guard(&shared.watching);
        let threads: Vec<String> = watching.viewers.keys().cloned().collect();
        threads
            .into_iter()
            .filter_map(|thread_id| {
                let servers = found.remove(&thread_id).unwrap_or_default();
                if watching.sent.get(&thread_id) == Some(&servers) {
                    return None;
                }
                watching.sent.insert(thread_id.clone(), servers.clone());
                Some(ThreadServers { thread_id, servers })
            })
            .collect()
    };
    for update in changed {
        (shared.notify)(Notification::Servers(update));
    }
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(thread_id: &str, origin: ServerOrigin) -> Option<Owner> {
        Some(Owner {
            thread_id: thread_id.into(),
            origin,
        })
    }

    fn process(owner: Option<Owner>, parent: u32) -> Process {
        Process {
            owner,
            name: Some("java".into()),
            started_at: Some(1_000),
            parent: Some(parent),
        }
    }

    fn listener(port: u16, pid: u32) -> Listener {
        Listener {
            port,
            pid: Some(pid),
        }
    }

    #[test]
    fn the_thread_variable_names_the_owner_and_a_terminal_id_says_a_terminal_started_it() {
        assert_eq!(
            owner_from_environment(["PATH=/bin", "HOSHI_THREAD_ID=t1"]),
            owner("t1", ServerOrigin::Agent)
        );
        assert_eq!(
            owner_from_environment(["HOSHI_TERMINAL_ID=term-2", "HOSHI_THREAD_ID=t1"]),
            owner("t1", ServerOrigin::Terminal)
        );
        assert_eq!(
            owner_from_environment(
                agent_environment("t3")
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
            ),
            owner("t3", ServerOrigin::Agent)
        );
        assert_eq!(owner_from_environment(["HOSHI_THREAD_ID="]), None);
        assert_eq!(owner_from_environment(["OTHER=1"]), None);
    }

    #[test]
    fn listeners_of_a_thread_s_processes_become_its_servers_by_port() {
        let seen = Seen {
            listeners: vec![listener(8080, 20), listener(8080, 20), listener(3000, 30)],
            processes: HashMap::from([
                (20, process(owner("t1", ServerOrigin::Agent), 15)),
                (30, process(owner("t2", ServerOrigin::Terminal), 16)),
            ]),
        };

        let threads = attribute(&seen, 1);

        assert_eq!(
            threads["t1"],
            [LocalServer {
                port: 8080,
                pid: 20,
                process_name: Some("java".into()),
                origin: ServerOrigin::Agent,
                started_at: Some(1_000),
            }]
        );
        assert_eq!(threads["t2"][0].port, 3000);
        assert_eq!(threads["t2"][0].origin, ServerOrigin::Terminal);
    }

    #[test]
    fn a_port_shared_with_a_process_of_no_thread_or_another_thread_stays_out() {
        let seen = Seen {
            listeners: vec![
                listener(8080, 20),
                Listener {
                    port: 8080,
                    pid: None,
                },
                listener(9000, 20),
                listener(9000, 40),
                listener(9100, 20),
                listener(9100, 30),
            ],
            processes: HashMap::from([
                (20, process(owner("t1", ServerOrigin::Agent), 15)),
                (30, process(owner("t2", ServerOrigin::Agent), 15)),
                (40, process(None, 15)),
            ]),
        };

        assert!(attribute(&seen, 1).is_empty());
    }

    #[test]
    fn the_ports_a_server_was_assigned_for_its_helpers_stay_out() {
        let seen = Seen {
            listeners: vec![
                listener(3000, 20),
                listener(53_857, 20),
                listener(53_869, 20),
                listener(60_000, 30),
                listener(60_001, 30),
            ],
            processes: HashMap::from([
                (20, process(owner("t1", ServerOrigin::Agent), 15)),
                (30, process(owner("t1", ServerOrigin::Agent), 15)),
            ]),
        };

        let ports: Vec<(u16, u32)> = attribute(&seen, 1)["t1"]
            .iter()
            .map(|server| (server.port, server.pid))
            .collect();

        assert_eq!(ports, [(3000, 20), (60_000, 30), (60_001, 30)]);
    }

    #[test]
    fn hoshi_and_its_agents_and_shells_are_not_servers() {
        let seen = Seen {
            listeners: vec![listener(5000, 1), listener(5001, 10)],
            processes: HashMap::from([
                (1, process(owner("t1", ServerOrigin::Agent), 0)),
                (10, process(owner("t1", ServerOrigin::Agent), 1)),
            ]),
        };

        assert!(attribute(&seen, 1).is_empty());
    }

    #[test]
    fn a_tree_may_stop_unless_it_holds_hoshi_or_is_one_of_its_children() {
        let parents = HashMap::from([(100, 1), (200, 100), (300, 200), (400, 300), (500, 300)]);

        assert!(may_stop_tree(&parents, 400, 200));
        assert!(!may_stop_tree(&parents, 200, 200));
        assert!(!may_stop_tree(&parents, 300, 200));
        assert!(!may_stop_tree(&parents, 100, 200));
        assert!(!may_stop_tree(&parents, 1, 200));
        let mut tree = descendants(&parents, 300);
        tree.sort_unstable();
        assert_eq!(tree, [300, 400, 500]);
    }

    struct Fake {
        seen: Arc<Mutex<Vec<Listener>>>,
    }

    impl Platform for Fake {
        fn scan(&mut self) -> Option<Seen> {
            Some(Seen {
                listeners: guard(&self.seen).clone(),
                processes: HashMap::from([
                    (20, process(owner("t1", ServerOrigin::Agent), 15)),
                    (21, process(owner("t1", ServerOrigin::Terminal), 16)),
                    (22, process(owner("t2", ServerOrigin::Agent), 17)),
                ]),
            })
        }
        fn reset(&mut self) {}
        fn stop(&mut self, pid: u32, _own_pid: u32) -> Result<(), StopFailure> {
            guard(&self.seen).retain(|listener| listener.pid != Some(pid));
            Ok(())
        }
    }

    #[test]
    fn only_the_agent_s_servers_started_since_its_command_are_ended_with_it() {
        let notify: Notify = Arc::new(|_| {});
        let seen = Arc::new(Mutex::new(vec![
            listener(8080, 20),
            listener(5173, 21),
            listener(9000, 22),
        ]));
        let servers = Servers::with_platform(notify, Box::new(Fake { seen: seen.clone() }), 1);
        let left = || -> Vec<u16> { guard(&seen).iter().map(|listener| listener.port).collect() };

        servers.stop_agent_servers_since("t1", 10_000);
        assert_eq!(left(), [8080, 5173, 9000]);
        servers.stop_agent_servers_since("t1", 2_500);
        assert_eq!(left(), [5173, 9000]);
    }

    #[test]
    fn a_watched_thread_hears_its_servers_leave_once_stopped() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let notify: Notify = Arc::new(move |notification| {
            if let Notification::Servers(update) = notification {
                let _ = sender.send(update);
            }
        });
        let seen = Arc::new(Mutex::new(vec![listener(8080, 20)]));
        let servers = Servers::with_platform(notify, Box::new(Fake { seen: seen.clone() }), 1);

        assert_eq!(servers.watch("t1")[0].port, 8080);
        assert!(matches!(
            servers.stop("t1", 20, 9999),
            Err(CommandError::ServerNotFound { port: 9999, .. })
        ));
        servers.stop("t1", 20, 8080).unwrap();

        let update = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(update.thread_id, "t1");
        assert!(update.servers.is_empty());
        servers.unwatch("t1");
        assert!(guard(&servers.shared.watching).viewers.is_empty());
    }
}
