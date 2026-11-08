//! The files and folders of a working tree, ranked for `@` mentions.

use hoshi_core::{EntryKind, WorkspaceEntry};
use ignore::WalkBuilder;
use std::path::Path;

/// Keeps the search fast and the index light in huge trees.
const MAX_ENTRIES: usize = 50_000;

pub(crate) struct Indexed {
    entry: WorkspaceEntry,
    lower: String,
    name_start: usize,
    depth: usize,
}

/// Everything under `root` that git would not ignore, without `.git` itself.
pub(crate) fn read(root: &Path) -> Vec<Indexed> {
    WalkBuilder::new(root)
        .hidden(false)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build()
        .flatten()
        .filter_map(|entry| {
            let relative = entry.path().strip_prefix(root).ok()?;
            if relative.as_os_str().is_empty() {
                return None;
            }
            let path = relative.to_string_lossy().replace('\\', "/");
            let kind = if entry.file_type().is_some_and(|kind| kind.is_dir()) {
                EntryKind::Directory
            } else {
                EntryKind::File
            };
            Some(indexed(WorkspaceEntry { path, kind }))
        })
        .take(MAX_ENTRIES)
        .collect()
}

fn indexed(entry: WorkspaceEntry) -> Indexed {
    let lower = entry.path.to_lowercase();
    let name_start = lower.rfind('/').map_or(0, |slash| slash + 1);
    let depth = lower.matches('/').count();
    Indexed {
        entry,
        lower,
        name_start,
        depth,
    }
}

/// The best `limit` matches; with an empty query, the shallowest entries.
pub(crate) fn search(entries: &[Indexed], query: &str, limit: usize) -> Vec<WorkspaceEntry> {
    let query = query.trim().replace('\\', "/").to_lowercase();
    let query = query.trim_start_matches("./");
    let mut ranked: Vec<_> = entries
        .iter()
        .filter_map(|entry| Some((score(entry, query)?, entry)))
        .collect();
    let order = |(a_score, a): &(u32, &Indexed), (b_score, b): &(u32, &Indexed)| {
        a_score
            .cmp(b_score)
            .then(a.depth.cmp(&b.depth))
            .then(a.lower.len().cmp(&b.lower.len()))
            .then_with(|| a.entry.path.cmp(&b.entry.path))
    };
    if ranked.len() > limit {
        ranked.select_nth_unstable_by(limit, order);
        ranked.truncate(limit);
    }
    ranked.sort_by(order);
    ranked
        .into_iter()
        .map(|(_, entry)| entry.entry.clone())
        .collect()
}

/// Lower is better; `None` when the entry does not match at all.
fn score(entry: &Indexed, query: &str) -> Option<u32> {
    if query.is_empty() {
        return Some(0);
    }
    let path = entry.lower.as_str();
    let name = &path[entry.name_start..];
    if name == query {
        Some(0)
    } else if name.starts_with(query) {
        Some(1)
    } else if path.starts_with(query) {
        Some(2)
    } else if name.contains(query) {
        Some(3)
    } else if path.contains(query) {
        Some(4)
    } else if let Some(spread) = spread(name, query) {
        Some(10 + spread)
    } else {
        spread(path, query).map(|spread| 100 + spread)
    }
}

/// How many extra characters a fuzzy match of `query` in `text` spans.
fn spread(text: &str, query: &str) -> Option<u32> {
    let mut wanted = query.chars().peekable();
    let (mut first, mut last) = (None, 0);
    for (index, char) in text.chars().enumerate() {
        if wanted.peek() == Some(&char) {
            wanted.next();
            first.get_or_insert(index);
            last = index;
        }
    }
    if wanted.peek().is_some() {
        return None;
    }
    let span = last + 1 - first.unwrap_or(0);
    Some((span - query.chars().count()) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn entries(paths: &[&str]) -> Vec<Indexed> {
        paths
            .iter()
            .map(|path| {
                let (path, kind) = match path.strip_suffix('/') {
                    Some(directory) => (directory, EntryKind::Directory),
                    None => (*path, EntryKind::File),
                };
                indexed(WorkspaceEntry {
                    path: path.into(),
                    kind,
                })
            })
            .collect()
    }

    fn paths(found: Vec<WorkspaceEntry>) -> Vec<String> {
        found.into_iter().map(|entry| entry.path).collect()
    }

    #[test]
    fn exact_names_come_before_prefixes_substrings_and_fuzzy_matches() {
        let index = entries(&[
            "src/components/thread/ComposerMenus.tsx",
            "src/lib/my-composer.ts",
            "docs/composer",
            "src/composer.ts",
            "src/c-o-m-p-o-s-e-r.txt",
            "README.md",
        ]);

        assert_eq!(
            paths(search(&index, "composer", 10)),
            [
                "docs/composer",
                "src/composer.ts",
                "src/components/thread/ComposerMenus.tsx",
                "src/lib/my-composer.ts",
                "src/c-o-m-p-o-s-e-r.txt",
            ]
        );
    }

    #[test]
    fn a_query_with_folders_matches_the_path() {
        let index = entries(&["apps/desktop/src/", "src/app.ts", "apps/desktop/"]);

        assert_eq!(
            paths(search(&index, "apps/desk", 10)),
            ["apps/desktop", "apps/desktop/src"]
        );
    }

    #[test]
    fn an_empty_query_lists_the_shallowest_entries_first() {
        let index = entries(&["src/lib/a.ts", "src/", "Cargo.toml", "src/b.ts"]);

        assert_eq!(
            paths(search(&index, "", 3)),
            ["src", "Cargo.toml", "src/b.ts"]
        );
    }

    #[test]
    fn reading_skips_ignored_files_and_the_git_folder() {
        let root = std::env::temp_dir().join(format!("hoshi-paths-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join(".gitignore"), "target/\n").unwrap();
        fs::write(root.join(".git/config"), "").unwrap();
        fs::write(root.join("target/debug/app"), "").unwrap();
        fs::write(root.join("src/main.rs"), "").unwrap();

        let mut found = paths(search(&read(&root), "", 10));
        found.sort();

        assert_eq!(found, [".gitignore", "src", "src/main.rs"]);
        let _ = fs::remove_dir_all(&root);
    }
}
