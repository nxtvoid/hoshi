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

/// How an editor is started with a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Launcher {
    /// Gets the folder as its argument.
    Program(PathBuf),
    /// A macOS application bundle, started through `open`.
    MacApp(PathBuf),
}

/// A JetBrains IDE, which rarely puts its command on the `PATH`: Toolbox and the
/// installers keep it in folders of their own.
struct JetBrains {
    /// Its launcher in `bin`, without extension: `idea`, `webstorm`.
    launcher: &'static str,
    /// How the folders it is installed in start, lowercase: `IntelliJ IDEA 2025.1`,
    /// `intellij-idea-ultimate`, `idea-IU-251.1`.
    folders: &'static [&'static str],
    /// Its snap packages on Linux.
    snaps: &'static [&'static str],
}

fn jetbrains(editor: Editor) -> Option<JetBrains> {
    let (launcher, folders, snaps): (_, &[&str], &[&str]) = match editor {
        Editor::Idea => (
            "idea",
            &["intellij idea", "intellij-idea", "idea"],
            &["intellij-idea-ultimate", "intellij-idea-community"],
        ),
        Editor::Webstorm => ("webstorm", &["webstorm"], &["webstorm"]),
        Editor::Pycharm => (
            "pycharm",
            &["pycharm"],
            &["pycharm-professional", "pycharm-community"],
        ),
        Editor::Goland => ("goland", &["goland"], &["goland"]),
        Editor::Rider => ("rider", &["jetbrains rider", "rider"], &["rider"]),
        Editor::Clion => ("clion", &["clion"], &["clion"]),
        Editor::Phpstorm => ("phpstorm", &["phpstorm"], &["phpstorm"]),
        Editor::Rubymine => ("rubymine", &["rubymine"], &["rubymine"]),
        Editor::Rustrover => ("rustrover", &["rustrover"], &["rustrover"]),
        _ => return None,
    };
    Some(JetBrains {
        launcher,
        folders,
        snaps,
    })
}

/// The code editors found on this machine, looked up once per run, then the file
/// manager, which every system has.
pub fn installed() -> Vec<Editor> {
    launchers().iter().map(|(editor, _)| *editor).collect()
}

fn launchers() -> &'static [(Editor, Launcher)] {
    static LAUNCHERS: OnceLock<Vec<(Editor, Launcher)>> = OnceLock::new();
    LAUNCHERS.get_or_init(|| {
        let places = Places::of_this_machine();
        Editor::CODE_EDITORS
            .into_iter()
            .filter_map(|editor| locate(editor, &places).map(|launcher| (editor, launcher)))
            .chain([(Editor::FileManager, Launcher::Program(FILE_MANAGER.into()))])
            .collect()
    })
}

/// Opens `folder` in `editor`, without waiting for it.
pub fn open(editor: Editor, folder: &str) -> Result<(), String> {
    let launcher = launchers()
        .iter()
        .find(|(found, _)| *found == editor)
        .map(|(_, launcher)| launcher.clone())
        .ok_or_else(|| format!("{editor:?} not found"))?;
    let mut command = match launcher {
        Launcher::Program(program) => {
            let mut command = Command::new(program);
            command.arg(folder);
            command
        }
        Launcher::MacApp(app) => {
            let mut command = Command::new("open");
            command.arg("-na").arg(app).arg("--args").arg(folder);
            command
        }
    };
    command
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

/// Where editors get installed besides the `PATH`.
#[derive(Debug, Default)]
struct Places {
    path: Vec<PathBuf>,
    /// The commands Toolbox writes, when it is set to.
    toolbox_scripts: Option<PathBuf>,
    /// Folders holding one folder per installed IDE.
    install_roots: Vec<PathBuf>,
    snap_bin: Option<PathBuf>,
    mac_applications: Vec<PathBuf>,
}

impl Places {
    fn of_this_machine() -> Self {
        let path = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default();
        let home = std::env::home_dir();
        let mut places = Self {
            path,
            ..Self::default()
        };
        if cfg!(windows) {
            if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
                places.toolbox_scripts = Some(local.join("JetBrains/Toolbox/scripts"));
                places.install_roots.push(local.join("Programs"));
                places
                    .install_roots
                    .push(local.join("JetBrains/Toolbox/apps"));
            }
            for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
                if let Some(programs) = std::env::var_os(variable).map(PathBuf::from) {
                    places.install_roots.push(programs.join("JetBrains"));
                }
            }
        } else if cfg!(target_os = "macos") {
            if let Some(home) = &home {
                places.toolbox_scripts =
                    Some(home.join("Library/Application Support/JetBrains/Toolbox/scripts"));
                places.mac_applications.push(home.join("Applications"));
            }
            places.mac_applications.push("/Applications".into());
        } else {
            if let Some(home) = &home {
                let toolbox = home.join(".local/share/JetBrains/Toolbox");
                places.toolbox_scripts = Some(toolbox.join("scripts"));
                places.install_roots.push(toolbox.join("apps"));
                places
                    .install_roots
                    .push(home.join(".local/share/JetBrains"));
                places.install_roots.push(home.join("Applications"));
            }
            places.install_roots.push("/opt".into());
            places.install_roots.push("/opt/JetBrains".into());
            places.snap_bin = Some("/snap/bin".into());
        }
        places
    }
}

