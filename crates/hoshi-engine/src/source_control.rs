//! The git behind the thread's commit, push and pull request actions.

use crate::{diff, git};
use hoshi_core::GitStatus;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const RECENT_SUBJECTS: &str = "20";
const TEMPLATES: [&str; 4] = [
    ".github/pull_request_template.md",
    ".github/PULL_REQUEST_TEMPLATE.md",
    "PULL_REQUEST_TEMPLATE.md",
    "docs/pull_request_template.md",
];

/// What `dir`'s checkout allows.
pub fn status(dir: &str) -> Result<GitStatus, String> {
    let output = git::stdout(
        dir,
        &[
            "status",
            "--porcelain=2",
            "--branch",
            "-z",
            "--untracked-files=normal",
        ],
    )?;
    let mut status = parse_status(&output);
    status.has_remote = git::stdout(dir, &["remote"])
        .is_ok_and(|remotes| remotes.lines().any(|remote| remote.trim() == "origin"));
    if let Some(branch) = status.branch.clone() {
        let base = base_branch(dir, &branch);
        status.default_branch = base.as_ref().map(|(name, _)| name.clone());
        if let Some((name, reference)) = base
            && name != branch
        {
            status.ahead_of_default =
                git::stdout(dir, &["rev-list", "--count", &format!("{reference}..HEAD")])
                    .ok()
                    .and_then(|count| count.trim().parse().ok())
                    .unwrap_or(0);
        }
    }
    Ok(status)
}

/// `git status --porcelain=2 --branch -z`.
fn parse_status(output: &str) -> GitStatus {
    let mut status = GitStatus::default();
    for entry in output.split('\0') {
        if let Some(head) = entry.strip_prefix("# branch.head ") {
            status.branch = (head != "(detached)").then(|| head.to_owned());
        } else if entry.starts_with("# branch.upstream ") {
            status.has_upstream = true;
        } else if let Some(counts) = entry.strip_prefix("# branch.ab ") {
            let mut counts = counts.split_whitespace();
            let count = |text: Option<&str>| {
                text.map(|text| text.trim_start_matches(['+', '-']))
                    .and_then(|text| text.parse().ok())
                    .unwrap_or(0)
            };
            status.ahead = count(counts.next());
            status.behind = count(counts.next());
        } else if ["1 ", "2 ", "u ", "? "]
            .iter()
            .any(|kind| entry.starts_with(kind))
        {
            status.has_changes = true;
        }
    }
    status
}

/// The branch `branch` goes back into, by name and by the ref to compare with: the base
/// hoshi recorded for it, else the repository's default branch. Origin's copy wins, as
/// the one a pull request targets.
pub fn base_branch(dir: &str, branch: &str) -> Option<(String, String)> {
    let name = match git::branch_config(dir, branch, git::BASE_CONFIG) {
        Some(base) => base,
        None => {
            let default = diff::default_branch(dir).ok()?;
            default
                .strip_prefix("origin/")
                .unwrap_or(&default)
                .to_owned()
        }
    };
    let reference = [format!("origin/{name}"), name.clone()]
        .into_iter()
        .find(|reference| {
            git::stdout(dir, &["rev-parse", "--verify", "--quiet", reference]).is_ok()
        })?;
    Some((name, reference))
}

/// What a commit of `paths` (every change when `None`) would hold, read from a copy of the
/// index so the user's own stays as it is.
pub struct Staged {
    /// `git diff --cached --name-status`.
    pub files: String,
    pub patch: String,
}

