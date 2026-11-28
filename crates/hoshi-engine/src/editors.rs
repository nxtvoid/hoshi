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
    /// What a desktop entry runs, `flatpak run …`, with the folder between `before` and
    /// `after`, where the entry takes its files.
    Exec {
        program: PathBuf,
        before: Vec<String>,
        after: Vec<String>,
    },
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
    /// Its Flathub apps, the one to prefer first.
    flatpaks: &'static [&'static str],
}

fn jetbrains(editor: Editor) -> Option<JetBrains> {
    let (launcher, folders, snaps, flatpaks): (_, &[&str], &[&str], &[&str]) = match editor {
        Editor::Idea => (
            "idea",
            &["intellij idea", "intellij-idea", "idea"],
            &[
                "intellij-idea",
                "intellij-idea-ultimate",
                "intellij-idea-community",
            ],
            &[
                "com.jetbrains.IntelliJ-IDEA-Ultimate",
                "com.jetbrains.IntelliJ-IDEA-Community",
            ],
        ),
        Editor::Webstorm => (
            "webstorm",
            &["webstorm"],
            &["webstorm"],
            &["com.jetbrains.WebStorm"],
        ),
        Editor::Pycharm => (
            "pycharm",
            &["pycharm"],
            &["pycharm", "pycharm-professional", "pycharm-community"],
            &[
                "com.jetbrains.PyCharm-Professional",
                "com.jetbrains.PyCharm-Community",
            ],
        ),
        Editor::Goland => (
            "goland",
            &["goland"],
            &["goland"],
            &["com.jetbrains.GoLand"],
        ),
        Editor::Rider => (
            "rider",
            &["jetbrains rider", "rider"],
            &["rider"],
            &["com.jetbrains.Rider"],
        ),
        Editor::Clion => ("clion", &["clion"], &["clion"], &["com.jetbrains.CLion"]),
        Editor::Phpstorm => (
            "phpstorm",
            &["phpstorm"],
            &["phpstorm"],
            &["com.jetbrains.PhpStorm"],
        ),
        Editor::Rubymine => (
            "rubymine",
            &["rubymine"],
            &["rubymine"],
            &["com.jetbrains.RubyMine"],
        ),
        Editor::Rustrover => (
            "rustrover",
            &["rustrover"],
            &["rustrover"],
            &["com.jetbrains.RustRover"],
        ),
        _ => return None,
    };
    Some(JetBrains {
        launcher,
        folders,
        snaps,
        flatpaks,
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

/// Opens `path`, a folder or a file, in `editor`; the file manager opens a file in the app the
/// system has for it.
pub fn open(editor: Editor, path: &str) -> Result<(), String> {
    let path = &native_path(path);
    let folder = match Path::new(path).parent() {
        Some(parent) if Path::new(path).is_file() => parent.to_string_lossy().into_owned(),
        _ => path.to_owned(),
    };
    let folder = folder.as_str();
    let launcher = launchers()
        .iter()
        .find(|(found, _)| *found == editor)
        .map(|(_, launcher)| launcher.clone())
        .ok_or_else(|| format!("{editor:?} not found"))?;
    let mut command = match launcher {
        Launcher::Program(program) => {
            let mut command = Command::new(program);
            command.arg(path);
            command
        }
        Launcher::MacApp(app) => {
            let mut command = Command::new("open");
            command.arg("-na").arg(app).arg("--args").arg(path);
            command
        }
        Launcher::Exec {
            program,
            before,
            after,
        } => {
            let mut command = Command::new(program);
            command.args(before).arg(path).args(after);
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

/// `path` with the system's separators: Explorer reads `C:/a/b.pdf` as no path at all.
fn native_path(path: &str) -> String {
    if cfg!(windows) {
        path.replace('/', "\\")
    } else {
        path.to_owned()
    }
}

/// Where editors get installed besides the `PATH`.
#[derive(Debug, Default)]
struct Places {
    path: Vec<PathBuf>,
    /// Where commands usually are, for an app started from a desktop whose `PATH` lacks them.
    bins: Vec<PathBuf>,
    /// Where the commands Toolbox writes, when it is set to, may be.
    toolbox_scripts: Vec<PathBuf>,
    /// Folders holding one folder per installed IDE.
    install_roots: Vec<PathBuf>,
    snap_bin: Option<PathBuf>,
    /// Where desktop entries are, the first ones taking precedence: the IDEs a menu
    /// lists, however they were installed.
    applications: Vec<PathBuf>,
    /// The desktop entries in `applications`, read when first asked for.
    desktop_entries: std::cell::OnceCell<Vec<DesktopEntry>>,
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
                places
                    .toolbox_scripts
                    .push(local.join("JetBrains/Toolbox/scripts"));
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
                places
                    .toolbox_scripts
                    .push(home.join("Library/Application Support/JetBrains/Toolbox/scripts"));
                places.mac_applications.push(home.join("Applications"));
            }
            places.mac_applications.push("/Applications".into());
        } else {
            let mut data_homes: Vec<PathBuf> = std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .filter(|dir| dir.is_absolute())
                .into_iter()
                .chain(home.as_ref().map(|home| home.join(".local/share")))
                .collect();
            data_homes.dedup();
            if let Some(home) = &home {
                places.bins.push(home.join(".local/bin"));
            }
            places
                .bins
                .extend(["/usr/local/bin", "/usr/bin"].map(PathBuf::from));
            for data in &data_homes {
                let toolbox = data.join("JetBrains/Toolbox");
                places.toolbox_scripts.push(toolbox.join("scripts"));
                places.install_roots.push(toolbox.join("apps"));
                places.install_roots.push(data.join("JetBrains"));
                places.applications.push(data.join("applications"));
                places
                    .applications
                    .push(data.join("flatpak/exports/share/applications"));
            }
            if let Some(home) = &home {
                places.install_roots.push(home.join("Applications"));
            }
            places.install_roots.push("/opt".into());
            places.install_roots.push("/opt/JetBrains".into());
            places.snap_bin = Some("/snap/bin".into());
            let data_dirs = std::env::var_os("XDG_DATA_DIRS")
                .filter(|dirs| !dirs.is_empty())
                .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
            places.applications.extend(
                std::env::split_paths(&data_dirs)
                    .filter(|dir| dir.is_absolute())
                    .map(|dir| dir.join("applications")),
            );
            places.applications.extend(
                [
                    "/var/lib/flatpak/exports/share/applications",
                    "/var/lib/snapd/desktop/applications",
                ]
                .map(PathBuf::from),
            );
            let mut seen = std::collections::HashSet::new();
            places.applications.retain(|dir| seen.insert(dir.clone()));
        }
        places
    }
}

fn locate(editor: Editor, places: &Places) -> Option<Launcher> {
    let command = editor.command()?;
    if let Some(found) = places
        .path
        .iter()
        .chain(&places.bins)
        .find_map(|dir| find_in(dir, command))
    {
        return Some(Launcher::Program(found));
    }
    let product = jetbrains(editor)?;
    places
        .toolbox_scripts
        .iter()
        .find_map(|dir| find_in(dir, command))
        .or_else(|| {
            places
                .toolbox_scripts
                .iter()
                .find_map(|dir| numbered_script(dir, command))
        })
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
            let mut launched = std::collections::HashSet::new();
            places
                .desktop_entries
                .get_or_init(|| read_desktop_entries(&places.applications))
                .iter()
                .filter_map(|entry| {
                    if launched.contains(entry.id.as_str()) {
                        return None;
                    }
                    let found = entry.launcher(&product, places)?;
                    launched.insert(entry.id.as_str());
                    Some(found)
                })
                .min_by_key(|(rank, _)| *rank)
                .map(|(_, launcher)| launcher)
        })
        .or_else(|| {
            places
                .mac_applications
                .iter()
                .find_map(|dir| mac_app(dir, &product))
                .map(Launcher::MacApp)
        })
}

/// A script Toolbox named `idea1`, `idea2`… because `idea` was taken when it wrote it.
fn numbered_script(dir: &Path, command: &str) -> Option<PathBuf> {
    let mut scripts: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stem = EXTENSIONS
                .iter()
                .filter(|extension| !extension.is_empty())
                .find_map(|extension| name.strip_suffix(extension))
                .unwrap_or(&name);
            let number = stem.strip_prefix(command)?;
            (!number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| (stem.to_owned(), entry.path()))
        })
        .filter(|(_, path)| path.is_file())
        .collect();
    scripts.sort_by(|(a, _), (b, _)| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    scripts.into_iter().next().map(|(_, path)| path)
}

