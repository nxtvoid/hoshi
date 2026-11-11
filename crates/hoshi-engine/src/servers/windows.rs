//! Listening sockets from the system's TCP table, and their processes' environments.

use super::{
    Listener, Platform, Process, Seen, StopFailure, may_stop_tree, owner_from_environment,
};
use netstat2::{AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo, get_sockets_info};
use std::collections::HashMap;
use std::net::IpAddr;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Default)]
pub(super) struct Windows {
    /// Made on first use: making it can fail.
    system: Option<System>,
}

impl Windows {
    pub(super) fn new() -> Self {
        Self::default()
    }

    fn system(&mut self) -> Result<&mut System, String> {
        let system = match self.system.take() {
            Some(system) => system,
            None => System::new().map_err(|error| error.to_string())?,
        };
        Ok(self.system.insert(system))
    }
}

fn parents(system: &mut System) -> HashMap<u32, u32> {
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    system
        .processes()
        .iter()
        .filter_map(|(pid, process)| Some((pid.as_u32(), process.parent()?.as_u32())))
        .collect()
}

/// A row of the TCP table, as far as finding listeners goes.
pub(super) struct Socket {
    pub address: IpAddr,
    pub port: u16,
    pub remote_port: u16,
    pub pids: Vec<u32>,
}

/// The listening sockets on a loopback or wildcard address. A listener is told by its
/// remote port of 0: some Windows builds report a fresh listener's state as closed.
pub(super) fn listeners(sockets: impl IntoIterator<Item = Socket>) -> Vec<Listener> {
    sockets
        .into_iter()
        .filter(|socket| {
            socket.remote_port == 0
                && socket.port > 0
                && (socket.address.is_loopback() || socket.address.is_unspecified())
        })
        .map(|socket| Listener {
            port: socket.port,
            pid: socket.pids.first().copied().filter(|pid| *pid > 0),
        })
        .collect()
}

fn process_name(name: &str) -> Option<String> {
    let name = name
        .strip_suffix(".exe")
        .or_else(|| name.strip_suffix(".EXE"))
        .unwrap_or(name);
    (!name.is_empty()).then(|| name.to_owned())
}

