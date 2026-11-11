//! Listening sockets and their processes, as Linux's `/proc` tells them.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use super::{
    Listener, Owner, Platform, Process, STOP_GRACE, STOP_POLL, Seen, StopFailure, descendants,
    may_stop_tree, owner_from_environment,
};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Instant;

const LISTEN: &str = "0A";
/// The kernel reports process times to user space in these ticks per second.
const USER_HZ: u64 = 100;

pub(super) struct ProcFs {
    root: PathBuf,
    /// Each live pid's thread, read once: a process keeps the environment it started with.
    owners: HashMap<u32, Option<Owner>>,
}

/// The fields of `/proc/<pid>/stat` stopping a server needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Stat {
    pub parent: u32,
    pub group: u32,
    /// Clock ticks after boot.
    pub started: u64,
}

impl ProcFs {
    pub(super) fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            owners: HashMap::new(),
        }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    fn pids(&self) -> Option<Vec<u32>> {
        let entries = fs::read_dir(&self.root).ok()?;
        Some(
            entries
                .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
                .collect(),
        )
    }

    /// `None` when a table exists but can't be read, which could hide a shared port.
    fn sockets(&self) -> Option<Vec<(u16, u64)>> {
        let mut sockets = Vec::new();
        for table in ["net/tcp", "net/tcp6"] {
            match fs::read_to_string(self.path(table)) {
                Ok(text) => sockets.extend(parse_tcp_table(&text)?),
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(_) => return None,
            }
        }
        Some(sockets)
    }

    fn owner(&mut self, pid: u32) -> Option<Owner> {
        let root = &self.root;
        self.owners
            .entry(pid)
            .or_insert_with(|| {
                let environ = fs::read(root.join(format!("{pid}/environ"))).ok()?;
                owner_from_environment(environ_entries(&environ))
            })
            .clone()
    }

    fn socket_inodes(&self, pid: u32) -> Vec<u64> {
        let Ok(entries) = fs::read_dir(self.path(&format!("{pid}/fd"))) else {
            return Vec::new();
        };
        entries
            .filter_map(|entry| {
                let link = fs::read_link(entry.ok()?.path()).ok()?;
                socket_inode(link.to_str()?)
            })
            .collect()
    }

    fn stat(&self, pid: u32) -> Option<Stat> {
        parse_stat(&fs::read_to_string(self.path(&format!("{pid}/stat"))).ok()?)
    }

    fn boot_time(&self) -> Option<u64> {
        parse_boot_time(&fs::read_to_string(self.path("stat")).ok()?)
    }

    fn process(&self, pid: u32, owner: Owner, boot_time: Option<u64>) -> Process {
        let stat = self.stat(pid);
        let name = fs::read_to_string(self.path(&format!("{pid}/comm")))
            .ok()
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty());
        Process {
            owner: Some(owner),
            name,
            started_at: stat
                .zip(boot_time)
                .map(|(stat, boot)| boot * 1000 + stat.started * 1000 / USER_HZ),
            parent: stat.map(|stat| stat.parent),
        }
    }

    fn table(&self) -> Option<HashMap<u32, Stat>> {
        Some(
            self.pids()?
                .into_iter()
                .filter_map(|pid| Some((pid, self.stat(pid)?)))
                .collect(),
        )
    }

    /// Whether any of `pids` still runs as the process it was, or in `group` when given.
    fn any_left(&self, pids: &[(u32, Stat)], group: Option<u32>) -> bool {
        pids.iter().any(|(pid, before)| {
            self.stat(*pid).is_some_and(|now| {
                now.started == before.started && group.is_none_or(|group| now.group == group)
            })
        })
    }
}

