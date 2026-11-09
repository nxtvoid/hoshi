//! Shells in pseudo-terminals, one per terminal id. They outlive the view that shows
//! them: a remounted view reattaches and redraws from the bounded history.

use hoshi_core::{CommandError, TerminalAttach, TerminalEvent, TerminalExit};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

/// Enough to redraw a screen and some scrollback, small enough to send at once.
const HISTORY_BYTES: usize = 256 * 1024;
const READ_BUFFER: usize = 16 * 1024;

pub type TerminalSink = Box<dyn Fn(TerminalEvent) + Send + Sync>;

/// Where a new shell starts and what it learns about the thread.
pub struct Launch<'a> {
    pub thread_id: &'a str,
    pub cwd: &'a str,
    pub project_root: &'a str,
    pub worktree_path: Option<&'a str>,
    /// Run by the shell, which exits with it, instead of an interactive session.
    pub command: Option<&'a str>,
}

#[derive(Default)]
pub struct Terminals {
    sessions: Mutex<HashMap<String, Session>>,
}

struct Session {
    thread_id: String,
    cwd: String,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    shared: Arc<Mutex<Shared>>,
}

/// What the reader and waiter threads share with the commands.
struct Shared {
    history: String,
    sink: Option<TerminalSink>,
    /// Hears everything too, whether a view is attached or not.
    observer: Option<TerminalSink>,
    /// Dropped once the shell exits, which ends the reader on every platform.
    master: Option<Box<dyn MasterPty + Send>>,
    exit: Option<TerminalExit>,
    reader_done: bool,
    exit_sent: bool,
}

impl Shared {
    fn send(&self, event: TerminalEvent) {
        if let Some(observer) = &self.observer {
            observer(event.clone());
        }
        if let Some(sink) = &self.sink {
            sink(event);
        }
    }

    /// The shell's exit is reported once both its output is drained and its status is known.
    fn report_exit(&mut self) {
        if self.exit_sent || !self.reader_done {
            return;
        }
        if let Some(exit) = self.exit {
            self.exit_sent = true;
            self.send(TerminalEvent::Exited { exit });
        }
    }
}

impl Terminals {
    /// Attaches `sink` to the terminal, starting its shell first if it is not running.
    pub fn open(
        &self,
        terminal_id: &str,
        launch: Launch<'_>,
        size: (u16, u16),
        sink: TerminalSink,
    ) -> Result<TerminalAttach, CommandError> {
        let mut sessions = guard(&self.sessions);
        if !sessions.contains_key(terminal_id) {
            let session = spawn(&launch, size, None)?;
            sessions.insert(terminal_id.to_owned(), session);
        }
        let session = &sessions[terminal_id];
        let mut shared = guard(&session.shared);
        if let Some(master) = &shared.master {
            let _ = master.resize(pty_size(size));
        }
        shared.sink = Some(sink);
        Ok(TerminalAttach {
            cwd: session.cwd.clone(),
            history: shared.history.clone(),
            exit: shared.exit,
        })
    }

    /// Starts `launch` afresh, replacing a terminal of the same id, with `observer` hearing
    /// its output and exit until it ends.
    pub fn run(
        &self,
        terminal_id: &str,
        launch: Launch<'_>,
        observer: TerminalSink,
    ) -> Result<(), CommandError> {
        let session = spawn(&launch, (120, 30), Some(observer))?;
        let replaced = guard(&self.sessions).insert(terminal_id.to_owned(), session);
        if let Some(replaced) = replaced {
            stop(replaced);
        }
        Ok(())
    }

    pub fn write(&self, terminal_id: &str, data: &str) -> Result<(), CommandError> {
        let writer = self.with_session(terminal_id, |session| session.writer.clone())?;
        let mut writer = guard(&writer);
        writer
            .write_all(data.as_bytes())
            .and_then(|()| writer.flush())
            .map_err(|error| CommandError::TerminalFailed {
                detail: error.to_string(),
            })
    }

    pub fn resize(&self, terminal_id: &str, size: (u16, u16)) -> Result<(), CommandError> {
        self.with_session(terminal_id, |session| {
            if let Some(master) = &guard(&session.shared).master {
                let _ = master.resize(pty_size(size));
            }
        })
    }

    /// Stops streaming to the view; the shell keeps running.
    pub fn detach(&self, terminal_id: &str) {
        if let Some(session) = guard(&self.sessions).get(terminal_id) {
            guard(&session.shared).sink = None;
        }
    }

    pub fn close(&self, terminal_id: &str) {
        let session = guard(&self.sessions).remove(terminal_id);
        if let Some(session) = session {
            stop(session);
        }
    }