/// A desktop entry that runs an application.
#[derive(Debug)]
struct DesktopEntry {
    /// Its file name without `.desktop`.
    id: String,
    /// `StartupWMClass`: JetBrains names its windows `jetbrains-idea`, `jetbrains-idea-ce`…
    class: Option<String>,
    /// `X-Flatpak`: the Flatpak app it runs.
    flatpak: Option<String>,
    /// The words of its `Exec`, field codes kept.
    exec: Vec<String>,
}

/// The desktop entries under `dirs`, earlier folders first; a hidden entry hides those
/// with its name in later folders.
fn read_desktop_entries(dirs: &[PathBuf]) -> Vec<DesktopEntry> {
    let mut hidden = std::collections::HashSet::new();
    let mut entries = Vec::new();
    for dir in dirs {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "desktop")
            })
            .collect();
        files.sort();
        for file in files {
            let Some(id) = file
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
            else {
                continue;
            };
            if hidden.contains(&id) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            match DesktopEntry::parse(id, &text) {
                Ok(entry) => entries.push(entry),
                Err(Some(id)) => {
                    hidden.insert(id);
                }
                Err(None) => {}
            }
        }
    }
    entries
}

impl DesktopEntry {
    /// The entry `text` describes; `Err(Some(id))` when it is hidden, `Err(None)` when it
    /// runs nothing.
    fn parse(id: String, text: &str) -> Result<Self, Option<String>> {
        let fields = desktop_fields(text);
        let field = |key: &str| {
            fields
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| unescape(value))
        };
        if field("Hidden").is_some_and(|hidden| hidden == "true") {
            return Err(Some(id));
        }
        if field("Type").is_some_and(|kind| kind != "Application") {
            return Err(None);
        }
        Ok(Self {
            id,
            class: field("StartupWMClass"),
            flatpak: field("X-Flatpak"),
            exec: exec_words(&field("Exec").ok_or(None)?),
        })
    }

    /// How to start `product` through this entry, when it is one of `product`'s, and how
    /// much less it is preferred: its Flathub app's place in `flatpaks`, or one more for a
    /// variant such as `jetbrains-idea-ce`.
    fn launcher(&self, product: &JetBrains, places: &Places) -> Option<(usize, Launcher)> {
        let class = format!("jetbrains-{}", product.launcher);
        let variant = self
            .class
            .as_deref()
            .and_then(|wm| wm.strip_prefix(class.as_str()))
            .filter(|rest| rest.is_empty() || rest.starts_with('-'))
            .map(|rest| !rest.is_empty());
        let by_class = variant.is_some();
        let flathub = product
            .flatpaks
            .iter()
            .position(|app| self.id == *app || self.flatpak.as_deref() == Some(*app));
        let by_flatpak = flathub.is_some();
        let start = self.exec.iter().enumerate().position(|(index, word)| {
            !(index == 0
                && Path::new(word)
                    .file_name()
                    .is_some_and(|name| name == "env")
                || index > 0 && is_assignment(word))
        })?;
        let runs = Path::new(&self.exec[start]);
        let script = format!("{}.sh", product.launcher);
        let by_program = runs
            .file_name()
            .is_some_and(|name| name == product.launcher || name == script.as_str());
        if !(by_class || by_flatpak || by_program) {
            return None;
        }
        let shell = runs.file_name().is_some_and(|name| {
            ["sh", "bash", "dash", "zsh"]
                .iter()
                .any(|shell| name == *shell)
        });
        if shell && self.exec.get(start + 1).is_some_and(|word| word == "-c")
            || runs.is_absolute() && !runs.is_file()
        {
            return None;
        }
        let program = Path::new(&self.exec[0]);
        let program = if program.is_absolute() {
            program.is_file().then(|| program.to_path_buf())?
        } else {
            places
                .path
                .iter()
                .chain(&places.bins)
                .find_map(|dir| find_in(dir, &self.exec[0]))?
        };
        let (mut before, mut after) = (Vec::new(), Vec::new());
        let mut slot = false;
        for word in &self.exec[1..] {
            if !slot && matches!(word.as_str(), "%f" | "%F" | "%u" | "%U") {
                slot = true;
                continue;
            }
            let Some(word) = without_field_codes(word) else {
                continue;
            };
            if slot { &mut after } else { &mut before }.push(word);
        }
        let rank = flathub.unwrap_or(usize::from(variant == Some(true)));
        Some((
            rank,
            Launcher::Exec {
                program,
                before,
                after,
            },
        ))
    }
}

