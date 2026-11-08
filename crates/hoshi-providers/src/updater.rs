//! Runs a CLI's updater and finds programs the way the system would.

use crate::process::{hide_console, kill_tree};
use hoshi_core::ProviderUpdateFailure;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

/// Native updaters download a large binary without printing progress, and stopping one
/// midway can leave a truncated binary behind.
const TIMEOUT: Duration = Duration::from_secs(30 * 60);
const TAIL_LINES: usize = 12;
const LINE_MAX_CHARS: usize = 200;

/// A command that updates a provider's CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Updater {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl Updater {
    /// As typed in a terminal.
    pub fn display(&self) -> String {
        let program = self
            .program
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        std::iter::once(program)
            .chain(self.args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateFailure {
    pub reason: ProviderUpdateFailure,
    /// The updater's last lines of output.
    pub output: Option<String>,
}

/// Runs `updater` to its end, handing each line it prints to `progress`.
pub fn run(updater: &Updater, mut progress: impl FnMut(String)) -> Result<(), UpdateFailure> {
    let mut command = Command::new(&updater.program);
    command
        .args(&updater.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console(&mut command);
    let mut child = command.spawn().map_err(|_| UpdateFailure {
        reason: ProviderUpdateFailure::SpawnFailed,
        output: None,
    })?;
    let (sender, lines) = mpsc::channel();
    let streams: [Option<Box<dyn Read + Send>>; 2] = [
        child.stdout.take().map(|out| Box::new(out) as _),
        child.stderr.take().map(|err| Box::new(err) as _),
    ];
    for stream in streams.into_iter().flatten() {
        let sender = sender.clone();
        thread::spawn(move || read_lines(stream, |line| sender.send(line).is_ok()));
    }
    drop(sender);
    let pid = child.id();
    let child = Mutex::new(child);
    let deadline = Instant::now() + TIMEOUT;
    let mut tail = VecDeque::with_capacity(TAIL_LINES);
    loop {
        match lines.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(line) => {
                if tail.len() == TAIL_LINES {
                    tail.pop_front();
                }
                tail.push_back(line.clone());
                progress(line);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                kill_tree(pid, &child);
                return Err(UpdateFailure {
                    reason: ProviderUpdateFailure::TimedOut,
                    output: joined(tail),
                });
            }
        }
    }
    let status = child
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .wait();
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(UpdateFailure {
            reason: ProviderUpdateFailure::Exited {
                code: status.code(),
            },
            output: joined(tail),
        }),
        Err(_) => Err(UpdateFailure {
            reason: ProviderUpdateFailure::Exited { code: None },
            output: joined(tail),
        }),
    }
}

fn joined(tail: VecDeque<String>) -> Option<String> {
    (!tail.is_empty()).then(|| Vec::from(tail).join("\n"))
}

/// Progress bars redraw a line with `\r`, so it ends a line too. Stops when `send` refuses.
fn read_lines(mut stream: impl Read, mut send: impl FnMut(String) -> bool) {
    let mut buffer = [0u8; 4096];
    let mut pending = Vec::new();
    loop {
        let read = match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        for &byte in &buffer[..read] {
            if byte == b'\n' || byte == b'\r' {
                if let Some(line) = clean_line(&pending)
                    && !send(line)
                {
                    return;
                }
                pending.clear();
            } else {
                pending.push(byte);
            }
        }
    }
    if let Some(line) = clean_line(&pending) {
        send(line);
    }
}

/// The text of a line of terminal output, without colors or cursor moves.
fn clean_line(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let mut clean = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    for c in chars.by_ref() {
                        if c == '\u{7}' {
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if !c.is_control() {
            clean.push(c);
        }
    }
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.is_empty() {
        return None;
    }
    Some(match clean.char_indices().nth(LINE_MAX_CHARS) {
        Some((index, _)) => format!("{}…", &clean[..index]),
        None => clean,
    })
}

/// Where the system finds `name`: itself when it is a path, else the first match on the
/// `PATH`. On Windows a bare name only matches an `.exe`, as `Command` does, unless
/// `any_extension` also allows the `PATHEXT` ones, such as `npm.cmd`.
pub(crate) fn find_program(name: &str, any_extension: bool) -> Option<PathBuf> {
    let path = Path::new(name);
    if path.components().count() > 1 {
        return path.is_file().then(|| path.to_owned());
    }
    let candidates = candidate_names(name, any_extension);
    std::env::split_paths(&std::env::var_os("PATH")?)
        .flat_map(|dir| candidates.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
}

fn candidate_names(name: &str, any_extension: bool) -> Vec<OsString> {
    if !cfg!(windows) || Path::new(name).extension().is_some() {
        return vec![name.into()];
    }
    if !any_extension {
        return vec![format!("{name}.exe").into()];
    }
    std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
        .split(';')
        .filter(|extension| !extension.is_empty())
        .map(|extension| format!("{name}{}", extension.to_ascii_lowercase()).into())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(output: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        read_lines(output, |line| {
            lines.push(line);
            true
        });
        lines
    }

    #[test]
    fn output_splits_on_redraws_and_loses_colors_and_blank_lines() {
        let output = b"\x1b[32mChecking for updates\x1b[0m\r\nDownloading 10%\rDownloading 55%\r\n\n\x1b]0;title\x07Updated to 2.1.320";

        assert_eq!(
            lines(output),
            [
                "Checking for updates",
                "Downloading 10%",
                "Downloading 55%",
                "Updated to 2.1.320"
            ]
        );
    }

    #[test]
    fn a_long_line_is_shortened() {
        let line = clean_line("x".repeat(500).as_bytes()).unwrap();

        assert_eq!(line.chars().count(), LINE_MAX_CHARS + 1);
        assert!(line.ends_with('…'));
    }

    #[test]
    fn the_command_reads_as_typed() {
        let updater = Updater {
            program: PathBuf::from("/home/me/.local/bin/claude.exe"),
            args: vec!["update".into()],
        };

        assert_eq!(updater.display(), "claude update");
    }

    #[test]
    fn a_failing_updater_reports_its_exit_code_and_last_lines() {
        let (program, script) = if cfg!(windows) {
            (
                "cmd",
                "/C echo downloading && echo no space left 1>&2 && exit 3",
            )
        } else {
            ("sh", "-c|echo downloading; echo no space left >&2; exit 3")
        };
        let args = if cfg!(windows) {
            script.splitn(2, ' ').map(str::to_owned).collect()
        } else {
            script.split('|').map(str::to_owned).collect()
        };
        let updater = Updater {
            program: find_program(program, false).unwrap(),
            args,
        };
        let mut seen = Vec::new();

        let failure = run(&updater, |line| seen.push(line)).unwrap_err();

        assert_eq!(
            failure.reason,
            ProviderUpdateFailure::Exited { code: Some(3) }
        );
        assert!(seen.contains(&"no space left".to_owned()));
        assert!(failure.output.unwrap().contains("downloading"));
    }

    #[test]
    fn a_missing_updater_fails_to_spawn() {
        let updater = Updater {
            program: PathBuf::from("hoshi-no-such-updater"),
            args: Vec::new(),
        };

        assert_eq!(
            run(&updater, |_| {}).unwrap_err().reason,
            ProviderUpdateFailure::SpawnFailed
        );
    }
}
