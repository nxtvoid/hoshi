use serde::Serialize;
use ts_rs::TS;

/// The enabled provider's CLI against its latest release.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderVersion {
    /// `None` when the CLI can't be run.
    pub installed: Option<String>,
    /// `None` when the latest release can't be looked up.
    pub latest: Option<String>,
    pub update_available: bool,
    /// What updating runs, as typed in a terminal. `None` when hoshi can't update it, as
    /// with a CLI a system package manager installed.
    pub update_command: Option<String>,
    pub update: ProviderUpdate,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ProviderUpdate {
    #[default]
    Idle,
    Running {
        /// The updater's last line of output.
        line: Option<String>,
    },
    Succeeded,
    /// The updater finished, yet the CLI still reports an older version.
    Unchanged,
    Failed {
        reason: ProviderUpdateFailure,
        /// The updater's last lines of output.
        output: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ProviderUpdateFailure {
    /// Turns are running on the CLI it would replace.
    TurnsRunning,
    /// hoshi doesn't know how this CLI was installed.
    Unsupported,
    SpawnFailed,
    /// `code` is `None` when a signal ended it.
    Exited {
        code: Option<i32>,
    },
    TimedOut,
}
