use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

/// How a search for theme extensions on Open VSX is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ThemeExtensionSort {
    DownloadCount,
    Rating,
    Timestamp,
    Relevance,
}

/// An open-source color theme extension on Open VSX that can be installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThemeExtension {
    /// `publisher.name`.
    pub id: String,
    /// Groups the themes it installs, so updating it replaces them.
    pub collection_id: String,
    pub name: String,
    pub publisher: String,
    pub description: String,
    pub download_count: u64,
    pub icon_url: Option<String>,
    pub source_url: Option<String>,
    pub manifest_url: String,
    pub sha256_url: String,
    pub vsix_url: String,
    pub version: String,
    pub license: String,
}

/// One color theme an extension contributes: its VS Code workbench colors, includes resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExtensionTheme {
    pub label: String,
    /// `light`, `dark`, `hc-black` or `hc-light`, when the extension says.
    pub ui_theme: Option<String>,
    /// Its file inside the package, which keeps its id stable across updates.
    pub source_path: String,
    pub colors: BTreeMap<String, String>,
}
