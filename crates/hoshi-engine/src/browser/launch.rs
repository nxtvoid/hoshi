//! Finds the Chromium-based browser installed on the machine and starts it headless.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const START_TIMEOUT: Duration = Duration::from_secs(45);
const POLL: Duration = Duration::from_millis(50);
/// How long a browser that was ended may take to let go of its profile.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(5);

/// Edge or Chrome as installed; Edge comes with Windows.
pub fn find_browser() -> Option<PathBuf> {
    candidates().into_iter().find(|path| path.is_file())
}

fn candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    #[cfg(windows)]
    {
        for variable in ["ProgramFiles(x86)", "ProgramFiles", "LocalAppData"] {
            let Some(root) = std::env::var_os(variable) else {
                continue;
            };
            let root = PathBuf::from(root);
            paths.push(root.join(r"Microsoft\Edge\Application\msedge.exe"));
            paths.push(root.join(r"Google\Chrome\Application\chrome.exe"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        for app in [
            "Google Chrome.app/Contents/MacOS/Google Chrome",
            "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "Chromium.app/Contents/MacOS/Chromium",
        ] {
            paths.push(Path::new("/Applications").join(app));
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for name in [
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "microsoft-edge",
        ] {
            for dir in ["/usr/bin", "/usr/local/bin", "/snap/bin"] {
                paths.push(Path::new(dir).join(name));
            }
        }
    }
    paths
}

/// A headless browser this process started; it stops when dropped.
pub struct BrowserProcess {
    child: Child,
    /// Where its DevTools Protocol listens.
    pub url: String,
    /// A profile of its own, used while another browser held the usual one; removed with it.
    spare_profile: Option<PathBuf>,
}

impl Drop for BrowserProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(profile) = &self.spare_profile {
            let deadline = Instant::now() + RELEASE_TIMEOUT;
            while std::fs::remove_dir_all(profile).is_err()
                && profile.exists()
                && Instant::now() < deadline
            {
                thread::sleep(POLL);
            }
        }
    }
}

enum StartError {
    /// It gave the work to a browser already on the profile and quit.
    HandedOver,
    Failed(String),
}

/// Starts `executable` headless with its own profile under `profile`, so it never touches
/// the user's browser. A browser left there by a hoshi that is gone is ended first; when
/// another one still holds the profile, a spare profile is used instead, as a new browser
/// would hand over to that one and never answer.
pub fn launch(executable: &Path, profile: &Path) -> Result<BrowserProcess, String> {
    end_left_behind(profile);
    if !in_use(profile) {
        match start(executable, profile) {
            Ok(process) => {
                claim(profile);
                return Ok(process);
            }
            Err(StartError::Failed(error)) => return Err(error),
            Err(StartError::HandedOver) => {}
        }
    }
    let spare = spare_profile(profile);
    let mut process = start(executable, &spare).map_err(|error| match error {
        StartError::HandedOver => "the browser didn't start".to_owned(),
        StartError::Failed(error) => error,
    })?;
    process.spare_profile = Some(spare);
    Ok(process)
}

fn spare_profile(profile: &Path) -> PathBuf {
    let name = profile
        .file_name()
        .map_or_else(|| "browser".into(), |name| name.to_string_lossy());
    profile.with_file_name(format!("{name}-{}", std::process::id()))
}

/// Its DevTools address is read from the `DevToolsActivePort` file it writes in the profile,
/// which works where it prints nothing, as on Windows.
fn start(executable: &Path, profile: &Path) -> Result<BrowserProcess, StartError> {
    std::fs::create_dir_all(profile).map_err(|error| StartError::Failed(error.to_string()))?;
    let port_file = profile.join("DevToolsActivePort");
    let _ = std::fs::remove_file(&port_file);
    let mut command = Command::new(executable);
    command
        .arg("--headless=new")
        .arg("--remote-debugging-port=0")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(format!("--force-device-scale-factor={}", super::MAX_SCALE))
        .args([
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-gpu",
            "--disable-extensions",
            "--disable-background-networking",
            "--disable-sync",
            "--hide-scrollbars",
            "--mute-audio",
            "about:blank",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .map_err(|error| StartError::Failed(error.to_string()))?;
    let deadline = Instant::now() + START_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(url) = read_port_file(&port_file) {
            return Ok(BrowserProcess {
                child,
                url,
                spare_profile: None,
            });
        }
        // On Windows the process started may hand over to another one and exit, as Edge can.
        if !cfg!(windows) && matches!(child.try_wait(), Ok(Some(_))) {
            return Err(StartError::HandedOver);
        }
        thread::sleep(POLL);
    }
    let _ = child.kill();
    let _ = child.wait();
    Err(StartError::Failed("the browser didn't start".into()))
}

/// Whether a running browser holds `profile`.
#[cfg(windows)]
fn in_use(profile: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    const SHARING_VIOLATION: i32 = 32;
    let lock = profile.join("lockfile");
    lock.exists()
        && std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&lock)
            .is_err_and(|error| error.raw_os_error() == Some(SHARING_VIOLATION))
}

#[cfg(not(windows))]
fn in_use(_: &Path) -> bool {
    false
}

/// Who started the browser on a profile, as `<pid> <start time>`; a browser outlives its hoshi
/// on Windows, as when hoshi was killed.
#[cfg(windows)]
const OWNER_FILE: &str = "hoshi-owner";

/// Notes this process as the one whose browser holds `profile`.
#[cfg(windows)]
fn claim(profile: &Path) {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let Ok(pid) = sysinfo::get_current_pid() else {
        return;
    };
    let Ok(mut system) = System::new() else {
        return;
    };
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    if let Some(process) = system.process(pid) {
        let owner = format!("{} {}", pid.as_u32(), process.start_time());
        let _ = std::fs::write(profile.join(OWNER_FILE), owner);
    }
}

#[cfg(not(windows))]
fn claim(_: &Path) {}

/// Whether the hoshi named in the owner file still runs: `started` gives when a process
/// started, `None` when there is none.
#[cfg_attr(not(windows), allow(dead_code))]
fn owner_runs(owner: Option<&str>, started: impl Fn(u32) -> Option<u64>) -> bool {
    let Some((pid, at)) = owner.and_then(|owner| owner.trim().split_once(' ')) else {
        return false;
    };
    let (Ok(pid), Ok(at)) = (pid.parse::<u32>(), at.parse::<u64>()) else {
        return false;
    };
    started(pid) == Some(at)
}

/// Ends the headless browser holding `profile` when the hoshi that started it is gone.
#[cfg(windows)]
fn end_left_behind(profile: &Path) {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    if !in_use(profile) {
        return;
    }
    let Ok(mut system) = System::new() else {
        return;
    };
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );
    let owner = std::fs::read_to_string(profile.join(OWNER_FILE)).ok();
    let started = |pid: u32| {
        system
            .process(sysinfo::Pid::from_u32(pid))
            .map(sysinfo::Process::start_time)
    };
    if owner_runs(owner.as_deref(), started) {
        return;
    }
    let flag = format!("--user-data-dir={}", profile.display());
    let mut ended = false;
    for process in system.processes().values() {
        let arguments: Vec<String> = process
            .cmd()
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        if left_behind(&arguments, &flag) {
            ended |= process.kill();
        }
    }
    if ended {
        let deadline = Instant::now() + RELEASE_TIMEOUT;
        while in_use(profile) && Instant::now() < deadline {
            thread::sleep(POLL);
        }
    }
}

