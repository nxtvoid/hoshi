//! Finds color themes on Open VSX and reads them out of their extension packages. Every size,
//! path and include is capped, and a package must match its published checksum.

use hoshi_core::{ExtensionTheme, ThemeExtension, ThemeExtensionSort};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Cursor, Read};
use std::time::Duration;

const API: &str = "https://open-vsx.org/api";
const TRUSTED_ORIGIN: &str = "https://open-vsx.org/";
const SEARCH_TIMEOUT: Duration = Duration::from_secs(10);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_SEARCH_BYTES: u64 = 512 * 1024;
const MAX_DETAIL_BYTES: u64 = 256 * 1024;
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
const MAX_VSIX_BYTES: u64 = 20 * 1024 * 1024;
const MAX_THEME_BYTES: u64 = 256 * 1024;
const MAX_ZIP_ENTRIES: usize = 5_000;
const MAX_UNCOMPRESSED_BYTES: u64 = 100 * 1024 * 1024;
const MAX_COMPRESSION_RATIO: u64 = 200;
const MAX_THEMES_PER_EXTENSION: usize = 40;
const MAX_INCLUDE_DEPTH: usize = 8;
const MAX_PACKAGE_PATH_LENGTH: usize = 1_024;
const MAX_COLOR_VALUE_LENGTH: usize = 128;
const SEARCH_SIZE: usize = 16;
const RESULTS: usize = 8;
const SUPPORTED_LICENSES: [&str; 9] = [
    "0BSD",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "CC0-1.0",
    "ISC",
    "MIT",
    "MPL-2.0",
    "Unlicense",
];
/// The workbench colors the UI maps to its own; the rest of a theme file is dropped.
const USED_WORKBENCH_COLORS: [&str; 46] = [
    "activityBar.background",
    "activityBarBadge.background",
    "badge.background",
    "button.background",
    "button.foreground",
    "contrastBorder",
    "descriptionForeground",
    "disabledForeground",
    "dropdown.background",
    "dropdown.border",
    "editor.background",
    "editor.foreground",
    "editor.findMatchBackground",
    "editor.findMatchForeground",
    "editor.findMatchHighlightBackground",
    "editor.findMatchHighlightForeground",
    "editor.selectionBackground",
    "editorCursor.foreground",
    "editorError.foreground",
    "editorGroup.border",
    "editorPane.background",
    "editorWarning.foreground",
    "editorWidget.background",
    "errorForeground",
    "focusBorder",
    "foreground",
    "input.border",
    "input.placeholderForeground",
    "list.activeSelectionBackground",
    "list.hoverBackground",
    "list.inactiveSelectionBackground",
    "menu.background",
    "panel.background",
    "panel.border",
    "progressBar.background",
    "quickInput.background",
    "scrollbarSlider.background",
    "sideBar.background",
    "sideBar.border",
    "sideBar.foreground",
    "terminal.background",
    "terminal.foreground",
    "terminal.selectionBackground",
    "terminalCursor.foreground",
    "textCodeBlock.background",
    "textLink.foreground",
];

/// Why a search or an install failed, as the code the UI words.
type Failure = &'static str;

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into()
}

fn fetch(agent: &ureq::Agent, url: &str, limit: u64, failure: Failure) -> Result<Vec<u8>, Failure> {
    let mut response = agent.get(url).call().map_err(|_| failure)?;
    response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|_| failure)
}

fn fetch_json(
    agent: &ureq::Agent,
    url: &str,
    limit: u64,
    failure: Failure,
) -> Result<Value, Failure> {
    let bytes = fetch(agent, url, limit, failure)?;
    parse_jsonc(&String::from_utf8_lossy(&bytes)).ok_or(failure)
}

fn trusted_url(value: &Value) -> Option<String> {
    let url = value.as_str()?;
    url.to_ascii_lowercase()
        .starts_with(TRUSTED_ORIGIN)
        .then(|| url.to_owned())
}

fn public_source_url(value: &Value) -> Option<String> {
    let url = value
        .as_str()
        .or_else(|| value.get("url").and_then(Value::as_str))?;
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split('/').next().unwrap_or_default();
    (!authority.is_empty() && !authority.contains('@')).then(|| url.to_owned())
}

