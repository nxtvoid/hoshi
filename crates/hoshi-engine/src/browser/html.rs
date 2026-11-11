//! Pages agents write, loaded in a headless browser of their own: a screenshot to check
//! them, and their height at each width the transcript may show them at.

use super::{Browser, Page, find_browser};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

const VIEWPORT_HEIGHT: u32 = 800;
/// A taller page is still measured whole, but its screenshot stops here.
const MAX_CAPTURE_HEIGHT: u32 = 4000;
const CONSOLE_LINES: usize = 20;
const CONSOLE_LINE_CHARS: usize = 500;

/// Waits for the page's load, its fonts and two frames painted; at most 10 seconds.
const SETTLE: &str = "new Promise((done) => { setTimeout(done, 10000); const painted = () => requestAnimationFrame(() => requestAnimationFrame(done)); const loaded = () => document.fonts.ready.then(painted, painted); if (document.readyState === 'complete') loaded(); else addEventListener('load', loaded, { once: true }); })";
const PAINTED: &str = "new Promise((done) => { setTimeout(done, 2000); requestAnimationFrame(() => requestAnimationFrame(done)); })";
const MEASURE: &str = "(() => { const root = document.documentElement; return Math.ceil(root.scrollHeight > root.clientHeight ? root.scrollHeight : root.getBoundingClientRect().height); })()";

pub(crate) struct Capture {
    pub content_height: u32,
    pub captured_height: u32,
    pub console: Vec<String>,
    /// A PNG, base64.
    pub image: String,
}

/// A browser started for one call, with a profile of its own it removes when done.
struct Scratch {
    browser: Option<Browser>,
    profile: PathBuf,
}

impl Scratch {
    fn start() -> Result<Self, String> {
        let executable =
            find_browser().ok_or("no Chrome or Edge is installed to load the page in")?;
        let profile = std::env::temp_dir().join(format!(
            "hoshi-page-{}",
            crate::delegation::new_id().replace('-', "")
        ));
        let browser = Browser::start(&executable, &profile)?;
        Ok(Self {
            browser: Some(browser),
            profile,
        })
    }

    fn open(&self, width: u32, appearance: &str, html: &str) -> Result<Page, String> {
        let browser = self.browser.as_ref().ok_or("the browser closed")?;
        let page = browser.open_page((width, VIEWPORT_HEIGHT), Arc::new(|_| {}))?;
        page.set_color_scheme(Some(appearance))?;
        let tree = page.call("Page.getFrameTree", json!({}))?;
        let frame = tree["frameTree"]["frame"]["id"]
            .as_str()
            .ok_or("the browser opened no page")?;
        page.call(
            "Page.setDocumentContent",
            json!({ "frameId": frame, "html": html }),
        )?;
        page.evaluate(SETTLE)?;
        Ok(page)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        drop(self.browser.take());
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

fn content_height(page: &Page) -> Result<u32, String> {
    let height = page.evaluate(MEASURE)?.as_f64().unwrap_or_default();
    Ok(height.max(0.0).ceil() as u32)
}

fn clip(line: String) -> String {
    if line.chars().count() <= CONSOLE_LINE_CHARS {
        return line;
    }
    let mut clipped: String = line.chars().take(CONSOLE_LINE_CHARS).collect();
    clipped.push('…');
    clipped
}

/// The page `width` pixels wide in `appearance`, `dark` or `light`.
pub(crate) fn capture(html: &str, width: u32, appearance: &str) -> Result<Capture, String> {
    let scratch = Scratch::start()?;
    let page = scratch.open(width, appearance, html)?;
    let content_height = content_height(&page)?;
    let captured_height = content_height.clamp(1, MAX_CAPTURE_HEIGHT);
    let shot = page.call(
        "Page.captureScreenshot",
        json!({
            "format": "png",
            "captureBeyondViewport": true,
            "clip": { "x": 0, "y": 0, "width": width, "height": captured_height, "scale": 1 },
        }),
    )?;
    let image = shot["data"]
        .as_str()
        .ok_or("the browser gave no screenshot")?
        .to_owned();
    Ok(Capture {
        content_height,
        captured_height,
        console: page.console(CONSOLE_LINES).into_iter().map(clip).collect(),
        image,
    })
}

/// The page's height at each of `widths`, narrowest first.
pub(crate) fn measure(html: &str, widths: &[u32]) -> Result<Vec<(u32, u32)>, String> {
    let first = *widths.first().ok_or("no width to measure at")?;
    let scratch = Scratch::start()?;
    let mut page = scratch.open(first, "dark", html)?;
    let mut heights = Vec::with_capacity(widths.len());
    for &width in widths {
        if width != page.size().0 {
            page.resize((width, VIEWPORT_HEIGHT))?;
            page.evaluate(PAINTED)?;
        }
        heights.push((width, content_height(&page)?));
    }
    Ok(heights)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "<!doctype html><html><head><style>:root{--a:1}</style></head><body><div style=\"height:300px\">a</div><script>console.log('ran', 1 + 1)</script></body></html>";

    #[test]
    #[ignore = "starts the installed Chrome or Edge"]
    fn a_page_is_captured_and_measured_in_the_installed_browser() {
        let shot = capture(PAGE, 400, "dark").unwrap();
        assert_eq!(shot.content_height, 316);
        assert_eq!(shot.captured_height, 316);
        assert_eq!(shot.console, ["log: ran 2"]);
        assert!(shot.image.len() > 100);

        let tall = "<!doctype html><div style=\"display:flex;flex-wrap:wrap\">".to_owned()
            + &"<div style=\"width:100px;height:100px\"></div>".repeat(6)
            + "</div>";
        let heights = measure(&tall, &[320, 640]).unwrap();
        assert_eq!(heights, [(320, 216), (640, 116)]);
    }
}
