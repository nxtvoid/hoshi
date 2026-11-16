//! A page the agent showed with `html_render`: live in the transcript on hoshi's browser,
//! or a card that opens it when that browser can't show it.

use crate::browser::{Browsers, PageFit, PageView, ViewportChanged};
use crate::m;
use crate::state::ThreadState;
use crate::tool_calls::is_html_render;
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::*;
use hoshi_core::ItemContent;
use hoshi_ui::assets::IconName;
use hoshi_ui::prelude::*;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

const MAX_TITLE_CHARS: usize = 200;
const COLUMN_WIDTH: f64 = 768.;
const MIN_HEIGHT: f64 = 80.;
const MAX_HEIGHT: f64 = 2000.;
const MAX_MEASURES: usize = 24;
/// The page's own gutter, which the row lets out past the column as the web does.
const GUTTER: f32 = 8.;
/// Pages kept open off screen, so a row scrolled back to shows at once.
const KEPT_PAGES: usize = 6;
/// The tallest a live page grows, as tall as hoshi's browser lays a page out.
const MAX_LIVE_HEIGHT: f64 = 3840.;
/// How often a shown page is asked for its height and the links clicked on it.
const WATCH_EVERY: Duration = Duration::from_millis(300);
const ZOOMS: [f64; 6] = [0.5, 0.75, 1., 1.25, 1.5, 2.];
const DEFAULT_ZOOM: usize = 2;
/// Keeps the http(s) links clicked on the page from navigating it and gives them, with the
/// page's height. The height reaches the lowest element, even one that sticks out of the page's
/// box, so it doesn't change with the frame's own height.
const REPORT_SCRIPT: &str = r##"(() => {
  if (!window.__hoshiLinks) {
    const links = [];
    window.__hoshiLinks = links;
    document.addEventListener("click", (event) => {
      const link = event.isTrusted ? event.composedPath().find((t) => t && t.matches && t.matches("a[href]")) : null;
      if (!link) return;
      let url;
      try { url = new URL(link.getAttribute("href"), document.baseURI); } catch (_) { return; }
      if (!/^https?:$/.test(url.protocol) || url.href.split("#")[0] === location.href.split("#")[0]) return;
      event.preventDefault();
      links.push(url.href);
    }, true);
  }
  const box = document.documentElement.getBoundingClientRect();
  let bottom = box.bottom;
  for (const element of document.body ? document.body.getElementsByTagName("*") : []) {
    const rect = element.getBoundingClientRect();
    if (rect.height > 0 && rect.bottom > bottom) bottom = rect.bottom;
  }
  const height = Math.ceil(bottom - box.top);
  return { height, links: window.__hoshiLinks.splice(0) };
})()"##;

/// A page an agent showed; mirrors `html_pages.rs` in hoshi-engine.
#[derive(Clone, Debug, PartialEq)]
pub struct HtmlPage {
    pub id: String,
    pub title: String,
    pub height: f64,
    /// Its height at each width it was measured at, narrowest first.
    pub heights: Option<Vec<(u32, f64)>>,
}

fn valid_id(id: &str) -> bool {
    let valid = |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric());
    id.split_once('_')
        .is_some_and(|(key, rest)| valid(key) && valid(rest))
}

fn clamp_height(height: f64) -> f64 {
    height.round().clamp(MIN_HEIGHT, MAX_HEIGHT)
}

fn read_heights(value: Option<&Value>) -> Option<Vec<(u32, f64)>> {
    let entries = value?.as_array()?;
    if entries.is_empty() || entries.len() > MAX_MEASURES {
        return None;
    }
    let mut heights = entries
        .iter()
        .map(|entry| {
            let [width, height] = entry.as_array()?.as_slice() else {
                return None;
            };
            let width = u32::try_from(width.as_u64()?).ok().filter(|w| *w > 0)?;
            let height = height.as_f64().filter(|h| h.is_finite())?;
            Some((width, clamp_height(height)))
        })
        .collect::<Option<Vec<_>>>()?;
    heights.sort_by_key(|(width, _)| *width);
    Some(heights)
}