fn short_hash(value: &str) -> String {
    Sha256::digest(value.as_bytes())[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn collection_id(extension_id: &str) -> String {
    let normalized = format!("open-vsx:{}", extension_id.to_lowercase());
    let valid = normalized.len() <= 128
        && normalized
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '-'));
    if valid {
        normalized
    } else {
        format!("open-vsx:{}", short_hash(extension_id))
    }
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_owned()
}

/// The extension a detail response describes, `None` when its license isn't one we import.
fn extension_from_detail(value: &Value) -> Result<Option<ThemeExtension>, Failure> {
    let files = value.get("files").ok_or("details_unreadable")?;
    let namespace = text(value, "namespace");
    let name = text(value, "name");
    let version = text(value, "version");
    let license = text(value, "license");
    let display_name = Some(text(value, "displayName"))
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| name.clone());
    let manifest_url = files.get("manifest").and_then(trusted_url);
    let sha256_url = files.get("sha256").and_then(trusted_url);
    let vsix_url = files.get("download").and_then(trusted_url);
    let (Some(manifest_url), Some(sha256_url), Some(vsix_url)) =
        (manifest_url, sha256_url, vsix_url)
    else {
        return Err("details_unreadable");
    };
    if namespace.is_empty() || name.is_empty() || version.is_empty() {
        return Err("details_unreadable");
    }
    if !SUPPORTED_LICENSES.contains(&license.as_str()) {
        return Ok(None);
    }
    let id = format!("{namespace}.{name}");
    Ok(Some(ThemeExtension {
        collection_id: collection_id(&id),
        id,
        name: display_name,
        publisher: namespace,
        description: value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        download_count: value
            .get("downloadCount")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        icon_url: files.get("icon").and_then(trusted_url),
        source_url: ["repository", "homepage", "url"]
            .iter()
            .find_map(|key| value.get(*key).and_then(public_source_url)),
        manifest_url,
        sha256_url,
        vsix_url,
        version,
        license,
    }))
}

fn theme_contributions(manifest: &Value) -> Vec<&Map<String, Value>> {
    manifest
        .get("contributes")
        .and_then(|contributes| contributes.get("themes"))
        .and_then(Value::as_array)
        .map(|themes| themes.iter().filter_map(Value::as_object).collect())
        .unwrap_or_default()
}

fn license_matches(manifest: &Value, license: &str) -> bool {
    manifest
        .get("license")
        .and_then(Value::as_str)
        .is_some_and(|value| value.trim().eq_ignore_ascii_case(license))
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn checked_extension(identity: (String, String)) -> Result<Option<ThemeExtension>, Failure> {
    let agent = agent(SEARCH_TIMEOUT);
    let (namespace, name) = identity;
    let detail = fetch_json(
        &agent,
        &format!("{API}/{namespace}/{name}"),
        MAX_DETAIL_BYTES,
        "details_unavailable",
    )?;
    let Some(extension) = extension_from_detail(&detail)? else {
        return Ok(None);
    };
    let manifest = fetch_json(
        &agent,
        &extension.manifest_url,
        MAX_MANIFEST_BYTES,
        "details_unavailable",
    )?;
    let Ok(package) = agent.head(&extension.vsix_url).call() else {
        return Ok(None);
    };
    let too_large = package
        .headers()
        .get("content-length")
        .and_then(|length| length.to_str().ok())
        .and_then(|length| length.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_VSIX_BYTES);
    let usable = !too_large
        && !theme_contributions(&manifest).is_empty()
        && license_matches(&manifest, &extension.license);
    Ok(usable.then_some(extension))
}

fn search(query: &str, sort_by: ThemeExtensionSort) -> Result<Vec<ThemeExtension>, Failure> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let sort = match sort_by {
        ThemeExtensionSort::DownloadCount => "downloadCount",
        ThemeExtensionSort::Rating => "rating",
        ThemeExtensionSort::Timestamp => "timestamp",
        ThemeExtensionSort::Relevance => "relevance",
    };
    let agent = agent(SEARCH_TIMEOUT);
    let mut response = agent
        .get(format!("{API}/-/search"))
        .query("query", query)
        .query("category", "Themes")
        .query("sortBy", sort)
        .query("sortOrder", "desc")
        .query("size", SEARCH_SIZE.to_string())
        .call()
        .map_err(|_| "search_unavailable")?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_SEARCH_BYTES)
        .read_to_vec()
        .map_err(|_| "search_unreadable")?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "search_unreadable")?;
    let identities: Vec<(String, String)> = value
        .get("extensions")
        .and_then(Value::as_array)
        .ok_or("search_unreadable")?
        .iter()
        .map(|candidate| (text(candidate, "namespace"), text(candidate, "name")))
        .filter(|(namespace, name)| is_identifier(namespace) && is_identifier(name))
        .take(SEARCH_SIZE)
        .collect();
    if identities.is_empty() {
        return Ok(Vec::new());
    }
    let details: Vec<Result<Option<ThemeExtension>, Failure>> = std::thread::scope(|scope| {
        let handles: Vec<_> = identities
            .into_iter()
            .map(|identity| scope.spawn(move || checked_extension(identity)))
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or(Err("details_unavailable")))
            .collect()
    });
    if details.iter().all(Result::is_err) {
        return Err("details_unavailable");
    }
    Ok(details
        .into_iter()
        .filter_map(|detail| detail.ok().flatten())
        .take(RESULTS)
        .collect())
}