#[cfg(not(windows))]
fn end_left_behind(_: &Path) {}

/// The main process of a headless browser started on the profile `flag` names.
#[cfg_attr(not(windows), allow(dead_code))]
fn left_behind(arguments: &[String], flag: &str) -> bool {
    let has = |wanted: &str| {
        arguments
            .iter()
            .any(|argument| argument.eq_ignore_ascii_case(wanted))
    };
    has(flag)
        && has("--headless=new")
        && !arguments
            .iter()
            .any(|argument| argument.starts_with("--type="))
}

/// `ws://127.0.0.1:<port><path>` from the file's two lines, once both are written.
fn read_port_file(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let port = lines.next()?.trim();
    let path = lines.next()?.trim();
    (!port.is_empty() && path.starts_with('/')).then(|| format!("ws://127.0.0.1:{port}{path}"))
}

#[cfg(test)]
mod tests {
    use super::super::cdp::Connection;
    use super::{
        BrowserProcess, POLL, RELEASE_TIMEOUT, START_TIMEOUT, find_browser, in_use, launch,
        left_behind, owner_runs, spare_profile,
    };
    use serde_json::json;
    use std::path::Path;
    use std::time::Instant;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn only_the_main_headless_process_on_the_profile_is_left_behind() {
        let flag = r"--user-data-dir=C:\Users\me\.hoshi\browser";
        let lower = r"--user-data-dir=c:\users\me\.hoshi\browser";
        assert!(left_behind(
            &args(&["msedge.exe", "--headless=new", lower]),
            flag
        ));
        assert!(!left_behind(
            &args(&["msedge.exe", "--headless=new", flag, "--type=renderer"]),
            flag
        ));
        assert!(!left_behind(&args(&["msedge.exe", flag]), flag));
        let other = r"--user-data-dir=C:\other";
        assert!(!left_behind(
            &args(&["msedge.exe", "--headless=new", other]),
            flag
        ));
    }