/// The page in the output of an `html_render` call; `None` when it names none.
pub fn read_html_page(output: &str) -> Option<HtmlPage> {
    let value: Value = serde_json::from_str(output).ok()?;
    let page = value.get("htmlRender")?;
    let id = page.get("id")?.as_str()?;
    let title = page.get("title")?.as_str()?;
    let height = page.get("height")?.as_f64().filter(|h| h.is_finite())?;
    valid_id(id).then(|| HtmlPage {
        id: id.to_owned(),
        title: title.trim().chars().take(MAX_TITLE_CHARS).collect(),
        height: clamp_height(height),
        heights: read_heights(page.get("heights")),
    })
}

/// The taller of the two measurements around `width`, as either may apply there.
fn measured_at(heights: &[(u32, f64)], width: f64) -> f64 {
    let high = heights
        .iter()
        .position(|(measured, _)| f64::from(*measured) >= width)
        .unwrap_or(heights.len() - 1);
    let low = if f64::from(heights[high].0) == width {
        high
    } else {
        high.saturating_sub(1)
    };
    heights[low].1.max(heights[high].1)
}

/// How tall the page shows at `width`: what was measured there, capped at the height the
/// agent asked for when it asked for less.
pub fn frame_height(page: &HtmlPage, width: f64) -> f64 {
    let Some(heights) = page.heights.as_deref().filter(|h| !h.is_empty()) else {
        return page.height;
    };
    let cap = if measured_at(heights, COLUMN_WIDTH) > page.height {
        page.height
    } else {
        MAX_HEIGHT
    };
    clamp_height(cap.min(measured_at(heights, width)))
}

/// How tall a live page shows once it reported its height.
pub fn live_height(content: f64) -> f64 {
    content.ceil().clamp(MIN_HEIGHT, MAX_LIVE_HEIGHT)
}

/// The heights a live page takes from its reports. A page whose content sticks out of its box
/// measures taller once it overflows and shorter once it fits, so it would swing between two
/// heights; once it swung back, it stays at the taller one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Settling {
    current: Option<f64>,
    last_shrink: Option<(f64, f64)>,
    settled: Option<(f64, f64)>,
}

impl Settling {
    /// The height to show after `reported`; `None` when it stays as it is.
    fn take(&mut self, reported: f64) -> Option<f64> {
        let Some(current) = self.current else {
            self.current = Some(reported);
            return Some(reported);
        };
        if reported == current || self.settled == Some((current, reported)) {
            return None;
        }
        if reported < current {
            self.last_shrink = Some((current, reported));
        } else if self.last_shrink == Some((reported, current)) {
            self.settled = Some((reported, current));
        }
        self.current = Some(reported);
        Some(reported)
    }

    /// The page reflows at a new width, where it may fit differently.
    fn unsettle(&mut self) {
        self.last_shrink = None;
        self.settled = None;
    }
}

/// What a shown page reported: its height, and the web links clicked on it.
#[derive(Debug, Default, PartialEq)]
struct Report {
    height: Option<f64>,
    links: Vec<String>,
}

fn read_report(value: &Value) -> Report {
    let height = value
        .get("height")
        .and_then(Value::as_f64)
        .filter(|height| height.is_finite() && *height > 0.);
    let links = value
        .get("links")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|url| {
            let lower = url.to_ascii_lowercase();
            lower.starts_with("https://") || lower.starts_with("http://")
        })
        .map(str::to_owned)
        .collect();
    Report { height, links }
}

