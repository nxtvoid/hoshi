use crate::git;
use hoshi_core::{DiffFile, DiffFileStatus, DiffScope, FileDiff};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// Longer diffs are cut so a huge file can't flood the UI.
const MAX_DIFF_LINES: usize = 3000;
/// Untracked files bigger than this are not read to count their lines.
const MAX_COUNTED_BYTES: u64 = 1_000_000;
/// Git's empty tree, the base of a repository without commits.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const DEFAULT_BRANCHES: [&str; 4] = ["origin/main", "origin/master", "main", "master"];

/// How long the default branch of a repository is trusted before it is looked up again.
const DEFAULT_BRANCH_TTL: Duration = Duration::from_secs(300);

static DEFAULT_BRANCHES_SEEN: Mutex<Option<HashMap<String, (String, Instant)>>> = Mutex::new(None);

/// Every changed file of the working tree at `dir`, sorted by path.
///
/// Plumbing (`diff-index`) rather than `git diff`, which rewrites the index to refresh it
/// and so races the agent's own `git add` for `index.lock`.
pub fn files(dir: &str, scope: DiffScope) -> Result<Vec<DiffFile>, String> {
    let base = base(dir, scope)?;
    let (changes, untracked) = thread::scope(|threads| {
        let untracked = threads
            .spawn(|| git::stdout(dir, &["ls-files", "--others", "--exclude-standard", "-z"]));
        let changes = git::stdout(
            dir,
            &["diff-index", "-M", "--raw", "--numstat", "-z", &base, "--"],
        );
        let untracked = untracked
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        (changes, untracked)
    });
    let mut files = parse_changes(&changes?);
    let untracked = untracked?;
    files.extend(
        untracked
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(|path| DiffFile {
                path: path.to_owned(),
                old_path: None,
                status: DiffFileStatus::Untracked,
                additions: count_lines(&Path::new(dir).join(path)),
                deletions: Some(0),
            }),
    );
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// The unified diff of one file listed by [`files`].
pub fn file(dir: &str, scope: DiffScope, file: &DiffFile) -> Result<FileDiff, String> {
    let text = if file.status == DiffFileStatus::Untracked {
        git::stdout_allowing_differences(
            dir,
            &[
                "diff",
                "--no-color",
                "--no-index",
                "--",
                "/dev/null",
                &file.path,
            ],
        )?
    } else {
        let base = base(dir, scope)?;
        let mut args = vec![
            "diff-index",
            "--patch",
            "--no-color",
            "--no-ext-diff",
            "-M",
            base.as_str(),
            "--",
        ];
        if let Some(old_path) = &file.old_path {
            args.push(old_path);
        }
        args.push(&file.path);
        git::stdout(dir, &args)?
    };
    Ok(truncate(&text))
}

/// The commit a scope compares the working tree against.
fn base(dir: &str, scope: DiffScope) -> Result<String, String> {
    let Ok(head) = git::stdout(dir, &["rev-parse", "--verify", "--quiet", "HEAD"]) else {
        return Ok(EMPTY_TREE.to_owned());
    };
    match scope {
        DiffScope::Uncommitted => Ok(head.trim().to_owned()),
        DiffScope::Branch => {
            let default = match recorded_base(dir) {
                Some(base) => base,
                None => default_branch(dir)?,
            };
            let merge_base = git::stdout(dir, &["merge-base", "HEAD", &default])?;
            Ok(merge_base.trim().to_owned())
        }
    }
}

/// The branch hoshi made the checked-out branch from, as it still exists here.
fn recorded_base(dir: &str) -> Option<String> {
    let branch = git::current_branch(dir)?;
    let base = git::branch_config(dir, &branch, git::BASE_CONFIG)?;
    [base.clone(), format!("origin/{base}")]
        .into_iter()
        .find(|name| git::stdout(dir, &["rev-parse", "--verify", "--quiet", name]).is_ok())
}

/// The branch a feature branch is compared with: origin's default, else the first that
/// exists of the usual names. Remembered for a while, since it almost never changes.
fn default_branch(dir: &str) -> Result<String, String> {
    let mut seen = DEFAULT_BRANCHES_SEEN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let seen = seen.get_or_insert_with(HashMap::new);
    if let Some((branch, at)) = seen.get(dir)
        && at.elapsed() < DEFAULT_BRANCH_TTL
    {
        return Ok(branch.clone());
    }
    let remote_default = git::stdout(dir, &["rev-parse", "--abbrev-ref", "origin/HEAD"])
        .ok()
        .map(|name| name.trim().to_owned());
    let branch = remote_default
        .into_iter()
        .chain(DEFAULT_BRANCHES.map(str::to_owned))
        .find(|name| git::stdout(dir, &["rev-parse", "--verify", "--quiet", name]).is_ok())
        .ok_or_else(|| "no default branch to compare with".to_owned())?;
    seen.insert(dir.to_owned(), (branch.clone(), Instant::now()));
    Ok(branch)
}

/// One `diff-index --raw` entry: its status letter, paths, and whether the mode changed.
struct RawChange {
    kind: char,
    path: String,
    old_path: Option<String>,
    mode_changed: bool,
}

/// `git diff-index --raw --numstat -z`: the raw entries, then the counts.
///
/// Without a refreshed index, a file only touched is listed as modified in the raw
/// entries but has no count; those are dropped.
pub(crate) fn parse_changes(output: &str) -> Vec<DiffFile> {
    let mut tokens = output.split('\0').filter(|token| !token.is_empty());
    let mut changes = Vec::new();
    let mut counts = HashMap::new();
    while let Some(token) = tokens.next() {
        if let Some(meta) = token.strip_prefix(':') {
            let fields: Vec<&str> = meta.split_whitespace().collect();
            let kind = fields
                .get(4)
                .and_then(|status| status.chars().next())
                .unwrap_or('M');
            let mode_changed = fields.first() != fields.get(1);
            let (old_path, path) = if matches!(kind, 'R' | 'C') {
                let (Some(old_path), Some(path)) = (tokens.next(), tokens.next()) else {
                    break;
                };
                (Some(old_path.to_owned()), path)
            } else {
                let Some(path) = tokens.next() else { break };
                (None, path)
            };
            changes.push(RawChange {
                kind,
                path: path.to_owned(),
                old_path,
                mode_changed,
            });
            continue;
        }
        let mut fields = token.splitn(3, '\t');
        let (Some(added), Some(deleted), Some(path)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let path = if path.is_empty() {
            let _old_path = tokens.next();
            match tokens.next() {
                Some(path) => path,
                None => break,
            }
        } else {
            path
        };
        counts.insert(
            path.to_owned(),
            (added.parse::<u32>().ok(), deleted.parse::<u32>().ok()),
        );
    }
    changes
        .into_iter()
        .filter(|change| {
            change.kind != 'M' || change.mode_changed || counts.contains_key(&change.path)
        })
        .map(|change| {
            let (additions, deletions) = counts.get(&change.path).copied().unwrap_or((None, None));
            let status = match change.kind {
                'R' => DiffFileStatus::Renamed,
                'A' | 'C' => DiffFileStatus::Added,
                'D' => DiffFileStatus::Deleted,
                _ => DiffFileStatus::Modified,
            };
            DiffFile {
                path: change.path,
                old_path: change.old_path,
                status,
                additions,
                deletions,
            }
        })
        .collect()
}

fn count_lines(path: &Path) -> Option<u32> {
    if std::fs::metadata(path).ok()?.len() > MAX_COUNTED_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.iter().take(8000).any(|byte| *byte == 0) {
        return None;
    }
    let newlines = bytes.iter().filter(|byte| **byte == b'\n').count();
    let unterminated = bytes.last().is_some_and(|byte| *byte != b'\n');
    Some((newlines + usize::from(unterminated)) as u32)
}

pub(crate) fn truncate(text: &str) -> FileDiff {
    match text.match_indices('\n').nth(MAX_DIFF_LINES - 1) {
        Some((index, _)) => FileDiff {
            text: text[..=index].to_owned(),
            truncated: true,
        },
        None => FileDiff {
            text: text.to_owned(),
            truncated: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_raw_entries_with_their_counts() {
        let raw = |modes: &str, status: &str| format!(":{modes} 1111111 0000000 {status}");
        let output = [
            raw("100644 100644", "M"),
            "src/a.rs".into(),
            raw("100644 100644", "R087"),
            "old/b.rs".into(),
            "new/b.rs".into(),
            raw("000000 100644", "A"),
            "c.txt".into(),
            raw("100644 000000", "D"),
            "gone.rs".into(),
            raw("100644 100644", "M"),
            "image.png".into(),
            raw("100644 100644", "M"),
            "touched.rs".into(),
            raw("100644 100755", "M"),
            "run.sh".into(),
            "3\t1\tsrc/a.rs".into(),
            "4\t0\t".into(),
            "old/b.rs".into(),
            "new/b.rs".into(),
            "1\t0\tc.txt".into(),
            "0\t2\tgone.rs".into(),
            "-\t-\timage.png".into(),
            String::new(),
        ]
        .join("\0");

        let files: Vec<_> = parse_changes(&output)
            .into_iter()
            .map(|file| {
                (
                    file.path,
                    file.old_path,
                    file.status,
                    file.additions,
                    file.deletions,
                )
            })
            .collect();

        assert_eq!(
            files,
            [
                (
                    "src/a.rs".into(),
                    None,
                    DiffFileStatus::Modified,
                    Some(3),
                    Some(1)
                ),
                (
                    "new/b.rs".into(),
                    Some("old/b.rs".into()),
                    DiffFileStatus::Renamed,
                    Some(4),
                    Some(0)
                ),
                (
                    "c.txt".into(),
                    None,
                    DiffFileStatus::Added,
                    Some(1),
                    Some(0)
                ),
                (
                    "gone.rs".into(),
                    None,
                    DiffFileStatus::Deleted,
                    Some(0),
                    Some(2)
                ),
                (
                    "image.png".into(),
                    None,
                    DiffFileStatus::Modified,
                    None,
                    None
                ),
                ("run.sh".into(), None, DiffFileStatus::Modified, None, None),
            ]
        );
    }

    #[test]
    fn long_diffs_are_cut() {
        let long = "line\n".repeat(MAX_DIFF_LINES + 10);

        let diff = truncate(&long);

        assert!(diff.truncated);
        assert_eq!(diff.text.lines().count(), MAX_DIFF_LINES);
        assert!(!truncate("a\nb\n").truncated);
    }

    #[test]
    fn a_branch_compares_with_the_base_it_was_made_from() {
        let repo = git::tests::temp_repository("diff-base");
        let dir = repo.to_string_lossy().into_owned();
        let commit = |name: &str| {
            std::fs::write(repo.join(name), "x").unwrap();
            git::stdout(&dir, &["add", name]).unwrap();
            git::stdout(&dir, &["commit", "-m", name]).unwrap();
        };
        git::stdout(&dir, &["switch", "-c", "dev"]).unwrap();
        commit("dev.txt");
        git::stdout(&dir, &["switch", "-c", "feature"]).unwrap();
        commit("feature.txt");
        let paths = || {
            files(&dir, DiffScope::Branch)
                .unwrap()
                .into_iter()
                .map(|file| file.path)
                .collect::<Vec<_>>()
        };
        assert_eq!(paths(), ["dev.txt", "feature.txt"]);

        git::set_branch_config(&dir, "feature", git::BASE_CONFIG, "dev").unwrap();

        assert_eq!(paths(), ["feature.txt"]);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn lists_changes_of_a_real_repository() {
        let repo = git::tests::temp_repository("diff");
        let dir = repo.to_string_lossy().into_owned();
        std::fs::write(repo.join("tracked.txt"), "one\n").unwrap();
        std::fs::write(repo.join("touched.txt"), "same\n").unwrap();
        git::stdout(&dir, &["add", "tracked.txt", "touched.txt"]).unwrap();
        git::stdout(&dir, &["commit", "-m", "add"]).unwrap();
        std::fs::write(repo.join("tracked.txt"), "one\ntwo\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(repo.join("touched.txt"), "same\n").unwrap();
        std::fs::write(repo.join("new.txt"), "a\nb\nc").unwrap();

        let files = files(&dir, DiffScope::Uncommitted).unwrap();

        let summary: Vec<_> = files
            .iter()
            .map(|file| (file.path.as_str(), file.status, file.additions))
            .collect();
        assert_eq!(
            summary,
            [
                ("new.txt", DiffFileStatus::Untracked, Some(3)),
                ("tracked.txt", DiffFileStatus::Modified, Some(1)),
            ]
        );
        let tracked = file(&dir, DiffScope::Uncommitted, &files[1]).unwrap();
        assert!(tracked.text.contains("+two"));
        let untracked = file(&dir, DiffScope::Uncommitted, &files[0]).unwrap();
        assert!(untracked.text.contains("+c"));
        let _ = std::fs::remove_dir_all(&repo);
    }
}