    /// Whether a shell still runs for the thread; one that exited keeps only its output.
    pub fn any_open_for(&self, thread_id: &str) -> bool {
        guard(&self.sessions)
            .values()
            .any(|session| session.thread_id == thread_id && guard(&session.shared).exit.is_none())
    }

    /// Closes the terminals of every thread `keep` rejects.
    pub fn retain_threads(&self, keep: impl Fn(&str) -> bool) {
        let closed: Vec<_> = {
            let mut sessions = guard(&self.sessions);
            let ids: Vec<_> = sessions
                .iter()
                .filter(|(_, session)| !keep(&session.thread_id))
                .map(|(id, _)| id.clone())
                .collect();
            ids.iter().filter_map(|id| sessions.remove(id)).collect()
        };
        closed.into_iter().for_each(stop);
    }

    fn with_session<T>(
        &self,
        terminal_id: &str,
        read: impl FnOnce(&Session) -> T,
    ) -> Result<T, CommandError> {
        guard(&self.sessions)
            .get(terminal_id)
            .map(read)
            .ok_or_else(|| CommandError::TerminalNotFound {
                terminal_id: terminal_id.to_owned(),
            })
    }
}

impl Drop for Terminals {
    fn drop(&mut self) {
        let sessions = std::mem::take(&mut *guard(&self.sessions));
        sessions.into_values().for_each(stop);
    }
}

fn stop(mut session: Session) {
    let _ = session.killer.kill();
    let master = {
        let mut shared = guard(&session.shared);
        shared.sink = None;
        shared.observer = None;
        shared.master.take()
    };
    drop(master);
}

fn spawn(
    launch: &Launch<'_>,
    size: (u16, u16),
    observer: Option<TerminalSink>,
) -> Result<Session, CommandError> {
    let failed = |error: &dyn std::fmt::Display| CommandError::TerminalFailed {
        detail: error.to_string(),
    };
    let pair = native_pty_system()
        .openpty(pty_size(size))
        .map_err(|error| failed(&error))?;
    let mut child = pair
        .slave
        .spawn_command(shell_command(launch))
        .map_err(|error| failed(&error))?;
    drop(pair.slave);
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| failed(&error))?;
    let writer: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(
        pair.master.take_writer().map_err(|error| failed(&error))?,
    ));
    let killer = child.clone_killer();
    let shared = Arc::new(Mutex::new(Shared {
        history: String::new(),
        sink: None,
        observer,
        master: Some(pair.master),
        exit: None,
        reader_done: false,
        exit_sent: false,
    }));

    let output = shared.clone();
    let answers = writer.clone();
    thread::spawn(move || read_output(reader, &output, &answers));
    let waited = shared.clone();
    thread::spawn(move || {
        let code = child.wait().ok().map(|status| status.exit_code());
        let master = {
            let mut shared = guard(&waited);
            shared.exit = Some(TerminalExit { code });
            shared.report_exit();
            shared.master.take()
        };
        // Closing a Windows pseudo console waits for its output to be read, so the
        // reader must be free to take the lock meanwhile.
        drop(master);
    });

    Ok(Session {
        thread_id: launch.thread_id.to_owned(),
        cwd: launch.cwd.to_owned(),
        writer,
        killer,
        shared,
    })
}

/// Asked by Windows' pseudo console before a shell starts; a view answers it, and with
/// none attached the shell would wait forever.
const CURSOR_POSITION_QUERY: &str = "\u{1b}[6n";

fn read_output(
    mut reader: Box<dyn Read + Send>,
    shared: &Mutex<Shared>,
    writer: &Mutex<Box<dyn Write + Send>>,
) {
    let mut buffer = vec![0; READ_BUFFER];
    let mut decoder = Utf8Decoder::default();
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let data = decoder.decode(&buffer[..read]);
        if data.is_empty() {
            continue;
        }
        let mut shared = guard(shared);
        if shared.sink.is_none() && data.contains(CURSOR_POSITION_QUERY) {
            let mut writer = guard(writer);
            let _ = writer.write_all(b"\x1b[1;1R").and_then(|()| writer.flush());
        }
        push_history(&mut shared.history, &data);
        shared.send(TerminalEvent::Output { data });
    }
    let mut shared = guard(shared);
    shared.reader_done = true;
    shared.report_exit();
}

fn push_history(history: &mut String, data: &str) {
    history.push_str(data);
    if history.len() <= HISTORY_BYTES {
        return;
    }
    let mut cut = history.len() - HISTORY_BYTES;
    while !history.is_char_boundary(cut) {
        cut += 1;
    }
    let cut = history[cut..]
        .find('\n')
        .map_or(cut, |newline| cut + newline + 1);
    history.drain(..cut);
}

