//! Runs a CLI's updater and finds programs the way the system would.

use crate::codex::install::CodexInstall;
use crate::process::{hide_console, kill_tree};
use hoshi_core::{ProviderUpdateFailure, UpdateStep};
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

/// What updates a provider's CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Updater {
    /// A command that updates it in place.
    Command {
        program: PathBuf,
        args: Vec<String>,
        /// Set on top of the environment hoshi runs in.
        env: Vec<(String, String)>,
    },
    /// hoshi downloads the release into its own folder.
    Managed(CodexInstall),
}

impl Updater {
    pub fn command(program: PathBuf, args: Vec<String>) -> Self {
        Self::Command {
            program,
            args,
            env: Vec::new(),
        }
    }

    /// As typed in a terminal; a managed install names the package it installs.
    pub fn display(&self) -> String {
        let (program, args) = match self {
            Self::Command { program, args, .. } => (program, args),
            Self::Managed(install) => return install.package().to_owned(),
        };
        let program = program
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        std::iter::once(program)
            .chain(args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// What an updater reports as it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateProgress {
    /// A line the updater command printed.
    Line(String),
    /// The step a managed install reached.
    Step(UpdateStep),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateFailure {
    pub reason: ProviderUpdateFailure,
    /// The updater's last lines of output.
    pub output: Option<String>,
}

/// Runs `updater` to its end, handing what it reports to `progress`.
pub fn run(
    updater: &Updater,
    mut progress: impl FnMut(UpdateProgress),
) -> Result<(), UpdateFailure> {
    match updater {
        Updater::Command { program, args, env } => run_command(program, args, env, |line| {
            progress(UpdateProgress::Line(line))
        }),
        Updater::Managed(install) => install
            .install(|step| progress(UpdateProgress::Step(step)))
            .map(|_| ()),
    }
}

fn run_command(
    program: &Path,
    args: &[String],
    env: &[(String, String)],
    mut progress: impl FnMut(String),
) -> Result<(), UpdateFailure> {
    let mut command = Command::new(program);
    with_search_path(&mut command);
    command
        .args(args)
        .envs(env.iter().map(|(name, value)| (name, value)))
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

/// Where the system finds `name`: itself when it is a path, else the first match in
/// [`search_path`]. On Windows a bare name only matches an `.exe`, as `Command` does, unless
/// `any_extension` also allows the `PATHEXT` ones, such as `npm.cmd`.
pub(crate) fn find_program(name: &str, any_extension: bool) -> Option<PathBuf> {
    find_in(name, any_extension, search_path)
}

/// Like [`find_program`], looking for a bare name only in the folders `dirs` gives.
pub(crate) fn find_in(
    name: &str,
    any_extension: bool,
    dirs: impl FnOnce() -> Vec<PathBuf>,
) -> Option<PathBuf> {
    let path = Path::new(name);
    if path.components().count() > 1 {
        if path.is_file() {
            return Some(path.to_owned());
        }
        let exe = path.with_extension("exe");
        return (cfg!(windows) && path.extension().is_none() && exe.is_file()).then_some(exe);
    }
    let candidates = candidate_names(name, any_extension);
    dirs()
        .iter()
        .flat_map(|dir| candidates.iter().map(move |name| dir.join(name)))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// An empty program at `path` that a `PATH` search takes.
#[cfg(test)]
pub(crate) fn fake_program(path: &Path) {
    std::fs::write(path, "").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// Starts `command` with [`search_path`] as its `PATH`, so a launcher such as `codex.cmd`
/// finds `node` even when it was installed after hoshi started.
pub(crate) fn with_search_path(command: &mut Command) {
    if let Some(path) = path_of(search_path()) {
        command.env("PATH", path);
    }
}

fn path_of(dirs: Vec<PathBuf>) -> Option<OsString> {
    if dirs.is_empty() {
        return None;
    }
    std::env::join_paths(dirs).ok()
}

/// The `PATH` hoshi started with, then, on Windows, the folders the machine's and the user's
/// `PATH` hold now, so a program installed since hoshi started is found.
pub(crate) fn search_path() -> Vec<PathBuf> {
    let process = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    merge_paths(process, stored_path())
}

/// The `PATH` for a shell hoshi starts: [`search_path`] plus the folders agent installers
/// put their CLIs in, so one just installed runs by name.
pub fn shell_path() -> Option<OsString> {
    let installed = crate::claude::install::native_dir(std::env::home_dir().as_deref());
    let dirs = merge_paths(
        search_path(),
        installed.into_iter().filter(|dir| dir.is_dir()).collect(),
    );
    path_of(dirs)
}

fn merge_paths(mut dirs: Vec<PathBuf>, more: Vec<PathBuf>) -> Vec<PathBuf> {
    let key = |dir: &Path| {
        let text = dir.to_string_lossy();
        let text = text.trim_end_matches(['/', '\\']);
        if cfg!(windows) {
            text.to_lowercase()
        } else {
            text.to_owned()
        }
    };
    let mut seen: std::collections::HashSet<String> = dirs.iter().map(|dir| key(dir)).collect();
    for dir in more {
        if !dir.as_os_str().is_empty() && seen.insert(key(&dir)) {
            dirs.push(dir);
        }
    }
    dirs
}

#[cfg(windows)]
fn stored_path() -> Vec<PathBuf> {
    use windows_registry::{CURRENT_USER, LOCAL_MACHINE};
    const MACHINE: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
    let machine = LOCAL_MACHINE.open(MACHINE).ok();
    let user = CURRENT_USER.open("Environment").ok();
    let stored = |name: &str| {
        [&user, &machine]
            .into_iter()
            .flatten()
            .find_map(|key| key.get_string(name).ok())
    };
    let lookup = |name: &str| std::env::var(name).ok().or_else(|| stored(name));
    [&machine, &user]
        .into_iter()
        .flatten()
        .filter_map(|key| key.get_string("Path").ok())
        .flat_map(|path| usable_dirs(&expand_variables(&path, lookup)))
        .collect()
}

/// The folders in an expanded `PATH` value that are absolute and have no variable left.
#[cfg_attr(not(windows), allow(dead_code))]
fn usable_dirs(path: &str) -> Vec<PathBuf> {
    std::env::split_paths(path)
        .filter(|dir| dir.is_absolute() && !dir.to_string_lossy().contains('%'))
        .collect()
}

#[cfg(not(windows))]
fn stored_path() -> Vec<PathBuf> {
    Vec::new()
}

/// `text` with each `%NAME%` replaced by what `lookup` gives for it; unknown ones stay as
/// written, as Windows leaves them.
#[cfg_attr(not(windows), allow(dead_code))]
fn expand_variables(text: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut expanded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        expanded.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            rest = &rest[start..];
            break;
        };
        let name = &after[..end];
        match lookup(name).filter(|_| !name.is_empty()) {
            Some(value) => {
                expanded.push_str(&value);
                rest = &after[end + 1..];
            }
            None => {
                expanded.push('%');
                expanded.push_str(name);
                rest = &after[end..];
            }
        }
    }
    expanded.push_str(rest);
    expanded
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
        let updater = Updater::Command {
            env: Vec::new(),
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
        let updater = Updater::Command {
            env: Vec::new(),
            program: find_program(program, false).unwrap(),
            args,
        };
        let mut seen = Vec::new();

        let failure = run(&updater, |progress| {
            if let UpdateProgress::Line(line) = progress {
                seen.push(line);
            }
        })
        .unwrap_err();

        assert_eq!(
            failure.reason,
            ProviderUpdateFailure::Exited { code: Some(3) }
        );
        assert!(seen.contains(&"no space left".to_owned()));
        assert!(failure.output.unwrap().contains("downloading"));
    }

    #[test]
    fn stored_path_folders_come_after_the_process_ones_without_repeats() {
        let process = vec![PathBuf::from("/usr/bin"), PathBuf::from("/home/dev/bin/")];
        let stored = vec![
            PathBuf::from("/home/dev/bin"),
            PathBuf::new(),
            PathBuf::from("/home/dev/.local/bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/home/dev/.local/bin"),
        ];

        assert_eq!(
            merge_paths(process, stored),
            [
                PathBuf::from("/usr/bin"),
                PathBuf::from("/home/dev/bin/"),
                PathBuf::from("/home/dev/.local/bin"),
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_folders_match_whatever_their_case() {
        let process = vec![PathBuf::from(r"C:\Windows\System32")];
        let stored = vec![
            PathBuf::from(r"c:\windows\system32\"),
            PathBuf::from(r"C:\Users\dev\.local\bin"),
        ];

        assert_eq!(
            merge_paths(process, stored),
            [
                PathBuf::from(r"C:\Windows\System32"),
                PathBuf::from(r"C:\Users\dev\.local\bin"),
            ]
        );
    }

    #[test]
    fn variables_in_a_stored_path_are_expanded_and_unknown_ones_kept() {
        let lookup = |name: &str| match name {
            "USERPROFILE" => Some(r"C:\Users\dev".to_owned()),
            "SystemRoot" => Some(r"C:\Windows".to_owned()),
            _ => None,
        };

        assert_eq!(
            expand_variables(
                r"%SystemRoot%\system32;%USERPROFILE%\.local\bin;%NOPE%\bin;50%;%%",
                lookup
            ),
            r"C:\Windows\system32;C:\Users\dev\.local\bin;%NOPE%\bin;50%;%%"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_stored_path_keeps_only_absolute_folders_without_variables() {
        assert_eq!(
            usable_dirs(r"C:\Windows\system32;%NOPE%\bin;relative\bin;;C:\Users\dev\.local\bin"),
            [
                PathBuf::from(r"C:\Windows\system32"),
                PathBuf::from(r"C:\Users\dev\.local\bin")
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_path_without_an_extension_finds_its_exe() {
        let dir = std::env::temp_dir().join(format!("hoshi-find-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        fake_program(&dir.join("tool.exe"));
        let bare = dir.join("tool");

        assert_eq!(
            find_in(bare.to_str().unwrap(), false, Vec::new),
            Some(dir.join("tool.exe"))
        );
    }

    #[test]
    fn no_folders_leave_the_path_as_it_is() {
        assert_eq!(path_of(Vec::new()), None);
        assert!(path_of(vec![PathBuf::from("bin")]).is_some());
    }

    #[test]
    fn a_bare_name_is_looked_up_only_in_the_given_folders() {
        let dir = std::env::temp_dir().join(format!("hoshi-find-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let name = if cfg!(windows) { "tool.exe" } else { "tool" };
        fake_program(&dir.join(name));

        assert_eq!(
            find_in("tool", false, || vec![dir.clone()]),
            Some(dir.join(name))
        );
        assert_eq!(find_in("tool", false, Vec::new), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_can_not_run_is_passed_over_for_one_later_on_the_path() {
        let first = std::env::temp_dir().join(format!("hoshi-find-{}", uuid::Uuid::new_v4()));
        let second = std::env::temp_dir().join(format!("hoshi-find-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(first.join("tool"), "").unwrap();
        fake_program(&second.join("tool"));

        assert_eq!(
            find_in("tool", false, || vec![first.clone(), second.clone()]),
            Some(second.join("tool"))
        );
    }

    #[test]
    fn a_missing_updater_fails_to_spawn() {
        let updater = Updater::Command {
            env: Vec::new(),
            program: PathBuf::from("hoshi-no-such-updater"),
            args: Vec::new(),
        };

        assert_eq!(
            run(&updater, |_| {}).unwrap_err().reason,
            ProviderUpdateFailure::SpawnFailed
        );
    }
}
