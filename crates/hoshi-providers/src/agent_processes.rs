//! The agent processes hoshi runs and everything they start: ending each one as a whole, and
//! noting them under hoshi's home so a later start ends what a crash or a kill left running.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

/// How long a process asked to end gets before it is killed.
#[cfg(unix)]
const KILL_GRACE: Duration = Duration::from_secs(5);
/// How long quitting waits for the agents to end before it kills them.
const QUIT_GRACE: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(50);

/// A process, told apart from a later one that reuses its pid by when it started, in Unix
/// seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct Identity {
    pid: u32,
    started: u64,
}

impl Identity {
    /// The process that holds `pid` now.
    pub(crate) fn of(pid: u32) -> Option<Self> {
        let mut system = System::new().ok()?;
        let id = Pid::from_u32(pid);
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[id]),
            true,
            ProcessRefreshKind::nothing(),
        );
        let started = system.process(id)?.start_time();
        Some(Self { pid, started })
    }
}

/// The processes running at one moment.
struct Processes(System);

impl Processes {
    fn read() -> Option<Self> {
        let mut system = System::new().ok()?;
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );
        Some(Self(system))
    }

    /// When the process holding `pid` started, whether or not it already exited.
    fn started(&self, pid: u32) -> Option<u64> {
        self.0
            .process(Pid::from_u32(pid))
            .map(sysinfo::Process::start_time)
    }

    fn runs(&self, process: Identity) -> bool {
        self.0
            .process(Pid::from_u32(process.pid))
            .is_some_and(|found| {
                found.start_time() == process.started && found.status() != ProcessStatus::Zombie
            })
    }

    /// `root`, while it runs, and the processes it started, theirs included. Those that
    /// outlived their parent count where the system still names it as their parent, as
    /// Windows does.
    fn tree(&self, root: Identity) -> Vec<Identity> {
        let mut children: HashMap<u32, Vec<Identity>> = HashMap::new();
        for (pid, process) in self.0.processes() {
            if let Some(parent) = process.parent() {
                children.entry(parent.as_u32()).or_default().push(Identity {
                    pid: pid.as_u32(),
                    started: process.start_time(),
                });
            }
        }
        let mut tree = Vec::new();
        if self.runs(root) {
            tree.push(root);
        }
        let mut seen = HashSet::from([root.pid]);
        let mut pending = vec![root];
        while let Some(parent) = pending.pop() {
            let reused_at = self
                .started(parent.pid)
                .filter(|&started| started != parent.started);
            for &child in children.get(&parent.pid).into_iter().flatten() {
                let started_by_parent = child.started >= parent.started
                    && reused_at.is_none_or(|reused_at| child.started < reused_at);
                if started_by_parent && seen.insert(child.pid) {
                    tree.push(child);
                    pending.push(child);
                }
            }
        }
        tree
    }

    /// [`Processes::tree`], with the processes of `earlier` that still run: those that
    /// outlived a parent the system no longer names, as unix does.
    fn tree_with(&self, root: Identity, earlier: &[Identity]) -> Vec<Identity> {
        let mut tree = self.tree(root);
        for &process in earlier {
            if !tree.contains(&process) && self.runs(process) {
                tree.push(process);
            }
        }
        tree
    }

    /// Whether the process group `root` led can still be only its own: the pid is free or
    /// still `root`'s.
    #[cfg(unix)]
    fn group_is_ours(&self, root: Identity) -> bool {
        self.started(root.pid)
            .is_none_or(|started| started == root.started)
    }
}

/// Ends the process holding `pid` and everything it started, as [`end`] does. `false` when
/// the processes can't be read or none holds `pid`.
pub(crate) fn end_tree(pid: u32) -> bool {
    let Some(processes) = Processes::read() else {
        return false;
    };
    let Some(started) = processes.started(pid) else {
        return false;
    };
    end_in(&processes, Identity { pid, started });
    true
}

/// Ends `root` and everything it started, even once it exited. On Windows they are killed
/// at once; elsewhere they are asked to end, and what still runs a few seconds later is
/// killed. `false` when the processes can't be read.
pub(crate) fn end(root: Identity) -> bool {
    let Some(processes) = Processes::read() else {
        return false;
    };
    end_in(&processes, root);
    true
}

fn end_in(processes: &Processes, root: Identity) {
    let tree = processes.tree(root);
    platform::ask_to_end(processes, root, &tree);
    thread::spawn(move || {
        thread::sleep(platform::FOLLOW_UP);
        if let Some(processes) = Processes::read() {
            let tree = processes.tree_with(root, &tree);
            platform::kill(&processes, root, &tree);
        }
    });
}

