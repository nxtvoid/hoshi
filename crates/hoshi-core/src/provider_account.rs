use serde::{Deserialize, Serialize};
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

/// Who is signed in to the provider, as its local login files say.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderAccount {
    pub email: Option<String>,
    /// `None` when the plan is unknown or can't be read.
    pub plan: Option<ProviderPlan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ProviderPlan {
    Pro,
    /// A Max plan whose tier isn't known.
    Max,
    Max5x,
    Max20x,
    Team,
    Enterprise,
}
