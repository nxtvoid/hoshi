use serde::Serialize;
use ts_rs::TS;

/// A published hoshi release and the notes it shipped with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppRelease {
    /// Without the tag's `v`.
    pub version: String,
    pub published_at: String,
    /// Markdown.
    pub body: String,
    pub url: String,
}