/// Kills what `root`, which exited, left running.
pub(crate) fn end_left_behind(root: Identity) {
    end_trees(&[root], Duration::ZERO);
}

/// Ends the trees of `roots`, killing what still runs after `grace`.
fn end_trees(roots: &[Identity], grace: Duration) {
    let Some(processes) = Processes::read() else {
        return;
    };
    let mut trees: Vec<(Identity, Vec<Identity>)> = roots
        .iter()
        .map(|&root| (root, processes.tree(root)))
        .collect();
    for (root, tree) in &trees {
        platform::ask_to_end(&processes, *root, tree);
    }
    let deadline = Instant::now() + grace;
    while let Some(processes) = Processes::read() {
        for (root, tree) in &mut trees {
            *tree = processes.tree_with(*root, tree);
        }
        let running = trees
            .iter()
            .any(|(root, tree)| platform::runs(&processes, *root, tree));
        if !running {
            return;
        }
        let past = Instant::now() >= deadline;
        if past || !platform::ASKS_FIRST {
            for (root, tree) in &trees {
                platform::kill(&processes, *root, tree);
            }
        }
        if past {
            return;
        }
        thread::sleep(POLL);
    }
}

#[cfg(windows)]
mod platform {
    use super::{Identity, Processes};
    use std::time::Duration;
    use sysinfo::Pid;

    pub(super) const ASKS_FIRST: bool = false;
    /// Catches the processes that were starting as their tree was killed.
    pub(super) const FOLLOW_UP: Duration = Duration::from_secs(1);

    pub(super) fn ask_to_end(processes: &Processes, root: Identity, tree: &[Identity]) {
        kill(processes, root, tree);
    }

    pub(super) fn kill(processes: &Processes, _root: Identity, tree: &[Identity]) {
        for &member in tree {
            if processes.runs(member)
                && let Some(process) = processes.0.process(Pid::from_u32(member.pid))
            {
                process.kill();
            }
        }
    }

    pub(super) fn runs(processes: &Processes, _root: Identity, tree: &[Identity]) -> bool {
        tree.iter().any(|&member| processes.runs(member))
    }
}

#[cfg(unix)]
mod platform {
    use super::{Identity, KILL_GRACE, Processes};
    use rustix::process::{
        Pid, Signal, getpgid, kill_process, kill_process_group, test_kill_process_group,
    };
    use std::time::Duration;

    pub(super) const ASKS_FIRST: bool = true;
    pub(super) const FOLLOW_UP: Duration = KILL_GRACE;

    pub(super) fn ask_to_end(processes: &Processes, root: Identity, tree: &[Identity]) {
        signal(processes, root, tree, Signal::TERM);
    }

    pub(super) fn kill(processes: &Processes, root: Identity, tree: &[Identity]) {
        signal(processes, root, tree, Signal::KILL);
    }

    pub(super) fn runs(processes: &Processes, root: Identity, tree: &[Identity]) -> bool {
        let group = pid(root.pid).filter(|_| processes.group_is_ours(root));
        group.is_some_and(|group| test_kill_process_group(group).is_ok())
            || tree.iter().any(|&member| processes.runs(member))
    }

    /// Signals the group `root` led and each process of its tree; one that leads a group of
    /// its own, as a job the agent started can, takes its group along.
    fn signal(processes: &Processes, root: Identity, tree: &[Identity], signal: Signal) {
        if processes.group_is_ours(root)
            && let Some(group) = pid(root.pid)
        {
            let _ = kill_process_group(group, signal);
        }
        for &member in tree {
            let Some(process) = pid(member.pid).filter(|_| processes.runs(member)) else {
                continue;
            };
            if getpgid(Some(process)) == Ok(process) {
                let _ = kill_process_group(process, signal);
            } else {
                let _ = kill_process(process, signal);
            }
        }
    }

    fn pid(pid: u32) -> Option<Pid> {
        Pid::from_raw(i32::try_from(pid).ok()?)
    }
}

/// What a run of hoshi notes in its file.
#[derive(Serialize, Deserialize)]
struct Note {
    owner: Identity,
    processes: Vec<Identity>,
}

/// The agent processes this run of hoshi started that still run, and the file it notes them
/// in once [`record_in`] named a folder.
struct Record {
    owner: Option<Identity>,
    file: Option<PathBuf>,
    processes: Vec<Identity>,
}

static RECORD: Mutex<Record> = Mutex::new(Record {
    owner: None,
    file: None,
    processes: Vec::new(),
});