/// The page's theme variables and the token each takes, as `PAGE_THEME_TOKENS` on the web.
fn theme_variables(colors: &Colors) -> [(&'static str, Hsla); 33] {
    [
        ("--background", colors.background),
        ("--foreground", colors.foreground),
        ("--muted", colors.muted),
        ("--muted-foreground", colors.muted_foreground),
        ("--card", colors.card),
        ("--card-foreground", colors.card_foreground),
        ("--popover", colors.popover),
        ("--popover-foreground", colors.popover_foreground),
        ("--secondary", colors.secondary),
        ("--secondary-foreground", colors.secondary_foreground),
        ("--border", colors.border),
        ("--input", colors.input),
        ("--ring", colors.ring),
        ("--primary", colors.primary),
        ("--primary-foreground", colors.primary_foreground),
        ("--accent", colors.send),
        ("--accent-foreground", colors.background),
        ("--destructive", colors.status_error),
        ("--destructive-foreground", colors.background),
        ("--warning", colors.warning),
        ("--warning-foreground", colors.background),
        ("--success", colors.status_done),
        ("--success-foreground", colors.background),
        ("--info", colors.status_working),
        ("--info-foreground", colors.background),
        ("--code-background", colors.muted),
        ("--code-foreground", colors.foreground),
        ("--chart-1", colors.link),
        ("--chart-2", colors.status_done),
        ("--chart-3", colors.status_waiting),
        ("--chart-4", colors.pr_merged),
        ("--chart-5", colors.status_error),
        ("--chart-6", colors.file_teal),
    ]
}

fn css_color(color: Hsla) -> String {
    let rgba = Rgba::from(color);
    let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
    format!(
        "rgba({},{},{},{})",
        channel(rgba.r),
        channel(rgba.g),
        channel(rgba.b),
        (rgba.a.clamp(0., 1.) * 1000.).round() / 1000.
    )
}

/// `text` as `encodeURIComponent` writes it.
fn encode_component(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => char::from(byte).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// The address fragment that starts a page in the app's theme.
pub fn theme_fragment(colors: &Colors, dark: bool) -> String {
    let mut variables: Map<String, Value> = theme_variables(colors)
        .into_iter()
        .map(|(name, color)| (name.to_owned(), Value::from(css_color(color))))
        .collect();
    variables.insert("--radius".into(), "0.625rem".into());
    variables.insert(
        "--font-sans".into(),
        "Geist, system-ui, \"Segoe UI\", sans-serif".into(),
    );
    variables.insert(
        "--font-mono".into(),
        "\"JetBrains Mono\", Consolas, Menlo, monospace".into(),
    );
    let theme = json!({
        "appearance": if dark { "dark" } else { "light" },
        "variables": variables,
    });
    format!("#hoshi-theme={}", encode_component(&theme.to_string()))
}

fn file_url(path: &str) -> String {
    let path = path.replace('\\', "/").replace(' ', "%20");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

/// Where a page's own browser page is kept, apart from its thread's.
pub fn page_key(thread_id: &str, page_id: &str) -> String {
    format!("{thread_id}#html-page:{page_id}")
}

/// Finds the page's file off the UI thread, as the engine may be busy.
fn read_page_path(id: &str, cx: &App) -> Task<Option<String>> {
    let Some(workspace) = crate::markdown::workspace(cx) else {
        return Task::ready(None);
    };
    let engine = workspace.read(cx).engine.clone();
    let id = id.to_owned();
    cx.background_spawn(async move {
        let path = engine.html_page_path(&id)?;
        Some(path.to_string_lossy().into_owned())
    })
}

fn themed_address(path: &str, cx: &App) -> String {
    let dark = cx.theme().mode.is_dark();
    file_url(path) + &theme_fragment(cx.hoshi(), dark)
}

fn open_page(id: &str, external: bool, cx: &mut App) {
    let path = read_page_path(id, cx);
    cx.spawn(async move |cx| {
        let Some(path) = path.await else {
            return;
        };
        cx.update(|cx| {
            let address = themed_address(&path, cx);
            if external || !crate::markdown::open_in_panel(&address, cx) {
                cx.open_url(&address);
            }
        });
    })
    .detach();
}

/// Asks the page under `key` for its height and the links clicked on it, while it is open,
/// until the entity goes.
fn watch<T: 'static>(
    key: String,
    browsers: Entity<Browsers>,
    take: fn(&mut T, Report, &mut Context<T>),
    cx: &mut Context<T>,
) {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(WATCH_EVERY).await;
            let Ok(asked) = this.update(cx, |_, cx| {
                let browsers = browsers.read(cx);
                browsers
                    .page(&key)
                    .is_some()
                    .then(|| browsers.evaluate(&key, REPORT_SCRIPT))
            }) else {
                return;
            };
            let Some(asked) = asked else {
                continue;
            };
            let Ok(value) = asked.await else {
                continue;
            };
            if this
                .update(cx, |this, cx| take(this, read_report(&value), cx))
                .is_err()
            {
                return;
            }
        }
    })
    .detach();
}

