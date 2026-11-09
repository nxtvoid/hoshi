//! Copies the files a project's worktrees need but the repository doesn't track, such as
//! `.env`, from the project root into a new worktree.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CopyReport {
    pub copied: u32,
    /// Patterns that matched nothing.
    pub missing: Vec<String>,
    /// Paths relative to the root that couldn't be copied or lead outside.
    pub failed: Vec<String>,
}

/// Copies what each pattern matches, folders with everything in them, over what the
/// worktree has at the same paths. Wildcards are expanded one segment at a time, so only
/// the folders a pattern walks into are listed.
pub fn copy_setup_files(root: &Path, worktree: &Path, patterns: &[String]) -> CopyReport {
    let mut report = CopyReport::default();
    let (Ok(root), Ok(worktree)) = (dunce::canonicalize(root), dunce::canonicalize(worktree))
    else {
        report.failed = patterns.to_vec();
        return report;
    };
    let mut copier = Copier {
        root: &root,
        worktree: &worktree,
        report: &mut report,
        visited: HashSet::new(),
    };
    for pattern in patterns {
        let Ok(Some(pattern)) = hoshi_core::setup_file_path(pattern) else {
            copier.report.failed.push(pattern.clone());
            continue;
        };
        let matches = expand(&root, &pattern);
        if matches.is_empty() {
            copier.report.missing.push(pattern);
            continue;
        }
        for relative in matches {
            copier.copy(&relative);
        }
    }
    report
}

/// The paths under `root`, relative and `/`-separated, that `pattern` matches.
fn expand(root: &Path, pattern: &str) -> Vec<String> {
    let segments: Vec<&str> = pattern.split('/').collect();
    let mut found = vec![String::new()];
    for (index, segment) in segments.iter().enumerate() {
        let last = index + 1 == segments.len();
        let mut next = Vec::new();
        for parent in &found {
            let folder = root.join(parent);
            if has_wildcard(segment) {
                let Ok(entries) = fs::read_dir(&folder) else {
                    continue;
                };
                let mut names: Vec<String> = entries
                    .filter_map(Result::ok)
                    .filter(|entry| last || entry.path().is_dir())
                    .filter_map(|entry| entry.file_name().into_string().ok())
                    .filter(|name| !(parent.is_empty() && name.eq_ignore_ascii_case(".git")))
                    .filter(|name| wildcard_matches(segment, name))
                    .collect();
                names.sort();
                next.extend(names.into_iter().map(|name| join(parent, &name)));
            } else {
                let path = folder.join(segment);
                let exists = if last {
                    fs::symlink_metadata(&path).is_ok()
                } else {
                    path.is_dir()
                };
                if exists {
                    next.push(join(parent, segment));
                }
            }
        }
        found = next;
        if found.is_empty() {
            break;
        }
    }
    found
}

fn join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

fn has_wildcard(segment: &str) -> bool {
    segment.contains(['*', '?'])
}

/// Whether `name` fits `pattern`, where `*` stands for any run of characters and `?` for one.
fn wildcard_matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, n));
            p += 1;
        } else if let Some((star_p, star_n)) = star {
            p = star_p + 1;
            n = star_n + 1;
            star = Some((star_p, star_n + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&char| char == '*')
}

struct Copier<'a> {
    root: &'a Path,
    worktree: &'a Path,
    report: &'a mut CopyReport,
    /// Folders already copied, by their real path, so a link back up doesn't loop.
    visited: HashSet<PathBuf>,
}

