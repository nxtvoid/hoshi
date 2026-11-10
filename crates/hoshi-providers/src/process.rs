use crate::OutputSink;
use hoshi_core::{ProviderOutput, TurnFailure, TurnOutcome};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

const STDERR_TAIL_BYTES: usize = 2_000;

/// One agent process, running a turn or kept between turns for the commands it left in the
/// background. Dropping it does not stop the process.
#[derive(Clone)]
pub struct ProcessTurn {
    child: Arc<Mutex<Child>>,
    pid: u32,
    interrupted: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
    stdin: Stdin,
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
        if !self.exited.load(Ordering::SeqCst) {
            kill_tree(self.pid, &self.child);
        }
    }
}

/// The process's stdin, open while the agent can be answered or given another turn.
#[derive(Clone, Default)]
pub struct Stdin(Arc<Mutex<Option<ChildStdin>>>);

impl Stdin {
    /// `false` when the process no longer reads.
    pub fn write_line(&self, line: &str) -> bool {
        let mut stdin = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(pipe) = stdin.as_mut() else {
            return false;
        };
        let written = pipe
            .write_all(format!("{line}\n").as_bytes())
            .and_then(|()| pipe.flush());
        if written.is_err() {
            *stdin = None;
        }
        written.is_ok()
    }

    pub fn is_open(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    /// An agent reading stream-json exits once its stdin closes.
    pub fn close(&self) {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

/// Spawns `command`, writes `input` to its stdin, then feeds each stdout line through
/// `parse` into `sink` on a background thread. `parse` closes stdin once the agent should
/// end. When the process ends, a turn still open (`in_turn`) completes as failed or
/// interrupted; otherwise the sink gets `ProcessEnded`.
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
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdin = Stdin(Arc::new(Mutex::new(child.stdin.take())));
    // A write error means the process already died; the reader reports how.
    stdin.write_line(input.trim_end());
    let child = Arc::new(Mutex::new(child));
    let interrupted = Arc::new(AtomicBool::new(false));
    let exited = Arc::new(AtomicBool::new(false));

    let stderr_tail = thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(mut stderr) = stderr {
            let _ = stderr.read_to_end(&mut buffer);
        }
        let start = buffer.len().saturating_sub(STDERR_TAIL_BYTES);
        String::from_utf8_lossy(&buffer[start..]).trim().to_owned()
    });

    let reader_child = Arc::clone(&child);
    let reader_interrupted = Arc::clone(&interrupted);
    let reader_exited = Arc::clone(&exited);
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
        let status = reader_child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .wait();
        reader_exited.store(true, Ordering::SeqCst);
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
        child,
        pid,
        interrupted,
        exited,
        stdin,
    })
}

#[cfg(windows)]
pub(crate) fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub(crate) fn hide_console(_command: &mut Command) {}

/// Agents start their own tools (shells, servers), so stopping a turn kills the whole tree.
#[cfg(windows)]
pub(crate) fn kill_tree(pid: u32, child: &Mutex<Child>) {
    let mut taskkill = Command::new("taskkill");
    taskkill
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_console(&mut taskkill);
    if !taskkill.status().is_ok_and(|status| status.success()) {
        kill_child(child);
    }
}

#[cfg(not(windows))]
pub(crate) fn kill_tree(_pid: u32, child: &Mutex<Child>) {
    kill_child(child);
}

fn kill_child(child: &Mutex<Child>) {
    if let Ok(mut child) = child.try_lock() {
        let _ = child.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn run(command: Command) -> Vec<ProviderOutput> {
        run_in_turn(command, true)
    }

    fn run_in_turn(command: Command, in_turn: bool) -> Vec<ProviderOutput> {
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
}
