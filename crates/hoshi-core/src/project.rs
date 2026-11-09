use crate::{CommandError, ForgeKind};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Project {
    pub id: String,
    pub title: String,
    /// Absolute, normalized folder path.
    pub workspace_root: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub worktree_setup: Option<WorktreeSetup>,
}

/// How hoshi prepares every worktree it makes for the project: files the repository
/// doesn't track copied from its root, then a command such as `bun install`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorktreeSetup {
    /// Paths relative to the project root with `/` separators; `*` and `?` match within
    /// one segment.
    #[serde(default)]
    pub files: Vec<String>,
    pub command: String,
    /// The agent starts only once the setup succeeded.
    pub wait: bool,
}

/// A file to copy as the user wrote it, made relative with `/` separators; `None` when blank.
/// Absolute paths, `..` and the repository's own `.git` are refused.
pub fn setup_file_path(raw: &str) -> Result<Option<String>, CommandError> {
    let invalid = || CommandError::SetupFileInvalid {
        path: raw.trim().to_owned(),
    };
    let unified = raw.trim().replace('\\', "/");
    if unified.is_empty() {
        return Ok(None);
    }
    if unified.starts_with('/') {
        return Err(invalid());
    }
    let segments: Vec<&str> = unified
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect();
    let Some(first) = segments.first() else {
        return Err(invalid());
    };
    let drive = first.len() >= 2
        && first.as_bytes()[0].is_ascii_alphabetic()
        && first.as_bytes()[1] == b':';
    if drive || first.eq_ignore_ascii_case(".git") || segments.contains(&"..") {
        return Err(invalid());
    }
    Ok(Some(segments.join("/")))
}

/// The files to copy, each made relative by [`setup_file_path`], without blanks or repeats.
pub fn setup_file_paths<S: AsRef<str>>(raw: &[S]) -> Result<Vec<String>, CommandError> {
    let mut files: Vec<String> = Vec::new();
    for path in raw {
        if let Some(path) = setup_file_path(path.as_ref())?
            && !files.contains(&path)
        {
            files.push(path);
        }
    }
    Ok(files)
}

/// A worktree's setup, running or finished: its files copied, then its command in a
/// terminal of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorktreeSetupRun {
    pub thread_id: String,
    /// `None` until the command starts, and when there is none.
    pub terminal_id: Option<String>,
    pub status: WorktreeSetupStatus,
    pub exit_code: Option<u32>,
    /// Files copied into the worktree, counting each one inside a copied folder.
    pub copied_files: u32,
    /// Files to copy that matched nothing in the project root.
    pub missing_files: Vec<String>,
    /// Files that couldn't be copied, or that lead outside the root or the worktree.
    pub failed_files: Vec<String>,
    /// Its last lines of output, without terminal styling.
    pub output: Vec<String>,
    /// Messages sent meanwhile, held for the agent until the setup succeeds.
    pub held_messages: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum WorktreeSetupStatus {
    Running,
    Succeeded,
    Failed,
}

/// The folders inside one folder, for choosing where a project lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FolderListing {
    /// Absolute and normalized.
    pub path: String,
    /// `None` at a root.
    pub parent: Option<String>,
    /// Folder names, sorted without regard to case.
    pub folders: Vec<String>,
}

/// A repository on a forge, found to be cloned as a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoteRepository {
    pub forge: ForgeKind,
    /// `owner/name`.
    pub full_name: String,
    pub name: String,
    pub description: Option<String>,
    pub web_url: String,
    pub clone_url: String,
    pub private: bool,
}