struct KeptPage {
    key: String,
    address: String,
    view: WeakEntity<PageView>,
}

/// The pages rows opened, oldest first.
#[derive(Default)]
struct KeptPages(VecDeque<KeptPage>);

impl Global for KeptPages {}

/// The last height each page reported, so a row shown again starts at it.
#[derive(Default)]
struct LiveHeights(HashMap<String, f64>);

impl Global for LiveHeights {}

/// Where the page's file is, once known.
#[derive(Clone, Debug, PartialEq)]
enum PagePath {
    Reading,
    Found(String),
    Gone,
}

/// A row's page, live while the row shows.
struct LivePage {
    key: String,
    page: HtmlPage,
    path: PagePath,
    browsers: Entity<Browsers>,
    view: Entity<PageView>,
    width: Option<u32>,
    /// The page's own height, once it reported it.
    content_height: Option<f64>,
    settling: Settling,
    _resized: Subscription,
}

impl LivePage {
    fn new(
        key: String,
        page: HtmlPage,
        browsers: Entity<Browsers>,
        cx: &mut Context<Self>,
    ) -> Self {
        let path = read_page_path(&page.id, cx);
        cx.spawn(async move |this, cx| {
            let path = path.await;
            this.update(cx, |this, cx| {
                this.path = path.map_or(PagePath::Gone, PagePath::Found);
                cx.notify();
            })
            .ok();
        })
        .detach();
        let view = cx.new(|cx| {
            let mut view = PageView::new(
                key.clone(),
                browsers.clone(),
                PageFit::Width,
                "html-page-view",
                cx,
            );
            view.set_scrolls(false);
            view
        });
        let resized = cx.subscribe(&view, |this, _, _: &ViewportChanged, cx| this.fit(cx));
        let width = COLUMN_WIDTH + f64::from(2. * GUTTER);
        let content_height = cx
            .try_global::<LiveHeights>()
            .and_then(|heights| heights.0.get(&key).copied());
        let height = content_height.unwrap_or_else(|| frame_height(&page, width));
        browsers.read(cx).fit(&key, width as u32, height as u32);
        watch(key.clone(), browsers.clone(), Self::take_report, cx);
        Self {
            key,
            page,
            path: PagePath::Reading,
            browsers,
            view,
            width: None,
            content_height,
            settling: Settling {
                current: content_height,
                ..Settling::default()
            },
            _resized: resized,
        }
    }

    fn width(&self) -> f64 {
        self.width
            .map_or(COLUMN_WIDTH + f64::from(2. * GUTTER), f64::from)
    }

    fn height(&self) -> f64 {
        self.content_height
            .map_or_else(|| frame_height(&self.page, self.width()), live_height)
    }

    fn take_report(&mut self, report: Report, cx: &mut Context<Self>) {
        for link in &report.links {
            cx.open_url(link);
        }
        let Some(height) = report
            .height
            .map(live_height)
            .and_then(|height| self.settling.take(height))
        else {
            return;
        };
        self.content_height = Some(height);
        cx.update_default_global(|heights: &mut LiveHeights, _| {
            heights.0.insert(self.key.clone(), height);
        });
        self.browsers
            .read(cx)
            .fit(&self.key, self.width() as u32, height as u32);
        cx.notify();
    }

    fn fit(&mut self, cx: &mut Context<Self>) {
        let Some(viewport) = self.view.read(cx).viewport() else {
            return;
        };
        let width = f32::from(viewport.size.width).floor();
        if width <= 0. || self.width == Some(width as u32) {
            return;
        }
        self.width = Some(width as u32);
        self.settling.unsettle();
        let height = self.height() as u32;
        self.browsers.read(cx).fit(&self.key, width as u32, height);
        cx.notify();
    }

