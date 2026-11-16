use serde::Serialize;

/// A provider's CLI as it changed, by provider id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderVersionUpdate {
    pub provider: String,
    /// `None` once the provider is disabled.
    pub version: Option<ProviderVersion>,
}

/// An enabled provider's CLI against its latest release.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ProviderUpdate {
    #[default]
    Idle,
    Running {
        /// The updater's last line of output.
        line: Option<String>,
        /// Where an install hoshi runs itself is; `None` for an updater command.
        step: Option<UpdateStep>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
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
    /// The release couldn't be looked up or downloaded.
    DownloadFailed,
    /// What was downloaded isn't the release it claims to be, or doesn't start.
    VerifyFailed,
    /// The release couldn't be written to disk.
    WriteFailed,
}

/// Where an install hoshi runs itself is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum UpdateStep {
    Downloading {
        /// `None` while the size is unknown.
        percent: Option<u8>,
    },
    Extracting,
    Verifying,
}
