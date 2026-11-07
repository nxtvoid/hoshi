use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct AppSettings {
    pub providers: ProviderSettings,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderSettings {
    pub claude_code: ClaudeCodeSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct ClaudeCodeSettings {
    pub enabled: bool,
    pub binary_path: Option<String>,
}

impl Default for ClaudeCodeSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            binary_path: None,
        }
    }
}