impl Platform for ProcFs {
    fn scan(&mut self) -> Option<Seen> {
        let sockets = self.sockets()?;
        if sockets.is_empty() {
            return Some(Seen::default());
        }
        let wanted: HashSet<u64> = sockets.iter().map(|(_, inode)| *inode).collect();
        let pids = self.pids()?;
        let live: HashSet<u32> = pids.iter().copied().collect();
        self.owners.retain(|pid, _| live.contains(pid));
        let boot_time = self.boot_time();
        let mut holders: HashMap<u64, u32> = HashMap::new();
        let mut processes = HashMap::new();
        for pid in pids {
            let Some(owner) = self.owner(pid) else {
                continue;
            };
            let held: Vec<u64> = self
                .socket_inodes(pid)
                .into_iter()
                .filter(|inode| wanted.contains(inode))
                .collect();
            if held.is_empty() {
                continue;
            }
            for inode in held {
                holders.entry(inode).or_insert(pid);
            }
            processes.insert(pid, self.process(pid, owner, boot_time));
        }
        let listeners = sockets
            .into_iter()
            .map(|(port, inode)| Listener {
                port,
                pid: holders.get(&inode).copied(),
            })
            .collect();
        Some(Seen {
            listeners,
            processes,
        })
    }

    fn reset(&mut self) {
        self.owners.clear();
    }

    fn stop(&mut self, pid: u32, own_pid: u32) -> Result<(), StopFailure> {
        let table = self
            .table()
            .ok_or_else(|| StopFailure::Failed("/proc can't be read".into()))?;
        if !table.contains_key(&pid) {
            return Ok(());
        }
        let parents: HashMap<u32, u32> = table
            .iter()
            .map(|(&pid, stat)| (pid, stat.parent))
            .collect();
        let (targets, group) = match stoppable_group(&table, pid, own_pid) {
            Some(group) => (
                table
                    .iter()
                    .filter(|(_, stat)| stat.group == group)
                    .map(|(&pid, &stat)| (pid, stat))
                    .collect::<Vec<_>>(),
                Some(group),
            ),
            None if may_stop_tree(&parents, pid, own_pid) => (
                descendants(&parents, pid)
                    .into_iter()
                    .filter_map(|pid| Some((pid, *table.get(&pid)?)))
                    .collect(),
                None,
            ),
            None => return Err(StopFailure::Refused),
        };
        let pids: Vec<u32> = targets.iter().map(|(pid, _)| *pid).collect();
        let sent = signal("TERM", group, &pids);
        if !self.any_left(&targets, group) {
            return Ok(());
        }
        sent?;
        let deadline = Instant::now() + STOP_GRACE;
        while Instant::now() < deadline {
            thread::sleep(STOP_POLL);
            if !self.any_left(&targets, group) {
                return Ok(());
            }
        }
        let _ = signal("KILL", group, &pids);
        Ok(())
    }
}

fn signal(name: &str, group: Option<u32>, pids: &[u32]) -> Result<(), StopFailure> {
    let mut kill = Command::new("kill");
    kill.args(["-s", name, "--"]);
    match group {
        Some(group) => {
            kill.arg(format!("-{group}"));
        }
        None => {
            kill.args(pids.iter().map(u32::to_string));
        }
    }
    let status = kill
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| StopFailure::Failed(error.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(StopFailure::Failed(format!("kill: {status}")))
    }
}

/// The process group stopping `pid` signals: shells give each job its own, so the group is
/// the job. `None` when the group also holds hoshi, one of its ancestors, or one of its
/// direct children (the agents and the terminals' shells).
pub(super) fn stoppable_group(table: &HashMap<u32, Stat>, pid: u32, own_pid: u32) -> Option<u32> {
    let group = table.get(&pid)?.group;
    if group <= 1 {
        return None;
    }
    let parents: HashMap<u32, u32> = table
        .iter()
        .map(|(&pid, stat)| (pid, stat.parent))
        .collect();
    let mut guarded = super::ancestors(&parents, own_pid);
    guarded.push(own_pid);
    let shared = table.iter().any(|(pid, stat)| {
        stat.group == group && (guarded.contains(pid) || stat.parent == own_pid)
    });
    (!shared).then_some(group)
}

