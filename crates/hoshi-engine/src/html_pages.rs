//! The `html_preview` and `html_render` tools: a page the agent writes, loaded in a headless
//! browser for it to check, then kept and shown in its thread.

use crate::browser::html as capture;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub(crate) const PREVIEW_TOOL: &str = "html_preview";
pub(crate) const RENDER_TOOL: &str = "html_render";

/// The width of the transcript's column, which a page shows at.
pub(crate) const COLUMN_WIDTH: u32 = 768;
pub(crate) const MIN_HEIGHT: u64 = 80;
pub(crate) const MAX_HEIGHT: u64 = 2000;
pub(crate) const MIN_WIDTH: u64 = 240;
pub(crate) const MAX_WIDTH: u64 = 1600;
pub(crate) const MAX_HTML_BYTES: usize = 512_000;
const MAX_TITLE_CHARS: usize = 200;
const MIB: u64 = 1024 * 1024;
const MAX_FILE_BYTES: u64 = 25 * MIB;
const MAX_IMAGE_BYTES: u64 = 10 * MIB;
const MAX_PAGE_BYTES: u64 = 25 * MIB;
const MAX_IMAGE_PATH_CHARS: usize = 2048;
/// The widths a page is measured at, so the transcript fits it at any width.
const MEASURE_WIDTHS: [u32; 9] = [320, 375, 430, 520, 640, 768, 860, 1000, 1144];

const RENDERED: &str = "Shown to the user above your reply. Don't mention or describe the page; reply with only what it doesn't already say.";

pub(crate) const PAGE_RULES: &str = "Self-contained: inline <style> and <script>. Local images as absolute paths are inlined. Remote URLs load as-is.";

pub(crate) const LAYOUT_GUIDE: &str = "The frame is as wide as the reply (768px at most), on the thread's background. Leave html, body and the outer element without a background color, outer card, border, banner title or side padding; this wins over any rule that sets a page background. hoshi adds an 8px html gutter for rings and shadows, which you can override; use fluid widths within it. A box that needs its own background gets 16px padding and var(--radius) corners. Give charts fixed pixel heights. Let the content set the height: no vh units, nor height:100% on html or body.";

pub(crate) const THEME_GUIDE: &str = "Use the theme's CSS variables on :root, never fixed colors: --background, --foreground, --muted, --card, --popover, --secondary, --border, --input, --ring, --primary, --accent, --destructive, --warning, --success, --info, --code-background (most have a matching -foreground), --chart-1 to --chart-6, --radius, --font-sans, --font-mono. The base sheet sets the background, text and font.";

