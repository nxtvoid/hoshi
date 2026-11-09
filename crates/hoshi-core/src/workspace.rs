use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where hoshi can open a thread's folder: a code editor, or the system's file manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Editor {
    Vscode,
    Cursor,
    Windsurf,
    Zed,
    Idea,
    Webstorm,
    Pycharm,
    Goland,
    Rider,
    Clion,
    Phpstorm,
    Rubymine,
    Rustrover,
    /// File Explorer, Finder or the desktop's file manager; always there.
    FileManager,
}

impl Editor {
    /// The code editors, which are only offered when installed.
    pub const CODE_EDITORS: [Self; 13] = [
        Self::Vscode,
        Self::Cursor,
        Self::Windsurf,
        Self::Zed,
        Self::Idea,
        Self::Webstorm,
        Self::Pycharm,
        Self::Goland,
        Self::Rider,
        Self::Clion,
        Self::Phpstorm,
        Self::Rubymine,
        Self::Rustrover,
    ];

    /// The command a code editor installs on the `PATH`; `None` for the file manager.
    pub fn command(self) -> Option<&'static str> {
        match self {
            Self::Vscode => Some("code"),
            Self::Cursor => Some("cursor"),
            Self::Windsurf => Some("windsurf"),
            Self::Zed => Some("zed"),
            Self::Idea => Some("idea"),
            Self::Webstorm => Some("webstorm"),
            Self::Pycharm => Some("pycharm"),
            Self::Goland => Some("goland"),
            Self::Rider => Some("rider"),
            Self::Clion => Some("clion"),
            Self::Phpstorm => Some("phpstorm"),
            Self::Rubymine => Some("rubymine"),
            Self::Rustrover => Some("rustrover"),
            Self::FileManager => None,
        }
    }
}

/// What a thread's working tree changed and has not committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangesSummary {
    pub files: u32,
    pub additions: u32,
    pub deletions: u32,
}
