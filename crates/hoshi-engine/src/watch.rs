use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread;
use std::time::Duration;

/// Quiet time after the last change before reacting; a checkout writes several files.
const DEBOUNCE: Duration = Duration::from_millis(300);

/// Watches each project's `.git` folder and reports projects whose checked-out
/// branch or remote branches may have changed, whoever changed them: hoshi, an agent
/// or a terminal. A push moves a remote branch, which can mean a new pull request.
pub struct GitWatcher {
    watcher: RecommendedWatcher,
    /// The watched `.git` folder of each project.
    git_dirs: Arc<Mutex<HashMap<String, PathBuf>>>,
}

impl GitWatcher {
    pub fn new(on_change: impl Fn(Vec<String>) + Send + 'static) -> notify::Result<Self> {
        let git_dirs = Arc::new(Mutex::new(HashMap::<String, PathBuf>::new()));
        let (sender, receiver) = mpsc::channel::<PathBuf>();
        let watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
            let Ok(event) = result else { return };
            for path in event.paths.into_iter().filter(|path| is_relevant(path)) {
                let _ = sender.send(path);
            }
        })?;
        let lookup = git_dirs.clone();
        thread::spawn(move || {
            while let Ok(first) = receiver.recv() {
                let mut changed = vec![first];
                while let Ok(path) = receiver.recv_timeout(DEBOUNCE) {
                    changed.push(path);
                }
                let project_ids: Vec<String> = lock_dirs(&lookup)
                    .iter()
                    .filter(|(_, git_dir)| changed.iter().any(|path| path.starts_with(git_dir)))
                    .map(|(project_id, _)| project_id.clone())
                    .collect();
                if !project_ids.is_empty() {
                    on_change(project_ids);
                }
            }
        });
        Ok(Self { watcher, git_dirs })
    }

    pub fn watch(&mut self, project_id: &str, git_dir: PathBuf) {
        let mut git_dirs = lock_dirs(&self.git_dirs);
        let watched = git_dirs.values().any(|dir| dir == &git_dir);
        if !watched && let Err(error) = self.watcher.watch(&git_dir, RecursiveMode::Recursive) {
            eprintln!("hoshi: can't watch {}: {error}", git_dir.display());
            return;
        }
        git_dirs.insert(project_id.to_owned(), git_dir);
    }

    pub fn unwatch(&mut self, project_id: &str) {
        let mut git_dirs = lock_dirs(&self.git_dirs);
        let Some(git_dir) = git_dirs.remove(project_id) else {
            return;
        };
        if !git_dirs.values().any(|dir| dir == &git_dir) {
            let _ = self.watcher.unwatch(&git_dir);
        }
    }
}

fn lock_dirs(
    git_dirs: &Mutex<HashMap<String, PathBuf>>,
) -> MutexGuard<'_, HashMap<String, PathBuf>> {
    git_dirs
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A `HEAD` file names the checked-out branch of the main worktree or of a linked one;
/// `refs/remotes` and `packed-refs` hold the remote branches. Copies under `logs` are
/// history.
fn is_relevant(path: &Path) -> bool {
    let has = |part: &str| {
        path.components()
            .any(|component| component == Component::Normal(part.as_ref()))
    };
    if has("logs") {
        return false;
    }
    let name = path.file_name().unwrap_or_default();
    name == "HEAD" || name == "packed-refs" || (has("refs") && has("remotes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heads_and_remote_branches_count_outside_logs() {
        assert!(is_relevant(Path::new("/repo/.git/HEAD")));
        assert!(is_relevant(Path::new("/repo/.git/worktrees/fix/HEAD")));
        assert!(is_relevant(Path::new("/repo/.git/refs/remotes/origin/fix")));
        assert!(is_relevant(Path::new("/repo/.git/packed-refs")));
        assert!(!is_relevant(Path::new("/repo/.git/logs/HEAD")));
        assert!(!is_relevant(Path::new(
            "/repo/.git/logs/refs/remotes/origin/fix"
        )));
        assert!(!is_relevant(Path::new("/repo/.git/FETCH_HEAD")));
        assert!(!is_relevant(Path::new("/repo/.git/HEAD.lock")));
        assert!(!is_relevant(Path::new("/repo/.git/objects/ab/cdef")));
    }
}
