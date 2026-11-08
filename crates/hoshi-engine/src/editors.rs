//! Finds the code editors installed on this machine and opens folders in them.

use hoshi_core::Editor;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// Extensions a command can have on Windows, in the order the shell tries them.
#[cfg(windows)]
const EXTENSIONS: &[&str] = &[".exe", ".cmd", ".bat"];
#[cfg(not(windows))]
const EXTENSIONS: &[&str] = &[""];

/// The program that shows a folder in the system's file manager.
#[cfg(windows)]
const FILE_MANAGER: &str = "explorer.exe";
#[cfg(target_os = "macos")]
const FILE_MANAGER: &str = "open";
#[cfg(not(any(windows, target_os = "macos")))]
const FILE_MANAGER: &str = "xdg-open";

/// The code editors whose command is on the `PATH`, looked up once per run, then the
/// file manager, which every system has.
pub fn installed() -> Vec<Editor> {
    static INSTALLED: OnceLock<Vec<Editor>> = OnceLock::new();
    INSTALLED
        .get_or_init(|| {
            Editor::CODE_EDITORS
                .into_iter()
                .filter(|editor| editor.command().and_then(find).is_some())
                .chain([Editor::FileManager])
                .collect()
        })
        .clone()
}

/// Opens `folder` in `editor`, without waiting for it.
pub fn open(editor: Editor, folder: &str) -> Result<(), String> {
    let program = match editor.command() {
        Some(name) => find(name).ok_or_else(|| format!("{name} not found"))?,
        None => PathBuf::from(FILE_MANAGER),
    };
    let mut command = Command::new(program);
    command
        .arg(folder)
        .current_dir(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn find(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| find_in(&dir, name))
}

fn find_in(dir: &Path, name: &str) -> Option<PathBuf> {
    EXTENSIONS
        .iter()
        .map(|extension| dir.join(format!("{name}{extension}")))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_command_with_a_platform_extension() {
        let dir = std::env::temp_dir().join(format!("hoshi-editors-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(format!("fake-editor{}", EXTENSIONS[0]));
        std::fs::write(&file, "").unwrap();

        assert_eq!(find_in(&dir, "fake-editor"), Some(file));
        assert_eq!(find_in(&dir, "missing-editor"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