fn locate(editor: Editor, places: &Places) -> Option<Launcher> {
    let command = editor.command()?;
    if let Some(found) = places.path.iter().find_map(|dir| find_in(dir, command)) {
        return Some(Launcher::Program(found));
    }
    let product = jetbrains(editor)?;
    places
        .toolbox_scripts
        .as_deref()
        .and_then(|dir| find_in(dir, command))
        .map(Launcher::Program)
        .or_else(|| {
            places
                .install_roots
                .iter()
                .find_map(|root| installed_launcher(root, &product))
                .map(Launcher::Program)
        })
        .or_else(|| {
            let snaps = places.snap_bin.as_deref()?;
            product
                .snaps
                .iter()
                .map(|snap| snaps.join(snap))
                .find(|candidate| candidate.is_file())
                .map(Launcher::Program)
        })
        .or_else(|| {
            places
                .mac_applications
                .iter()
                .find_map(|dir| mac_app(dir, &product))
                .map(Launcher::MacApp)
        })
}

/// The newest install of `product` under `root`: its folder holds `bin` directly, or,
/// as older Toolbox versions laid it out, a channel with one folder per build.
fn installed_launcher(root: &Path, product: &JetBrains) -> Option<PathBuf> {
    let mut folders = matching_folders(root, product);
    folders.reverse();
    folders.into_iter().find_map(|folder| {
        bin_launcher(&folder, product.launcher).or_else(|| {
            let channels = subfolders(&folder);
            channels.iter().rev().find_map(|channel| {
                subfolders(channel)
                    .iter()
                    .rev()
                    .find_map(|build| bin_launcher(build, product.launcher))
            })
        })
    })
}

fn bin_launcher(folder: &Path, launcher: &str) -> Option<PathBuf> {
    let bin = folder.join("bin");
    let names: &[String] = if cfg!(windows) {
        &[format!("{launcher}64.exe"), format!("{launcher}.exe")]
    } else {
        &[launcher.to_owned(), format!("{launcher}.sh")]
    };
    names
        .iter()
        .map(|name| bin.join(name))
        .find(|candidate| candidate.is_file())
}

fn mac_app(dir: &Path, product: &JetBrains) -> Option<PathBuf> {
    matching_folders(dir, product)
        .into_iter()
        .rev()
        .find(|app| app.extension().is_some_and(|extension| extension == "app"))
}

/// The folders under `root` named for `product`, sorted by name, so the newest
/// version comes last.
fn matching_folders(root: &Path, product: &JetBrains) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = subfolders(root)
        .into_iter()
        .filter(|folder| {
            folder
                .file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .is_some_and(|name| product.folders.iter().any(|start| name.starts_with(start)))
        })
        .collect();
    folders.sort();
    folders
}

fn subfolders(dir: &Path) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    folders
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

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hoshi-editors-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    fn launcher_name(launcher: &str) -> String {
        if cfg!(windows) {
            format!("{launcher}64.exe")
        } else {
            launcher.to_owned()
        }
    }

    #[test]
    fn finds_a_command_with_a_platform_extension() {
        let dir = scratch("path");
        let file = dir.join(format!("fake-editor{}", EXTENSIONS[0]));
        touch(&file);

        assert_eq!(find_in(&dir, "fake-editor"), Some(file));
        assert_eq!(find_in(&dir, "missing-editor"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_the_newest_jetbrains_install_off_the_path() {
        let root = scratch("installs");
        let name = launcher_name("idea");
        touch(&root.join("IntelliJ IDEA 2024.3/bin").join(&name));
        let newest = root.join("IntelliJ IDEA 2025.1/bin").join(&name);
        touch(&newest);
        touch(
            &root
                .join("WebStorm 2025.1/bin")
                .join(launcher_name("webstorm")),
        );
        let places = Places {
            install_roots: vec![root.clone()],
            ..Places::default()
        };

        assert_eq!(
            locate(Editor::Idea, &places),
            Some(Launcher::Program(newest))
        );
        assert_eq!(locate(Editor::Goland, &places), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn finds_an_ide_in_an_older_toolbox_layout() {
        let root = scratch("toolbox");
        let launcher = root
            .join("IDEA-U/ch-0/251.23774.435/bin")
            .join(launcher_name("idea"));
        touch(&launcher);
        let places = Places {
            install_roots: vec![root.clone()],
            ..Places::default()
        };

        assert_eq!(
            locate(Editor::Idea, &places),
            Some(Launcher::Program(launcher))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_path_wins_over_an_install_folder() {
        let dir = scratch("both");
        let on_path = dir.join("path").join(format!("idea{}", EXTENSIONS[0]));
        touch(&on_path);
        touch(
            &dir.join("installs/IntelliJ IDEA 2025.1/bin")
                .join(launcher_name("idea")),
        );
        let places = Places {
            path: vec![dir.join("path")],
            install_roots: vec![dir.join("installs")],
            ..Places::default()
        };

        assert_eq!(
            locate(Editor::Idea, &places),
            Some(Launcher::Program(on_path))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
