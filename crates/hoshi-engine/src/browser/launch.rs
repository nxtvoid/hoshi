//! Finds the Chromium-based browser installed on the machine and starts it headless.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const START_TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(50);

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
}

impl Drop for BrowserProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts `executable` headless with its own profile under `profile`, so it never touches
/// the user's browser. Its DevTools address is read from the `DevToolsActivePort` file it
/// writes there, which works where it prints nothing, as on Windows.
pub fn launch(executable: &Path, profile: &Path) -> Result<BrowserProcess, String> {
    std::fs::create_dir_all(profile).map_err(|error| error.to_string())?;
    let port_file = profile.join("DevToolsActivePort");
    let _ = std::fs::remove_file(&port_file);
    let mut command = Command::new(executable);
    command
        .arg("--headless=new")
        .arg("--remote-debugging-port=0")
        .arg(format!("--user-data-dir={}", profile.display()))
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
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let deadline = Instant::now() + START_TIMEOUT;
    while Instant::now() < deadline {
        // The process started may hand over to another one and exit, as Edge can.
        if let Some(url) = read_port_file(&port_file) {
            return Ok(BrowserProcess { child, url });
        }
        thread::sleep(POLL);
    }
    let _ = child.kill();
    let _ = child.wait();
    Err("the browser didn't start".into())
}

/// `ws://127.0.0.1:<port><path>` from the file's two lines, once both are written.
fn read_port_file(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let port = lines.next()?.trim();
    let path = lines.next()?.trim();
    (!port.is_empty() && path.starts_with('/')).then(|| format!("ws://127.0.0.1:{port}{path}"))
}