/// The LISTEN sockets on a loopback or wildcard address of `/proc/net/tcp` or `tcp6`, as
/// their port and inode. `None` when the text is not such a table.
pub(super) fn parse_tcp_table(text: &str) -> Option<Vec<(u16, u64)>> {
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next()?.split_whitespace().collect();
    let remote = header.get(2)?;
    if header.get(..2) != Some(&["sl", "local_address"][..])
        || !(*remote == "rem_address" || *remote == "remote_address")
        || header.get(3) != Some(&"st")
    {
        return None;
    }
    Some(
        lines
            .filter_map(|line| {
                let fields: Vec<&str> = line.split_whitespace().collect();
                let (address, port) = fields.get(1)?.split_once(':')?;
                if *fields.get(3)? != LISTEN || !local_address(&address.to_ascii_uppercase()) {
                    return None;
                }
                let port = u16::from_str_radix(port, 16)
                    .ok()
                    .filter(|port| *port > 0)?;
                let inode = fields.get(9)?.parse().ok().filter(|inode| *inode > 0)?;
                Some((port, inode))
            })
            .collect(),
    )
}

/// Each 32-bit word of the hex address is in host order, little-endian on the machines
/// hoshi runs on.
fn local_address(hex: &str) -> bool {
    match hex.len() {
        8 => hex == "00000000" || hex == "0100007F",
        32 => {
            hex == "00000000000000000000000000000000"
                || hex == "00000000000000000000000001000000"
                || (hex.starts_with("0000000000000000FFFF0000") && local_address(&hex[24..]))
        }
        _ => false,
    }
}

/// The NUL-separated `NAME=value` entries of `/proc/<pid>/environ`.
pub(super) fn environ_entries(blob: &[u8]) -> Vec<String> {
    blob.split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8_lossy(entry).into_owned())
        .collect()
}