/// The variables a page's theme sets, light then dark: hoshi's default theme, until the
/// transcript sends the one on screen.
const THEME: [(&str, &str, &str); 34] = [
    ("--background", "oklch(1 0 0)", "oklch(0.145 0 0)"),
    ("--foreground", "oklch(0.145 0 0)", "oklch(0.985 0 0)"),
    ("--muted", "oklch(0.97 0 0)", "oklch(0.269 0 0)"),
    ("--muted-foreground", "oklch(0.556 0 0)", "oklch(0.708 0 0)"),
    ("--card", "oklch(1 0 0)", "oklch(0.205 0 0)"),
    ("--card-foreground", "oklch(0.145 0 0)", "oklch(0.985 0 0)"),
    ("--popover", "oklch(1 0 0)", "oklch(0.205 0 0)"),
    (
        "--popover-foreground",
        "oklch(0.145 0 0)",
        "oklch(0.985 0 0)",
    ),
    ("--secondary", "oklch(0.97 0 0)", "oklch(0.269 0 0)"),
    (
        "--secondary-foreground",
        "oklch(0.205 0 0)",
        "oklch(0.985 0 0)",
    ),
    ("--border", "oklch(0.922 0 0)", "oklch(1 0 0 / 10%)"),
    ("--input", "oklch(0.922 0 0)", "oklch(1 0 0 / 15%)"),
    ("--ring", "oklch(0.708 0 0)", "oklch(0.556 0 0)"),
    ("--primary", "oklch(0.205 0 0)", "oklch(0.922 0 0)"),
    (
        "--primary-foreground",
        "oklch(0.985 0 0)",
        "oklch(0.205 0 0)",
    ),
    ("--accent", "oklch(0.56 0.16 250)", "oklch(0.74 0.13 245)"),
    (
        "--accent-foreground",
        "oklch(0.985 0 0)",
        "oklch(0.145 0 0)",
    ),
    ("--destructive", "oklch(0.6 0.21 25)", "oklch(0.7 0.18 22)"),
    (
        "--destructive-foreground",
        "oklch(0.985 0 0)",
        "oklch(0.985 0 0)",
    ),
    ("--warning", "oklch(0.58 0.15 60)", "oklch(0.83 0.15 80)"),
    (
        "--warning-foreground",
        "oklch(0.985 0 0)",
        "oklch(0.145 0 0)",
    ),
    ("--success", "oklch(0.6 0.15 155)", "oklch(0.76 0.15 155)"),
    (
        "--success-foreground",
        "oklch(0.985 0 0)",
        "oklch(0.145 0 0)",
    ),
    ("--info", "oklch(0.62 0.16 240)", "oklch(0.75 0.13 235)"),
    ("--info-foreground", "oklch(0.985 0 0)", "oklch(0.145 0 0)"),
    ("--code-background", "oklch(0.97 0 0)", "oklch(0.269 0 0)"),
    ("--code-foreground", "oklch(0.145 0 0)", "oklch(0.985 0 0)"),
    ("--chart-1", "oklch(0.55 0.17 252)", "oklch(0.74 0.13 245)"),
    ("--chart-2", "oklch(0.6 0.15 155)", "oklch(0.76 0.15 155)"),
    ("--chart-3", "oklch(0.72 0.16 70)", "oklch(0.83 0.15 80)"),
    ("--chart-4", "oklch(0.55 0.2 300)", "oklch(0.72 0.16 300)"),
    ("--chart-5", "oklch(0.6 0.21 25)", "oklch(0.7 0.18 22)"),
    ("--chart-6", "#17a5af", "#64d1db"),
    ("--radius", "0.625rem", "0.625rem"),
];

const FONT_SANS: &str = "system-ui, -apple-system, \"Segoe UI\", Roboto, Ubuntu, sans-serif";
const FONT_MONO: &str =
    "\"Cascadia Mono\", \"SF Mono\", Menlo, Consolas, \"DejaVu Sans Mono\", monospace";

const BASE_CSS: &str = "html{box-sizing:border-box;padding:8px;background:var(--background);color:var(--foreground);font-family:var(--font-sans);font-size:14px;line-height:1.5;-webkit-font-smoothing:antialiased;-webkit-text-size-adjust:100%;scrollbar-width:none}html::-webkit-scrollbar{display:none}body{margin:0}code,kbd,pre,samp{font-family:var(--font-mono)}:where(:focus-visible){outline:2px solid var(--ring)}:focus-visible{outline-width:2px!important;outline-offset:-2px!important}";