impl Copier<'_> {
    fn copy(&mut self, relative: &str) {
        let Ok(source) = dunce::canonicalize(self.root.join(relative)) else {
            self.report.failed.push(relative.to_owned());
            return;
        };
        if !source.starts_with(self.root) {
            self.report.failed.push(relative.to_owned());
            return;
        }
        if source.is_dir() {
            if !self.visited.insert(source.clone()) {
                return;
            }
            let made = self
                .destination(relative)
                .is_some_and(|target| fs::create_dir_all(target).is_ok());
            let entries = fs::read_dir(&source).ok().filter(|_| made);
            let Some(entries) = entries else {
                self.report.failed.push(relative.to_owned());
                return;
            };
            let mut names: Vec<String> = entries
                .filter_map(Result::ok)
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect();
            names.sort();
            for name in names {
                self.copy(&join(relative, &name));
            }
        } else if self
            .destination(relative)
            .is_none_or(|target| fs::copy(&source, target).is_err())
        {
            self.report.failed.push(relative.to_owned());
        } else {
            self.report.copied = self.report.copied.saturating_add(1);
        }
    }

    /// Where `relative` goes in the worktree, its parent folders made; `None` when that
    /// would lead outside the worktree, as through a link.
    fn destination(&self, relative: &str) -> Option<PathBuf> {
        let target = self.worktree.join(relative);
        let parent = target.parent()?;
        fs::create_dir_all(parent).ok()?;
        let real_parent = dunce::canonicalize(parent).ok()?;
        if !real_parent.starts_with(self.worktree) {
            return None;
        }
        if fs::symlink_metadata(&target).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return None;
        }
        Some(real_parent.join(target.file_name()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> (PathBuf, PathBuf) {
        let base =
            std::env::temp_dir().join(format!("hoshi-setup-files-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let root = base.join("root");
        let worktree = base.join("worktree");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        (root, worktree)
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn patterns(list: &[&str]) -> Vec<String> {
        list.iter().map(|&pattern| pattern.to_owned()).collect()
    }

    #[test]
    fn wildcards_match_within_a_name() {
        assert!(wildcard_matches("*.env", "local.env"));
        assert!(wildcard_matches("*.env", ".env"));
        assert!(wildcard_matches("a?c*", "abcdef"));
        assert!(wildcard_matches("*", "anything"));
        assert!(!wildcard_matches("*.env", "local.env.bak"));
        assert!(!wildcard_matches("a?c", "ac"));
    }

    #[test]
    fn copies_files_and_folders_at_the_same_paths() {
        let (root, worktree) = scratch("copy");
        write(&root.join(".env"), "KEY=1");
        write(&root.join("config/app.properties"), "a=b");
        write(&root.join("config/nested/more.txt"), "more");
        write(&worktree.join(".env"), "old");

        let report = copy_setup_files(&root, &worktree, &patterns(&[".env", "config"]));

        assert_eq!(report.copied, 3);
        assert!(report.missing.is_empty() && report.failed.is_empty());
        assert_eq!(fs::read_to_string(worktree.join(".env")).unwrap(), "KEY=1");
        assert_eq!(
            fs::read_to_string(worktree.join("config/nested/more.txt")).unwrap(),
            "more"
        );
    }

    #[test]
    fn wildcards_expand_segment_by_segment() {
        let (root, worktree) = scratch("wildcards");
        write(&root.join("secrets/a.env"), "a");
        write(&root.join("secrets/b.env"), "b");
        write(&root.join("secrets/notes.txt"), "n");
        write(&root.join("apps/web/.env.local"), "w");
        write(&root.join("apps/api/.env.local"), "x");
        write(&root.join("apps/readme.md"), "r");

        let report = copy_setup_files(
            &root,
            &worktree,
            &patterns(&["secrets/*.env", "apps/*/.env.?ocal", "nothing/*"]),
        );

        assert_eq!(report.copied, 4);
        assert_eq!(report.missing, ["nothing/*"]);
        assert!(report.failed.is_empty());
        assert!(worktree.join("secrets/b.env").is_file());
        assert!(!worktree.join("secrets/notes.txt").exists());
        assert!(worktree.join("apps/api/.env.local").is_file());
    }

    #[test]
    fn never_copies_the_repository_itself() {
        let (root, worktree) = scratch("git");
        write(&root.join(".git/config"), "[core]");
        write(&root.join("a.txt"), "a");
        write(&worktree.join(".git"), "gitdir: elsewhere");

        let report = copy_setup_files(&root, &worktree, &patterns(&["*", ".git", "../x"]));

        assert_eq!(report.copied, 1);
        assert_eq!(report.failed, [".git", "../x"]);
        assert_eq!(
            fs::read_to_string(worktree.join(".git")).unwrap(),
            "gitdir: elsewhere"
        );
    }
}