/// A JSON document that may carry comments and trailing commas, as VS Code's files do.
fn parse_jsonc(source: &str) -> Option<Value> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut stripped = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            stripped.push(c);
            if c == '\\' {
                if let Some(escaped) = chars.next() {
                    stripped.push(escaped);
                }
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                stripped.push(c);
            }
            ('/', Some('/')) => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        stripped.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut previous = '\0';
                for next in chars.by_ref() {
                    if previous == '*' && next == '/' {
                        break;
                    }
                    previous = next;
                }
                stripped.push(' ');
            }
            _ => stripped.push(c),
        }
    }
    serde_json::from_str(&without_trailing_commas(&stripped)).ok()
}

fn without_trailing_commas(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut result = String::with_capacity(source.len());
    let mut in_string = false;
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if in_string {
            result.push(c);
            if c == '\\' {
                if let Some(&escaped) = chars.get(index + 1) {
                    result.push(escaped);
                    index += 1;
                }
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            result.push(c);
        } else if c == ',' {
            let next = chars[index + 1..].iter().find(|next| !next.is_whitespace());
            if !matches!(next, Some('}' | ']')) {
                result.push(c);
            }
        } else {
            result.push(c);
        }
        index += 1;
    }
    result
}

/// A package path relative to `relative_to`'s folder, refused when it would leave the package.
fn normalize_package_path(path: &str, relative_to: &str) -> Result<String, Failure> {
    let drive = path.len() >= 2 && path.as_bytes()[1] == b':';
    if path.len() > MAX_PACKAGE_PATH_LENGTH || path.contains('\0') || path.starts_with('/') || drive
    {
        return Err("unsafe_package");
    }
    let mut segments: Vec<&str> = relative_to.split('/').collect();
    segments.pop();
    let input = path.replace('\\', "/");
    for segment in input.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.len() <= 1 {
                    return Err("unsafe_package");
                }
                segments.pop();
            }
            _ => segments.push(segment),
        }
    }
    let mut normalized = segments.join("/");
    if segments.first() != Some(&"extension") {
        normalized = format!("extension/{normalized}");
    }
    Ok(normalized)
}

fn contribution_type(ui_theme: Option<&Value>) -> Option<String> {
    match ui_theme?.as_str()? {
        "vs" => Some("light".into()),
        "vs-dark" => Some("dark".into()),
        kind @ ("hc-black" | "hc-light") => Some(kind.into()),
        _ => None,
    }
}

struct Package {
    archive: zip::ZipArchive<Cursor<Vec<u8>>>,
}