/// Applies the theme in the address's `#hoshi-theme=` and the ones the transcript sends;
/// in a frame, reports the page's height and hands links to the app.
const BOOTSTRAP: &str = r##"(function(){var s=document.getElementById("hoshi-theme");if(!s)return;var b=BASE;function a(t){if(!t||typeof t!=="object"||!t.variables||typeof t.variables!=="object")return;var c=":root{color-scheme:"+(t.appearance==="light"?"light":"dark")+";";for(var k in t.variables){if(/^--[a-z0-9-]+$/.test(k))c+=k+":"+String(t.variables[k]).replace(/[;{}<>]/g,"")+";";}s.textContent=c+"}"+b;}try{var m=/[#&]hoshi-theme=([^&]*)/.exec(location.hash);if(m){a(JSON.parse(decodeURIComponent(m[1])));history.replaceState(history.state,"",location.pathname+location.search);}}catch(e){}if(window.parent===window)return;window.addEventListener("message",function(e){var d=e.data,p=d&&d.params;if(e.source===window.parent&&d&&d.jsonrpc==="2.0"&&d.method==="ui/notifications/host-context-changed"&&p&&p.styles)a({appearance:p.theme,variables:p.styles.variables});});var n=0;document.addEventListener("click",function(e){var l=e.isTrusted?e.composedPath().find(function(t){return t&&t.matches&&t.matches("a[href]");}):null,u;if(!l)return;try{u=new URL(l.getAttribute("href"),document.baseURI);}catch(x){return;}if(!/^https?:$/.test(u.protocol)||u.href.split("#")[0]===location.href.split("#")[0])return;e.preventDefault();window.parent.postMessage({jsonrpc:"2.0",id:"hoshi-link-"+(++n),method:"ui/open-link",params:{url:u.href}},"*");},true);var h,o,z=function(){var r=document.documentElement,v=Math.ceil(r.scrollHeight>r.clientHeight?r.scrollHeight:r.getBoundingClientRect().height);if(v===h)return;h=v;window.parent.postMessage({jsonrpc:"2.0",method:"ui/notifications/size-changed",params:{height:v}},"*");};if(window.ResizeObserver){o=new ResizeObserver(z);o.observe(document.documentElement);}document.addEventListener("DOMContentLoaded",function(){if(o&&document.body)o.observe(document.body);z();});window.addEventListener("load",z);})();"##;

/// Where pages are kept: a folder per thread, so they go with it.
#[derive(Clone)]
pub(crate) struct Pages {
    dir: PathBuf,
}

/// A thread's id as a folder name and the start of its pages' ids.
fn thread_key(thread_id: &str) -> String {
    thread_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|char| char.to_ascii_lowercase())
        .collect()
}

impl Pages {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn save(&self, thread_id: &str, html: &str) -> Result<String, String> {
        let key = thread_key(thread_id);
        let id = format!("{key}_{}", crate::delegation::new_id().replace('-', ""));
        let folder = self.dir.join(&key);
        std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
        std::fs::write(folder.join(format!("{id}.html")), html)
            .map_err(|error| error.to_string())?;
        Ok(id)
    }

    /// `None` for ids that could reach outside the folder.
    pub(crate) fn path(&self, id: &str) -> Option<PathBuf> {
        let (key, rest) = id.split_once('_')?;
        let valid =
            |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric());
        (valid(key) && valid(rest)).then(|| self.dir.join(key).join(format!("{id}.html")))
    }

    pub(crate) fn read(&self, id: &str) -> Option<Vec<u8>> {
        std::fs::read(self.path(id)?).ok()
    }

    pub(crate) fn remove_thread(&self, thread_id: &str) {
        let key = thread_key(thread_id);
        if !key.is_empty() {
            let _ = std::fs::remove_dir_all(self.dir.join(key));
        }
    }
}

fn text<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments[key]
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// The page the call passes, inline or as a file.
fn source(arguments: &Value, full_access: bool) -> Result<String, String> {
    let html = arguments["html"]
        .as_str()
        .filter(|html| !html.trim().is_empty());
    match (html, text(arguments, "path")) {
        (Some(html), None) if html.len() > MAX_HTML_BYTES => Err(format!(
            "`html` is {} bytes, past {MAX_HTML_BYTES}: write it to a file and pass `path`",
            html.len()
        )),
        (Some(html), None) => Ok(html.to_owned()),
        (None, Some(_)) if !full_access => Err(
            "pass `html` instead of `path`: only threads with full access may pass a path".into(),
        ),
        (None, Some(path)) => read_page_file(Path::new(path)),
        _ => Err("pass exactly one of `html` or `path`".into()),
    }
}

