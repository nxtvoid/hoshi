use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use std::path::{Path, PathBuf};

const MAX_ICON_BYTES: u64 = 512 * 1024;

/// Best first: vector and full-size favicons before touch icons and small sizes.
const CANDIDATES: &[&str] = &[
    "favicon.svg",
    "favicon.ico",
    "favicon.png",
    "public/favicon.svg",
    "public/favicon.ico",
    "public/favicon.png",
    "static/favicon.svg",
    "static/favicon.ico",
    "static/favicon.png",
    "app/icon.svg",
    "app/favicon.ico",
    "app/icon.png",
    "src/app/icon.svg",
    "src/app/favicon.ico",
    "src/app/icon.png",
    "src/favicon.svg",
    "src/favicon.ico",
    "public/icon.svg",
    "public/icon.png",
    "app/apple-icon.png",
    "src/app/apple-icon.png",
    "public/apple-touch-icon.png",
    "public/favicon-32x32.png",
    "assets/icon.svg",
    "assets/icon.png",
    "assets/logo.svg",
    "assets/logo.png",
    "src-tauri/icons/icon.png",
    "src-tauri/icons/128x128.png",
    "src-tauri/icons/32x32.png",
    ".idea/icon.svg",
];

const HTML_SOURCES: &[&str] = &[
    "index.html",
    "public/index.html",
    "src/index.html",
    "app/root.tsx",
    "src/root.tsx",
    "app/routes/__root.tsx",
    "src/routes/__root.tsx",
    "src/app.html",
];

const WORKSPACE_FOLDERS: &[&str] = &["apps"];
const PREFERRED_APPS: &[&str] = &["web", "www", "site", "app", "frontend", "client", "desktop"];

/// The icon of the project at `root`, as a data URL an `<img>` can show.
pub fn data_url(root: &Path) -> Option<String> {
    let path = find(root)?;
    let mime = mime(&path)?;
    let bytes = std::fs::read(&path).ok()?;
    Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

/// Looks in the project folder first, then in each app of a monorepo.
pub fn find(root: &Path) -> Option<PathBuf> {
    find_in(root).or_else(|| app_dirs(root).iter().find_map(|dir| find_in(dir)))
}

fn find_in(dir: &Path) -> Option<PathBuf> {
    CANDIDATES
        .iter()
        .map(|candidate| dir.join(candidate))
        .find(|path| usable(path))
        .or_else(|| {
            HTML_SOURCES
                .iter()
                .find_map(|source| linked_icon(dir, source))
        })
}

fn app_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = WORKSPACE_FOLDERS
        .iter()
        .filter_map(|folder| std::fs::read_dir(root.join(folder)).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect();
    dirs.sort_by_key(|dir| {
        let name = dir
            .file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let rank = PREFERRED_APPS
            .iter()
            .position(|preferred| *preferred == name)
            .unwrap_or(PREFERRED_APPS.len());
        (rank, name)
    });
    dirs
}

fn usable(path: &Path) -> bool {
    mime(path).is_some()
        && std::fs::metadata(path)
            .is_ok_and(|meta| meta.is_file() && meta.len() > 0 && meta.len() <= MAX_ICON_BYTES)
}

fn mime(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => return None,
    })
}

fn linked_icon(dir: &Path, source: &str) -> Option<PathBuf> {
    let text = std::fs::read_to_string(dir.join(source)).ok()?;
    let href = icon_href(&text)?;
    let relative = clean_href(&href)?;
    ["public", "static", ""]
        .iter()
        .map(|base| dir.join(base).join(relative))
        .find(|path| usable(path))
}

/// The `href` of the first icon link, in HTML (`<link rel="icon" …>`) or in a
/// JS head config (`{ rel: 'icon', href: '…' }`).
fn icon_href(text: &str) -> Option<String> {
    for tag in text.split("<link").skip(1) {
        let tag = tag.split('>').next().unwrap_or_default();
        if attribute(tag, "rel", '=').is_some_and(|rel| is_icon_rel(&rel))
            && let Some(href) = attribute(tag, "href", '=')
        {
            return Some(href);
        }
    }
    text.split('}').find_map(|object| {
        attribute(object, "rel", ':')
            .filter(|rel| is_icon_rel(rel))
            .and_then(|_| attribute(object, "href", ':'))
    })
}

