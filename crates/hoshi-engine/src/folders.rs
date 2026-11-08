//! Folders a project can live in: listed to choose one, created for a new one, or
//! filled by a clone.

use crate::git;
use hoshi_core::{CommandError, FolderListing};
use std::path::Path;

pub(crate) fn list(path: &str) -> Result<FolderListing, CommandError> {
    let unavailable = || CommandError::FolderUnavailable {
        path: path.to_owned(),
    };
    let dir = dunce::canonicalize(path).map_err(|_| unavailable())?;
    let mut folders: Vec<String> = std::fs::read_dir(&dir)
        .map_err(|_| unavailable())?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir() && !is_hidden(entry))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    folders.sort_by_key(|name| name.to_lowercase());
    Ok(FolderListing {
        path: text(&dir),
        parent: dir.parent().map(text),
        folders,
    })
}

/// Creates `path` and its parents. With `init_repository`, it must be new or empty, and
/// becomes a git repository. Returns the normalized path.
pub(crate) fn create(path: &str, init_repository: bool) -> Result<String, CommandError> {
    let unavailable = || CommandError::FolderUnavailable {
        path: path.to_owned(),
    };
    let target = Path::new(path.trim());
    if !target.is_absolute() {
        return Err(unavailable());
    }
    if init_repository && has_entries(target) {
        return Err(CommandError::FolderNotEmpty {
            path: path.to_owned(),
        });
    }
    std::fs::create_dir_all(target).map_err(|_| unavailable())?;
    let created = dunce::canonicalize(target).map_err(|_| unavailable())?;
    if init_repository {
        git::init(&text(&created)).map_err(|detail| CommandError::GitFailed { detail })?;
    }
    Ok(text(&created))
}

/// Clones `url` into `destination`, which must be new or empty. Returns the normalized path.
pub(crate) fn clone(url: &str, destination: &str) -> Result<String, CommandError> {
    let unavailable = || CommandError::FolderUnavailable {
        path: destination.to_owned(),
    };
    let target = Path::new(destination.trim());
    if !target.is_absolute() {
        return Err(unavailable());
    }
    if has_entries(target) {
        return Err(CommandError::FolderNotEmpty {
            path: destination.to_owned(),
        });
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|_| unavailable())?;
    }
    git::clone(url.trim(), target).map_err(|detail| CommandError::GitFailed { detail })?;
    dunce::canonicalize(target)
        .map(|path| text(&path))
        .map_err(|_| unavailable())
}

fn has_entries(path: &Path) -> bool {
    std::fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_some())
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(windows)]
fn is_hidden(entry: &std::fs::DirEntry) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    entry
        .metadata()
        .is_ok_and(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
}

#[cfg(not(windows))]
fn is_hidden(_entry: &std::fs::DirEntry) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("hoshi-folders-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dunce::canonicalize(dir).unwrap()
    }

    #[test]
    fn lists_only_folders_sorted_without_case() {
        let dir = temp("list");
        for name in ["beta", "Alpha", "gamma"] {
            std::fs::create_dir(dir.join(name)).unwrap();
        }
        std::fs::write(dir.join("file.txt"), "x").unwrap();

        let listing = list(&text(&dir)).unwrap();

        assert_eq!(listing.folders, ["Alpha", "beta", "gamma"]);
        assert_eq!(listing.path, text(&dir));
        assert_eq!(listing.parent, dir.parent().map(text));
        assert!(matches!(
            list(&text(&dir.join("missing"))),
            Err(CommandError::FolderUnavailable { .. })
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_project_gets_an_empty_repository() {
        let dir = temp("create");
        let project = dir.join("nested").join("app");

        let created = create(&text(&project), true).unwrap();

        assert!(git::is_repository(&created));
        std::fs::write(project.join("README.md"), "x").unwrap();
        assert!(matches!(
            create(&text(&project), true),
            Err(CommandError::FolderNotEmpty { .. })
        ));
        assert_eq!(create(&text(&project), false).unwrap(), created);
        assert!(matches!(
            create("relative/app", false),
            Err(CommandError::FolderUnavailable { .. })
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clones_into_a_new_folder_only() {
        let source = git::tests::temp_repository("clone-source");
        let dir = temp("clone");
        let destination = dir.join("copy");

        let cloned = clone(&text(&source), &text(&destination)).unwrap();

        assert_eq!(git::current_branch(&cloned).as_deref(), Some("main"));
        assert!(matches!(
            clone(&text(&source), &text(&destination)),
            Err(CommandError::FolderNotEmpty { .. })
        ));
        assert!(matches!(
            clone(&text(&dir.join("missing")), &text(&dir.join("other"))),
            Err(CommandError::GitFailed { .. })
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&source);
    }
}