fn read_page_file(path: &Path) -> Result<String, String> {
    if !path.is_absolute() {
        return Err("`path` must be absolute".into());
    }
    let is_html = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "html" | "htm"));
    if !is_html {
        return Err("`path` must be an .html file".into());
    }
    let size = std::fs::metadata(path)
        .map_err(|error| format!("couldn't read {}: {error}", path.display()))?
        .len();
    if size > MAX_FILE_BYTES {
        return Err(format!(
            "{} is past {} MiB",
            path.display(),
            MAX_FILE_BYTES / MIB
        ));
    }
    let html = std::fs::read_to_string(path)
        .map_err(|error| format!("couldn't read {}: {error}", path.display()))?;
    let start = html.trim_start().to_ascii_lowercase();
    if !start.starts_with("<!doctype html") && !start.starts_with("<html") {
        return Err(format!("{} isn't an HTML document", path.display()));
    }
    Ok(html)
}

/// The page as it is loaded and kept: checked, with the theme's bootstrap and its local
/// images inlined. Also the local images that aren't there.
fn prepare(html: &str) -> Result<(String, Vec<String>), String> {
    if uses_viewport_height(html) {
        return Err("the page sizes something in vh units, which grow with the frame that fits them: use pixels or let the content set the height".into());
    }
    inline_images(&inject_bootstrap(html))
}

/// The CSS of `<style>` elements and `style` attributes.
fn style_texts(lower: &str) -> Vec<&str> {
    let mut texts = Vec::new();
    let mut from = 0;
    while let Some(start) = lower[from..].find("<style").map(|at| from + at) {
        let Some(open) = lower[start..].find('>').map(|at| start + at + 1) else {
            break;
        };
        let close = lower[open..]
            .find("</style")
            .map_or(lower.len(), |at| open + at);
        texts.push(&lower[open..close]);
        from = close;
    }
    let mut from = 0;
    while let Some(at) = lower[from..].find("style=").map(|at| from + at) {
        from = at + "style=".len();
        let attribute = at > 0 && lower.as_bytes()[at - 1].is_ascii_whitespace();
        let Some(quote) = lower[from..]
            .chars()
            .next()
            .filter(|c| *c == '"' || *c == '\'')
        else {
            continue;
        };
        if !attribute {
            continue;
        }
        let value = from + 1;
        let end = lower[value..]
            .find(quote)
            .map_or(lower.len(), |at| value + at);
        texts.push(&lower[value..end]);
        from = end;
    }
    texts
}

fn uses_viewport_height(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    style_texts(&lower).into_iter().any(has_vh)
}

