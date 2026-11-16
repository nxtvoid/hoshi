//! Typed folder paths: what the add-project palette browses, completes and creates.

fn is_separator(char: char, sep: char) -> bool {
    char == sep || (sep == '\\' && char == '/')
}

pub fn is_absolute(path: &str, sep: char) -> bool {
    if sep == '\\' {
        let bytes = path.as_bytes();
        let drive = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/');
        drive || path.starts_with("\\\\")
    } else {
        path.starts_with('/')
    }
}

pub fn ends_with_separator(path: &str, sep: char) -> bool {
    path.chars()
        .last()
        .is_some_and(|last| is_separator(last, sep))
}

/// The folder being looked into, up to its last separator, and the name typed after it.
pub fn split_query(query: &str, sep: char) -> (&str, &str) {
    match query
        .char_indices()
        .rfind(|(_, char)| is_separator(*char, sep))
    {
        Some((index, char)) => query.split_at(index + char.len_utf8()),
        None => ("", query),
    }
}

pub fn join(directory: &str, name: &str, sep: char) -> String {
    if directory.is_empty() {
        name.to_owned()
    } else if ends_with_separator(directory, sep) {
        format!("{directory}{name}")
    } else {
        format!("{directory}{sep}{name}")
    }
}

pub fn with_trailing_separator(path: &str, sep: char) -> String {
    if ends_with_separator(path, sep) {
        path.to_owned()
    } else {
        format!("{path}{sep}")
    }
}

/// The path a typed query names: its folder when it ends in a separator.
pub fn resolve_query(query: &str, sep: char) -> String {
    let trimmed = query.trim();
    if !ends_with_separator(trimmed, sep) {
        return trimmed.to_owned();
    }
    let stripped = &trimmed[..trimmed.len() - 1];
    let drive = stripped.len() == 2
        && stripped.as_bytes()[0].is_ascii_alphabetic()
        && stripped.as_bytes()[1] == b':';
    if drive || stripped.is_empty() {
        trimmed.to_owned()
    } else {
        stripped.to_owned()
    }
}

/// `~` at the start stands for the home folder.
pub fn expand_home(query: &str, home: Option<&str>, sep: char) -> String {
    let Some(home) = home.filter(|_| query.starts_with('~')) else {
        return query.to_owned();
    };
    if query == "~" {
        return with_trailing_separator(home, sep);
    }
    match query[1..].chars().next() {
        Some(next) if is_separator(next, sep) => join(home, &query[2..], sep),
        _ => query.to_owned(),
    }
}

/// Folders whose name starts with `name` first, then those that contain it. Hidden ones
/// (a leading dot) only once the name starts with a dot.
pub fn filter_folders<'a>(folders: &'a [String], name: &str) -> Vec<&'a str> {
    let needle = name.to_lowercase();
    let visible = folders
        .iter()
        .filter(|folder| needle.starts_with('.') || !folder.starts_with('.'))
        .map(String::as_str);
    if needle.is_empty() {
        return visible.collect();
    }
    let (mut starts, mut contains) = (Vec::new(), Vec::new());
    for folder in visible {
        let lower = folder.to_lowercase();
        if lower.starts_with(&needle) {
            starts.push(folder);
        } else if lower.contains(&needle) {
            contains.push(folder);
        }
    }
    starts.append(&mut contains);
    starts
}

/// What Tab completes `name` to.
#[derive(Debug, PartialEq, Eq)]
pub struct Completion {
    /// What every folder starting with `name` has in common.
    pub name: String,
    /// Only one folder matched, so the completion is its whole name.
    pub complete: bool,
}

pub fn complete_name(folders: &[&str], name: &str) -> Option<Completion> {
    let needle = name.to_lowercase();
    let matches: Vec<&str> = folders
        .iter()
        .copied()
        .filter(|folder| folder.to_lowercase().starts_with(&needle))
        .collect();
    let first = *matches.first()?;
    let first_chars: Vec<char> = first.chars().collect();
    let mut length = first_chars.len();
    for other in &matches {
        let shared = first_chars
            .iter()
            .zip(other.chars())
            .take(length)
            .take_while(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
            .count();
        length = shared;
    }
    Some(Completion {
        name: first_chars[..length].iter().collect(),
        complete: matches.len() == 1,
    })
}

/// The folder a clone of `url` gets by default, as git names it. Empty when there is none.
pub fn repository_folder_name(url: &str) -> String {
    let path = url.trim();
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let path = path.trim_end_matches(['\\', '/']);
    let last = path.rsplit(['\\', '/', ':']).next().unwrap_or("");
    match last.len().checked_sub(4) {
        Some(at) if last.is_char_boundary(at) && last[at..].eq_ignore_ascii_case(".git") => {
            last[..at].to_owned()
        }
        _ => last.to_owned(),
    }
}

pub fn is_valid_folder_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty()
        && trimmed != "."
        && trimmed != ".."
        && !trimmed.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|'])
}