fn is_icon_rel(rel: &str) -> bool {
    rel.split_whitespace()
        .any(|token| token.eq_ignore_ascii_case("icon"))
}

/// The quoted value after `name` and `separator`, e.g. `rel="icon"` or `rel: 'icon'`.
fn attribute(text: &str, name: &str, separator: char) -> Option<String> {
    let mut rest = text;
    while let Some(index) = rest.find(name) {
        let before = rest[..index].chars().next_back();
        let after = rest[index + name.len()..].trim_start();
        rest = &rest[index + name.len()..];
        if before.is_some_and(|char| char.is_alphanumeric() || char == '-' || char == '_') {
            continue;
        }
        let Some(value) = after.strip_prefix(separator) else {
            continue;
        };
        let value = value.trim_start();
        let quote = value
            .chars()
            .next()
            .filter(|char| matches!(char, '"' | '\''))?;
        let value = &value[1..];
        return value.find(quote).map(|end| value[..end].to_owned());
    }
    None
}

fn clean_href(href: &str) -> Option<&str> {
    let href = href.split(['?', '#']).next()?.trim();
    if href.is_empty() || href.contains("://") || href.starts_with("data:") || href.contains("..") {
        return None;
    }
    let href = href.strip_prefix("%PUBLIC_URL%").unwrap_or(href);
    Some(href.trim_start_matches("./").trim_start_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_project(name: &str, files: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hoshi-icon-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for file in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "icon").unwrap();
        }
        dir
    }

    fn found(dir: &Path) -> Option<String> {
        find(dir).map(|path| {
            path.strip_prefix(dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
    }

    #[test]
    fn prefers_root_favicon_over_touch_icon() {
        let dir = temp_project(
            "root",
            &["public/apple-touch-icon.png", "src/app/favicon.ico"],
        );
        assert_eq!(found(&dir).as_deref(), Some("src/app/favicon.ico"));
    }

    #[test]
    fn looks_inside_monorepo_apps_with_web_first() {
        let dir = temp_project(
            "monorepo",
            &["apps/docs/public/favicon.ico", "apps/web/src/app/icon.svg"],
        );
        assert_eq!(found(&dir).as_deref(), Some("apps/web/src/app/icon.svg"));
    }

    #[test]
    fn finds_tauri_icons_and_skips_icns() {
        let dir = temp_project(
            "tauri",
            &["AppIcon.icns", "apps/desktop/src-tauri/icons/icon.png"],
        );
        assert_eq!(
            found(&dir).as_deref(),
            Some("apps/desktop/src-tauri/icons/icon.png")
        );
    }

    #[test]
    fn follows_the_icon_link_of_index_html() {
        let dir = temp_project("html", &["public/brand/mark.webp"]);
        std::fs::write(
            dir.join("index.html"),
            r#"<head><link href="/brand/mark.webp?v=2" rel="shortcut icon"></head>"#,
        )
        .unwrap();
        assert_eq!(found(&dir).as_deref(), Some("public/brand/mark.webp"));
    }

    #[test]
    fn reads_icon_links_from_html_and_js_configs() {
        assert_eq!(
            icon_href(r#"<link rel="stylesheet" href="a.css"><link rel='icon' href='/f.svg'>"#),
            Some("/f.svg".into())
        );
        assert_eq!(
            icon_href(
                "links: [{ rel: 'preload', href: '/x' }, { rel: \"icon\", href: \"/i.png\" }]"
            ),
            Some("/i.png".into())
        );
        assert_eq!(
            icon_href(r#"<link rel="apple-touch-icon" href="/t.png">"#),
            None
        );
        assert_eq!(clean_href("%PUBLIC_URL%/favicon.ico"), Some("favicon.ico"));
        assert_eq!(clean_href("https://cdn/x.png"), None);
    }

    #[test]
    fn nothing_when_no_icon() {
        let dir = temp_project("empty", &["README.md"]);
        assert_eq!(found(&dir), None);
    }
}
