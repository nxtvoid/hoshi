//! Reads hoshi's published releases on GitHub: the newest one, and the notes of an update.

use hoshi_core::AppRelease;
use serde_json::Value;
use std::cmp::Reverse;
use std::time::Duration;

const RELEASES: &str = "https://api.github.com/repos/nxtvoid/hoshi/releases?per_page=30";
const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BYTES: u64 = 2 * 1024 * 1024;

type Version = (u64, u64, u64);

fn parse_version(text: &str) -> Option<Version> {
    let mut parts = text.split('.').map(|part| {
        (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| part.parse().ok())
            .flatten()
    });
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

fn published(feed: &Value) -> impl Iterator<Item = (Version, AppRelease)> + '_ {
    feed.as_array()
        .into_iter()
        .flatten()
        .filter(|release| !flag(release, "draft") && !flag(release, "prerelease"))
        .filter_map(|release| {
            let version = text(release, "tag_name")?.strip_prefix('v')?.to_owned();
            let parsed = parse_version(&version)?;
            let release = AppRelease {
                version,
                published_at: text(release, "published_at")?.to_owned(),
                body: text(release, "body").unwrap_or_default().to_owned(),
                url: text(release, "html_url").unwrap_or_default().to_owned(),
            };
            Some((parsed, release))
        })
}

/// The published releases after `installed` up to `available`, newest first.
fn releases_between(feed: &Value, installed: &str, available: &str) -> Vec<AppRelease> {
    let (Some(after), Some(until)) = (parse_version(installed), parse_version(available)) else {
        return Vec::new();
    };
    let mut releases: Vec<(Version, AppRelease)> = published(feed)
        .filter(|(version, _)| *version > after && *version <= until)
        .collect();
    releases.sort_by_key(|(version, _)| Reverse(*version));
    releases.into_iter().map(|(_, release)| release).collect()
}

/// The newest published release, when it is newer than `current`.
fn newest_after(feed: &Value, current: &str) -> Option<AppRelease> {
    let current = parse_version(current)?;
    published(feed)
        .filter(|(version, _)| *version > current)
        .max_by_key(|(version, _)| *version)
        .map(|(_, release)| release)
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn fetch_feed() -> Result<Value, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let mut response = agent
        .get(RELEASES)
        .header("User-Agent", "hoshi")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|error| error.to_string())?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_BYTES)
        .read_to_vec()
        .map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

/// The releases after `installed` up to `available`, newest first, with their notes.
pub fn release_notes(installed: &str, available: &str) -> Result<Vec<AppRelease>, String> {
    let feed = fetch_feed()?;
    Ok(releases_between(&feed, installed, available))
}

/// The newest published release, when it is newer than `current`.
pub fn latest_release(current: &str) -> Result<Option<AppRelease>, String> {
    let feed = fetch_feed()?;
    Ok(newest_after(&feed, current))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn release(tag: &str, published_at: &str) -> Value {
        json!({
            "tag_name": tag,
            "published_at": published_at,
            "body": format!("- Change in {tag} (#1)"),
            "html_url": format!("https://github.com/nxtvoid/hoshi/releases/tag/{tag}"),
            "draft": false,
            "prerelease": false,
        })
    }

    fn versions(releases: &[AppRelease]) -> Vec<&str> {
        releases
            .iter()
            .map(|release| release.version.as_str())
            .collect()
    }

    #[test]
    fn keeps_releases_after_installed_up_to_available_newest_first() {
        let feed = json!([
            release("v0.0.3", "2026-10-01T00:00:00Z"),
            release("v0.0.6", "2026-11-04T00:00:00Z"),
            release("v0.0.5", "2026-10-20T00:00:00Z"),
            release("v0.0.4", "2026-10-10T00:00:00Z"),
            release("v0.0.2", "2026-09-01T00:00:00Z"),
        ]);
        let releases = releases_between(&feed, "0.0.3", "0.0.5");
        assert_eq!(versions(&releases), ["0.0.5", "0.0.4"]);
        assert_eq!(releases[0].body, "- Change in v0.0.5 (#1)");
        assert_eq!(releases[0].published_at, "2026-10-20T00:00:00Z");
        assert_eq!(
            releases[0].url,
            "https://github.com/nxtvoid/hoshi/releases/tag/v0.0.5"
        );
    }

    #[test]
    fn compares_versions_as_numbers() {
        let feed = json!([
            release("v0.10.0", "2026-11-01T00:00:00Z"),
            release("v0.9.0", "2026-10-01T00:00:00Z")
        ]);
        assert_eq!(
            versions(&releases_between(&feed, "0.2.0", "0.10.0")),
            ["0.10.0", "0.9.0"]
        );
    }

    #[test]
    fn skips_drafts_prereleases_and_other_tags() {
        let mut draft = release("v0.0.5", "2026-11-01T00:00:00Z");
        draft["draft"] = json!(true);
        let mut prerelease = release("v0.0.6", "2026-11-02T00:00:00Z");
        prerelease["prerelease"] = json!(true);
        let feed = json!([
            draft,
            prerelease,
            release("v0.0.7-beta.1", "2026-11-03T00:00:00Z"),
            release("nightly", "2026-11-03T00:00:00Z"),
            release("v0.0.4", "2026-10-10T00:00:00Z"),
        ]);
        assert_eq!(
            versions(&releases_between(&feed, "0.0.3", "0.0.9")),
            ["0.0.4"]
        );
    }

    #[test]
    fn unparsable_versions_or_feed_give_nothing() {
        let feed = json!([release("v0.0.4", "2026-10-10T00:00:00Z")]);
        assert!(releases_between(&feed, "dev", "0.0.4").is_empty());
        assert!(releases_between(&feed, "0.0.3", "0.0.4.1").is_empty());
        assert!(
            releases_between(&json!({ "message": "rate limited" }), "0.0.3", "0.0.4").is_empty()
        );
    }

    #[test]
    fn finds_the_newest_release_after_current() {
        let mut prerelease = release("v0.1.0", "2026-11-05T00:00:00Z");
        prerelease["prerelease"] = json!(true);
        let feed = json!([
            release("v0.0.8", "2026-11-01T00:00:00Z"),
            prerelease,
            release("v0.0.10", "2026-11-04T00:00:00Z"),
            release("v0.0.9", "2026-11-02T00:00:00Z"),
        ]);
        let newest = newest_after(&feed, "0.0.9").expect("a newer release");
        assert_eq!(newest.version, "0.0.10");
        assert_eq!(
            newest.url,
            "https://github.com/nxtvoid/hoshi/releases/tag/v0.0.10"
        );
        assert_eq!(newest_after(&feed, "0.0.10"), None);
        assert_eq!(newest_after(&feed, "dev"), None);
    }

    #[test]
    fn parses_plain_versions_only() {
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3-rc.1"), None);
        assert_eq!(parse_version("1..3"), None);
        assert_eq!(parse_version("+1.2.3"), None);
    }
}
