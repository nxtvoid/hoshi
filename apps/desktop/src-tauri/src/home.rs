//! hoshi keeps everything in one folder: `~/.hoshi`, or `~/.hoshi-dev` for development builds
//! so they never touch the installed app's data. `HOSHI_HOME` points it elsewhere.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Where the webview keeps its storage, such as the UI's preferences.
pub const WEBVIEW_DIR: &str = "webview";
const SETTINGS_FILE: &str = "settings.json";
/// What hoshi itself wrote to the old data folder. Worktrees stay there: git registered
/// their paths in each repository, so moving them would break them.
const DATA_ENTRIES: &[&str] = &[
    "state.sqlite-wal",
    "state.sqlite-shm",
    "state.sqlite",
    "attachments",
    "chats",
];
const KEPT_IN_PLACE: &[&str] = &["worktrees"];

/// The folder every part of hoshi reads and writes under.
pub struct Home(pub PathBuf);

impl Home {
    pub fn settings(&self) -> PathBuf {
        self.0.join(SETTINGS_FILE)
    }
}

pub fn resolve() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("HOSHI_HOME").filter(|dir| !dir.is_empty()) {
        let dir = PathBuf::from(dir);
        return if dir.is_absolute() {
            Ok(dir)
        } else {
            Ok(std::env::current_dir()?.join(dir))
        };
    }
    let home = std::env::home_dir().ok_or_else(|| io::Error::other("no home folder"))?;
    let name = if cfg!(debug_assertions) {
        ".hoshi-dev"
    } else {
        ".hoshi"
    };
    Ok(home.join(name))
}

/// The folders Tauri derived from the app's identifier, where hoshi lived before.
pub struct Legacy {
    pub data: PathBuf,
    pub config: PathBuf,
    /// The webview's storage; the same folder as `data` on Linux.
    pub local: PathBuf,
}

/// Moves what hoshi kept in the old folders into `home`, once: only while `home` has no
/// database yet. The first build to start afterwards keeps the data.
pub fn adopt_legacy(home: &Path, legacy: &Legacy) -> io::Result<bool> {
    if home.join("state.sqlite").exists() || !legacy.data.join("state.sqlite").exists() {
        return Ok(false);
    }
    fs::create_dir_all(home)?;
    for name in DATA_ENTRIES {
        move_if_present(&legacy.data.join(name), &home.join(name))?;
    }
    move_if_present(
        &legacy.config.join(SETTINGS_FILE),
        &home.join(SETTINGS_FILE),
    )?;
    if let Ok(entries) = fs::read_dir(&legacy.local) {
        let webview = home.join(WEBVIEW_DIR);
        for entry in entries.flatten() {
            let name = entry.file_name();
            if KEPT_IN_PLACE.iter().any(|kept| name == *kept) {
                continue;
            }
            fs::create_dir_all(&webview)?;
            move_if_present(&entry.path(), &webview.join(&name))?;
        }
    }
    for dir in [&legacy.local, &legacy.data, &legacy.config] {
        let _ = fs::remove_dir(dir);
    }
    Ok(true)
}

/// A rename when both sides share a disk, a copy and a delete otherwise.
fn move_if_present(from: &Path, to: &Path) -> io::Result<()> {
    if !from.exists() || to.exists() {
        return Ok(());
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_all(from, to)?;
    if from.is_dir() {
        fs::remove_dir_all(from)
    } else {
        fs::remove_file(from)
    }
}

fn copy_all(from: &Path, to: &Path) -> io::Result<()> {
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_all(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(from, to).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hoshi-home-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn windows_folders_move_into_home_and_worktrees_stay() {
        let root = scratch("windows");
        let legacy = Legacy {
            data: root.join("Roaming/dev.hoshi.app"),
            config: root.join("Roaming/dev.hoshi.app"),
            local: root.join("Local/dev.hoshi.app"),
        };
        write(&legacy.data.join("state.sqlite"), "db");
        write(&legacy.data.join("state.sqlite-wal"), "wal");
        write(&legacy.data.join("settings.json"), "{}");
        write(&legacy.data.join("chats/c1/note.md"), "chat");
        write(&legacy.data.join("attachments/a.png"), "png");
        write(&legacy.data.join("worktrees/repo/t1/file"), "wt");
        write(&legacy.local.join("EBWebView/Local State"), "webview");
        let home = root.join(".hoshi");

        assert!(adopt_legacy(&home, &legacy).unwrap());

        for (path, text) in [
            ("state.sqlite", "db"),
            ("state.sqlite-wal", "wal"),
            ("settings.json", "{}"),
            ("chats/c1/note.md", "chat"),
            ("attachments/a.png", "png"),
            ("webview/EBWebView/Local State", "webview"),
        ] {
            assert_eq!(fs::read_to_string(home.join(path)).unwrap(), text, "{path}");
        }
        assert!(legacy.data.join("worktrees/repo/t1/file").exists());
        assert!(!home.join("worktrees").exists());
        assert!(!legacy.local.exists());
    }

    #[test]
    fn linux_webview_files_sit_beside_the_data_and_move_to_their_own_folder() {
        let root = scratch("linux");
        let legacy = Legacy {
            data: root.join("share/dev.hoshi.app"),
            config: root.join("config/dev.hoshi.app"),
            local: root.join("share/dev.hoshi.app"),
        };
        write(&legacy.data.join("state.sqlite"), "db");
        write(&legacy.config.join("settings.json"), "{}");
        write(&legacy.data.join("localstorage/item"), "theme");
        let home = root.join(".hoshi");

        assert!(adopt_legacy(&home, &legacy).unwrap());

        assert_eq!(
            fs::read_to_string(home.join("settings.json")).unwrap(),
            "{}"
        );
        assert_eq!(
            fs::read_to_string(home.join("webview/localstorage/item")).unwrap(),
            "theme"
        );
        assert!(!home.join("webview/state.sqlite").exists());
        assert!(!legacy.config.exists());
    }

    #[test]
    fn a_home_with_data_is_never_overwritten() {
        let root = scratch("existing");
        let legacy = Legacy {
            data: root.join("old"),
            config: root.join("old"),
            local: root.join("old-local"),
        };
        write(&legacy.data.join("state.sqlite"), "old");
        let home = root.join(".hoshi");
        write(&home.join("state.sqlite"), "new");

        assert!(!adopt_legacy(&home, &legacy).unwrap());

        assert_eq!(
            fs::read_to_string(home.join("state.sqlite")).unwrap(),
            "new"
        );
        assert!(legacy.data.join("state.sqlite").exists());
        assert!(
            !adopt_legacy(
                &root.join("fresh"),
                &Legacy {
                    data: root.join("missing"),
                    config: root.join("missing"),
                    local: root.join("missing"),
                }
            )
            .unwrap()
        );
    }
}