    /// Opens the page at `address` unless it is already open there, and closes the oldest
    /// pages no row shows.
    fn show(&self, address: String, cx: &mut Context<Self>) {
        let open = self.browsers.read(cx).page(&self.key).is_some();
        let view = self.view.downgrade();
        let key = self.key.clone();
        let (reopen, closing) = cx.update_default_global(|kept: &mut KeptPages, _| {
            let index = kept.0.iter().position(|page| page.key == key);
            let current = index.map(|index| &kept.0[index]);
            let same_view = current.is_some_and(|page| page.view.entity_id() == view.entity_id());
            let same_address = current.is_some_and(|page| page.address == address);
            if same_view && same_address {
                return (false, Vec::new());
            }
            let reopen = !same_address || !open;
            if let Some(index) = index {
                kept.0.remove(index);
            }
            kept.0.push_back(KeptPage {
                key: key.clone(),
                address: address.clone(),
                view,
            });
            let mut closing = Vec::new();
            while kept.0.len() > KEPT_PAGES {
                let Some(index) = kept.0.iter().position(|page| page.view.upgrade().is_none())
                else {
                    break;
                };
                if let Some(page) = kept.0.remove(index) {
                    closing.push(page.key);
                }
            }
            (reopen, closing)
        });
        if !reopen && closing.is_empty() {
            return;
        }
        self.browsers.update(cx, |browsers, cx| {
            for gone in &closing {
                browsers.close(gone, cx);
            }
            if reopen {
                browsers.open(&key, &address, cx);
            }
        });
    }
}

#[derive(IntoElement)]
struct HtmlPageRow {
    item_id: String,
    thread_id: String,
    page: HtmlPage,
}

impl RenderOnce for HtmlPageRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let browsers = crate::markdown::workspace(cx).map(|w| w.read(cx).browsers.clone());
        let key = page_key(&self.thread_id, &self.page.id);
        let Some(browsers) = browsers.filter(|browsers| !browsers.read(cx).failed(&key)) else {
            return page_card(&self.item_id, self.page, &colors);
        };
        let state_id = SharedString::from(format!("html-page-live-{}", self.item_id));
        let page = self.page.clone();
        let live = window.use_keyed_state(state_id, cx, move |_, cx| {
            LivePage::new(key, page, browsers, cx)
        });
        match live.read(cx).path.clone() {
            PagePath::Gone => return page_card(&self.item_id, self.page, &colors),
            PagePath::Found(path) => {
                let address = themed_address(&path, cx);
                live.update(cx, |live, cx| live.show(address, cx));
            }
            PagePath::Reading => {}
        }
        let live = live.read(cx);
        live_row(&self.item_id, &self.page.id, live, &colors)
    }
}

fn live_row(item_id: &str, page_id: &str, live: &LivePage, colors: &Colors) -> AnyElement {
    let selector = format!("html-page-{item_id}");
    let group = SharedString::from(selector.clone());
    let (panel_id, external_id) = (page_id.to_owned(), page_id.to_owned());
    let expanded = match &live.path {
        PagePath::Found(path) => Some((
            live.key.clone(),
            path.clone(),
            live.page.title.clone(),
            live.browsers.clone(),
        )),
        PagePath::Reading | PagePath::Gone => None,
    };
    let action = |id: String, icon: IconName, label: String| {
        Button::new(SharedString::from(id))
            .icon(icon)
            .size(ButtonSize::IconSm)
            .variant(ButtonVariant::Ghost)
            .tooltip(Tooltip::new(label).side(Side::Bottom))
    };
    let toolbar = div()
        .absolute()
        .top_2()
        .right(px(GUTTER + 8.))
        .flex()
        .items_center()
        .gap(px(2.))
        .p(px(2.))
        .rounded_lg()
        .bg(colors.background.opacity(0.8))
        .opacity(0.)
        .group_hover(group.clone(), |toolbar| toolbar.opacity(1.))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .when_some(expanded, |toolbar, (key, path, title, browsers)| {
            toolbar.child(
                action(
                    format!("html-page-expand-{item_id}"),
                    IconName::Maximize,
                    m::html_page_expand(),
                )
                .on_click(move |_, window, cx| {
                    expand(&key, &path, title.clone(), browsers.clone(), window, cx)
                }),
            )
        })
        .child(
            action(
                format!("html-page-panel-{item_id}"),
                IconName::PanelRight,
                m::mini_browser_open_in_panel(),
            )
            .on_click(move |_, _, cx| open_page(&panel_id, false, cx)),
        )
        .child(
            action(
                format!("html-page-external-{item_id}"),
                IconName::ExternalLink,
                m::browser_open_external(),
            )
            .on_click(move |_, _, cx| open_page(&external_id, true, cx)),
        );
    div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .group(group)
        .relative()
        .mx(px(-GUTTER))
        .h(px(live.height() as f32))
        .child(live.view.clone())
        .child(toolbar)
        .into_any_element()
}

