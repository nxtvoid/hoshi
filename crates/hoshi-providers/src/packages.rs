//! CLIs published on npm: which package manager installed one, its latest release and the
//! command that updates it.

use crate::process::hide_console;
use crate::updater::{Updater, find_program};
use crate::version::parse_version;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const REGISTRY: &str = "https://registry.npmjs.org";
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);
const LATEST_FRESH_FOR: Duration = Duration::from_secs(60 * 60);

/// The package manager a global install belongs to.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Manager {
    /// npm's, under this prefix.
    Npm(PathBuf),
    Bun,
    Pnpm,
}

/// Who installed `package`'s CLI: `path` is where it was found and `real` the file it leads
/// to. `None` when none of the managers hoshi drives did.
pub(crate) fn manager_of(package: &str, path: &Path, real: &Path) -> Option<Manager> {
    if any_path(path, real, |path| path.contains("/.bun/")) {
        return Some(Manager::Bun);
    }
    if any_path(path, real, |path| path.contains("/pnpm/")) {
        return Some(Manager::Pnpm);
    }
    npm_prefix(package, path, real).map(Manager::Npm)
}

/// Whether `test` holds for either path, lowercase and with forward slashes.
pub(crate) fn any_path(path: &Path, real: &Path, test: impl Fn(&str) -> bool) -> bool {
    [path, real].into_iter().any(|path| {
        test(
            &path
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase(),
        )
    })
}

/// On Unix the CLI links into `<prefix>/lib/node_modules`; on Windows npm puts its
/// launchers right in the prefix, next to `node_modules`. A project's own `node_modules` is
/// neither.
fn npm_prefix(package: &str, path: &Path, real: &Path) -> Option<PathBuf> {
    let mut folder = PathBuf::from("node_modules");
    folder.extend(package.split('/'));
    let prefix = real
        .ancestors()
        .find(|dir| dir.ends_with(&folder))
        .and_then(|dir| dir.ancestors().nth(3))
        .filter(|lib| lib.ends_with("lib"))
        .and_then(Path::parent)
        .or_else(|| path.parent().filter(|dir| dir.join(&folder).is_dir()))?;
    let nested = prefix
        .components()
        .any(|part| part.as_os_str() == "node_modules");
    (!nested && !belongs_to_mise(prefix)).then(|| prefix.to_owned())
}

/// mise lays its tools out as npm prefixes; only those under its Node are npm's.
fn belongs_to_mise(prefix: &Path) -> bool {
    prefix.ancestors().any(|dir| {
        dir.parent()
            .is_some_and(|parent| parent.ends_with("mise/installs"))
            && dir.file_name().is_none_or(|tool| tool != "node")
    })
}

/// `None` when the manager itself isn't found.
pub(crate) fn updater(manager: Manager, package: &str) -> Option<Updater> {
    let latest = format!("{package}@latest");
    let (program, args) = match manager {
        // npm 12 refuses install scripts it wasn't told to trust; older ones ignore the flag.
        Manager::Npm(prefix) => (
            find_program("npm", true)?,
            vec![
                "install".into(),
                "-g".into(),
                "--prefix".into(),
                prefix.to_string_lossy().into_owned(),
                format!("--allow-scripts={package}"),
                latest,
            ],
        ),
        Manager::Bun => (
            find_program("bun", true)?,
            vec!["add".into(), "-g".into(), latest],
        ),
        Manager::Pnpm => {
            let pnpm = find_program("pnpm", true)?;
            let mut args = vec!["add".to_owned(), "-g".to_owned()];
            // pnpm 10 skips install scripts it wasn't told to trust; the flag came in 10.4.
            if program_version(&pnpm).is_some_and(|version| version >= (10, 4, 0)) {
                args.push(format!("--allow-build={package}"));
            }
            args.push(latest);
            (pnpm, args)
        }
    };
    Some(Updater::command(program, args))
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

/// The release npm names latest; reaches the network.
pub(crate) fn npm_latest(package: &str) -> Option<String> {
    let url = format!("{REGISTRY}/{package}/latest");
    cached(&url, || {
        let latest: Value = serde_json::from_str(&get(&url)?).ok()?;
        Some(latest["version"].as_str()?.to_owned())
    })
}

/// What `look_up` last found at `url`, asked again once it is old. A failure isn't kept.
pub(crate) fn cached(url: &str, look_up: impl FnOnce() -> Option<String>) -> Option<String> {
    static FOUND: Mutex<BTreeMap<String, (Instant, String)>> = Mutex::new(BTreeMap::new());
    let found = || {
        FOUND
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    };
    if let Some((at, version)) = found().get(url)
        && at.elapsed() < LATEST_FRESH_FOR
    {
        return Some(version.clone());
    }
    let version = look_up()?;
    found().insert(url.to_owned(), (Instant::now(), version.clone()));
    Some(version)
}

pub(crate) fn get(url: &str) -> Option<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(LOOKUP_TIMEOUT))
        .build()
        .into();
    agent.get(url).call().ok()?.body_mut().read_to_string().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_package_belongs_to_the_npm_prefix_it_is_under() {
        let path = Path::new("/usr/local/bin/codex");
        let real = Path::new("/usr/local/lib/node_modules/@openai/codex/bin/codex.js");

        assert_eq!(
            manager_of("@openai/codex", path, real),
            Some(Manager::Npm(PathBuf::from("/usr/local")))
        );
        assert_eq!(manager_of("@github/copilot", path, real), None);
    }

    #[test]
    fn a_project_s_own_install_and_mise_s_tools_are_not_npm_s() {
        let manager = |real: &str| {
            manager_of(
                "@openai/codex",
                Path::new("/home/dev/bin/codex"),
                Path::new(real),
            )
        };
        let mise = "/home/dev/.local/share/mise/installs";

        assert_eq!(
            manager("/home/dev/shop/node_modules/@openai/codex/bin/codex.js"),
            None
        );
        assert_eq!(
            manager("/usr/lib/node_modules/tool/lib/node_modules/@openai/codex/codex.js"),
            None
        );
        assert_eq!(
            manager(&format!(
                "{mise}/npm-openai-codex/1.0.0/lib/node_modules/@openai/codex/codex.js"
            )),
            None
        );
        assert_eq!(
            manager(&format!(
                "{mise}/node/24.0.0/lib/node_modules/@openai/codex/codex.js"
            )),
            Some(Manager::Npm(PathBuf::from(format!("{mise}/node/24.0.0"))))
        );
    }

    #[test]
    fn a_release_found_is_kept_and_a_failure_is_not() {
        let url = "test://kept";

        assert_eq!(cached(url, || None), None);
        assert_eq!(cached(url, || Some("1.0.0".into())), Some("1.0.0".into()));
        assert_eq!(cached(url, || panic!("asked again")), Some("1.0.0".into()));
    }
}
