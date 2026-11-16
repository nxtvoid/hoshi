use serde::{Deserialize, Serialize};

/// What the UI gets when it opens or reattaches to a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalAttach {
    /// Where the shell started; the thread may have moved to a worktree since.
    pub cwd: String,
    /// The latest output, bounded, to redraw the screen after a remount.
    pub history: String,
    /// Set when the shell has already exited.
    pub exit: Option<TerminalExit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalExit {
    /// `None` when the platform did not report one.
    pub code: Option<u32>,
}

/// Streamed to the view a terminal is attached to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TerminalEvent {
    Output { data: String },
    Exited { exit: TerminalExit },
}
