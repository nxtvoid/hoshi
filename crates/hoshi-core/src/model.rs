use crate::Effort;
use serde::Serialize;
use ts_rs::TS;

/// The models a provider offers, as far as its installed CLI supports them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderModels {
    pub provider: String,
    /// Runs a thread that has no model of its own.
    pub default_model: Option<String>,
    pub models: Vec<ModelInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
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
