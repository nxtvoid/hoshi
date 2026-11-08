use crate::{OutputSink, RunningTurn};
use hoshi_core::{ProviderOutput, TurnFailure, TurnOutcome};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

const STDERR_TAIL_BYTES: usize = 2_000;

/// One agent process running one turn. Dropping it does not stop the process.
pub struct ProcessTurn {
    child: Arc<Mutex<Child>>,
    pid: u32,
    interrupted: Arc<AtomicBool>,
}

impl RunningTurn for ProcessTurn {
    fn interrupt(&self) {
        self.interrupted.store(true, Ordering::SeqCst);
        kill_tree(self.pid, &self.child);
    }
}

/// Spawns `command`, writes `input` to its stdin and closes it, then feeds
/// each stdout line through `parse` into `sink` on a background thread.
pub fn spawn_turn(
    mut command: Command,
    input: String,
    mut sink: OutputSink,
    mut parse: impl FnMut(&str) -> Vec<ProviderOutput> + Send + 'static,
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
    if let Some(mut stdin) = child.stdin.take() {
        // A write error means the process already died; the reader reports how.
        let _ = stdin.write_all(input.as_bytes());
    }
    let child = Arc::new(Mutex::new(child));
    let interrupted = Arc::new(AtomicBool::new(false));

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
    thread::spawn(move || {
        let mut completed = false;
        if let Some(stdout) = stdout {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                for output in parse(&line) {
                    completed |= matches!(output, ProviderOutput::TurnCompleted { .. });
                    sink(output);
                }
            }
        }
        let status = reader_child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .wait();
        let stderr = stderr_tail.join().unwrap_or_default();
        if completed {
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
        let (sender, receiver) = mpsc::channel();
        let sink: OutputSink = Box::new(move |output| {
            let _ = sender.send(output);
        });
        match spawn_turn(command, String::new(), sink, |_| Vec::new()) {
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
}