/// `NAME=value`, which `env` sets before running what follows.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && !name.starts_with(|char: char| char.is_ascii_digit())
            && name
                .chars()
                .all(|char| char.is_ascii_alphanumeric() || char == '_')
    })
}

/// `word` with the field codes a menu would fill in taken out and `%%` made `%`; `None`
/// when only codes were left.
fn without_field_codes(word: &str) -> Option<String> {
    let mut plain = String::new();
    let mut coded = false;
    let mut chars = word.chars();
    while let Some(char) = chars.next() {
        match (char, char == '%') {
            (_, false) => plain.push(char),
            _ => match chars.next() {
                Some('%') => plain.push('%'),
                _ => coded = true,
            },
        }
    }
    (!coded || !plain.is_empty()).then_some(plain)
}

/// The keys and values of a desktop entry's `[Desktop Entry]` group, as written.
fn desktop_fields(text: &str) -> Vec<(&str, &str)> {
    let mut inside = false;
    text.lines()
        .map(str::trim)
        .filter_map(|line| {
            if line.starts_with('[') {
                inside = line == "[Desktop Entry]";
                return None;
            }
            if !inside || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            Some((key.trim(), value.trim()))
        })
        .collect()
}

/// A desktop entry's value with its escapes, `\s`, `\n`, `\t`, `\r` and `\\`, read.
fn unescape(value: &str) -> String {
    let mut plain = String::new();
    let mut chars = value.chars();
    while let Some(char) = chars.next() {
        if char != '\\' {
            plain.push(char);
            continue;
        }
        match chars.next() {
            Some('s') => plain.push(' '),
            Some('n') => plain.push('\n'),
            Some('t') => plain.push('\t'),
            Some('r') => plain.push('\r'),
            Some(other) => {
                if other != '\\' {
                    plain.push('\\');
                }
                plain.push(other);
            }
            None => plain.push('\\'),
        }
    }
    plain
}

/// The words of an `Exec`, unquoted as desktops do: double quotes, in which a backslash
/// escapes `"`, `` ` ``, `$` and `\`; single quotes; a backslash outside quotes.
fn exec_words(exec: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote = None;
    let mut chars = exec.chars().peekable();
    while let Some(char) = chars.next() {
        match (quote, char) {
            (Some('"'), '"') | (Some('\''), '\'') => quote = None,
            (Some('"'), '\\') => match chars.peek() {
                Some(&next @ ('"' | '`' | '$' | '\\')) => {
                    word.push(next);
                    chars.next();
                }
                _ => word.push('\\'),
            },
            (Some(_), _) => word.push(char),
            (None, '"' | '\'') => {
                quote = Some(char);
                started = true;
            }
            (None, '\\') => {
                word.extend(chars.next());
                started = true;
            }
            (None, ' ' | '\t' | '\n') => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (None, _) => {
                word.push(char);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
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
mod tests;