#[cfg(test)]
mod tests {
    use super::{
        Completion, complete_name, ends_with_separator, expand_home, filter_folders, is_absolute,
        is_valid_folder_name, join, repository_folder_name, resolve_query, split_query,
        with_trailing_separator,
    };

    #[test]
    fn absolute_paths_by_platform() {
        assert!(is_absolute("C:\\code", '\\'));
        assert!(is_absolute("c:/code", '\\'));
        assert!(is_absolute("\\\\server\\share", '\\'));
        assert!(!is_absolute("code\\app", '\\'));
        assert!(!is_absolute("C:", '\\'));
        assert!(is_absolute("/home", '/'));
        assert!(!is_absolute("home", '/'));
    }

    #[test]
    fn a_query_splits_at_its_last_separator() {
        assert_eq!(split_query("C:\\code\\ho", '\\'), ("C:\\code\\", "ho"));
        assert_eq!(split_query("C:/code/", '\\'), ("C:/code/", ""));
        assert_eq!(split_query("/a/b", '/'), ("/a/", "b"));
        assert_eq!(split_query("name", '/'), ("", "name"));
        assert!(ends_with_separator("C:\\code/", '\\'));
        assert!(!ends_with_separator("/a/b\\", '/'));
    }

    #[test]
    fn paths_join_with_one_separator() {
        assert_eq!(join("/a/", "b", '/'), "/a/b");
        assert_eq!(join("/a", "b", '/'), "/a/b");
        assert_eq!(join("", "b", '/'), "b");
        assert_eq!(with_trailing_separator("C:\\a", '\\'), "C:\\a\\");
        assert_eq!(with_trailing_separator("C:\\a/", '\\'), "C:\\a/");
    }

    #[test]
    fn a_trailing_separator_names_its_folder_except_at_a_root() {
        assert_eq!(resolve_query(" /a/b/ ", '/'), "/a/b");
        assert_eq!(resolve_query("/", '/'), "/");
        assert_eq!(resolve_query("C:\\", '\\'), "C:\\");
        assert_eq!(resolve_query("C:\\code\\", '\\'), "C:\\code");
    }

    #[test]
    fn a_tilde_stands_for_home() {
        let home = Some("/home/nxtvoid");
        assert_eq!(expand_home("~", home, '/'), "/home/nxtvoid/");
        assert_eq!(expand_home("~/code", home, '/'), "/home/nxtvoid/code");
        assert_eq!(expand_home("~code", home, '/'), "~code");
        assert_eq!(expand_home("~/code", None, '/'), "~/code");
        assert_eq!(expand_home("/x", home, '/'), "/x");
    }

    #[test]
    fn folders_starting_with_the_name_come_first_and_hidden_ones_wait_for_a_dot() {
        let folders: Vec<String> = [".git", "apps", "hoshi", "photos", "shop"]
            .map(String::from)
            .into();
        assert_eq!(
            filter_folders(&folders, ""),
            ["apps", "hoshi", "photos", "shop"]
        );
        assert_eq!(filter_folders(&folders, "HO"), ["hoshi", "photos", "shop"]);
        assert_eq!(filter_folders(&folders, ".g"), [".git"]);
    }

    #[test]
    fn tab_completes_the_shared_start() {
        let folders = ["Hoshi", "hoshi-web", "apps"];
        assert_eq!(
            complete_name(&folders, "ho"),
            Some(Completion {
                name: "Hoshi".into(),
                complete: false
            })
        );
        assert_eq!(
            complete_name(&folders, "a"),
            Some(Completion {
                name: "apps".into(),
                complete: true
            })
        );
        assert_eq!(complete_name(&folders, "x"), None);
    }

    #[test]
    fn a_clone_is_named_after_its_repository() {
        assert_eq!(
            repository_folder_name("https://github.com/nxtvoid/hoshi.git"),
            "hoshi"
        );
        assert_eq!(
            repository_folder_name("git@github.com:nxtvoid/hoshi.GIT"),
            "hoshi"
        );
        assert_eq!(
            repository_folder_name("https://example.com/a/repo/?tab=1"),
            "repo"
        );
        assert_eq!(repository_folder_name(" "), "");
    }

    #[test]
    fn folder_names_reject_separators_and_dots() {
        assert!(is_valid_folder_name(" app "));
        assert!(!is_valid_folder_name(".."));
        assert!(!is_valid_folder_name("a/b"));
        assert!(!is_valid_folder_name("what?"));
        assert!(!is_valid_folder_name("  "));
    }
}