/// The page large over the window, on a browser page of its own, zoomed as the user picks.
struct ExpandedPage {
    key: String,
    title: String,
    browsers: Entity<Browsers>,
    view: Entity<PageView>,
    zoom: usize,
    dialog: Option<DialogHandle>,
    _resized: Subscription,
}

/// A side of the view as the page lays it out at `zoom`.
fn zoomed(side: f32, zoom: f64) -> u32 {
    (f64::from(side) / zoom).floor().max(1.) as u32
}

impl ExpandedPage {
    fn new(
        key: String,
        address: &str,
        title: String,
        browsers: Entity<Browsers>,
        cx: &mut Context<Self>,
    ) -> Self {
        let view = cx.new(|cx| {
            PageView::new(
                key.clone(),
                browsers.clone(),
                PageFit::Width,
                "html-page-expanded",
                cx,
            )
        });
        let resized = cx.subscribe(&view, |this, _, _: &ViewportChanged, cx| this.fit(cx));
        cx.on_release(|this, cx| {
            let key = this.key.clone();
            this.browsers
                .update(cx, |browsers, cx| browsers.close(&key, cx));
        })
        .detach();
        browsers.update(cx, |browsers, cx| browsers.open(&key, address, cx));
        watch(key.clone(), browsers.clone(), Self::take_links, cx);
        Self {
            key,
            title,
            browsers,
            view,
            zoom: DEFAULT_ZOOM,
            dialog: None,
            _resized: resized,
        }
    }

    fn take_links(&mut self, report: Report, cx: &mut Context<Self>) {
        for link in &report.links {
            cx.open_url(link);
        }
    }

    fn fit(&mut self, cx: &mut Context<Self>) {
        let Some(viewport) = self.view.read(cx).viewport() else {
            return;
        };
        let zoom = ZOOMS[self.zoom];
        let width = zoomed(f32::from(viewport.size.width), zoom);
        let height = zoomed(f32::from(viewport.size.height), zoom);
        self.browsers.read(cx).fit(&self.key, width, height);
    }

    fn set_zoom(&mut self, zoom: usize, cx: &mut Context<Self>) {
        if zoom < ZOOMS.len() && zoom != self.zoom {
            self.zoom = zoom;
            self.fit(cx);
            cx.notify();
        }
    }
}

impl Render for ExpandedPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let zoom = self.zoom;
        let button = |id: &'static str, icon: IconName, label: String| {
            Button::new(id)
                .icon(icon)
                .size(ButtonSize::IconSm)
                .variant(ButtonVariant::Ghost)
                .tooltip(Tooltip::new(label).side(Side::Bottom))
        };
        let (muted, foreground) = (colors.muted_foreground, colors.foreground);
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .border_b_1()
            .border_color(colors.border)
            .py(px(6.))
            .pr(px(6.))
            .pl_4()
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.title.clone()),
            )
            .child(
                button(
                    "html-page-zoom-out",
                    IconName::Minus,
                    m::html_page_zoom_out(),
                )
                .disabled(zoom == 0)
                .on_click(
                    cx.listener(move |this, _, _, cx| this.set_zoom(zoom.saturating_sub(1), cx)),
                ),
            )
            .child(
                div()
                    .id("html-page-zoom-reset")
                    .debug_selector(|| "html-page-zoom-reset".into())
                    .w_12()
                    .text_center()
                    .text_xs()
                    .text_color(muted)
                    .hover(move |label| label.text_color(foreground))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.set_zoom(DEFAULT_ZOOM, cx)))
                    .child(format!("{}%", (ZOOMS[zoom] * 100.).round())),
            )
            .child(
                button("html-page-zoom-in", IconName::Plus, m::html_page_zoom_in())
                    .disabled(zoom + 1 == ZOOMS.len())
                    .on_click(cx.listener(move |this, _, _, cx| this.set_zoom(zoom + 1, cx))),
            )
            .child(
                button("html-page-close", IconName::X, m::html_page_close()).on_click(cx.listener(
                    |this, _, window, cx| {
                        if let Some(dialog) = this.dialog.clone() {
                            dialog.close(window, cx);
                        }
                    },
                )),
            );
        div().size_full().flex().flex_col().child(header).child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .child(self.view.clone()),
        )
    }
}