fn shell_command(launch: &Launch<'_>) -> CommandBuilder {
    let mut command = default_shell();
    if let Some(script) = launch.command {
        command.args(script_args(script));
    }
    command.cwd(launch.cwd);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("HOSHI_PROJECT_ROOT", launch.project_root);
    if let Some(worktree) = launch.worktree_path {
        command.env("HOSHI_WORKTREE_PATH", worktree);
    }
    command
}

#[cfg(windows)]
fn default_shell() -> CommandBuilder {
    let program = if on_path("pwsh.exe") {
        "pwsh.exe"
    } else {
        "powershell.exe"
    };
    let mut command = CommandBuilder::new(program);
    command.arg("-NoLogo");
    command
}

#[cfg(not(windows))]
fn default_shell() -> CommandBuilder {
    let program = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut command = CommandBuilder::new(program);
    if cfg!(target_os = "macos") {
        command.arg("-l");
    }
    command
}

#[cfg(windows)]
fn script_args(script: &str) -> [&str; 2] {
    ["-Command", script]
}

#[cfg(not(windows))]
fn script_args(script: &str) -> [&str; 2] {
    ["-c", script]
}

#[cfg(windows)]
fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

fn pty_size((cols, rows): (u16, u16)) -> PtySize {
    PtySize {
        cols: cols.max(1),
        rows: rows.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Decodes a byte stream whose reads may split a character, keeping the split part
/// for the next read.
#[derive(Default)]
struct Utf8Decoder {
    pending: Vec<u8>,
}

impl Utf8Decoder {
    fn decode(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut text = String::new();
        let mut rest = self.pending.as_slice();
        loop {
            match std::str::from_utf8(rest) {
                Ok(valid) => {
                    text.push_str(valid);
                    rest = &[];
                    break;
                }
                Err(error) => {
                    let (valid, after) = rest.split_at(error.valid_up_to());
                    text.push_str(std::str::from_utf8(valid).unwrap_or_default());
                    match error.error_len() {
                        Some(invalid) => {
                            text.push(char::REPLACEMENT_CHARACTER);
                            rest = &after[invalid..];
                        }
                        None => {
                            rest = after;
                            break;
                        }
                    }
                }
            }
        }
        self.pending = rest.to_vec();
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn a_character_split_across_reads_is_kept_for_the_next_one() {
        let mut decoder = Utf8Decoder::default();
        let bytes = "añ".as_bytes();

        assert_eq!(decoder.decode(&bytes[..2]), "a");
        assert_eq!(decoder.decode(&bytes[2..]), "ñ");
    }

    #[test]
    fn invalid_bytes_become_replacement_characters() {
        let mut decoder = Utf8Decoder::default();

        assert_eq!(decoder.decode(b"a\xffb"), "a\u{fffd}b");
    }

    #[test]
    fn history_keeps_the_latest_whole_lines() {
        let mut history = String::new();
        push_history(&mut history, &"x".repeat(HISTORY_BYTES));
        push_history(&mut history, "\nlast line\n");

        assert!(history.len() <= HISTORY_BYTES);
        assert_eq!(history, "last line\n");
    }

    #[test]
    fn a_shell_starts_in_the_given_folder_and_reports_its_exit() {
        let dir = std::env::temp_dir();
        let cwd = dir.to_string_lossy().into_owned();
        let terminals = Terminals::default();
        let (sender, receiver) = std::sync::mpsc::channel();
        let launch = Launch {
            thread_id: "t",
            cwd: &cwd,
            project_root: &cwd,
            worktree_path: None,
            command: None,
        };

        let attach = terminals
            .open(
                "term",
                launch,
                (80, 24),
                Box::new(move |event| {
                    let _ = sender.send(event);
                }),
            )
            .unwrap();
        terminals.write("term", "exit\r").unwrap();

        assert_eq!(attach.cwd, cwd);
        let deadline = Instant::now() + Duration::from_secs(20);
        let exited = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match receiver.recv_timeout(left).expect("the shell did not exit") {
                TerminalEvent::Exited { exit } => break exit,
                // A Windows pseudo console asks where the cursor is and waits for the answer.
                TerminalEvent::Output { data } if data.contains("\x1b[6n") => {
                    terminals.write("term", "\x1b[1;1R").unwrap();
                }
                TerminalEvent::Output { .. } => {}
            }
        };
        assert_eq!(exited.code, Some(0));
    }
}
