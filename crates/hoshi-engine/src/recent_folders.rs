//! The folders the agents worked in before, offered as projects on first run.

use hoshi_providers::SessionFolder;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentFolder {
    pub path: String,
    pub name: String,
    /// The providers that ran sessions there, the busiest first.
    pub providers: Vec<String>,
    pub sessions: usize,
    /// When a session there was last written, in RFC 3339.
    pub active_at: String,
    /// A project already opens it.
    pub added: bool,
}

/// What is never offered: the home folder, temporary and app data folders, downloads, and
/// anything inside a hidden folder of the home, where tools such as hoshi keep their own.
pub(crate) struct Excluded {
    roots: Vec<String>,
    ancestors: Vec<String>,
    home: Option<String>,
}

impl Excluded {
    pub(crate) fn new(home: Option<&Path>, data_dir: &Path, temp_dir: &Path) -> Self {
        let mut roots = Vec::new();
        let mut ancestors = vec![key(data_dir), key(&canonical(temp_dir))];
        if let Some(home) = home {
            roots.push(key(home));
            for folder in ["Downloads", "AppData", "Library"] {
                ancestors.push(key(&home.join(folder)));
            }
        }
        Self {
            roots,
            ancestors,
            home: home.map(key),
        }
    }

    fn excludes(&self, folder: &str) -> bool {
        let under = |ancestor: &String| {
            folder
                .strip_prefix(ancestor.as_str())
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        };
        let in_hidden = |home: &String| {
            folder
                .strip_prefix(home.as_str())
                .and_then(|rest| rest.strip_prefix("/."))
                .is_some_and(|rest| rest.contains('/'))
        };
        self.roots.iter().any(|root| root == folder)
            || self.ancestors.iter().any(under)
            || self.home.as_ref().is_some_and(in_hidden)
    }
}

