use serde::Serialize;

/// A published hoshi release and the notes it shipped with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRelease {
    /// Without the tag's `v`.
    pub version: String,
    pub published_at: String,
    /// Markdown.
    pub body: String,
    pub url: String,
    /// The installer this machine updates itself with; `None` where hoshi can't install itself.
    pub installer: Option<AppInstaller>,
}

/// A release's installer for this machine and its signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInstaller {
    pub name: String,
    pub url: String,
    pub signature_url: String,
    /// Bytes, as the release lists it.
    pub size: u64,
}
