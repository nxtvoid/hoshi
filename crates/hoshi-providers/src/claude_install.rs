//! Where the Claude Code CLI comes from, its latest release and how to update it.

use crate::Installation;
use crate::claude_models::parse_version;
use crate::process::hide_console;
use crate::updater::{Updater, find_program};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const PACKAGE: &str = "@anthropic-ai/claude-code";
const NATIVE_RELEASES: &str = "https://downloads.claude.ai/claude-code-releases";
const NPM_LATEST: &str = "https://registry.npmjs.org/@anthropic-ai/claude-code/latest";
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, PartialEq, Eq)]
enum Source {
    /// The native installer's, which updates itself with `claude update`.
    Native,
    /// A global npm install under this prefix.
    Npm(PathBuf),
    Bun,
    Pnpm,
    /// A package manager hoshi doesn't drive, such as Homebrew or WinGet.
    Other,
}

/// `installed` is what the CLI reported; reaches the network for the latest release.
pub(crate) fn installation(binary: &str, installed: Option<String>) -> Installation {
    let Some(path) = find_program(binary, false) else {
        return Installation::default();
    };
    let real = dunce::canonicalize(&path).unwrap_or_else(|_| path.clone());
    let source = source_of(&path, &real);
    let latest = match source {
        Source::Native => native_latest(),
        _ => npm_latest(),
    };
    Installation {
        installed,
        latest,
        updater: updater_for(source, path),
    }
}

/// `path` is where the CLI was found and `real` the file it leads to.
fn source_of(path: &Path, real: &Path) -> Source {
    let paths = [normalized(path), normalized(real)];
    let any = |test: fn(&str) -> bool| paths.iter().any(|path| test(path));
    if any(|path| path.contains("/.bun/")) {
        return Source::Bun;
    }
    if any(|path| path.contains("/pnpm/")) {
        return Source::Pnpm;
    }
    if let Some(prefix) = npm_prefix(path, real) {
        return Source::Npm(prefix);
    }
    if any(|path| {
        path.ends_with("/.local/bin/claude")
            || path.ends_with("/.local/bin/claude.exe")
            || path.contains("/.local/share/claude/")
    }) {
        return Source::Native;
    }
    Source::Other
}

fn normalized(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

/// On Unix the CLI links into `<prefix>/lib/node_modules`; on Windows npm puts its
/// launchers right in the prefix, next to `node_modules`.
fn npm_prefix(path: &Path, real: &Path) -> Option<PathBuf> {
    let package = Path::new("node_modules")
        .join("@anthropic-ai")
        .join("claude-code");
    let linked = real.ancestors().find(|dir| dir.ends_with(&package));
    if let Some(prefix) = linked.and_then(|dir| dir.ancestors().nth(3)) {
        let prefix = match prefix.file_name() {
            Some(name) if name == "lib" => prefix.parent()?,
            _ => prefix,
        };
        return Some(prefix.to_owned());
    }
    let dir = path.parent()?;
    dir.join(&package).is_dir().then(|| dir.to_owned())
}

fn updater_for(source: Source, path: PathBuf) -> Option<Updater> {
    let latest = format!("{PACKAGE}@latest");
    let (program, args) = match source {
        Source::Native => (path, vec!["update".to_owned()]),
        // npm 12 refuses install scripts it wasn't told to trust; older ones ignore the flag.
        Source::Npm(prefix) => (
            find_program("npm", true)?,
            vec![
                "install".into(),
                "-g".into(),
                "--prefix".into(),
                prefix.to_string_lossy().into_owned(),
                format!("--allow-scripts={PACKAGE}"),
                latest,
            ],
        ),
        Source::Bun => (
            find_program("bun", true)?,
            vec!["add".into(), "-g".into(), latest],
        ),
        Source::Pnpm => {
            let pnpm = find_program("pnpm", true)?;
            let mut args = vec!["add".to_owned(), "-g".to_owned()];
            // pnpm 10 skips install scripts it wasn't told to trust; the flag came in 10.4.
            if program_version(&pnpm).is_some_and(|version| version >= (10, 4, 0)) {
                args.push(format!("--allow-build={PACKAGE}"));
            }
            args.push(latest);
            (pnpm, args)
        }
        Source::Other => return None,
    };
    Some(Updater { program, args })
}

fn program_version(program: &Path) -> Option<(u32, u32, u32)> {
    let mut command = Command::new(program);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    hide_console(&mut command);
    parse_version(&String::from_utf8_lossy(&command.output().ok()?.stdout))
}

/// The release the native updater would install, on the channel the user picked.
fn native_latest() -> Option<String> {
    let text = get(&format!("{NATIVE_RELEASES}/{}", update_channel()))?;
    let version = text.trim();
    parse_version(version).map(|_| version.to_owned())
}

fn npm_latest() -> Option<String> {
    let text = get(NPM_LATEST)?;
    let package: Value = serde_json::from_str(&text).ok()?;
    Some(package["version"].as_str()?.to_owned())
}

fn update_channel() -> String {
    let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|home| home.join(".claude")));
    let channel = config_dir
        .and_then(|dir| std::fs::read_to_string(dir.join("settings.json")).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|settings| settings["autoUpdatesChannel"].as_str().map(str::to_owned));
    match channel.as_deref() {
        Some("stable") => "stable".into(),
        _ => "latest".into(),
    }
}

fn get(url: &str) -> Option<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(LOOKUP_TIMEOUT))
        .build()
        .into();
    agent.get(url).call().ok()?.body_mut().read_to_string().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(path: &str, real: &str) -> Source {
        source_of(Path::new(path), Path::new(real))
    }

    #[test]
    fn the_native_installer_updates_itself() {
        let windows = r"C:\Users\me\.local\bin\claude.exe";
        let linked = "/home/me/.local/share/claude/versions/2.1.319";

        assert_eq!(source(windows, windows), Source::Native);
        assert_eq!(source("/home/me/.local/bin/claude", linked), Source::Native);
    }

    #[test]
    fn a_global_npm_install_is_updated_under_its_prefix() {
        let real = "/usr/local/lib/node_modules/@anthropic-ai/claude-code/bin/claude.exe";

        assert_eq!(
            source("/usr/local/bin/claude", real),
            Source::Npm(PathBuf::from("/usr/local"))
        );
    }

    #[test]
    fn bun_and_pnpm_installs_are_their_own() {
        let bun = "/home/me/.bun/install/global/node_modules/@anthropic-ai/claude-code/cli.js";
        let pnpm = r"C:\Users\me\AppData\Local\pnpm\claude.exe";

        assert_eq!(source("/home/me/.bun/bin/claude", bun), Source::Bun);
        assert_eq!(source(pnpm, pnpm), Source::Pnpm);
    }

    #[test]
    fn other_package_managers_are_left_to_the_user() {
        let cask = "/opt/homebrew/Caskroom/claude-code/2.1.319/claude";
        let winget = r"C:\Users\me\AppData\Local\Microsoft\WinGet\Links\claude.exe";

        assert_eq!(source("/opt/homebrew/bin/claude", cask), Source::Other);
        assert_eq!(source(winget, winget), Source::Other);
    }

    #[test]
    fn native_updates_run_the_cli_itself() {
        let path = PathBuf::from("/home/me/.local/bin/claude");

        let updater = updater_for(Source::Native, path.clone()).unwrap();

        assert_eq!(updater.program, path);
        assert_eq!(updater.args, ["update"]);
        assert_eq!(updater_for(Source::Other, path), None);
    }
}