    #[test]
    fn a_spare_profile_sits_next_to_the_usual_one() {
        let spare = spare_profile(Path::new("/home/me/.hoshi/browser"));
        assert_eq!(spare.parent(), Some(Path::new("/home/me/.hoshi")));
        assert_eq!(
            spare.file_name().unwrap().to_string_lossy(),
            format!("browser-{}", std::process::id())
        );
    }

    #[test]
    fn a_profile_another_browser_holds_starts_on_a_spare_one() {
        let Some(executable) = find_browser() else {
            eprintln!("no Chromium-based browser here; skipping");
            return;
        };
        let profile = std::env::temp_dir().join(format!("hoshi-held-{}", std::process::id()));
        let first = held(&executable, &profile);
        let started = Instant::now();
        let second = launch(&executable, &profile).unwrap();
        assert!(started.elapsed() < START_TIMEOUT / 2);
        assert!(first.spare_profile.is_none());
        assert_eq!(second.spare_profile, Some(spare_profile(&profile)));
        assert_ne!(first.url, second.url);
        close(second);
        close(first);
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[cfg(windows)]
    #[test]
    fn a_browser_whose_hoshi_is_gone_is_ended_and_its_profile_taken_back() {
        let Some(executable) = find_browser() else {
            eprintln!("no Chromium-based browser here; skipping");
            return;
        };
        let profile = std::env::temp_dir().join(format!("hoshi-left-{}", std::process::id()));
        let left = held(&executable, &profile);
        std::fs::write(profile.join(super::OWNER_FILE), "1 1").unwrap();

        let again = launch(&executable, &profile).unwrap();

        assert!(again.spare_profile.is_none());
        assert!(Connection::connect(&left.url).is_err());
        close(again);
        drop(left);
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[test]
    fn only_a_running_owner_with_the_same_start_keeps_the_profile() {
        let started = |pid: u32| (pid == 7).then_some(100);
        assert!(owner_runs(Some("7 100\n"), started));
        assert!(!owner_runs(Some("7 99"), started));
        assert!(!owner_runs(Some("8 100"), started));
        assert!(!owner_runs(Some("garbage"), started));
        assert!(!owner_runs(None, started));
    }

    /// A browser started on `profile` once it holds it.
    fn held(executable: &Path, profile: &Path) -> BrowserProcess {
        let process = launch(executable, profile).unwrap();
        let since = Instant::now();
        while !in_use(profile) && since.elapsed() < RELEASE_TIMEOUT {
            std::thread::sleep(POLL);
        }
        process
    }

    /// Closes it as `Browser` does: the process started may have handed over to another.
    fn close(process: BrowserProcess) {
        if let Ok(connection) = Connection::connect(&process.url) {
            let _ = connection.call_within("Browser.close", json!({}), None, RELEASE_TIMEOUT);
        }
        let spare = process.spare_profile.clone();
        drop(process);
        assert!(spare.is_none_or(|spare| !spare.exists()));
    }

    #[cfg(windows)]
    #[test]
    fn a_profile_is_in_use_while_its_lock_is_held() {
        use std::os::windows::fs::OpenOptionsExt;
        let profile = std::env::temp_dir().join(format!("hoshi-lock-{}", std::process::id()));
        std::fs::create_dir_all(&profile).unwrap();
        assert!(!in_use(&profile));
        let held = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(profile.join("lockfile"))
            .unwrap();
        assert!(in_use(&profile));
        drop(held);
        assert!(!in_use(&profile));
        let _ = std::fs::remove_dir_all(&profile);
    }
}