fn expand(
    key: &str,
    path: &str,
    title: String,
    browsers: Entity<Browsers>,
    window: &mut Window,
    cx: &mut App,
) {
    let address = themed_address(path, cx);
    let key = format!("{key}#expanded");
    let expanded = cx.new(|cx| ExpandedPage::new(key, &address, title, browsers, cx));
    let shown = expanded.clone();
    let dialog = DialogLayer::open(window, cx, move |dialog, _, _| {
        dialog.page().close_button(false).child(shown.clone())
    });
    expanded.update(cx, |expanded, _| expanded.dialog = Some(dialog));
}

fn page_card(item_id: &str, page: HtmlPage, colors: &Colors) -> AnyElement {
    let selector = format!("html-page-{item_id}");
    let (panel_id, external_id) = (page.id.clone(), page.id.clone());
    let hover = colors.foreground.opacity(0.05);
    div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .flex()
        .w_full()
        .items_center()
        .gap(px(10.))
        .rounded_xl()
        .border_1()
        .border_color(colors.border.opacity(0.7))
        .px_3()
        .py_2()
        .text_sm()
        .cursor_pointer()
        .hover(move |card| card.bg(hover))
        .press_selects_nothing()
        .on_click(move |_, _, cx| open_page(&panel_id, false, cx))
        .child(
            Icon::new(IconName::AppWindow)
                .size_4()
                .flex_none()
                .text_color(colors.muted_foreground),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .truncate()
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors.foreground)
                .child(page.title),
        )
        .child(
            Button::new(SharedString::from(format!("html-page-external-{item_id}")))
                .icon(IconName::ExternalLink)
                .size(ButtonSize::Xs)
                .variant(ButtonVariant::Ghost)
                .tooltip(m::browser_open_external())
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    open_page(&external_id, true, cx);
                }),
        )
        .child(crate::subagents::open_arrow(colors))
        .into_any_element()
}

/// The pages the transcript holds only the start of, which it reads whole to show them.
pub fn pages_to_read(thread: &ThreadState) -> Vec<String> {
    thread
        .items
        .iter()
        .filter(|item| {
            matches!(
                thread.content(item),
                ItemContent::ToolCall {
                    name,
                    is_error: false,
                    output_omitted: true,
                    ..
                } if is_html_render(name)
            )
        })
        .map(|item| item.id.clone())
        .collect()
}

