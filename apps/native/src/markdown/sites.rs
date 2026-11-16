//! The icon shown before a web link: GitHub's mark, the site's favicon once read, or a globe.
//!
//! Each link gets an inline image before it whose address names its host; the text view asks
//! [`resolve`] for that image. Favicons are read once per host, off the UI thread, and the
//! windows draw again when one arrives.

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui_kit::{App, AssetSource as _, Hsla, Image, ImageFormat, ImageSource, Rgba, SharedUri};
use hoshi_ui::assets::{Assets, IconName};
use std::collections::HashMap;
use std::io::Read as _;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// The scheme of the addresses that stand for a site's icon: `hoshi-site:github.com`.
pub const SCHEME: &str = "hoshi-site:";
/// Above this, a favicon is not an icon.
const MAX_FAVICON_BYTES: u64 = 512 * 1024;
const ICON_PIXELS: u32 = 32;

enum Favicon {
    Loading,
    Ready(Arc<Image>),
    Failed,
}

fn favicons() -> &'static Mutex<HashMap<String, Favicon>> {
    static FAVICONS: OnceLock<Mutex<HashMap<String, Favicon>>> = OnceLock::new();
    FAVICONS.get_or_init(Default::default)
}

/// Marks drawn in a color, by icon and color.
type Tinted = HashMap<(&'static str, String), Arc<Image>>;

fn tinted() -> &'static Mutex<Tinted> {
    static TINTED: OnceLock<Mutex<Tinted>> = OnceLock::new();
    TINTED.get_or_init(Default::default)
}

static REDRAW: OnceLock<UnboundedSender<()>> = OnceLock::new();

/// Draws the windows again whenever a favicon arrives.
pub fn init(cx: &mut App) {
    let (sender, mut arrivals) = unbounded();
    if REDRAW.set(sender).is_err() {
        return;
    }
    cx.spawn(async move |cx| {
        while arrivals.next().await.is_some() {
            cx.update(|cx| cx.refresh_windows());
        }
    })
    .detach();
}

/// Draws the windows again, as when an image read off the UI thread arrives.
pub fn redraw() {
    if let Some(redraw) = REDRAW.get() {
        let _ = redraw.unbounded_send(());
    }
}

/// The lowercased host of an http(s) link; `None` for anything else.
pub fn web_host(href: &str) -> Option<String> {
    let lower = href.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = match host.strip_prefix('[') {
        Some(ipv6) => ipv6.split(']').next()?,
        None => host.split(':').next()?,
    };
    (!host.is_empty()).then(|| host.to_owned())
}

fn hex(color: Hsla) -> String {
    let rgba = Rgba::from(color);
    let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(rgba.r),
        channel(rgba.g),
        channel(rgba.b)
    )
}

fn mark(icon: IconName, name: &'static str, color: Hsla) -> Option<Arc<Image>> {
    let color = hex(color);
    let key = (name, color.clone());
    if let Some(image) = tinted().lock().ok()?.get(&key) {
        return Some(image.clone());
    }
    let svg = Assets.load(&icon.path()).ok()??;
    let svg = String::from_utf8_lossy(&svg).replace("currentColor", &color);
    let image = Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes()));
    tinted().lock().ok()?.insert(key, image.clone());
    Some(image)
}

/// The image for an address [`SCHEME`] names, in `color` when it is a mark.
pub fn resolve(uri: &SharedUri, color: Hsla) -> Option<ImageSource> {
    let host = uri.as_ref().strip_prefix(SCHEME)?;
    if host == "github.com" {
        return mark(IconName::Github, "github", color).map(ImageSource::Image);
    }
    let favicon = {
        let mut favicons = favicons().lock().ok()?;
        match favicons.get(host) {
            Some(Favicon::Ready(image)) => Some(image.clone()),
            Some(Favicon::Loading | Favicon::Failed) => None,
            None => {
                favicons.insert(host.to_owned(), Favicon::Loading);
                let host = host.to_owned();
                std::thread::spawn(move || read_favicon(host));
                None
            }
        }
    };
    favicon
        .or_else(|| mark(IconName::Globe, "globe", color))
        .map(ImageSource::Image)
}

fn read_favicon(host: String) {
    let image = fetch_favicon(&host);
    if let Ok(mut favicons) = favicons().lock() {
        favicons.insert(
            host,
            match image {
                Some(image) => Favicon::Ready(Arc::new(image)),
                None => Favicon::Failed,
            },
        );
    }
    redraw();
}

fn fetch_favicon(host: &str) -> Option<Image> {
    if cfg!(test) {
        return None;
    }
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(8)))
            .build(),
    );
    let response = agent
        .get(format!("https://{host}/favicon.ico"))
        .call()
        .ok()?;
    let mut bytes = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(MAX_FAVICON_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    let decoded = image::load_from_memory(&bytes).ok()?;
    let icon = decoded.resize(
        ICON_PIXELS,
        ICON_PIXELS,
        image::imageops::FilterType::Lanczos3,
    );
    let mut png = Vec::new();
    icon.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(Image::from_bytes(ImageFormat::Png, png))
}

#[cfg(test)]
mod tests {
    use super::web_host;

    #[test]
    fn hosts_of_web_links_only() {
        assert_eq!(
            web_host("https://Bitbucket.org/noshi/test").as_deref(),
            Some("bitbucket.org")
        );
        assert_eq!(
            web_host("http://user@localhost:3000/x").as_deref(),
            Some("localhost")
        );
        assert_eq!(web_host("mailto:a@b.c"), None);
        assert_eq!(web_host("C:/a/b.rs"), None);
    }
}