fn canonical(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Merges each provider's folders into one list, newest first. A linked worktree counts as
/// its main checkout; folders that are gone or excluded are left out.
pub(crate) fn gather(
    found: Vec<(String, Vec<SessionFolder>)>,
    excluded: &Excluded,
    project_roots: &[String],
) -> Vec<RecentFolder> {
    struct Merged {
        path: PathBuf,
        sessions: HashMap<String, usize>,
        active_at: SystemTime,
    }
    let mut merged: HashMap<String, Merged> = HashMap::new();
    for (provider, folders) in found {
        for folder in folders {
            let path = PathBuf::from(&folder.path);
            if !path.is_absolute() {
                continue;
            }
            let path = main_checkout(&path).unwrap_or(path);
            let Ok(path) = dunce::canonicalize(&path) else {
                continue;
            };
            let folder_key = key(&path);
            if !path.is_dir() || excluded.excludes(&folder_key) {
                continue;
            }
            let entry = merged.entry(folder_key).or_insert_with(|| Merged {
                path,
                sessions: HashMap::new(),
                active_at: folder.active_at,
            });
            *entry.sessions.entry(provider.clone()).or_default() += folder.sessions;
            entry.active_at = entry.active_at.max(folder.active_at);
        }
    }
    let added: Vec<String> = project_roots
        .iter()
        .map(|root| key(&dunce::canonicalize(root).unwrap_or_else(|_| PathBuf::from(root))))
        .collect();
    let mut folders: Vec<_> = merged
        .into_iter()
        .map(|(folder_key, merged)| {
            let mut providers: Vec<_> = merged.sessions.iter().collect();
            providers.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
            let path = merged.path.to_string_lossy().into_owned();
            RecentFolder {
                name: merged
                    .path
                    .file_name()
                    .map_or_else(|| path.clone(), |name| name.to_string_lossy().into_owned()),
                path,
                providers: providers.iter().map(|(id, _)| (*id).clone()).collect(),
                sessions: merged.sessions.values().sum(),
                active_at: chrono::DateTime::<chrono::Utc>::from(merged.active_at)
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                added: added.contains(&folder_key),
            }
        })
        .collect();
    folders.sort_by(|a, b| {
        b.active_at
            .cmp(&a.active_at)
            .then_with(|| a.path.cmp(&b.path))
    });
    folders
}

/// The checkout a linked worktree belongs to; `None` for any other folder.
fn main_checkout(folder: &Path) -> Option<PathBuf> {
    let pointer = std::fs::read_to_string(folder.join(".git")).ok()?;
    let git_dir = pointer
        .lines()
        .find_map(|line| line.strip_prefix("gitdir:"))?
        .trim();
    let git_dir = folder.join(git_dir);
    let common = std::fs::read_to_string(git_dir.join("commondir")).ok()?;
    let common = lexical(&git_dir.join(common.trim()));
    if common.file_name()? != ".git" {
        return None;
    }
    let checkout = common.parent()?.to_path_buf();
    checkout.is_dir().then_some(checkout)
}

fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// How two folders compare: one separator, no trailing one, and any case on Windows.
fn key(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let text = text.trim_end_matches('/');
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{Excluded, gather};
    use hoshi_providers::SessionFolder;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    fn scratch() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hoshi-recent-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dunce::canonicalize(dir).unwrap()
    }

    fn session(path: &std::path::Path, hours_ago: u64, sessions: usize) -> SessionFolder {
        SessionFolder {
            path: path.to_string_lossy().into_owned(),
            active_at: SystemTime::now() - Duration::from_secs(hours_ago * 3600),
            sessions,
        }
    }

    #[test]
    fn folders_merge_across_agents_newest_first_and_skip_what_is_never_a_project() {
        let dir = scratch();
        let home = dir.join("home");
        let app = home.join("code").join("app");
        let api = home.join("code").join("api");
        let data = home.join(".hoshi");
        let chat = data.join("chats").join("c1");
        let other_home = home.join(".hoshi-dev").join("worktrees").join("w1");
        let downloads = home.join("Downloads").join("zip");
        let app_data = home.join("AppData").join("Local").join("cache");
        let scratch_folder = dir.join("tmp").join("scratch");
        let dotfiles = home.join(".dotfiles");
        for folder in [
            &app,
            &api,
            &chat,
            &other_home,
            &downloads,
            &app_data,
            &scratch_folder,
            &dotfiles,
        ] {
            fs::create_dir_all(folder).unwrap();
        }
        let gone = home.join("code").join("gone");

        let folders = gather(
            vec![
                (
                    "claudeCode".to_owned(),
                    vec![
                        session(&app, 5, 3),
                        session(&api, 1, 1),
                        session(&chat, 0, 9),
                        session(&home, 0, 4),
                        session(&gone, 0, 1),
                        session(&other_home, 0, 1),
                        session(&downloads, 0, 1),
                        session(&app_data, 0, 1),
                        session(&scratch_folder, 0, 1),
                        session(&dotfiles, 30, 1),
                    ],
                ),
                ("codex".to_owned(), vec![session(&app, 2, 5)]),
            ],
            &Excluded::new(Some(&home), &data, &dir.join("tmp")),
            &[api.to_string_lossy().into_owned()],
        );

        let found: Vec<_> = folders
            .iter()
            .map(|folder| {
                (
                    folder.name.as_str(),
                    folder.providers.clone(),
                    folder.sessions,
                    folder.added,
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                ("api", vec!["claudeCode".to_owned()], 1, true),
                (
                    "app",
                    vec!["codex".to_owned(), "claudeCode".to_owned()],
                    8,
                    false
                ),
                (".dotfiles", vec!["claudeCode".to_owned()], 1, false),
            ]
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_linked_worktree_counts_as_its_main_checkout() {
        let dir = scratch();
        let repo = dir.join("repo");
        let git = repo.join(".git").join("worktrees").join("feature");
        fs::create_dir_all(&git).unwrap();
        fs::write(git.join("commondir"), "../..\n").unwrap();
        let worktree = dir.join("elsewhere").join("feature");
        fs::create_dir_all(&worktree).unwrap();
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", git.to_string_lossy()),
        )
        .unwrap();

        let folders = gather(
            vec![(
                "claudeCode".to_owned(),
                vec![session(&worktree, 1, 2), session(&repo, 3, 1)],
            )],
            &Excluded::new(None, &dir.join("data"), &dir.join("tmp")),
            &[],
        );
        assert_eq!(folders.len(), 1);
        assert_eq!(PathBuf::from(&folders[0].path), repo);
        assert_eq!(folders[0].sessions, 3);
        fs::remove_dir_all(dir).ok();
    }
}