/// The inode of a `/proc/<pid>/fd` link to a socket.
pub(super) fn socket_inode(link: &str) -> Option<u64> {
    link.strip_prefix("socket:[")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

/// `/proc/<pid>/stat`, whose second field, the name, may hold spaces and parentheses.
pub(super) fn parse_stat(text: &str) -> Option<Stat> {
    let rest = &text[text.rfind(')')? + 1..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    Some(Stat {
        parent: fields.get(1)?.parse().ok()?,
        group: fields.get(2)?.parse().ok()?,
        started: fields.get(19)?.parse().ok()?,
    })
}

/// When the machine booted, in Unix seconds, from `/proc/stat`.
pub(super) fn parse_boot_time(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix("btime "))?
        .trim()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoshi_core::ServerOrigin;

    const TCP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 41001 1 0000000000000000 100 0 0 10 0
   1: 00000000:0BB8 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 41002 1 0000000000000000 100 0 0 10 0
   2: 0F02000A:1F91 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 41003 1 0000000000000000 100 0 0 10 0
   3: 0100007F:1F90 0100007F:D3C2 01 00000000:00000000 00:00000000 00000000  1000        0 41004 1 0000000000000000 20 4 30 10 -1
";
    const TCP6: &str = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000001000000:1389 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 52001 1 0000000000000000 100 0 0 10 0
   1: 00000000000000000000000000000000:0050 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 52002 1 0000000000000000 100 0 0 10 0
   2: 0000000000000000FFFF00000100007F:1F92 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 52003 1 0000000000000000 100 0 0 10 0
   3: 000080FE00000000FF005450B6AD1DFE:1F93 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 52004 1 0000000000000000 100 0 0 10 0
";

    #[test]
    fn only_listeners_on_loopback_or_every_address_are_read() {
        assert_eq!(
            parse_tcp_table(TCP),
            Some(vec![(8080, 41001), (3000, 41002)])
        );
        assert_eq!(
            parse_tcp_table(TCP6),
            Some(vec![(5001, 52001), (80, 52002), (8082, 52003)])
        );
        assert_eq!(parse_tcp_table("not a table\n"), None);
    }

    #[test]
    fn environ_socket_links_stat_and_boot_time_parse() {
        assert_eq!(
            environ_entries(b"PATH=/bin\0HOSHI_THREAD_ID=t1\0HOSHI_TERMINAL_ID=\0"),
            ["PATH=/bin", "HOSHI_THREAD_ID=t1", "HOSHI_TERMINAL_ID="]
        );
        assert_eq!(socket_inode("socket:[41001]"), Some(41001));
        assert_eq!(socket_inode("pipe:[41001]"), None);
        assert_eq!(socket_inode("/dev/null"), None);
        let stat = "4242 (java (main) x) S 1 4240 4240 0 -1 4194560 100 0 0 0 5 3 0 0 20 0 30 0 123456 0 0";
        assert_eq!(
            parse_stat(stat),
            Some(Stat {
                parent: 1,
                group: 4240,
                started: 123_456
            })
        );
        assert_eq!(
            parse_boot_time("cpu  1 2 3\nbtime 1700000000\nprocesses 9\n"),
            Some(1_700_000_000)
        );
    }

    fn stat(parent: u32, group: u32) -> Stat {
        Stat {
            parent,
            group,
            started: 1,
        }
    }

    #[test]
    fn a_job_s_group_is_stopped_unless_it_holds_hoshi_an_agent_or_a_shell() {
        let table = HashMap::from([
            (1, stat(0, 1)),
            (100, stat(1, 100)),
            (200, stat(100, 100)),
            (300, stat(200, 100)),
            (400, stat(1, 350)),
            (500, stat(400, 350)),
            (600, stat(200, 600)),
            (700, stat(600, 700)),
            (800, stat(600, 600)),
        ]);

        assert_eq!(stoppable_group(&table, 400, 200), Some(350));
        assert_eq!(stoppable_group(&table, 700, 200), Some(700));
        assert_eq!(stoppable_group(&table, 800, 200), None);
        assert_eq!(stoppable_group(&table, 300, 200), None);
        assert_eq!(stoppable_group(&table, 1, 200), None);
    }

    #[test]
    fn a_scan_of_a_proc_tree_finds_the_owner_name_and_age_of_a_listener() {
        let root = std::env::temp_dir().join(format!("hoshi-procfs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let write = |path: &str, contents: &[u8]| {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        };
        write("net/tcp", TCP.as_bytes());
        write("stat", b"btime 1700000000\n");
        write("4242/environ", b"HOSHI_THREAD_ID=t1\0");
        write("4242/comm", b"java\n");
        write(
            "4242/stat",
            b"4242 (java) S 4000 4240 4240 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 1500 0 0",
        );
        fs::create_dir_all(root.join("4242/fd")).unwrap();
        write("4300/environ", b"PATH=/bin\0");
        let mut procfs = ProcFs::new(&root);

        let seen = procfs.scan().unwrap();
        let process = procfs.process(
            4242,
            procfs.owners[&4242].clone().unwrap(),
            procfs.boot_time(),
        );

        let _ = fs::remove_dir_all(&root);
        assert_eq!(
            seen.listeners,
            [
                Listener {
                    port: 8080,
                    pid: None
                },
                Listener {
                    port: 3000,
                    pid: None
                }
            ]
        );
        assert!(seen.processes.is_empty());
        assert_eq!(
            procfs.owners.get(&4242),
            Some(&Some(Owner {
                thread_id: "t1".into(),
                origin: ServerOrigin::Agent
            }))
        );
        assert_eq!(procfs.owners.get(&4300), Some(&None));
        assert_eq!(process.name.as_deref(), Some("java"));
        assert_eq!(process.parent, Some(4000));
        assert_eq!(process.started_at, Some(1_700_000_015_000));
    }
}
