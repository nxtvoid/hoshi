use crate::PullRequestState;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EntryKind {
    File,
    Directory,
}

/// A file or folder of a thread's working tree, offered when the user types `@`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkspaceEntry {
    /// Relative to the working tree, with `/` separators.
    pub path: String,
    pub kind: EntryKind,
}

/// A command or skill the provider runs when a prompt starts with `/name`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SlashCommand {
    pub name: String,
    /// As the provider writes it; not translated.
    pub description: String,
    pub argument_hint: Option<String>,
    pub aliases: Vec<String>,
    /// Defined by the user, the project or a plugin rather than built into the CLI;
    /// these can also be mentioned anywhere in a prompt as `$name`.
    pub custom: bool,
}

/// Something sent along with a message's text: an image, or what a `#123` or
/// `$name` mention in the text stands for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum MessageAttachment {
    /// Saved by the engine; `id` names its file.
    Image {
        id: String,
        name: String,
        mime_type: String,
        size: u64,
    },
    PullRequest {
        number: u32,
        title: String,
        url: String,
        head_branch: String,
        base_branch: String,
        state: PullRequestState,
    },
    /// A provider command or skill mentioned as `$name`.
    Skill { name: String },
}
