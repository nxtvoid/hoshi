use crate::{diff, git};
use hoshi_core::{DiffFile, FileDiff};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;
use std::time::Duration;

/// Hoshi's own index, next to the repository's: unchanged files keep their stat data
/// from one checkpoint to the next, so only what changed is hashed again.
const INDEX_NAME: &str = "hoshi-checkpoint-index";
const REFS: &str = "refs/hoshi/checkpoints";
const WORKERS: usize = 4;
const SETTLE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy)]
pub enum Moment {
    Before,
    After,
}

/// The commit holding a turn's working tree as it was before it started or after it ended.
pub fn reference(thread_id: &str, turn_id: &str, moment: Moment) -> String {
    let moment = match moment {
        Moment::Before => "before",
        Moment::After => "after",
    };
    format!("{REFS}/{thread_id}/{turn_id}/{moment}")
}

/// Records the working tree at `dir`, untracked files included, as a commit on `reference`
/// whose parent is `HEAD`. The repository's own index is never written.
pub fn capture(dir: &str, reference: &str) -> Result<(), String> {
    let paths = git::stdout(
        dir,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "index",
            "--git-path",
            INDEX_NAME,
        ],
    )?;
    let mut paths = paths.lines();
    let (Some(user_index), Some(own_index)) = (paths.next(), paths.next()) else {
        return Err("git named no index".to_owned());
    };
    let own_index = PathBuf::from(own_index);
    let index_lock = index_lock(&own_index);
    let _held = index_lock.lock().unwrap_or_else(PoisonError::into_inner);
    if !own_index.exists() {
        seed(Path::new(user_index), &own_index);
    }
    let tree = match stage(dir, &own_index) {
        Ok(tree) => tree,
        Err(_) => {
            let _ = std::fs::remove_file(&own_index);
            stage(dir, &own_index)?
        }
    };
    let head = git::stdout(dir, &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]).ok();
    let mut args = vec!["commit-tree", "--no-gpg-sign", "-m", "hoshi checkpoint"];
    if let Some(head) = head.as_deref() {
        args.extend(["-p", head.trim()]);
    }
    args.push(&tree);
    let author = [
        ("GIT_AUTHOR_NAME", OsStr::new("hoshi")),
        ("GIT_AUTHOR_EMAIL", OsStr::new("hoshi@localhost")),
        ("GIT_COMMITTER_NAME", OsStr::new("hoshi")),
        ("GIT_COMMITTER_EMAIL", OsStr::new("hoshi@localhost")),
    ];
    let commit = git::stdout_with_env(dir, &args, &author)?;
    git::stdout(dir, &["update-ref", reference, commit.trim()]).map(|_| ())
}

/// Stages the whole working tree into `index` and returns its tree.
fn stage(dir: &str, index: &Path) -> Result<String, String> {
    let env = [("GIT_INDEX_FILE", index.as_os_str())];
    git::stdout_with_env(
        dir,
        &[
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.splitIndex=false",
            "-c",
            "core.safecrlf=false",
            "add",
            "--all",
        ],
        &env,
    )?;
    git::stdout_with_env(dir, &["write-tree"], &env).map(|tree| tree.trim().to_owned())
}

/// Starts hoshi's index as a copy of the repository's, which already knows which files
/// are unchanged. Made a second older, so files changed just before it was written are
/// still hashed rather than trusted.
fn seed(user_index: &Path, own_index: &Path) {
    let Ok(modified) = std::fs::metadata(user_index).and_then(|metadata| metadata.modified())
    else {
        return;
    };
    if std::fs::copy(user_index, own_index).is_err() {
        return;
    }
    let older = modified
        .checked_sub(Duration::from_secs(1))
        .unwrap_or(modified);
    if let Ok(file) = std::fs::File::options().write(true).open(own_index) {
        let _ = file.set_modified(older);
    }
}

fn index_lock(index: &Path) -> Arc<Mutex<()>> {
    static LOCKS: Mutex<Option<HashMap<PathBuf, Arc<Mutex<()>>>>> = Mutex::new(None);
    let mut locks = LOCKS.lock().unwrap_or_else(PoisonError::into_inner);
    locks
        .get_or_insert_with(HashMap::new)
        .entry(index.to_path_buf())
        .or_default()
        .clone()
}

/// The files that differ between two checkpoints.
pub fn changes(dir: &str, from: &str, to: &str) -> Result<Vec<DiffFile>, String> {
    let output = git::stdout(
        dir,
        &[
            "diff-tree",
            "-r",
            "-M",
            "--raw",
            "--numstat",
            "-z",
            from,
            to,
            "--",
        ],
    )?;
    Ok(diff::parse_changes(&output))
}

/// The unified diff of one file listed by [`changes`].
pub fn file_diff(dir: &str, from: &str, to: &str, file: &DiffFile) -> Result<FileDiff, String> {
    let mut args = vec![
        "diff-tree",
        "-r",
        "--patch",
        "--no-color",
        "--no-ext-diff",
        "-M",
        from,
        to,
        "--",
    ];
    if let Some(old_path) = &file.old_path {
        args.push(old_path);
    }
    args.push(&file.path);
    git::stdout(dir, &args).map(|text| diff::truncate(&text))
}