/// A length such as `100vh`, `50svh` or `.5dvh`.
fn has_vh(css: &str) -> bool {
    let bytes = css.as_bytes();
    let word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-';
    let mut from = 0;
    while let Some(at) = css[from..].find("vh").map(|at| from + at) {
        from = at + 2;
        if bytes.get(at + 2).copied().is_some_and(word) {
            continue;
        }
        let mut start = at;
        if start > 0 && matches!(bytes[start - 1], b's' | b'l' | b'd') {
            start -= 1;
        }
        let mut digits = start;
        while digits > 0 && (bytes[digits - 1].is_ascii_digit() || bytes[digits - 1] == b'.') {
            digits -= 1;
        }
        let number = &css[digits..start];
        if !number.bytes().any(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let before = digits.checked_sub(1).map(|index| bytes[index]);
        if !before.is_some_and(|byte| word(byte) || matches!(byte, b'.' | b'+' | b'/')) {
            return true;
        }
    }
    false
}

/// Where the opening tag `<name ...>` ends.
fn open_tag_end(lower: &str, name: &str) -> Option<usize> {
    let tag = format!("<{name}");
    let mut from = 0;
    while let Some(at) = lower[from..].find(&tag).map(|at| from + at) {
        from = at + tag.len();
        let next = lower.as_bytes().get(from).copied();
        if next.is_some_and(|byte| byte == b'>' || byte.is_ascii_whitespace()) {
            return lower[from..].find('>').map(|end| from + end + 1);
        }
    }
    None
}

fn root_rule(dark: bool) -> String {
    let scheme = if dark { "dark" } else { "light" };
    let mut rule = format!(":root{{color-scheme:{scheme};");
    for (name, light, dark_value) in THEME {
        rule.push_str(&format!(
            "{name}:{};",
            if dark { dark_value } else { light }
        ));
    }
    rule.push_str(&format!(
        "--font-sans:{FONT_SANS};--font-mono:{FONT_MONO};}}"
    ));
    rule
}

fn bootstrap_markup(lower: &str) -> String {
    let head = &lower.as_bytes()[..lower.len().min(4096)];
    let has_charset = head.windows(7).any(|window| window == b"charset");
    let has_viewport = ["name=\"viewport\"", "name='viewport'", "name=viewport"]
        .iter()
        .any(|attribute| lower.contains(attribute));
    let css = format!(
        "{}@media (prefers-color-scheme: light){{{}}}{BASE_CSS}",
        root_rule(true),
        root_rule(false)
    );
    let script = BOOTSTRAP.replace("BASE", &serde_json::to_string(BASE_CSS).unwrap_or_default());
    format!(
        "{}{}<style id=\"hoshi-theme\">{css}</style><script>{script}</script>",
        if has_charset {
            ""
        } else {
            "<meta charset=\"utf-8\">"
        },
        if has_viewport {
            ""
        } else {
            "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"
        },
    )
}

/// The page with the theme's style and script at the start of its `<head>`, in standards
/// mode: in quirks mode `<html>` fills the frame, which then can't fit the content.
fn inject_bootstrap(html: &str) -> String {
    if !html
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("<!doctype")
    {
        return inject_bootstrap(&format!("<!doctype html>{html}"));
    }
    let lower = html.to_ascii_lowercase();
    let markup = bootstrap_markup(&lower);
    if let Some(at) = open_tag_end(&lower, "head") {
        return format!("{}{markup}{}", &html[..at], &html[at..]);
    }
    if let Some(at) = open_tag_end(&lower, "html") {
        return format!("{}<head>{markup}</head>{}", &html[..at], &html[at..]);
    }
    let trimmed = lower.trim_start();
    let at = lower.len() - trimmed.len() + trimmed.find('>').map_or(0, |end| end + 1);
    format!("{}<head>{markup}</head>{}", &html[..at], &html[at..])
}

const IMAGE_EXTENSIONS: [&str; 9] = [
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".avif", ".svg", ".bmp", ".ico",
];

/// An absolute path to an image on this machine, as the page names it.
fn local_image_path(candidate: &str) -> Option<&str> {
    let path = candidate.strip_prefix("file://").unwrap_or(candidate);
    let bytes = path.as_bytes();
    let posix = bytes.first() == Some(&b'/') && bytes.get(1) != Some(&b'/');
    let windows = bytes.len() > 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    let image = IMAGE_EXTENSIONS
        .iter()
        .any(|extension| path.to_ascii_lowercase().ends_with(extension));
    let plain = !path.contains(['\n', '\r', '<', '>', '"', '\'']);
    ((posix || windows) && image && plain && path.len() <= MAX_IMAGE_PATH_CHARS).then_some(path)
}

/// What each quote and `url(` of the page starts, as written.
fn image_references(html: &str) -> BTreeSet<&str> {
    let mut found = BTreeSet::new();
    let bytes = html.as_bytes();
    for (at, byte) in bytes.iter().enumerate() {
        let (start, end_char) = match byte {
            b'"' => (at + 1, '"'),
            b'\'' => (at + 1, '\''),
            b'(' if at >= 3 && bytes[at - 3..at].eq_ignore_ascii_case(b"url") => {
                if matches!(bytes.get(at + 1), Some(b'"' | b'\'')) {
                    continue;
                }
                (at + 1, ')')
            }
            _ => continue,
        };
        let mut limit = (start + MAX_IMAGE_PATH_CHARS + 16).min(html.len());
        while !html.is_char_boundary(limit) {
            limit -= 1;
        }
        let rest = &html[start..limit];
        let Some(end) = rest.find(end_char) else {
            continue;
        };
        let candidate = rest[..end].trim();
        if candidate.len() <= MAX_IMAGE_PATH_CHARS + "file://".len() {
            found.insert(candidate);
        }
    }
    found
}

/// The image type `bytes` start like.
fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.len() > 12
        && &bytes[4..8] == b"ftyp"
        && matches!(&bytes[8..12], b"avif" | b"avis")
    {
        Some("image/avif")
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp")
    } else if bytes.starts_with(&[0, 0, 1, 0]) {
        Some("image/x-icon")
    } else {
        let start = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_ascii_lowercase();
        start.contains("<svg").then_some("image/svg+xml")
    }
}