fn record() -> MutexGuard<'static, Record> {
    RECORD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Record {
    fn save(&self) {
        let (Some(owner), Some(file)) = (self.owner, &self.file) else {
            return;
        };
        if self.processes.is_empty() {
            let _ = fs::remove_file(file);
            return;
        }
        let note = Note {
            owner,
            processes: self.processes.clone(),
        };
        let Ok(text) = serde_json::to_string(&note) else {
            return;
        };
        let pending = file.with_extension("pending");
        if fs::write(&pending, text).is_ok() && fs::rename(&pending, file).is_err() {
            let _ = fs::remove_file(&pending);
        }
    }
}

/// Notes the agent processes in `dir` from now on, one file per run of hoshi, after ending
/// what the runs that are gone left noted there. Only the first call notes; later ones only
/// end what was left.
pub fn record_in(dir: &Path) {
    let Some(owner) = sysinfo::get_current_pid()
        .ok()
        .and_then(|pid| Identity::of(pid.as_u32()))
    else {
        return;
    };
    end_left_in(dir, owner);
    let mut record = record();
    if record.file.is_some() || fs::create_dir_all(dir).is_err() {
        return;
    }
    record.owner = Some(owner);
    record.file = Some(dir.join(format!("{}-{}.json", owner.pid, owner.started)));
    record.save();
}

/// Ends what the runs of hoshi that are gone noted in `dir`, and forgets their notes.
fn end_left_in(dir: &Path, owner: Identity) {
    let notes: Vec<(PathBuf, Note)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter_map(|path| {
            let note = serde_json::from_str::<Note>(&fs::read_to_string(&path).ok()?).ok()?;
            Some((path, note))
        })
        .filter(|(_, note)| note.owner != owner)
        .collect();
    if notes.is_empty() {
        return;
    }
    let Some(processes) = Processes::read() else {
        return;
    };
    for (path, note) in notes {
        if processes.runs(note.owner) {
            continue;
        }
        end_trees(&note.processes, Duration::ZERO);
        let _ = fs::remove_file(path);
    }
}

/// An agent process noted while it runs; dropping it forgets it.
pub(crate) struct Noted(Option<Identity>);

impl Noted {
    /// Notes the process holding `pid`, which hoshi just started.
    pub(crate) fn new(pid: u32) -> Self {
        let identity = Identity::of(pid);
        if let Some(identity) = identity {
            let mut record = record();
            record.processes.push(identity);
            record.save();
        }
        Self(identity)
    }

    pub(crate) fn identity(&self) -> Option<Identity> {
        self.0
    }
}

impl Drop for Noted {
    fn drop(&mut self) {
        let Some(identity) = self.0 else {
            return;
        };
        let mut record = record();
        record.processes.retain(|process| *process != identity);
        record.save();
    }
}

/// Ends every agent process hoshi runs and everything they started, as when it quits: what
/// still runs after a moment is killed.
pub fn end_all() {
    let processes = record().processes.clone();
    if !processes.is_empty() {
        end_trees(&processes, QUIT_GRACE);
    }
}