/// The page live in the transcript; a card that opens it when hoshi's browser can't show it.
pub fn html_page_row(item_id: &str, thread_id: &str, content: &ItemContent) -> AnyElement {
    let Some(page) = (match content {
        ItemContent::ToolCall {
            output: Some(output),
            ..
        } => read_html_page(output),
        _ => None,
    }) else {
        return div().into_any_element();
    };
    HtmlPageRow {
        item_id: item_id.to_owned(),
        thread_id: thread_id.to_owned(),
        page,
    }
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_ZOOM, HtmlPage, Report, Settling, ZOOMS, encode_component, file_url, frame_height,
        live_height, page_key, read_html_page, read_report, zoomed,
    };
    use serde_json::json;

    #[test]
    fn a_page_swinging_between_two_heights_stays_at_the_taller() {
        let mut settling = Settling::default();
        let shown: Vec<Option<f64>> = [345., 340., 345., 340., 345., 340.]
            .into_iter()
            .map(|height| settling.take(height))
            .collect();
        assert_eq!(
            shown,
            [Some(345.), Some(340.), Some(345.), None, None, None]
        );
        assert_eq!(settling.take(500.), Some(500.));
        assert_eq!(settling.take(200.), Some(200.));
        settling.unsettle();
        assert_eq!(settling.take(345.), Some(345.));
        assert_eq!(settling.take(340.), Some(340.));
    }

    fn page(height: f64, heights: Option<Vec<(u32, f64)>>) -> HtmlPage {
        HtmlPage {
            id: "ab12_cd34".into(),
            title: "Chart".into(),
            height,
            heights,
        }
    }

    #[test]
    fn a_render_output_names_its_page() {
        assert_eq!(
            read_html_page(r#"{"htmlRender":{"id":"ab12_cd34","title":"  Chart ","height":300}}"#),
            Some(page(300., None))
        );
        assert_eq!(
            read_html_page(
                r#"{"htmlRender":{"id":"ab12_cd34","title":"Chart","height":3,"heights":[[768,500],[320,900.4]]}}"#
            ),
            Some(page(80., Some(vec![(320, 900.), (768, 500.)])))
        );
        assert_eq!(
            read_html_page(
                r#"{"htmlRender":{"id":"ab12_cd34","title":"Chart","height":300,"heights":[[0,1]]}}"#
            ),
            Some(page(300., None))
        );
        assert_eq!(
            read_html_page(r#"{"htmlRender":{"id":"../x_y","title":"","height":1}}"#),
            None
        );
        assert_eq!(
            read_html_page(r#"{"htmlRender":{"id":"a_b","title":"t"}}"#),
            None
        );
        assert_eq!(read_html_page("Shown to the user"), None);
    }

    #[test]
    fn a_page_is_as_tall_as_it_was_measured_at_that_width() {
        assert_eq!(frame_height(&page(300., None), 500.), 300.);
        let measured = page(2000., Some(vec![(320, 900.), (768, 500.), (1000, 400.)]));
        assert_eq!(frame_height(&measured, 768.), 500.);
        assert_eq!(frame_height(&measured, 600.), 900.);
        assert_eq!(frame_height(&measured, 1000.), 400.);
        assert_eq!(frame_height(&measured, 1200.), 500.);
        assert_eq!(frame_height(&measured, 200.), 900.);
        let capped = page(300., Some(vec![(320, 900.), (768, 500.)]));
        assert_eq!(frame_height(&capped, 784.), 300.);
    }

    #[test]
    fn a_page_file_opens_as_a_file_url() {
        assert_eq!(
            file_url(r"C:\Users\me\.hoshi\pages\a b\a_b.html"),
            "file:///C:/Users/me/.hoshi/pages/a%20b/a_b.html"
        );
        assert_eq!(file_url("/home/me/a_b.html"), "file:///home/me/a_b.html");
    }

    #[test]
    fn a_theme_goes_in_the_address_as_encode_uri_component_writes_it() {
        assert_eq!(
            encode_component(r#"{"a":"b c#%"}"#),
            "%7B%22a%22%3A%22b%20c%23%25%22%7D"
        );
        assert_eq!(encode_component("(x)!*'~"), "(x)!*'~");
    }

    #[test]
    fn a_shown_page_reports_its_height_and_the_web_links_clicked() {
        assert_eq!(
            read_report(&json!({
                "height": 412.5,
                "links": ["https://a.io/x", "javascript:alert(1)", "HTTP://b.io", 3]
            })),
            Report {
                height: Some(412.5),
                links: vec!["https://a.io/x".into(), "HTTP://b.io".into()],
            }
        );
        assert_eq!(read_report(&json!({ "height": 0 })), Report::default());
        assert_eq!(read_report(&json!(null)), Report::default());
        assert_eq!(live_height(412.2), 413.);
        assert_eq!(live_height(10.), 80.);
        assert_eq!(live_height(90_000.), 3840.);
    }

    #[test]
    fn a_zoomed_page_lays_out_smaller_than_it_shows() {
        assert_eq!(ZOOMS[DEFAULT_ZOOM], 1.);
        assert_eq!(zoomed(1000., 2.), 500);
        assert_eq!(zoomed(1000., 0.5), 2000);
        assert_eq!(zoomed(0., 2.), 1);
    }

    #[test]
    fn each_page_has_a_browser_page_apart_from_its_thread() {
        assert_eq!(page_key("t", "ab_cd"), "t#html-page:ab_cd");
    }
}