/// The page with its local images as data URLs, and those that aren't there or aren't
/// images.
fn inline_images(html: &str) -> Result<(String, Vec<String>), String> {
    let mut inlined = Vec::new();
    let mut missing = Vec::new();
    let mut total = html.len() as u64;
    for reference in image_references(html) {
        let Some(path) = local_image_path(reference) else {
            continue;
        };
        let size = match std::fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => metadata.len(),
            _ => {
                missing.push(path.to_owned());
                continue;
            }
        };
        if size > MAX_IMAGE_BYTES {
            return Err(format!("{path} is past {} MiB", MAX_IMAGE_BYTES / MIB));
        }
        total += size * 4 / 3;
        if total > MAX_PAGE_BYTES {
            return Err(format!(
                "with its images the page is past {} MiB",
                MAX_PAGE_BYTES / MIB
            ));
        }
        let Ok(bytes) = std::fs::read(path) else {
            missing.push(path.to_owned());
            continue;
        };
        let Some(mime_type) = image_type(&bytes) else {
            missing.push(path.to_owned());
            continue;
        };
        inlined.push((
            reference.to_owned(),
            format!("data:{mime_type};base64,{}", STANDARD.encode(&bytes)),
        ));
    }
    let mut page = html.to_owned();
    for (reference, url) in inlined {
        for (open, close) in [("\"", "\""), ("'", "'"), ("(", ")")] {
            page = page.replace(
                &format!("{open}{reference}{close}"),
                &format!("{open}{url}{close}"),
            );
        }
    }
    Ok((page, missing))
}

fn width(arguments: &Value) -> u32 {
    arguments["width"].as_u64().map_or(COLUMN_WIDTH, |width| {
        width.clamp(MIN_WIDTH, MAX_WIDTH) as u32
    })
}

/// `html_preview`: the page as the user would see it, for the agent to check.
pub(crate) fn preview(arguments: &Value, full_access: bool) -> Result<Value, String> {
    let (page, missing) = prepare(&source(arguments, full_access)?)?;
    let appearance = match arguments["appearance"].as_str() {
        Some("light") => "light",
        _ => "dark",
    };
    let width = width(arguments);
    let shot = capture::capture(&page, width, appearance)?;
    let mut result = json!({
        "width": width,
        "contentHeight": shot.content_height,
        "capturedHeight": shot.captured_height,
        "console": shot.console,
        "image": shot.image,
    });
    if !missing.is_empty() {
        result["missingImages"] = json!(missing);
    }
    Ok(result)
}

