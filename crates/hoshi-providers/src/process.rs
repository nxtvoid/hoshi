use crate::OutputSink;
use crate::agent_processes::{self, Identity, Noted};
use hoshi_core::{ProviderOutput, TurnFailure, TurnOutcome};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread;
use std::time::Duration;

const STDERR_TAIL_BYTES: usize = 2_000;
/// How long a process whose stdin hoshi closed gets to exit before it is ended.
const ENDING_GRACE: Duration = Duration::from_secs(5);

/// One agent process, running a turn or kept between turns for the commands it left in the
/// background. Dropping it does not stop the process; closing its stdin does, after
/// [`ENDING_GRACE`].
#[derive(Clone)]
pub struct ProcessTurn {
    process: Process,
    interrupted: Arc<AtomicBool>,
    stdin: Stdin,
}

#[derive(Clone)]
struct Process {
    child: Arc<Mutex<Child>>,
    pid: u32,
    /// `None` when it couldn't be read as the process started.
    identity: Option<Identity>,
    exited: Arc<AtomicBool>,
}

impl Process {
    /// Ends the process and everything it started, the processes it started that outlived it
    /// included.
    fn end(&self) {
        if self.exited.load(Ordering::SeqCst) {
            return;
        }
        if !self.identity.is_some_and(agent_processes::end) {
            kill_tree(self.pid, &self.child);
        }
    }
}

impl ProcessTurn {
    pub fn stdin(&self) -> &Stdin {
        &self.stdin
    }
}

impl ProcessTurn {
    /// Kills the process and everything it started.
    pub fn interrupt(&self) {
        self.interrupted.store(true, Ordering::SeqCst);
        self.process.end();
    }
}

/// The process's stdin, open while the agent can be answered or given another turn. Lines
/// go to the process in order from a thread of their own, so writing one never waits for the
/// process to read: it may not read until it has started, and a long line fills the pipe.
#[derive(Clone, Default)]
pub struct Stdin {
    lines: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    /// Lines queued and not written yet.
    unwritten: Arc<AtomicUsize>,
    process: Option<Process>,
}

impl Stdin {
    fn new(pipe: Option<ChildStdin>, process: Process) -> Self {
        let (sender, lines) = mpsc::channel::<String>();
        let stdin = Self {
            lines: Arc::new(Mutex::new(pipe.as_ref().map(|_| sender))),
            unwritten: Arc::default(),
            process: Some(process.clone()),
        };
        if let Some(mut pipe) = pipe {
            let writer = stdin.clone();
            thread::spawn(move || {
                for line in lines {
                    let written = pipe
                        .write_all(format!("{line}\n").as_bytes())
                        .and_then(|()| pipe.flush());
                    writer.unwritten.fetch_sub(1, Ordering::SeqCst);
                    // A process that can't be written to can't be answered or stopped
                    // either: ending it ends its turn.
                    if written.is_err() {
                        writer.sender().take();
                        process.end();
                        return;
                    }
                }
            });
        }
        stdin
    }

    fn sender(&self) -> MutexGuard<'_, Option<mpsc::Sender<String>>> {
        self.lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Queues `line` after those not written yet; `false` when the process no longer reads.
    pub fn write_line(&self, line: &str) -> bool {
        let sender = self.sender();
        let Some(sender) = sender.as_ref() else {
            return false;
        };
        self.unwritten.fetch_add(1, Ordering::SeqCst);
        let queued = sender.send(line.to_owned()).is_ok();
        if !queued {
            self.unwritten.fetch_sub(1, Ordering::SeqCst);
        }
        queued
    }

    /// Whether every line queued so far reached the process.
    pub fn is_written(&self) -> bool {
        self.unwritten.load(Ordering::SeqCst) == 0
    }

    pub fn is_open(&self) -> bool {
        self.sender().is_some()
    }

    /// An agent reading stream-json exits once its stdin closes, which it does once the lines
    /// queued before are written; one still running after [`ENDING_GRACE`] is ended with
    /// what it started.
    pub fn close(&self) {
        let closed = self.sender().take().is_some();
        if let Some(process) = self.process.clone().filter(|_| closed) {
            thread::spawn(move || {
                thread::sleep(ENDING_GRACE);
                process.end();
            });
        }
    }
}