pub fn preview(dir: &str, paths: Option<&[String]>) -> Result<Staged, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let index = git::stdout(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )?;
    let index = PathBuf::from(index.trim());
    let copy = index.with_file_name(format!(
        "hoshi-commit-index-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    copy_index(&index, &copy);
    let staged = (|| {
        let env = [("GIT_INDEX_FILE", copy.as_os_str())];
        git::stdout_with_env(dir, &add_args(paths), &env)?;
        let files = cached(dir, &env, &["--name-status"])?;
        let patch = cached(dir, &env, &["--patch", "--minimal", "--no-color"])?;
        Ok(Staged { files, patch })
    })();
    let _ = std::fs::remove_file(&copy);
    let _ = std::fs::remove_file(copy.with_extension("lock"));
    staged
}

fn cached(dir: &str, env: &[(&str, &OsStr)], args: &[&str]) -> Result<String, String> {
    let mut all = vec!["diff", "--cached", "--no-ext-diff"];
    all.extend_from_slice(args);
    git::stdout_with_env(dir, &all, env)
}

/// A copy older by a second, so files changed just before are hashed again.
fn copy_index(index: &Path, copy: &Path) {
    let Ok(modified) = std::fs::metadata(index).and_then(|metadata| metadata.modified()) else {
        return;
    };
    if std::fs::copy(index, copy).is_err() {
        return;
    }
    let older = modified
        .checked_sub(Duration::from_secs(1))
        .unwrap_or(modified);
    if let Ok(file) = std::fs::File::options().write(true).open(copy) {
        let _ = file.set_modified(older);
    }
}

fn add_args(paths: Option<&[String]>) -> Vec<&str> {
    let mut args = vec!["add", "--all"];
    if let Some(paths) = paths {
        args.push("--");
        args.extend(paths.iter().map(String::as_str));
    }
    args
}

/// The repository's latest commit subjects, newest first.
pub fn recent_subjects(dir: &str) -> Vec<String> {
    git::stdout(dir, &["log", "-n", RECENT_SUBJECTS, "--format=%s"])
        .map(|log| log.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Stages `paths` (every change when `None`) and commits them. `None` when nothing was
/// staged; otherwise the commit's short hash.
pub fn commit(
    dir: &str,
    paths: Option<&[String]>,
    subject: &str,
    body: &str,
) -> Result<Option<String>, String> {
    git::stdout(dir, &add_args(paths))?;
    if git::stdout(dir, &["diff", "--cached", "--name-only"])?
        .trim()
        .is_empty()
    {
        return Ok(None);
    }
    let mut args = vec!["commit", "--quiet", "-m", subject];
    if !body.trim().is_empty() {
        args.extend(["-m", body]);
    }
    git::slow_stdout(dir, &args)?;
    git::stdout(dir, &["rev-parse", "--short", "HEAD"]).map(|hash| Some(hash.trim().to_owned()))
}

/// Starts `branch` where the checkout is, keeping its changes, and switches to it.
pub fn create_branch(dir: &str, branch: &str) -> Result<(), String> {
    git::stdout(dir, &["switch", "-c", branch]).map(|_| ())
}

pub enum PushFailure {
    /// The remote has commits the branch lacks.
    Behind,
    Other(String),
}

/// Pushes `branch` to origin and tracks it there.
pub fn push(dir: &str, branch: &str) -> Result<(), PushFailure> {
    git::slow_stdout(dir, &["push", "--set-upstream", "origin", branch])
        .map(|_| ())
        .map_err(|detail| {
            let behind = ["non-fast-forward", "fetch first", "[rejected]"]
                .iter()
                .any(|sign| detail.contains(sign));
            if behind {
                PushFailure::Behind
            } else {
                PushFailure::Other(detail)
            }
        })
}

/// Brings in the upstream's commits when that needs no merge.
pub fn pull(dir: &str) -> Result<(), String> {
    git::slow_stdout(dir, &["pull", "--ff-only"]).map(|_| ())
}

/// What a pull request from HEAD into `base` holds: its commits, a stat and the patch.
pub struct Range {
    pub commits: String,
    pub stat: String,
    pub patch: String,
}

pub fn range(dir: &str, base: &str) -> Result<Range, String> {
    let commits = git::stdout(dir, &["log", "--oneline", &format!("{base}..HEAD")])?;
    let diff = format!("{base}...HEAD");
    let stat = git::stdout(dir, &["diff", "--no-color", "--stat", &diff])?;
    let patch = git::stdout(dir, &["diff", "--no-color", "--no-ext-diff", &diff])?;
    Ok(Range {
        commits,
        stat,
        patch,
    })
}

/// The repository's pull request template, when it has one.
pub fn template(dir: &str) -> Option<String> {
    TEMPLATES
        .iter()
        .find_map(|path| std::fs::read_to_string(Path::new(dir).join(path)).ok())
        .filter(|text| !text.trim().is_empty())
}

/// The subject and body of a message the user wrote: its first line, then the rest.
pub fn split_message(message: &str) -> Option<(String, String)> {
    let message = message.trim();
    let (subject, body) = message.split_once('\n').unwrap_or((message, ""));
    let subject = subject.trim();
    (!subject.is_empty()).then(|| (subject.to_owned(), body.trim().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_branch_its_counts_and_whether_anything_changed() {
        let clean = "# branch.oid abc\0# branch.head feat/x\0# branch.upstream origin/feat/x\0# branch.ab +2 -1\0";
        let dirty = "# branch.oid abc\0# branch.head (detached)\0? new.txt\0";

        let clean = parse_status(clean);
        let dirty = parse_status(dirty);

        assert_eq!(clean.branch.as_deref(), Some("feat/x"));
        assert!(clean.has_upstream && !clean.has_changes);
        assert_eq!((clean.ahead, clean.behind), (2, 1));
        assert_eq!(dirty.branch, None);
        assert!(dirty.has_changes && !dirty.has_upstream);
    }

    #[test]
    fn a_message_splits_into_its_subject_and_body() {
        assert_eq!(
            split_message("  fix: stop the loop\n\n- keep the token\n"),
            Some(("fix: stop the loop".into(), "- keep the token".into()))
        );
        assert_eq!(
            split_message("feat: export"),
            Some(("feat: export".into(), String::new()))
        );
        assert_eq!(split_message("  \n"), None);
    }

    #[test]
    fn a_preview_leaves_the_index_alone_and_a_commit_then_a_push_reach_origin() {
        let origin = git::tests::temp_repository("sc-origin");
        let origin_dir = origin.to_string_lossy().into_owned();
        git::stdout(
            &origin_dir,
            &["config", "receive.denyCurrentBranch", "ignore"],
        )
        .unwrap();
        let repo = origin.with_extension("clone");
        let _ = std::fs::remove_dir_all(&repo);
        git::clone(&origin_dir, &repo).unwrap();
        let dir = repo.to_string_lossy().into_owned();
        for (key, value) in [("user.email", "hoshi@example.com"), ("user.name", "hoshi")] {
            git::stdout(&dir, &["config", key, value]).unwrap();
        }
        git::stdout(&dir, &["switch", "-c", "feature"]).unwrap();
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        std::fs::write(repo.join("b.txt"), "two\n").unwrap();

        let staged = preview(&dir, Some(&["a.txt".to_owned()])).unwrap();
        assert!(staged.files.contains("a.txt") && !staged.files.contains("b.txt"));
        assert!(staged.patch.contains("+one"));
        let index = git::stdout(&dir, &["diff", "--cached", "--name-only"]).unwrap();
        assert!(index.trim().is_empty());
        let before = status(&dir).unwrap();
        assert!(before.has_changes && before.has_remote && !before.has_upstream);

        let hash = commit(&dir, Some(&["a.txt".to_owned()]), "feat: add a", "- one")
            .unwrap()
            .unwrap();
        assert!(!hash.is_empty());
        assert!(
            commit(&dir, Some(&["a.txt".to_owned()]), "again", "")
                .unwrap()
                .is_none()
        );
        assert!(push(&dir, "feature").is_ok());

        let after = status(&dir).unwrap();
        assert!(after.has_upstream && after.has_changes);
        assert_eq!(
            (after.ahead, after.behind, after.ahead_of_default),
            (0, 0, 1)
        );
        assert_eq!(after.default_branch.as_deref(), Some("main"));
        let (base, reference) = base_branch(&dir, "feature").unwrap();
        assert_eq!((base.as_str(), reference.as_str()), ("main", "origin/main"));
        let range = range(&dir, &reference).unwrap();
        assert!(range.commits.contains("feat: add a"));
        assert!(range.patch.contains("+one"));
        assert_eq!(recent_subjects(&dir)[0], "feat: add a");
        for dir in [repo, origin] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
