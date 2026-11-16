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

/// Who Codex is signed in as, as its `auth.json` says.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CodexAccount {
    /// `false` when it runs on an API key instead of a ChatGPT account.
    pub chatgpt: bool,
    pub email: Option<String>,
    /// `None` when the plan is unknown or can't be read.
    pub plan: Option<ChatGptPlan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ChatGptPlan {
    Free,
    Go,
    Plus,
    Pro,
    Team,
    Business,
    Enterprise,
    Edu,
}

/// A sign-in hoshi runs for a provider, through the provider's CLI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ProviderSignIn {
    #[default]
    Idle,
    /// The CLI is getting the sign-in page ready.
    Starting,
    /// The user signs in on `url`, in their browser.
    Waiting {
        url: String,
    },
    Succeeded,
    Failed {
        reason: ProviderSignInFailure,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ProviderSignInFailure {
    NotInstalled,
    SpawnFailed,
    /// The CLI ended or stopped answering before the sign-in finished.
    Ended,
    /// The user didn't finish signing in in time.
    TimedOut,
    Cancelled,
    /// The provider turned the sign-in down; `detail` is its own words.
    Rejected {
        detail: Option<String>,
    },
}
