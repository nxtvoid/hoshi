//! Where the Claude Code CLI comes from, its latest release and how to update it.

use crate::Installation;
use crate::packages::{self, Manager, any_path, get, manager_of, npm_latest};
use crate::updater::{Updater, find_in};
use crate::version::parse_version;
use serde_json::Value;
use std::path::{Path, PathBuf};

const PACKAGE: &str = "@anthropic-ai/claude-code";
const NATIVE_RELEASES: &str = "https://downloads.claude.ai/claude-code-releases";

#[derive(Debug, PartialEq, Eq)]
enum Source {
    /// The native installer's, which updates itself with `claude update`.
    Native,
    Package(Manager),
    /// A package manager hoshi doesn't drive, such as Homebrew or WinGet.
    Other,
}

/// The CLI hoshi runs: the settings' binary, else `claude` in `dirs`, else where the native
/// installer puts it under `home`, which it may leave off the `PATH`.
pub(crate) fn resolve(
    configured: Option<&str>,
    dirs: impl FnOnce() -> Vec<PathBuf>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(configured) = configured.map(str::trim).filter(|path| !path.is_empty()) {
        return find_in(configured, false, dirs);
    }
    find_in("claude", false, dirs).or_else(|| {
        let name = if cfg!(windows) {
            "claude.exe"
        } else {
            "claude"
        };
        let native = native_dir(home)?.join(name);
        native.is_file().then_some(native)
    })
}

/// Where the native installer puts the CLI.
pub(crate) fn native_dir(home: Option<&Path>) -> Option<PathBuf> {
    Some(home?.join(".local").join("bin"))
}

/// `installed` is what the CLI reported; reaches the network for the latest release.
pub(crate) fn installation(binary: Option<PathBuf>, installed: Option<String>) -> Installation {
    let Some(path) = binary else {
        return Installation::default();
    };
    let real = dunce::canonicalize(&path).unwrap_or_else(|_| path.clone());
    let source = source_of(&path, &real);
    let latest = match source {
        Source::Native => native_latest(),
        _ => npm_latest(PACKAGE),
    };
    Installation {
        installed,
        latest,
        updater: updater_for(source, path),
    }
}

/// `path` is where the CLI was found and `real` the file it leads to.
fn source_of(path: &Path, real: &Path) -> Source {
    if let Some(manager) = manager_of(PACKAGE, path, real) {
        return Source::Package(manager);
    }
    let native = any_path(path, real, |path| {
        path.ends_with("/.local/bin/claude")
            || path.ends_with("/.local/bin/claude.exe")
            || path.contains("/.local/share/claude/")
    });
    if native {
        Source::Native
    } else {
        Source::Other
    }
}

fn updater_for(source: Source, path: PathBuf) -> Option<Updater> {
    match source {
        Source::Native => Some(Updater::command(path, vec!["update".into()])),
        Source::Package(manager) => packages::updater(manager, PACKAGE),
        Source::Other => None,
    }
}

/// The release the native updater would install, on the channel the user picked.
fn native_latest() -> Option<String> {
    let text = get(&format!("{NATIVE_RELEASES}/{}", update_channel()))?;
    let version = text.trim();
    parse_version(version).map(|_| version.to_owned())
}

/// Where Claude Code keeps its settings, sessions and task lists.
pub(crate) fn config_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|home| home.join(".claude")))
}

fn update_channel() -> String {
    let channel = config_dir()
        .and_then(|dir| std::fs::read_to_string(dir.join("settings.json")).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|settings| settings["autoUpdatesChannel"].as_str().map(str::to_owned));
    match channel.as_deref() {
        Some("stable") => "stable".into(),
        _ => "latest".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    };

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hoshi-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cli_in(dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(EXE);
        std::fs::write(&path, "").unwrap();
        path
    }

    fn native_home() -> PathBuf {
        let home = folder("claude-home");
        cli_in(&home.join(".local").join("bin"));
        home
    }

    #[test]
    fn the_settings_binary_wins_over_the_path_and_the_native_install() {
        let home = native_home();
        let on_path = folder("claude-path");
        cli_in(&on_path);
        let configured = cli_in(&folder("claude-configured"));

        let found = resolve(
            Some(&configured.to_string_lossy()),
            || vec![on_path.clone()],
            Some(&home),
        );

        assert_eq!(found, Some(configured));
    }

    #[test]
    fn a_missing_settings_binary_is_not_replaced_by_another() {
        let home = native_home();
        let on_path = folder("claude-path");
        cli_in(&on_path);
        let missing = on_path.join("gone").join(EXE);

        let found = resolve(
            Some(&missing.to_string_lossy()),
            || vec![on_path.clone()],
            Some(&home),
        );

        assert_eq!(found, None);
    }

    #[test]
    fn the_path_wins_over_the_native_install() {
        let home = native_home();
        let on_path = folder("claude-path");
        let expected = cli_in(&on_path);

        assert_eq!(
            resolve(Some("  "), || vec![on_path.clone()], Some(&home)),
            Some(expected)
        );
    }

    #[test]
    fn the_native_install_is_found_off_the_path() {
        let home = native_home();

        assert_eq!(
            resolve(None, || vec![folder("claude-empty")], Some(&home)),
            Some(home.join(".local").join("bin").join(EXE))
        );
    }

    #[test]
    fn nothing_installed_is_none() {
        let home = folder("claude-home");

        assert_eq!(
            resolve(None, || vec![folder("claude-empty")], Some(&home)),
            None
        );
        assert_eq!(resolve(None, Vec::new, None), None);
    }

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
            Source::Package(Manager::Npm(PathBuf::from("/usr/local")))
        );
    }

    #[test]
    fn bun_and_pnpm_installs_are_their_own() {
        let bun = "/home/me/.bun/install/global/node_modules/@anthropic-ai/claude-code/cli.js";
        let pnpm = r"C:\Users\me\AppData\Local\pnpm\claude.exe";

        assert_eq!(
            source("/home/me/.bun/bin/claude", bun),
            Source::Package(Manager::Bun)
        );
        assert_eq!(source(pnpm, pnpm), Source::Package(Manager::Pnpm));
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

        assert_eq!(
            updater,
            Updater::command(path.clone(), vec!["update".into()])
        );
        assert_eq!(updater_for(Source::Other, path), None);
    }
}