/// Spawns `command`, writes `input` to its stdin without waiting for the process to read
/// it, then feeds each stdout line through `parse` into `sink` on a background thread.
/// `parse` closes stdin once the agent should end. When the process ends, a turn still open
/// (`in_turn`) completes as failed or interrupted; otherwise the sink gets `ProcessEnded`. What the process leaves running when
/// it exits is ended with it.
pub fn spawn_turn(
    mut command: Command,
    input: String,
    mut sink: OutputSink,
    mut parse: impl FnMut(&str, &Stdin) -> Vec<ProviderOutput> + Send + 'static,
    in_turn: impl Fn() -> bool + Send + 'static,
) -> Result<ProcessTurn, TurnFailure> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console(&mut command);
    let mut child = command.spawn().map_err(|error| TurnFailure::SpawnFailed {
        detail: error.to_string(),
    })?;
    let pid = child.id();
    let noted = Noted::new(pid);
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let pipe = child.stdin.take();
    let process = Process {
        child: Arc::new(Mutex::new(child)),
        pid,
        identity: noted.identity(),
        exited: Arc::default(),
    };
    let stdin = Stdin::new(pipe, process.clone());
    // A write error means the process already died; the reader reports how.
    stdin.write_line(input.trim_end());
    let interrupted = Arc::new(AtomicBool::new(false));

    let stderr_tail = thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(mut stderr) = stderr {
            let _ = stderr.read_to_end(&mut buffer);
        }
        let start = buffer.len().saturating_sub(STDERR_TAIL_BYTES);
        String::from_utf8_lossy(&buffer[start..]).trim().to_owned()
    });

    let reader_process = process.clone();
    let reader_interrupted = Arc::clone(&interrupted);
    let reader_stdin = stdin.clone();
    thread::spawn(move || {
        if let Some(stdout) = stdout {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                for output in parse(&line, &reader_stdin) {
                    sink(output);
                }
            }
        }
        reader_stdin.close();
        let status = reader_process
            .child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .wait();
        reader_process.exited.store(true, Ordering::SeqCst);
        if let Some(identity) = noted.identity() {
            agent_processes::end_left_behind(identity);
        }
        drop(noted);
        let stderr = stderr_tail.join().unwrap_or_default();
        if !in_turn() {
            sink(ProviderOutput::ProcessEnded);
            return;
        }
        let outcome = if reader_interrupted.load(Ordering::SeqCst) {
            TurnOutcome::Interrupted
        } else {
            let status = status.map_or_else(|error| error.to_string(), |status| status.to_string());
            TurnOutcome::Failed {
                reason: TurnFailure::ProcessExited {
                    detail: format!("{status}: {stderr}"),
                },
            }
        };
        sink(ProviderOutput::TurnCompleted { outcome });
    });

    Ok(ProcessTurn {
        process,
        interrupted,
        stdin,
    })
}

