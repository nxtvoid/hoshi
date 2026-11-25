//! Where the Copilot CLI comes from, its latest release and how to update it.

use crate::Installation;
use crate::packages::{any_path, manager_of, npm_latest, updater};
use crate::updater::Updater;
use std::path::{Path, PathBuf};

const PACKAGE: &str = "@github/copilot";

/// `installed` is what the CLI reported; reaches the network for the latest release.
pub(super) fn installation(binary: Option<PathBuf>, installed: Option<String>) -> Installation {
    let Some(path) = binary.filter(|_| installed.is_some()) else {
        return Installation::default();
    };
    let real = dunce::canonicalize(&path).unwrap_or_else(|_| path.clone());
    Installation {
        installed,
        latest: npm_latest(PACKAGE),
        updater: updater_for(&path, &real),
    }
}

/// `copilot update` downloads into the user's folder, which the installed launcher then
/// prefers, so it works wherever the launcher is, unless a manager that pins it put it there.
fn updater_for(path: &Path, real: &Path) -> Option<Updater> {
    if let Some(manager) = manager_of(PACKAGE, path, real) {
        return updater(manager, PACKAGE);
    }
    let system = any_path(path, real, |path| {
        [
            "/cellar/",
            "/caskroom/",
            "/nix/store/",
            "/scoop/",
            "/chocolatey/",
        ]
        .iter()
        .any(|folder| path.contains(folder))
    });
    (!system).then(|| Updater::command(path.to_owned(), vec!["update".into()]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outside_a_package_manager_it_updates_itself() {
        let path = Path::new("/home/dev/.local/bin/copilot");

        assert_eq!(
            updater_for(path, path),
            Some(Updater::command(path.to_owned(), vec!["update".into()]))
        );
    }

    #[test]
    fn system_package_managers_are_left_to_the_user() {
        let cask = Path::new("/opt/homebrew/Caskroom/copilot-cli/1.0.0/copilot");
        let scoop = Path::new(r"C:\Users\dev\scoop\shims\copilot.exe");
        let nix = Path::new("/nix/store/abc-copilot/bin/copilot");

        assert_eq!(
            updater_for(Path::new("/opt/homebrew/bin/copilot"), cask),
            None
        );
        assert_eq!(updater_for(scoop, scoop), None);
        assert_eq!(updater_for(nix, nix), None);
    }
}
