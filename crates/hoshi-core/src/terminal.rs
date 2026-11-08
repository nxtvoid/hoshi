use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What the UI gets when it opens or reattaches to a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TerminalAttach {
    /// Where the shell started; the thread may have moved to a worktree since.
    pub cwd: String,
    /// The latest output, bounded, to redraw the screen after a remount.
    pub history: String,
    /// Set when the shell has already exited.
    pub exit: Option<TerminalExit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TerminalExit {
    /// `None` when the platform did not report one.
    pub code: Option<u32>,
}

/// Streamed to the view a terminal is attached to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum TerminalEvent {
    Output { data: String },
    Exited { exit: TerminalExit },
}