impl Platform for Windows {
    fn scan(&mut self) -> Option<Seen> {
        let sockets = get_sockets_info(
            AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6,
            ProtocolFlags::TCP,
        )
        .ok()?;
        let listeners = listeners(sockets.into_iter().filter_map(|socket| {
            let ProtocolSocketInfo::Tcp(tcp) = socket.protocol_socket_info else {
                return None;
            };
            Some(Socket {
                address: tcp.local_addr,
                port: tcp.local_port,
                remote_port: tcp.remote_port,
                pids: socket.associated_pids,
            })
        }));
        let mut pids: Vec<Pid> = listeners
            .iter()
            .filter_map(|listener| listener.pid.map(Pid::from_u32))
            .collect();
        pids.sort_unstable();
        pids.dedup();
        if pids.is_empty() {
            return Some(Seen {
                listeners,
                processes: HashMap::new(),
            });
        }
        let system = self.system().ok()?;
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::nothing().with_environ(UpdateKind::Always),
        );
        let processes = pids
            .iter()
            .filter_map(|pid| {
                let process = system.process(*pid)?;
                let owner = owner_from_environment(
                    process
                        .environ()
                        .iter()
                        .map(|entry| entry.to_string_lossy()),
                )?;
                Some((
                    pid.as_u32(),
                    Process {
                        owner: Some(owner),
                        name: process_name(&process.name().to_string_lossy()),
                        started_at: Some(process.start_time() * 1000)
                            .filter(|started| *started > 0),
                        parent: process.parent().map(Pid::as_u32),
                    },
                ))
            })
            .collect();
        Some(Seen {
            listeners,
            processes,
        })
    }

    fn reset(&mut self) {
        self.system = None;
    }

    fn stop(&mut self, pid: u32, own_pid: u32) -> Result<(), StopFailure> {
        let system = self.system().map_err(StopFailure::Failed)?;
        let parents = parents(system);
        if !system.processes().contains_key(&Pid::from_u32(pid)) {
            return Ok(());
        }
        if !may_stop_tree(&parents, pid, own_pid) {
            return Err(StopFailure::Refused);
        }
        let output = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|error| StopFailure::Failed(error.to_string()))?;
        if output.status.success() {
            return Ok(());
        }
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            true,
            ProcessRefreshKind::nothing(),
        );
        if system.process(Pid::from_u32(pid)).is_none() {
            return Ok(());
        }
        Err(StopFailure::Failed(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::servers::{TERMINAL_VARIABLE, THREAD_VARIABLE, attribute};
    use hoshi_core::ServerOrigin;
    use std::net::{Ipv4Addr, Ipv6Addr, TcpStream};
    use std::time::{Duration, Instant};

    fn socket(address: IpAddr, port: u16, listening: bool, pid: u32) -> Socket {
        Socket {
            address,
            port,
            remote_port: if listening { 0 } else { 51_000 },
            pids: vec![pid],
        }
    }

    #[test]
    fn only_listeners_on_loopback_or_every_address_are_kept() {
        let rows = [
            socket(Ipv4Addr::LOCALHOST.into(), 8080, true, 10),
            socket(Ipv4Addr::UNSPECIFIED.into(), 3000, true, 11),
            socket(Ipv6Addr::LOCALHOST.into(), 5173, true, 12),
            socket(Ipv6Addr::UNSPECIFIED.into(), 3000, true, 11),
            socket(Ipv4Addr::new(192, 168, 1, 5).into(), 9000, true, 13),
            socket(Ipv4Addr::LOCALHOST.into(), 50_000, false, 14),
            socket(Ipv4Addr::LOCALHOST.into(), 7000, true, 0),
        ];

        let found: Vec<_> = listeners(rows)
            .into_iter()
            .map(|listener| (listener.port, listener.pid))
            .collect();

        assert_eq!(
            found,
            [
                (8080, Some(10)),
                (3000, Some(11)),
                (5173, Some(12)),
                (3000, Some(11)),
                (7000, None)
            ]
        );
        assert_eq!(process_name("java.exe").as_deref(), Some("java"));
        assert_eq!(process_name("node").as_deref(), Some("node"));
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn listening(port: u16) -> bool {
        TcpStream::connect_timeout(
            &(Ipv4Addr::LOCALHOST, port).into(),
            Duration::from_millis(200),
        )
        .is_ok()
    }

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// A server started detached through a shell that exits, as an agent's command would,
    /// is the thread's until stopped.
    #[test]
    fn a_detached_server_is_attributed_to_its_thread_and_stops() {
        let port = free_port();
        let thread_id = format!("servers-test-{port}");
        let script = format!(
            "Start-Process -WindowStyle Hidden -FilePath powershell.exe -ArgumentList \
             '-NoProfile','-Command','$l=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,{port});$l.Start();Start-Sleep 120'"
        );
        let status = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .env(THREAD_VARIABLE, &thread_id)
            .env(TERMINAL_VARIABLE, "")
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .unwrap();
        assert!(status.success());
        wait_until("the server to listen", || listening(port));
        let mut windows = Windows::new();

        let started = Instant::now();
        let seen = windows.scan().unwrap();
        let took = started.elapsed();
        let threads = attribute(&seen, std::process::id());
        let server = threads
            .get(&thread_id)
            .and_then(|servers| servers.iter().find(|server| server.port == port))
            .cloned();
        let Some(server) = server else {
            panic!("port {port} not attributed to {thread_id}: {threads:?}");
        };
        windows.stop(server.pid, std::process::id()).unwrap();
        wait_until("the port to be released", || !listening(port));

        assert_eq!(server.origin, ServerOrigin::Agent);
        assert_eq!(server.process_name.as_deref(), Some("powershell"));
        assert!(server.started_at.is_some());
        assert!(
            attribute(&windows.scan().unwrap(), std::process::id())
                .get(&thread_id)
                .is_none_or(|servers| servers.iter().all(|server| server.port != port))
        );
        eprintln!("scan took {took:?}");
    }

    #[test]
    fn hoshi_itself_and_its_direct_children_are_never_stopped() {
        let mut windows = Windows::new();
        let own = std::process::id();
        assert_eq!(windows.stop(own, own), Err(StopFailure::Refused));
        let mut child = Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", "Start-Sleep 30"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .unwrap();
        let refused = windows.stop(child.id(), own);
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(refused, Err(StopFailure::Refused));
    }
}
