use crate::ForgeKind;
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

/// A command run in every worktree hoshi makes for the project, such as `bun install`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorktreeSetup {
    pub command: String,
    /// The agent starts only once the command succeeded.
    pub wait: bool,
}

/// A worktree's setup command, running or finished, in its own terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorktreeSetupRun {
    pub thread_id: String,
    pub terminal_id: String,
    pub status: WorktreeSetupStatus,
    pub exit_code: Option<u32>,
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
