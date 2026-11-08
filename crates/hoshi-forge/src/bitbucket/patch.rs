//! Bitbucket sends a pull request's changes as one unified diff; hoshi wants each file's
//! hunks on their own.

use std::collections::HashMap;

/// Each file's hunks, without its headers, by its new path (the old one for deletions).
pub fn split(diff: &str) -> HashMap<String, String> {
    let mut patches = HashMap::new();
    let mut current: Option<(String, String)> = None;
    let mut old_path: Option<String> = None;
    let mut in_hunks = false;
    for line in diff.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            if let Some((path, hunks)) = current.take() {
                patches.insert(path, hunks);
            }
            current = Some((git_header_path(line).unwrap_or_default(), String::new()));
            old_path = None;
            in_hunks = false;
            continue;
        }
        let Some((path, hunks)) = current.as_mut() else {
            continue;
        };
        if in_hunks {
            hunks.push_str(line);
            continue;
        }
        if let Some(name) = line.strip_prefix("--- ") {
            old_path = header_path(name);
        } else if let Some(name) = line.strip_prefix("+++ ") {
            if let Some(new) = header_path(name).or_else(|| old_path.clone()) {
                *path = new;
            }
        } else if line.starts_with("@@") {
            in_hunks = true;
            hunks.push_str(line);
        }
    }
    if let Some((path, hunks)) = current {
        patches.insert(path, hunks);
    }
    patches
}

/// `a/x` or `b/x` from a `---` or `+++` line; `None` for `/dev/null`.
fn header_path(name: &str) -> Option<String> {
    let name = name.trim_end_matches(['\n', '\r']);
    let name = name.split('\t').next().unwrap_or(name);
    if name == "/dev/null" {
        return None;
    }
    Some(
        name.strip_prefix("a/")
            .or_else(|| name.strip_prefix("b/"))
            .unwrap_or(name)
            .to_owned(),
    )
}

/// The new path in `diff --git a/x b/y`, for files with no `+++` line (binaries).
fn git_header_path(line: &str) -> Option<String> {
    let rest = line.trim_end().strip_prefix("diff --git ")?;
    let at = rest.find(" b/")?;
    Some(rest[at + 3..].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "diff --git a/src/a.rs b/src/a.rs
index 1..2 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,2 +1,2 @@
-old
+new
 same
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
--- a/gone.txt
+++ /dev/null
@@ -1 +0,0 @@
-bye
diff --git a/logo.png b/logo.png
Binary files a/logo.png and b/logo.png differ
";

    #[test]
    fn splits_a_diff_by_file() {
        let patches = split(DIFF);
        assert_eq!(patches["src/a.rs"], "@@ -1,2 +1,2 @@\n-old\n+new\n same\n");
        assert_eq!(patches["gone.txt"], "@@ -1 +0,0 @@\n-bye\n");
        assert_eq!(patches["logo.png"], "");
        assert_eq!(patches.len(), 3);
    }
}
