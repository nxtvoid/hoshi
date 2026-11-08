use crate::git;
use hoshi_core::{DiffFile, DiffFileStatus, DiffScope, FileDiff};
use std::collections::HashMap;
use std::path::Path;

/// Longer diffs are cut so a huge file can't flood the UI.
const MAX_DIFF_LINES: usize = 3000;
/// Untracked files bigger than this are not read to count their lines.
const MAX_COUNTED_BYTES: u64 = 1_000_000;
/// Git's empty tree, the base of a repository without commits.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const DEFAULT_BRANCHES: [&str; 4] = ["origin/main", "origin/master", "main", "master"];

/// Every changed file of the working tree at `dir`, sorted by path.
pub fn files(dir: &str, scope: DiffScope) -> Result<Vec<DiffFile>, String> {
    let base = base(dir, scope)?;
    let statuses = git::stdout(dir, &["diff", "--name-status", "-z", "-M", &base])?;
    let numstats = git::stdout(dir, &["diff", "--numstat", "-z", "-M", &base])?;
    let untracked = git::stdout(dir, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    let counts = parse_numstat(&numstats);
    let mut files: Vec<DiffFile> = parse_name_status(&statuses)
        .into_iter()
        .map(|(status, path, old_path)| {
            let (additions, deletions) = counts.get(&path).copied().unwrap_or((None, None));
            DiffFile {
                path,
                old_path,
                status,
                additions,
                deletions,
            }
        })
        .collect();
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
        let mut args = vec!["diff", "--no-color", "-M", base.as_str(), "--"];
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
            let remote_default = git::stdout(dir, &["rev-parse", "--abbrev-ref", "origin/HEAD"])
                .ok()
                .map(|name| name.trim().to_owned());
            let default = remote_default
                .into_iter()
                .chain(DEFAULT_BRANCHES.map(str::to_owned))
                .find(|name| git::stdout(dir, &["rev-parse", "--verify", "--quiet", name]).is_ok())
                .ok_or_else(|| "no default branch to compare with".to_owned())?;
            let merge_base = git::stdout(dir, &["merge-base", "HEAD", &default])?;
            Ok(merge_base.trim().to_owned())
        }
    }
}

/// `git diff --name-status -z`: a status, then one path, or two for renames and copies.
fn parse_name_status(output: &str) -> Vec<(DiffFileStatus, String, Option<String>)> {
    let mut tokens = output.split('\0').filter(|token| !token.is_empty());
    let mut files = Vec::new();
    while let Some(status) = tokens.next() {
        let kind = status.chars().next().unwrap_or('M');
        if matches!(kind, 'R' | 'C') {
            let (Some(old_path), Some(path)) = (tokens.next(), tokens.next()) else {
                break;
            };
            let status = if kind == 'R' {
                DiffFileStatus::Renamed
            } else {
                DiffFileStatus::Added
            };
            files.push((status, path.to_owned(), Some(old_path.to_owned())));
            continue;
        }
        let Some(path) = tokens.next() else { break };
        let status = match kind {
            'A' => DiffFileStatus::Added,
            'D' => DiffFileStatus::Deleted,
            _ => DiffFileStatus::Modified,
        };
        files.push((status, path.to_owned(), None));
    }
    files
}

/// `git diff --numstat -z`: `added\tdeleted\tpath`, or `added\tdeleted\t` then the old and
/// new paths for a rename. `-` counts mean a binary file.
fn parse_numstat(output: &str) -> HashMap<String, (Option<u32>, Option<u32>)> {
    let mut tokens = output.split('\0');
    let mut counts = HashMap::new();
    while let Some(token) = tokens.next() {
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
        counts.insert(path.to_owned(), (added.parse().ok(), deleted.parse().ok()));
    }
    counts
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
    fn reads_statuses_with_renames() {
        let output = "M\0src/a.rs\0R087\0old/b.rs\0new/b.rs\0A\0c.txt\0D\0gone.rs\0";

        assert_eq!(
            parse_name_status(output),
            [
                (DiffFileStatus::Modified, "src/a.rs".into(), None),
                (
                    DiffFileStatus::Renamed,
                    "new/b.rs".into(),
                    Some("old/b.rs".into())
                ),
                (DiffFileStatus::Added, "c.txt".into(), None),
                (DiffFileStatus::Deleted, "gone.rs".into(), None),
            ]
        );
    }

    #[test]
    fn reads_counts_with_renames_and_binaries() {
        let output = "3\t1\tsrc/a.rs\x004\t0\t\0old/b.rs\0new/b.rs\0-\t-\timage.png\0";

        let counts = parse_numstat(output);

        assert_eq!(counts["src/a.rs"], (Some(3), Some(1)));
        assert_eq!(counts["new/b.rs"], (Some(4), Some(0)));
        assert_eq!(counts["image.png"], (None, None));
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
    fn lists_changes_of_a_real_repository() {
        let repo = git::tests::temp_repository("diff");
        let dir = repo.to_string_lossy().into_owned();
        std::fs::write(repo.join("tracked.txt"), "one\n").unwrap();
        git::stdout(&dir, &["add", "tracked.txt"]).unwrap();
        git::stdout(&dir, &["commit", "-m", "add"]).unwrap();
        std::fs::write(repo.join("tracked.txt"), "one\ntwo\n").unwrap();
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
