use serde::Serialize;
use ts_rs::TS;

/// Whether the enabled provider's CLI can run turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ProviderAccountStatus {
    Ready,
    NotInstalled,
    SignedOut,
    /// The CLI didn't answer in time, or no provider is enabled.
    Unknown,
}
