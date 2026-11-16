use crate::Effort;
use serde::{Deserialize, Serialize};

/// The provider a thread runs on unless it names another.
pub const DEFAULT_PROVIDER: &str = "claudeCode";

pub fn default_provider() -> String {
    DEFAULT_PROVIDER.to_owned()
}

/// The models a provider offers, as far as its installed CLI supports them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModels {
    pub provider: String,
    /// Runs a thread that has no model of its own.
    pub default_model: Option<String>,
    pub models: Vec<ModelInfo>,
    /// Compacts idle threads whose prompt cache runs out, as the settings'
    /// [`IdleCompaction`](crate::IdleCompaction) says.
    pub compacts_idle_threads: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub slug: String,
    pub name: String,
    /// The name without its family prefix, for tight spaces.
    pub short_name: String,
    /// Superseded; still offered for threads that use it.
    pub legacy: bool,
    /// Released in the last few days.
    pub is_new: bool,
    /// Needs a newer CLI than the one installed.
    pub update_required: bool,
    pub efforts: Vec<Effort>,
    pub default_effort: Option<Effort>,
    pub fast_mode: bool,
    pub ultracode: bool,
}