/// Deletes every checkpoint of a thread, so git can collect what only they held.
pub fn forget(dir: &str, thread_id: &str) -> Result<(), String> {
    let prefix = format!("{REFS}/{thread_id}/");
    let listing = git::stdout(dir, &["for-each-ref", "--format=%(refname)", &prefix])?;
    let deletions: String = listing
        .lines()
        .filter(|reference| !reference.is_empty())
        .map(|reference| format!("delete {reference}\n"))
        .collect();
    if deletions.is_empty() {
        return Ok(());
    }
    git::stdout_with_input(dir, &["update-ref", "--stdin"], deletions).map(|_| ())
}

type Job = Box<dyn FnOnce() + Send>;

/// Runs checkpoint work off the engine's lock. Jobs for one folder run in the order they
/// were queued, so a turn's tree after it ended is read before the next turn's before.
pub struct Checkpointer {
    workers: Vec<mpsc::Sender<Job>>,
}

impl Checkpointer {
    pub fn new() -> Self {
        let workers = (0..WORKERS)
            .map(|_| {
                let (sender, receiver) = mpsc::channel::<Job>();
                thread::spawn(move || {
                    for job in receiver {
                        job();
                    }
                });
                sender
            })
            .collect();
        Self { workers }
    }

    /// Jobs must not take the engine's lock, which [`Checkpointer::settle`] may hold.
    pub fn queue(&self, dir: &str, job: impl FnOnce() + Send + 'static) {
        let mut hasher = DefaultHasher::new();
        dir.hash(&mut hasher);
        let worker = (hasher.finish() % self.workers.len() as u64) as usize;
        let _ = self.workers[worker].send(Box::new(job));
    }

    /// Waits for the jobs queued for `dir` so far, as before removing it: on Windows a
    /// folder some git process runs in can't be deleted.
    pub fn settle(&self, dir: &str) {
        let (done, settled) = mpsc::channel();
        self.queue(dir, move || {
            let _ = done.send(());
        });
        let _ = settled.recv_timeout(SETTLE_TIMEOUT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoshi_core::DiffFileStatus;

    #[test]
    fn a_turn_s_changes_come_from_its_checkpoints_without_touching_the_index() {
        let repo = git::tests::temp_repository("checkpoint");
        let dir = repo.to_string_lossy().into_owned();
        std::fs::write(repo.join("kept.txt"), "one\n").unwrap();
        std::fs::write(repo.join("gone.txt"), "bye\n").unwrap();
        git::stdout(&dir, &["add", "."]).unwrap();
        git::stdout(&dir, &["commit", "-m", "files"]).unwrap();
        std::fs::write(repo.join("staged.txt"), "staged\n").unwrap();
        git::stdout(&dir, &["add", "staged.txt"]).unwrap();
        let index_before = git::stdout(&dir, &["ls-files", "--stage"]).unwrap();
        let before = reference("t", "m1", Moment::Before);
        let after = reference("t", "m1", Moment::After);

        capture(&dir, &before).unwrap();
        std::fs::write(repo.join("kept.txt"), "one\ntwo\n").unwrap();
        std::fs::remove_file(repo.join("gone.txt")).unwrap();
        std::fs::write(repo.join("new.txt"), "a\nb\n").unwrap();
        capture(&dir, &after).unwrap();

        let files: Vec<_> = changes(&dir, &before, &after)
            .unwrap()
            .into_iter()
            .map(|file| (file.path, file.status, file.additions, file.deletions))
            .collect();
        assert_eq!(
            files,
            [
                ("gone.txt".into(), DiffFileStatus::Deleted, Some(0), Some(1)),
                (
                    "kept.txt".into(),
                    DiffFileStatus::Modified,
                    Some(1),
                    Some(0)
                ),
                ("new.txt".into(), DiffFileStatus::Added, Some(2), Some(0)),
            ]
        );
        let kept = changes(&dir, &before, &after).unwrap().remove(1);
        assert!(
            file_diff(&dir, &before, &after, &kept)
                .unwrap()
                .text
                .contains("+two")
        );
        assert_eq!(
            git::stdout(&dir, &["ls-files", "--stage"]).unwrap(),
            index_before
        );
        let head = git::stdout(&dir, &["rev-parse", "HEAD"]).unwrap();
        let parent = git::stdout(&dir, &["rev-parse", &format!("{after}^")]).unwrap();
        assert_eq!(parent, head);

        forget(&dir, "t").unwrap();
        let left = git::stdout(&dir, &["for-each-ref", REFS]).unwrap();
        assert!(left.trim().is_empty());
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn jobs_for_one_folder_run_in_order() {
        let checkpointer = Checkpointer::new();
        let (sender, receiver) = mpsc::channel();
        for index in 0..20 {
            let sender = sender.clone();
            checkpointer.queue("/repo", move || {
                let _ = sender.send(index);
            });
        }
        let order: Vec<_> = (0..20).map(|_| receiver.recv().unwrap()).collect();
        assert_eq!(order, (0..20).collect::<Vec<_>>());
    }
}