impl Package {
    fn open(bytes: Vec<u8>) -> Result<Self, Failure> {
        let mut archive =
            zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "package_unreadable")?;
        if archive.len() > MAX_ZIP_ENTRIES {
            return Err("unsafe_package");
        }
        let mut total = 0u64;
        for index in 0..archive.len() {
            let entry = archive
                .by_index_raw(index)
                .map_err(|_| "package_unreadable")?;
            let (compressed, size) = (entry.compressed_size(), entry.size());
            total += size;
            if total > MAX_UNCOMPRESSED_BYTES
                || (size > 0 && (compressed == 0 || size / compressed > MAX_COMPRESSION_RATIO))
            {
                return Err("unsafe_package");
            }
            let name = entry.name().map_err(|_| "unsafe_package")?;
            normalize_package_path(&name, "extension/")?;
        }
        Ok(Self { archive })
    }

    fn read(&mut self, path: &str) -> Result<String, Failure> {
        let file = self.archive.by_name(path).map_err(|_| "theme_unreadable")?;
        if file.size() > MAX_THEME_BYTES {
            return Err("theme_unreadable");
        }
        let mut bytes = Vec::new();
        file.take(MAX_THEME_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "theme_unreadable")?;
        if bytes.len() as u64 > MAX_THEME_BYTES {
            return Err("theme_unreadable");
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn read_json(&mut self, path: &str) -> Result<Value, Failure> {
        let source = self.read(path)?;
        parse_jsonc(&source)
            .filter(Value::is_object)
            .ok_or("theme_unreadable")
    }
}

type Colors = BTreeMap<String, String>;

fn used_colors(theme: &Value) -> Colors {
    theme
        .get("colors")
        .and_then(Value::as_object)
        .map(|colors| {
            colors
                .iter()
                .filter(|(key, _)| USED_WORKBENCH_COLORS.contains(&key.as_str()))
                .filter_map(|(key, color)| {
                    let color = color.as_str()?;
                    (color.len() <= MAX_COLOR_VALUE_LENGTH).then(|| (key.clone(), color.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

struct ThemeReader<'a> {
    package: &'a mut Package,
    cache: HashMap<String, Colors>,
    files: usize,
}

impl ThemeReader<'_> {
    /// A theme file's colors over those of the files it includes.
    fn colors(&mut self, path: &str, ancestors: &mut Vec<String>) -> Result<Colors, Failure> {
        if ancestors.len() >= MAX_INCLUDE_DEPTH || ancestors.iter().any(|seen| seen == path) {
            return Err("theme_unreadable");
        }
        if let Some(cached) = self.cache.get(path) {
            return Ok(cached.clone());
        }
        self.files += 1;
        if self.files > MAX_THEMES_PER_EXTENSION * MAX_INCLUDE_DEPTH {
            return Err("too_many_themes");
        }
        let theme = self.package.read_json(path)?;
        let own = used_colors(&theme);
        let colors = match theme.get("include").and_then(Value::as_str) {
            Some(include) => {
                let include = normalize_package_path(include, path)?;
                ancestors.push(path.to_owned());
                let mut base = self.colors(&include, ancestors)?;
                ancestors.pop();
                base.extend(own);
                base
            }
            None => own,
        };
        self.cache.insert(path.to_owned(), colors.clone());
        Ok(colors)
    }
}

fn import(extension: &ThemeExtension) -> Result<Vec<ExtensionTheme>, Failure> {
    let lookup = agent(SEARCH_TIMEOUT);
    let manifest = fetch_json(
        &lookup,
        &extension.manifest_url,
        MAX_MANIFEST_BYTES,
        "no_manifest",
    )?;
    let advertised = theme_contributions(&manifest).len();
    if advertised == 0 {
        return Err("no_themes");
    }
    if advertised > MAX_THEMES_PER_EXTENSION {
        return Err("too_many_themes");
    }
    for url in [
        &extension.manifest_url,
        &extension.sha256_url,
        &extension.vsix_url,
    ] {
        if !url.to_ascii_lowercase().starts_with(TRUSTED_ORIGIN) {
            return Err("download_failed");
        }
    }
    let bytes = fetch(
        &agent(DOWNLOAD_TIMEOUT),
        &extension.vsix_url,
        MAX_VSIX_BYTES,
        "download_failed",
    )?;
    let checksum = fetch(&lookup, &extension.sha256_url, 256, "no_checksum")?;
    let expected = String::from_utf8_lossy(&checksum)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if expected.len() != 64 || !expected.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("invalid_checksum");
    }
    let actual: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if actual != expected {
        return Err("integrity_failed");
    }
    let mut package = Package::open(bytes)?;
    let packaged = package
        .read_json("extension/package.json")
        .map_err(|_| "package_unreadable")?;
    let publisher = text(&packaged, "publisher");
    let name = text(&packaged, "name");
    let matches = publisher.eq_ignore_ascii_case(&extension.publisher)
        && format!("{publisher}.{name}").eq_ignore_ascii_case(&extension.id)
        && packaged.get("version").and_then(Value::as_str) == Some(extension.version.as_str());
    if !matches {
        return Err("package_mismatch");
    }
    if !license_matches(&packaged, &extension.license) {
        return Err("license_mismatch");
    }
    let contributions: Vec<Map<String, Value>> = theme_contributions(&packaged)
        .into_iter()
        .cloned()
        .collect();
    if contributions.is_empty() {
        return Err("no_themes");
    }
    if contributions.len() > MAX_THEMES_PER_EXTENSION {
        return Err("too_many_themes");
    }
    let mut reader = ThemeReader {
        package: &mut package,
        cache: HashMap::new(),
        files: 0,
    };
    let mut themes = Vec::new();
    let mut seen = HashSet::new();
    for contribution in &contributions {
        let path = contribution
            .get("path")
            .and_then(Value::as_str)
            .ok_or("theme_unreadable")?;
        let path = normalize_package_path(path, "extension/")?;
        let colors = reader.colors(&path, &mut Vec::new())?;
        let label = contribution
            .get("label")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .unwrap_or(&extension.name)
            .to_owned();
        let source_path = if seen.insert(path.clone()) {
            path
        } else {
            format!("{path}#{}", themes.len())
        };
        themes.push(ExtensionTheme {
            label,
            ui_theme: contribution_type(contribution.get("uiTheme")),
            source_path,
            colors,
        });
    }
    Ok(themes)
}

#[tauri::command(async)]
pub fn search_theme_extensions(
    query: String,
    sort_by: ThemeExtensionSort,
) -> Result<Vec<ThemeExtension>, String> {
    search(&query, sort_by).map_err(str::to_owned)
}

#[tauri::command(async)]
pub fn import_theme_extension(extension: ThemeExtension) -> Result<Vec<ExtensionTheme>, String> {
    import(&extension).map_err(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn jsonc_drops_comments_and_trailing_commas() {
        let source = "\u{feff}{\n  // a comment\n  \"a\": \"x // not a comment\",\n  /* block */ \"b\": [1, 2,],\n}";

        assert_eq!(
            parse_jsonc(source),
            Some(json!({ "a": "x // not a comment", "b": [1, 2] }))
        );
    }

    #[test]
    fn package_paths_stay_inside_the_extension() {
        assert_eq!(
            normalize_package_path("./themes/dark.json", "extension/").as_deref(),
            Ok("extension/themes/dark.json")
        );
        assert_eq!(
            normalize_package_path("../base.json", "extension/themes/dark.json").as_deref(),
            Ok("extension/base.json")
        );
        assert!(normalize_package_path("../../etc/passwd", "extension/").is_err());
        assert!(normalize_package_path("/etc/passwd", "extension/").is_err());
        assert!(normalize_package_path("C:\\theme.json", "extension/").is_err());
    }

    #[test]
    fn only_the_colors_the_ui_uses_are_kept() {
        let theme = json!({
            "colors": {
                "editor.background": "#1e1e1e",
                "statusBar.background": "#007acc",
                "focusBorder": 7,
            }
        });

        assert_eq!(
            used_colors(&theme),
            BTreeMap::from([("editor.background".to_owned(), "#1e1e1e".to_owned())])
        );
    }

    #[test]
    fn details_from_elsewhere_or_with_other_licenses_are_refused() {
        let detail = |license: &str, download: &str| {
            json!({
                "namespace": "acme",
                "name": "night",
                "version": "1.0.0",
                "license": license,
                "files": {
                    "manifest": "https://open-vsx.org/api/acme/night/1.0.0/file/package.json",
                    "sha256": "https://open-vsx.org/api/acme/night/1.0.0/file/acme.night-1.0.0.sha256",
                    "download": download,
                },
                "repository": "https://user:secret@example.com/acme/night",
            })
        };
        let trusted = "https://open-vsx.org/api/acme/night/1.0.0/file/acme.night-1.0.0.vsix";

        let extension = extension_from_detail(&detail("MIT", trusted))
            .unwrap()
            .unwrap();
        assert_eq!(extension.collection_id, "open-vsx:acme.night");
        assert_eq!(extension.name, "night");
        assert_eq!(extension.source_url, None);
        assert_eq!(extension_from_detail(&detail("GPL-3.0", trusted)), Ok(None));
        assert!(extension_from_detail(&detail("MIT", "https://example.com/x.vsix")).is_err());
    }

    #[test]
    fn contribution_types_map_to_appearances() {
        assert_eq!(
            contribution_type(Some(&json!("vs"))).as_deref(),
            Some("light")
        );
        assert_eq!(
            contribution_type(Some(&json!("vs-dark"))).as_deref(),
            Some("dark")
        );
        assert_eq!(contribution_type(Some(&json!("other"))), None);
    }
}