/// `html_render`: keeps the page and shows it in the thread of `caller`.
pub(crate) fn render(
    pages: &Pages,
    caller: &str,
    arguments: &Value,
    full_access: bool,
) -> Result<Value, String> {
    let title: String = text(arguments, "title")
        .ok_or("`title` is required")?
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect();
    let height = arguments["height"]
        .as_u64()
        .ok_or("`height` is required")?
        .clamp(MIN_HEIGHT, MAX_HEIGHT);
    let (page, missing) = prepare(&source(arguments, full_access)?)?;
    if !missing.is_empty() {
        return Err(format!(
            "these images aren't there or aren't images: {}",
            missing.join(", ")
        ));
    }
    let heights = capture::measure(&page, &MEASURE_WIDTHS)
        .inspect_err(|error| eprintln!("hoshi: kept a page without measuring it: {error}"))
        .ok();
    let id = pages.save(caller, &page)?;
    let mut reference = json!({ "id": id, "title": title, "height": height });
    if let Some(heights) = heights {
        reference["heights"] = json!(heights);
    }
    Ok(json!({ "htmlRender": reference, "message": RENDERED }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_heights_are_found_only_in_css() {
        assert!(uses_viewport_height(
            "<style>main{min-height:100vh}</style>"
        ));
        assert!(uses_viewport_height("<div style=\"height: 50dvh\"></div>"));
        assert!(uses_viewport_height(
            "<style>a{height:calc(.5svh + 1px)}</style>"
        ));
        assert!(!uses_viewport_height("<p>about 100vh of text</p>"));
        assert!(!uses_viewport_height(
            "<style>.vh{width:10vw}#a100vh{}</style>"
        ));
    }

    #[test]
    fn the_bootstrap_goes_first_in_the_head() {
        let page = inject_bootstrap("<!DOCTYPE html><html><HEAD><title>x</title></HEAD></html>");
        let head = page.find("<HEAD>").unwrap() + "<HEAD>".len();
        assert!(page[head..].starts_with("<meta charset=\"utf-8\"><meta name=\"viewport\""));
        assert!(page.contains("<style id=\"hoshi-theme\">"));

        let bare = inject_bootstrap("<p>hi</p>");
        assert!(bare.starts_with("<!doctype html><head>"));
        assert!(bare.ends_with("</head><p>hi</p>"));

        let header = inject_bootstrap("<html><header>a</header></html>");
        assert!(header.starts_with("<!doctype html><html><head>"));
    }

    #[test]
    fn local_images_are_inlined_and_missing_ones_named() {
        let dir = std::env::temp_dir().join(format!("hoshi-html-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let image = dir.join("dot.png");
        std::fs::write(&image, b"\x89PNG\r\n\x1a\nrest").unwrap();
        let path = image.to_string_lossy().replace('\\', "/");
        let gone = dir.join("gone.png").to_string_lossy().replace('\\', "/");
        let html = format!(
            "<img src=\"{path}\"><div style=\"background:url({path})\"></div><img src='{gone}'><img src=\"https://example.com/a.png\">"
        );
        let (page, missing) = inline_images(&html).unwrap();
        assert_eq!(page.matches("data:image/png;base64,").count(), 2);
        assert!(!page.contains(&path));
        assert!(page.contains("https://example.com/a.png"));
        assert_eq!(missing, [gone]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_path_needs_full_access_and_one_source() {
        let path = json!({ "path": "/tmp/page.html" });
        assert!(source(&path, false).unwrap_err().contains("full access"));
        assert!(
            source(&json!({}), true)
                .unwrap_err()
                .contains("exactly one")
        );
        assert!(source(&json!({ "html": "<p>a</p>", "path": "/a.html" }), true).is_err());
        assert_eq!(
            source(&json!({ "html": "<p>a</p>" }), false).unwrap(),
            "<p>a</p>"
        );
    }

    #[test]
    fn page_ids_stay_inside_their_folder() {
        let pages = Pages::new(PathBuf::from("pages"));
        assert_eq!(
            pages.path("abc_123"),
            Some(PathBuf::from("pages").join("abc").join("abc_123.html"))
        );
        assert_eq!(pages.path("../x_1"), None);
        assert_eq!(pages.path("abc"), None);
        assert_eq!(pages.path("a_b/c"), None);
        assert_eq!(thread_key("4f1c-9a:toolu_01"), "4f1c9atoolu01");
    }
}