/// Keeps the process off the user's console: without a window on Windows, and in a process
/// group of its own elsewhere, which [`kill_tree`] ends as a whole.
#[cfg(windows)]
pub(crate) fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(unix)]
pub(crate) fn hide_console(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

/// Agents start their own tools (shells, servers), so stopping a turn kills the whole tree.
pub(crate) fn kill_tree(pid: u32, child: &Mutex<Child>) {
    if !agent_processes::end_tree(pid)
        && let Ok(mut child) = child.try_lock()
    {
        let _ = child.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_processes::tests::{
        children_of, leaves_a_child_running, long_running, noting, runs, shell_running_a_child,
        tree_runs, wait_until,
    };
    use std::sync::mpsc;

    fn run(command: Command) -> Vec<ProviderOutput> {
        run_in_turn(command, true)
    }

    fn run_in_turn(command: Command, in_turn: bool) -> Vec<ProviderOutput> {
        let _noting = noting();
        let (sender, receiver) = mpsc::channel();
        let sink: OutputSink = Box::new(move |output| {
            let _ = sender.send(output);
        });
        match spawn_turn(
            command,
            String::new(),
            sink,
            |_, _| Vec::new(),
            move || in_turn,
        ) {
            Ok(_turn) => receiver.iter().collect(),
            Err(failure) => vec![ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Failed { reason: failure },
            }],
        }
    }

    fn exit_with(code: u8) -> Command {
        let mut command = if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.arg("/C");
            command
        } else {
            let mut command = Command::new("sh");
            command.arg("-c");
            command
        };
        command.arg(format!("exit {code}"));
        command
    }

    #[test]
    fn a_missing_binary_fails_to_spawn() {
        let outputs = run(Command::new("hoshi-no-such-binary"));

        assert!(matches!(
            outputs.as_slice(),
            [ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Failed {
                    reason: TurnFailure::SpawnFailed { .. }
                }
            }]
        ));
    }

    #[test]
    fn a_process_that_exits_without_a_result_fails_the_turn() {
        let outputs = run(exit_with(3));

        assert!(matches!(
            outputs.as_slice(),
            [ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Failed {
                    reason: TurnFailure::ProcessExited { .. }
                }
            }]
        ));
    }

    #[test]
    fn a_process_that_ends_between_turns_reports_only_its_end() {
        assert_eq!(
            run_in_turn(exit_with(0), false),
            [ProviderOutput::ProcessEnded]
        );
    }

    fn start(command: Command, in_turn: bool) -> (ProcessTurn, mpsc::Receiver<ProviderOutput>) {
        let (sender, receiver) = mpsc::channel();
        let sink: OutputSink = Box::new(move |output| {
            let _ = sender.send(output);
        });
        let turn = spawn_turn(
            command,
            String::new(),
            sink,
            |_, _| Vec::new(),
            move || in_turn,
        )
        .unwrap();
        (turn, receiver)
    }

    fn last_output(receiver: &mpsc::Receiver<ProviderOutput>) -> ProviderOutput {
        receiver
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("the process ended")
    }

    #[test]
    fn a_long_message_a_process_does_not_read_yet_does_not_hold_up_its_start() {
        let _noting = noting();
        let (sender, receiver) = mpsc::channel();
        let sink: OutputSink = Box::new(move |output| {
            let _ = sender.send(output);
        });
        let started = std::time::Instant::now();
        let turn = spawn_turn(
            long_running(),
            "x".repeat(4 << 20),
            sink,
            |_, _| Vec::new(),
            || true,
        )
        .unwrap();
        assert!(turn.stdin().write_line(&"y".repeat(4 << 20)));

        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        assert!(turn.stdin().is_open());
        assert!(!turn.stdin().is_written());
        turn.interrupt();
        assert!(matches!(
            last_output(&receiver),
            ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Interrupted
            }
        ));
    }

    #[test]
    fn closing_stdin_does_not_wait_for_a_message_the_process_does_not_read() {
        let _noting = noting();
        let (sender, receiver) = mpsc::channel();
        let sink: OutputSink = Box::new(move |output| {
            let _ = sender.send(output);
        });
        let turn = spawn_turn(
            long_running(),
            "x".repeat(4 << 20),
            sink,
            |_, _| Vec::new(),
            || false,
        )
        .unwrap();

        let closing = std::time::Instant::now();
        turn.stdin().close();

        assert!(closing.elapsed() < std::time::Duration::from_secs(1));
        assert_eq!(last_output(&receiver), ProviderOutput::ProcessEnded);
    }

    #[test]
    fn a_process_still_running_once_its_stdin_closed_is_ended() {
        let _noting = noting();
        let (turn, receiver) = start(long_running(), false);

        turn.stdin().close();

        assert_eq!(last_output(&receiver), ProviderOutput::ProcessEnded);
    }

    #[test]
    fn interrupting_ends_what_the_agent_started() {
        let _noting = noting();
        let (turn, receiver) = start(shell_running_a_child(), true);
        let children = children_of(turn.process.pid);

        turn.interrupt();

        assert_eq!(
            last_output(&receiver),
            ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Interrupted
            }
        );
        wait_until("its child ended", || {
            !children.iter().any(|&child| runs(child))
        });
    }

    #[test]
    fn what_an_agent_leaves_running_ends_as_it_exits() {
        let _noting = noting();
        let (turn, receiver) = start(leaves_a_child_running(), false);
        let root = turn.process.identity.unwrap();
        let children = children_of(turn.process.pid);

        assert_eq!(last_output(&receiver), ProviderOutput::ProcessEnded);
        wait_until("what it left ended", || !tree_runs(root));
        assert!(!children.iter().any(|&child| runs(child)));
    }

    #[test]
    fn an_agent_process_is_noted_while_it_runs() {
        let _noting = noting();
        let (turn, receiver) = start(long_running(), true);
        let identity = turn.process.identity.unwrap();
        assert!(agent_processes::noted().contains(&identity));

        turn.interrupt();
        last_output(&receiver);

        wait_until("it is forgotten", || {
            !agent_processes::noted().contains(&identity)
        });
    }
}
