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