#[cfg(test)]
pub(crate) fn noted() -> Vec<Identity> {
    record().processes.clone()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

    /// A process that runs for a minute unless ended.
    pub(crate) fn long_running() -> Command {
        let mut command = if cfg!(windows) {
            let mut command = Command::new("ping");
            command.args(["-n", "60", "127.0.0.1"]);
            command
        } else {
            let mut command = Command::new("sleep");
            command.arg("60");
            command
        };
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    /// A shell that starts a child that runs for a minute, in a group of its own on unix.
    pub(crate) fn shell_running_a_child() -> Command {
        let mut command = if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/C", "ping -n 60 127.0.0.1 >NUL"]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 60; true"]);
            command
        };
        crate::process::hide_console(&mut command);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    /// A shell that starts a child that runs for a minute without it, and exits a moment
    /// later.
    pub(crate) fn leaves_a_child_running() -> Command {
        if cfg!(windows) {
            let mut command = Command::new("powershell");
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Process ping -ArgumentList '-n','60','127.0.0.1' -WindowStyle Hidden; \
                 Start-Sleep -Milliseconds 1500",
            ]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 60 >/dev/null 2>&1 </dev/null & sleep 1"]);
            command
        }
    }

    static NOTED_PROCESSES: RwLock<()> = RwLock::new(());

    /// Held by tests that start noted processes, which [`end_all`] would end.
    pub(crate) fn noting() -> RwLockReadGuard<'static, ()> {
        NOTED_PROCESSES
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Held by tests that end every noted process.
    pub(crate) fn exclusive() -> RwLockWriteGuard<'static, ()> {
        NOTED_PROCESSES
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether anything of `root`'s tree or group runs.
    pub(crate) fn tree_runs(root: Identity) -> bool {
        let processes = Processes::read().unwrap();
        platform::runs(&processes, root, &processes.tree(root))
    }

    pub(crate) fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting until {what}");
            thread::sleep(POLL);
        }
    }

    /// The processes `pid` started, once the one that runs for a minute is among them.
    pub(crate) fn children_of(pid: u32) -> Vec<Identity> {
        let mut children = Vec::new();
        wait_until("the child started", || {
            let processes = Processes::read().unwrap();
            let root = Identity {
                pid,
                started: processes.started(pid).unwrap(),
            };
            children = processes.tree(root).into_iter().skip(1).collect();
            children.iter().any(|child| {
                let name = processes
                    .0
                    .process(Pid::from_u32(child.pid))
                    .unwrap()
                    .name();
                let name = name.to_string_lossy().to_lowercase();
                name.starts_with("ping") || name.starts_with("sleep")
            })
        });
        children
    }

    pub(crate) fn runs(process: Identity) -> bool {
        Processes::read().unwrap().runs(process)
    }

    fn reap(child: &mut Child) -> bool {
        matches!(child.try_wait(), Ok(Some(_)))
    }

    #[test]
    fn a_process_and_what_it_started_end_together() {
        let mut shell = shell_running_a_child().spawn().unwrap();
        let children = children_of(shell.id());

        assert!(end_tree(shell.id()));

        wait_until("the shell ended", || reap(&mut shell));
        wait_until("its child ended", || {
            !children.iter().any(|&child| runs(child))
        });
    }

    #[test]
    fn what_a_process_that_exited_left_running_ends() {
        let mut shell = shell_running_a_child().spawn().unwrap();
        let root = Identity::of(shell.id()).unwrap();
        let children = children_of(shell.id());
        shell.kill().unwrap();
        shell.wait().unwrap();
        assert!(children.iter().all(|&child| runs(child)));

        end_left_behind(root);

        wait_until("the orphan ended", || {
            !children.iter().any(|&child| runs(child))
        });
    }

    #[test]
    fn a_reused_pid_is_not_taken_for_the_process_noted() {
        let mut process = long_running().spawn().unwrap();
        let actual = Identity::of(process.id()).unwrap();
        let earlier = Identity {
            pid: actual.pid,
            started: actual.started - 1,
        };

        end_trees(&[earlier], Duration::ZERO);

        thread::sleep(Duration::from_millis(300));
        assert!(!reap(&mut process));
        process.kill().unwrap();
        process.wait().unwrap();
    }

    #[test]
    fn quitting_ends_every_agent_process_and_what_it_started() {
        let _exclusive = exclusive();
        let mut shell = shell_running_a_child().spawn().unwrap();
        let noted = Noted::new(shell.id());
        let children = children_of(shell.id());

        end_all();

        wait_until("the shell ended", || reap(&mut shell));
        wait_until("its child ended", || {
            !children.iter().any(|&child| runs(child))
        });
        drop(noted);
    }

    fn note_file(dir: &Path, owner: Identity, processes: Vec<Identity>) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let path = dir.join(format!("{}-{}.json", owner.pid, owner.started));
        let note = Note { owner, processes };
        fs::write(&path, serde_json::to_string(&note).unwrap()).unwrap();
        path
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hoshi-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_start_ends_what_a_run_that_is_gone_left_running() {
        let dir = temp_dir("left-running");
        let mut survivor = long_running().spawn().unwrap();
        let identity = Identity::of(survivor.id()).unwrap();
        let mut gone_owner = long_running().spawn().unwrap();
        let owner = Identity::of(gone_owner.id()).unwrap();
        gone_owner.kill().unwrap();
        gone_owner.wait().unwrap();
        let note = note_file(&dir, owner, vec![identity]);
        let me = Identity::of(std::process::id()).unwrap();

        end_left_in(&dir, me);

        wait_until("the survivor ended", || reap(&mut survivor));
        assert!(!note.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_start_leaves_alone_what_a_live_run_noted() {
        let dir = temp_dir("live-run");
        let mut kept = long_running().spawn().unwrap();
        let identity = Identity::of(kept.id()).unwrap();
        let mut live_owner = long_running().spawn().unwrap();
        let owner = Identity::of(live_owner.id()).unwrap();
        let note = note_file(&dir, owner, vec![identity]);
        let me = Identity::of(std::process::id()).unwrap();

        end_left_in(&dir, me);

        thread::sleep(Duration::from_millis(300));
        assert!(!reap(&mut kept));
        assert!(note.exists());
        for child in [&mut kept, &mut live_owner] {
            child.kill().unwrap();
            child.wait().unwrap();
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
