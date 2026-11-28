//! A PDF in the side panel: its pages one under another, each drawn as it comes into view at
//! the panel's width or a zoom picked, with a field to find text in it.

use super::file_surface::path_bar;
use super::panel_message;
use crate::m;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{Icon, Selectable as _, Sizable as _};
use gpui_kit::*;
use hoshi_engine::pdf::{Bitmap, Document, MAX_MATCHES, Match, PageSize, PdfFailure};
use hoshi_ui::prelude::*;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// Space around and between pages.
const GAP: f32 = 12.;
/// Pixels a point takes at 100%.
const POINT: f32 = 96. / 72.;
const ZOOMS: [f32; 9] = [0.25, 0.5, 0.75, 1., 1.25, 1.5, 2., 3., 4.];
/// Pages drawn ahead of those in view, each way; drawn pages farther than twice that are let go.
const AHEAD: usize = 1;
/// The most memory drawn pages out of view keep.
const MAX_KEPT_BYTES: usize = 192 * 1024 * 1024;
/// Room left above a match scrolled to.
const MATCH_MARGIN: f32 = 48.;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Zoom {
    Fit,
    Scale(f32),
}

enum Body {
    Loading { download: bool },
    Failed(PdfFailure),
    Ready(Document),
}

struct Drawn {
    width: u32,
    bytes: usize,
    image: Arc<RenderImage>,
}

pub struct PdfSurface {
    path: String,
    home: Option<PathBuf>,
    body: Body,
    zoom: Zoom,
    /// The pages' area, once laid out.
    viewport: Option<Size<Pixels>>,
    scale_factor: f32,
    list: ListState,
    drawn: HashMap<usize, Drawn>,
    /// Drawings replaced on screen, freed once a frame no longer shows them.
    retired: Vec<Arc<RenderImage>>,
    drawing: Option<(usize, u32)>,
    unreadable: HashSet<(usize, u32)>,
    find: Entity<InputState>,
    query: String,
    matches: Vec<Match>,
    current: usize,
    /// When the file opened was last changed.
    modified: Option<SystemTime>,
    /// How many times the file was opened, so a page drawn from an earlier opening is dropped.
    openings: usize,
    opening: Option<Task<()>>,
    searching: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// Pixels a point takes at `zoom` for pages at most `widest` points wide, in a viewport
/// `viewport` pixels wide.
fn pixels_per_point(zoom: Zoom, widest: f32, viewport: f32) -> f32 {
    match zoom {
        Zoom::Scale(scale) => scale * POINT,
        Zoom::Fit => ((viewport - 2. * GAP) / widest.max(1.)).max(0.05),
    }
}

/// The zoom a step in or out of `current` lands on.
fn stepped(current: f32, inward: bool) -> f32 {
    if inward {
        ZOOMS
            .into_iter()
            .find(|zoom| *zoom > current + 0.01)
            .unwrap_or(ZOOMS[ZOOMS.len() - 1])
    } else {
        ZOOMS
            .into_iter()
            .rev()
            .find(|zoom| *zoom < current - 0.01)
            .unwrap_or(ZOOMS[0])
    }
}

/// The match after (or before) `current` among `count`, going round.
fn next_match(current: usize, count: usize, forward: bool) -> usize {
    match (count, forward) {
        (0, _) => 0,
        (_, true) => (current + 1) % count,
        (_, false) => (current + count - 1) % count,
    }
}

/// The pages a view `height` pixels tall shows, from page `top` scrolled `offset` pixels into,
/// with pages `heights` tall.
fn pages_in_view(
    heights: impl IntoIterator<Item = f32>,
    top: usize,
    offset: f32,
    height: f32,
) -> Range<usize> {
    let mut room = height + offset;
    let mut end = top;
    for page in heights.into_iter().skip(top) {
        end += 1;
        room -= page;
        if room <= 0. {
            break;
        }
    }
    top..end
}

/// The page counted as current with page `top` scrolled `offset` pixels into: the next one
/// once most of it went by.
fn page_at_top(heights: &[f32], top: usize, offset: f32) -> usize {
    let last = heights.len().saturating_sub(1);
    match heights.get(top) {
        Some(height) if offset > height / 2. => (top + 1).min(last),
        Some(_) => top,
        None => last,
    }
}

/// The pages to draw, in the order to draw them: those in view, then those below, then above.
fn draw_order(visible: Range<usize>, count: usize) -> impl Iterator<Item = usize> {
    let below = visible.end..(visible.end + AHEAD).min(count);
    let above = visible.start.saturating_sub(AHEAD)..visible.start;
    visible.chain(below).chain(above.rev())
}

fn modified(path: &str) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
}

fn image_of(bitmap: Bitmap) -> Option<Arc<RenderImage>> {
    let pixels = image::RgbaImage::from_raw(bitmap.width, bitmap.height, bitmap.bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])))
}

impl PdfSurface {
    pub fn new(
        path: String,
        home: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let find = cx.new(|cx| crate::locale::input(m::pdf_find_placeholder, window, cx));
        let found = cx.subscribe(&find, |this, find, event, cx| match event {
            InputEvent::PressEnter { shift, .. } => {
                let query = find.read(cx).value().trim().to_owned();
                this.find(query, !shift, cx);
            }
            InputEvent::Change if find.read(cx).value().trim().is_empty() => {
                this.clear_matches(cx);
            }
            _ => {}
        });
        let list = ListState::new(0, ListAlignment::Top, px(800.));
        let this = cx.weak_entity();
        list.set_scroll_handler(move |_, _, cx| {
            this.update(cx, |_, cx| cx.notify()).ok();
        });
        cx.on_release(|this: &mut Self, cx| {
            for image in this
                .drawn
                .drain()
                .map(|(_, drawn)| drawn.image)
                .chain(this.retired.drain(..))
            {
                cx.drop_image(image, None);
            }
        })
        .detach();
        let mut this = Self {
            path,
            home,
            body: Body::Loading { download: false },
            zoom: Zoom::Fit,
            viewport: None,
            scale_factor: window.scale_factor(),
            list,
            drawn: HashMap::new(),
            retired: Vec::new(),
            drawing: None,
            unreadable: HashSet::new(),
            find,
            query: String::new(),
            matches: Vec::new(),
            current: 0,
            modified: None,
            openings: 0,
            opening: None,
            searching: None,
            _subscriptions: vec![found],
        };
        this.open(cx);
        this
    }

    fn open(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            self.body = Body::Failed(PdfFailure::Library);
            return;
        };
        self.openings += 1;
        self.drawing = None;
        self.body = Body::Loading {
            download: hoshi_engine::pdf::needs_download(&home),
        };
        let path = self.path.clone();
        self.opening = Some(cx.spawn(async move |this, cx| {
            let (opened, modified) = cx
                .background_spawn(async move {
                    let modified = modified(&path);
                    (Document::open(&home, Path::new(&path)), modified)
                })
                .await;
            this.update(cx, |this, cx| {
                this.opening = None;
                this.modified = modified;
                this.body = match opened {
                    Ok(document) => {
                        this.list.reset(document.pages().len());
                        Body::Ready(document)
                    }
                    Err(failure) => {
                        eprintln!("hoshi: could not open {}: {failure:?}", this.path);
                        Body::Failed(failure)
                    }
                };
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Opens the file again when it changed since it was opened.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.opening.is_some() {
            return;
        }
        let path = self.path.clone();
        self.opening = Some(cx.spawn(async move |this, cx| {
            let now = cx.background_spawn(async move { modified(&path) }).await;
            this.update(cx, |this, cx| {
                this.opening = None;
                if now != this.modified {
                    this.forget_drawn(cx);
                    this.clear_matches(cx);
                    this.open(cx);
                }
            })
            .ok();
        }));
    }

    fn forget_drawn(&mut self, cx: &mut Context<Self>) {
        for image in self
            .drawn
            .drain()
            .map(|(_, drawn)| drawn.image)
            .chain(self.retired.drain(..))
        {
            cx.drop_image(image, None);
        }
        self.unreadable.clear();
    }

    fn document(&self) -> Option<&Document> {
        match &self.body {
            Body::Ready(document) => Some(document),
            _ => None,
        }
    }

    fn pixels_per_point(&self) -> Option<f32> {
        let document = self.document()?;
        let viewport = self.viewport?;
        let widest = document
            .pages()
            .iter()
            .map(|page| page.width)
            .fold(0., f32::max);
        Some(pixels_per_point(
            self.zoom,
            widest,
            f32::from(viewport.width),
        ))
    }

    /// The pages' heights on screen, each with the gap above it.
    fn heights(&self) -> Option<Vec<f32>> {
        let (document, scale) = self.document().zip(self.pixels_per_point())?;
        Some(
            document
                .pages()
                .iter()
                .map(|page: &PageSize| page.height * scale + GAP)
                .collect(),
        )
    }

    /// The page at the top of the view, or past it once most of that page scrolled away.
    fn current_page(&self) -> usize {
        let Some(heights) = self.heights() else {
            return 0;
        };
        let top = self.list.logical_scroll_top();
        page_at_top(&heights, top.item_ix, f32::from(top.offset_in_item))
    }

    /// The pages in view, by their sizes from the top of the view down.
    fn visible(&self) -> Range<usize> {
        let (Some(heights), Some(viewport)) = (self.heights(), self.viewport) else {
            return 0..0;
        };
        let top = self.list.logical_scroll_top();
        pages_in_view(
            heights,
            top.item_ix,
            f32::from(top.offset_in_item),
            f32::from(viewport.height),
        )
    }

    fn set_viewport(&mut self, size: Size<Pixels>, scale_factor: f32, cx: &mut Context<Self>) {
        let resized = self.viewport.map(|viewport| viewport.width) != Some(size.width);
        self.viewport = Some(size);
        self.scale_factor = scale_factor;
        if resized && self.zoom == Zoom::Fit {
            self.relayout(None);
        }
        cx.notify();
    }

    /// Lays the pages out again at their new size, the page on top staying there.
    fn relayout(&mut self, ratio: Option<f32>) {
        let Some(count) = self.document().map(|document| document.pages().len()) else {
            return;
        };
        let top = self.list.logical_scroll_top();
        self.list.reset(count);
        self.list.scroll_to(ListOffset {
            item_ix: top.item_ix,
            offset_in_item: top.offset_in_item * ratio.unwrap_or(1.),
        });
    }

    fn set_zoom(&mut self, zoom: Zoom, cx: &mut Context<Self>) {
        let before = self.pixels_per_point();
        self.zoom = zoom;
        let after = self.pixels_per_point();
        let ratio = before.zip(after).map(|(before, after)| after / before);
        self.relayout(ratio);
        cx.notify();
    }

    fn zoom_by(&mut self, inward: bool, cx: &mut Context<Self>) {
        let Some(scale) = self.pixels_per_point() else {
            return;
        };
        self.set_zoom(Zoom::Scale(stepped(scale / POINT, inward)), cx);
    }

    fn go_to_page(&mut self, page: usize, cx: &mut Context<Self>) {
        let Some(count) = self.document().map(|document| document.pages().len()) else {
            return;
        };
        if count == 0 {
            return;
        }
        self.list.scroll_to(ListOffset {
            item_ix: page.min(count - 1),
            offset_in_item: px(0.),
        });
        cx.notify();
    }

    fn find(&mut self, query: String, forward: bool, cx: &mut Context<Self>) {
        if query.is_empty() {
            self.clear_matches(cx);
            return;
        }
        if query == self.query && self.searching.is_none() {
            if !self.matches.is_empty() {
                self.current = next_match(self.current, self.matches.len(), forward);
                self.reveal_match(cx);
            }
            return;
        }
        let Some(document) = self.document().cloned() else {
            return;
        };
        self.query = query.clone();
        let from_page = self.current_page();
        self.searching = Some(cx.spawn(async move |this, cx| {
            let mut matches = Vec::new();
            for page in 0..document.pages().len() {
                let (document, searched) = (document.clone(), query.clone());
                let found = cx
                    .background_spawn(async move { document.search_page(page, &searched) })
                    .await;
                matches.extend(found);
                if matches.len() >= MAX_MATCHES {
                    matches.truncate(MAX_MATCHES);
                    break;
                }
            }
            this.update(cx, |this, cx| {
                if this.query != query {
                    return;
                }
                this.searching = None;
                this.current = matches
                    .iter()
                    .position(|found| found.page >= from_page)
                    .unwrap_or(0);
                this.matches = matches;
                this.reveal_match(cx);
            })
            .ok();
        }));
    }

    fn clear_matches(&mut self, cx: &mut Context<Self>) {
        self.query.clear();
        self.matches.clear();
        self.current = 0;
        self.searching = None;
        cx.notify();
    }

    fn reveal_match(&mut self, cx: &mut Context<Self>) {
        let (Some(found), Some(scale)) = (self.matches.get(self.current), self.pixels_per_point())
        else {
            cx.notify();
            return;
        };
        let height = self
            .document()
            .and_then(|document| document.pages().get(found.page))
            .map_or(0., |page| page.height * scale);
        let top = found.areas.first().map_or(0., |area| area.top * height);
        self.list.scroll_to(ListOffset {
            item_ix: found.page,
            offset_in_item: px((GAP + top - MATCH_MARGIN).max(0.)),
        });
        cx.notify();
    }

    /// Draws the next page in view, or about to be, that isn't drawn at its size yet.
    fn draw_next(&mut self, cx: &mut Context<Self>) {
        if self.drawing.is_some() {
            return;
        }
        let (Some(document), Some(scale)) = (self.document().cloned(), self.pixels_per_point())
        else {
            return;
        };
        let pages = document.pages();
        let wanted = draw_order(self.visible(), pages.len())
            .map(|page| {
                let width = (pages[page].width * scale * self.scale_factor).round() as u32;
                (page, width.max(1))
            })
            .find(|(page, width)| {
                self.drawn
                    .get(page)
                    .is_none_or(|drawn| drawn.width != *width)
                    && !self.unreadable.contains(&(*page, *width))
            });
        let Some((page, width)) = wanted else {
            return;
        };
        self.drawing = Some((page, width));
        let opening = self.openings;
        cx.spawn(async move |this, cx| {
            let bitmap = cx
                .background_spawn(async move { document.render(page, width) })
                .await;
            this.update(cx, |this, cx| {
                if this.openings != opening {
                    return;
                }
                this.drawing = None;
                for image in this.retired.drain(..) {
                    cx.drop_image(image, None);
                }
                let drawn = bitmap.ok().and_then(|bitmap| {
                    let bytes = bitmap.bgra.len();
                    image_of(bitmap).map(|image| Drawn {
                        width,
                        bytes,
                        image,
                    })
                });
                match drawn {
                    Some(drawn) => {
                        if let Some(before) = this.drawn.insert(page, drawn) {
                            this.retired.push(before.image);
                        }
                    }
                    None => {
                        this.unreadable.insert((page, width));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Lets go of drawings of pages far from view, and of those out of view, farthest first,
    /// past the memory they may keep.
    fn forget_far(&mut self, cx: &mut Context<Self>) {
        let visible = self.visible();
        let wanted = visible.start.saturating_sub(AHEAD)..visible.end + AHEAD;
        let kept = visible.start.saturating_sub(2 * AHEAD)..visible.end + 2 * AHEAD;
        let distance = |page: usize| {
            page.saturating_sub(visible.end)
                .max(visible.start.saturating_sub(page))
        };
        let mut out_of_view: Vec<(usize, usize)> = self
            .drawn
            .iter()
            .filter(|(page, _)| !wanted.contains(*page))
            .map(|(page, drawn)| (*page, drawn.bytes))
            .collect();
        out_of_view.sort_by_key(|(page, _)| distance(*page));
        let mut budget = MAX_KEPT_BYTES;
        let far: Vec<usize> = out_of_view
            .into_iter()
            .filter(|(page, bytes)| {
                let fits = kept.contains(page) && *bytes <= budget;
                if fits {
                    budget -= bytes;
                }
                !fits
            })
            .map(|(page, _)| page)
            .collect();
        for page in far {
            if let Some(drawn) = self.drawn.remove(&page) {
                cx.drop_image(drawn.image, None);
            }
        }
    }

    fn render_page(&mut self, index: usize, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (Some(document), Some(scale)) = (self.document(), self.pixels_per_point()) else {
            return div().into_any_element();
        };
        let count = document.pages().len();
        let Some(page) = document.pages().get(index) else {
            return div().into_any_element();
        };
        let colors = cx.hoshi();
        let (width, height) = (page.width * scale, page.height * scale);
        let highlights: Vec<AnyElement> = self
            .matches
            .iter()
            .enumerate()
            .filter(|(_, found)| found.page == index)
            .flat_map(|(at, found)| found.areas.iter().map(move |area| (at, *area)))
            .map(|(at, area)| {
                let strength = if at == self.current { 0.55 } else { 0.25 };
                div()
                    .absolute()
                    .left(px(area.left * width - 1.))
                    .top(px(area.top * height - 1.))
                    .w(px(area.width * width + 2.))
                    .h(px(area.height * height + 2.))
                    .rounded_sm()
                    .bg(colors.warning.opacity(strength))
                    .into_any_element()
            })
            .collect();
        let image = self.drawn.get(&index).map(|drawn| drawn.image.clone());
        div()
            .w_full()
            .flex()
            .justify_center()
            .pt(px(GAP))
            .when(index + 1 == count, |item| item.pb(px(GAP)))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .w(px(width))
                    .h(px(height))
                    .bg(colors.pdf_page)
                    .shadow_sm()
                    .children(image.map(|image| img(image).size_full()))
                    .children(highlights),
            )
            .into_any_element()
    }

    fn page_bar(&self, colors: &Colors, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let count = self.document().map_or(0, |document| document.pages().len());
        let page = self.current_page();
        let percent = self
            .pixels_per_point()
            .map_or(100, |scale| (scale / POINT * 100.).round() as u32);
        let ready = count > 0;
        let icon_button = |id: &'static str, icon: IconName, label: String, enabled: bool| {
            Button::new(id)
                .icon(icon)
                .size(ButtonSize::IconSm)
                .variant(ButtonVariant::Ghost)
                .disabled(!enabled)
                .tooltip(Tooltip::new(label).side(Side::Bottom))
        };
        let found = (!self.query.is_empty() && self.searching.is_none()).then(|| {
            if self.matches.is_empty() {
                m::pdf_no_matches()
            } else {
                m::pdf_match_position(self.current + 1, self.matches.len())
            }
        });
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(colors.border)
            .text_xs()
            .child(
                div()
                    .flex_none()
                    .px_1()
                    .text_color(colors.muted_foreground)
                    .when(ready, |label| label.child(m::pdf_pages(page + 1, count))),
            )
            .child(
                icon_button(
                    "pdf-previous-page",
                    IconName::ChevronUp,
                    m::pdf_previous_page(),
                    ready && page > 0,
                )
                .on_click(
                    cx.listener(move |this, _, _, cx| this.go_to_page(page.saturating_sub(1), cx)),
                ),
            )
            .child(
                icon_button(
                    "pdf-next-page",
                    IconName::ChevronDown,
                    m::pdf_next_page(),
                    ready && page + 1 < count,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.go_to_page(page + 1, cx))),
            )
            .child(div().flex_1())
            .child(
                icon_button("pdf-zoom-out", IconName::Minus, m::pdf_zoom_out(), ready)
                    .on_click(cx.listener(|this, _, _, cx| this.zoom_by(false, cx))),
            )
            .child(
                div()
                    .flex_none()
                    .min_w(px(36.))
                    .text_center()
                    .text_color(colors.muted_foreground)
                    .child(m::pdf_zoom(percent)),
            )
            .child(
                icon_button("pdf-zoom-in", IconName::Plus, m::pdf_zoom_in(), ready)
                    .on_click(cx.listener(|this, _, _, cx| this.zoom_by(true, cx))),
            )
            .child(
                Button::new("pdf-fit-width")
                    .label(m::pdf_fit_width())
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Ghost)
                    .disabled(!ready)
                    .selected(self.zoom == Zoom::Fit)
                    .on_click(cx.listener(|this, _, _, cx| this.set_zoom(Zoom::Fit, cx))),
            )
            .child(div().flex_none().w_px().h(px(14.)).mx_1().bg(colors.border))
            .child(
                div().flex_none().w(px(200.)).child(
                    text_input(&self.find)
                        .small()
                        .prefix(
                            Icon::new(IconName::Search)
                                .size(px(14.))
                                .text_color(colors.muted_foreground),
                        )
                        .when_some(found, |input, found| {
                            input.suffix(
                                div()
                                    .text_xs()
                                    .text_color(colors.muted_foreground)
                                    .child(found),
                            )
                        })
                        .disabled(!ready),
                ),
            )
    }
}

impl Render for PdfSurface {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        self.draw_next(cx);
        self.forget_far(cx);
        let (viewport, scale_factor) = (self.viewport, self.scale_factor);
        let this = cx.weak_entity();
        let measure = canvas(
            move |bounds, window, cx| {
                let scale = window.scale_factor();
                if viewport != Some(bounds.size) || scale_factor != scale {
                    cx.defer(move |cx| {
                        this.update(cx, |this, cx| this.set_viewport(bounds.size, scale, cx))
                            .ok();
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let body = match &self.body {
            Body::Loading { download } => div()
                .flex_1()
                .flex()
                .flex_col()
                .gap_2()
                .items_center()
                .justify_center()
                .child(spinner(px(14.), colors.muted_foreground))
                .when(*download, |body| {
                    body.child(panel_message(m::pdf_downloading(), &colors))
                })
                .into_any_element(),
            Body::Failed(failure) => {
                let text = match failure {
                    PdfFailure::Download => m::pdf_download_failed(),
                    PdfFailure::Unsupported => m::pdf_unsupported(),
                    PdfFailure::Password => m::pdf_password(),
                    PdfFailure::Library | PdfFailure::Read => m::file_read_failed(),
                };
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(panel_message(text, &colors))
                    .when(*failure == PdfFailure::Download, |body| {
                        body.child(
                            Button::new("pdf-retry")
                                .label(m::pdf_retry())
                                .size(ButtonSize::Xs)
                                .variant(ButtonVariant::Outline)
                                .on_click(cx.listener(|this, _, _, cx| this.open(cx))),
                        )
                    })
                    .into_any_element()
            }
            Body::Ready(document) => {
                let widest = self.pixels_per_point().map_or(0., |scale| {
                    document
                        .pages()
                        .iter()
                        .map(|page| page.width * scale)
                        .fold(0., f32::max)
                });
                let content = self
                    .viewport
                    .map_or(px(0.), |viewport| viewport.width.max(px(widest + 2. * GAP)));
                div()
                    .id("pdf-pages")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .bg(colors.muted)
                    .overflow_x_scroll()
                    .child(measure)
                    .child(div().h_full().w(content).child(
                        list(self.list.clone(), cx.processor(Self::render_page)).size_full(),
                    ))
                    .into_any_element()
            }
        };
        let bar = path_bar(&self.path, &colors, Vec::new());
        div()
            .debug_selector(|| "pdf-surface".into())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(bar)
            .child(self.page_bar(&colors, cx))
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GAP, POINT, Zoom, draw_order, next_match, page_at_top, pages_in_view, pixels_per_point,
        stepped,
    };

    #[test]
    fn fitting_takes_the_widest_page_to_the_view_s_width() {
        let scale = pixels_per_point(Zoom::Fit, 612., 612. + 2. * GAP);
        assert!((scale - 1.).abs() < 1e-4);
        assert_eq!(pixels_per_point(Zoom::Scale(2.), 612., 100.), 2. * POINT);
    }

    #[test]
    fn zoom_steps_go_to_the_next_stop_and_stay_within_them() {
        assert_eq!(stepped(1., true), 1.25);
        assert_eq!(stepped(1., false), 0.75);
        assert_eq!(stepped(0.9, true), 1.);
        assert_eq!(stepped(0.9, false), 0.75);
        assert_eq!(stepped(4., true), 4.);
        assert_eq!(stepped(0.25, false), 0.25);
    }

    #[test]
    fn matches_go_round_both_ways() {
        assert_eq!(next_match(2, 3, true), 0);
        assert_eq!(next_match(0, 3, false), 2);
        assert_eq!(next_match(0, 0, true), 0);
    }

    #[test]
    fn the_pages_in_view_run_until_the_view_is_filled() {
        let heights = [100., 100., 100., 100.];
        assert_eq!(pages_in_view(heights, 0, 0., 150.), 0..2);
        assert_eq!(pages_in_view(heights, 1, 60., 150.), 1..4);
        assert_eq!(pages_in_view(heights, 3, 0., 500.), 3..4);
        assert_eq!(pages_in_view([], 0, 0., 500.), 0..0);
    }

    #[test]
    fn the_current_page_turns_once_most_of_the_top_one_went_by() {
        let heights = [100., 100., 100.];
        assert_eq!(page_at_top(&heights, 0, 40.), 0);
        assert_eq!(page_at_top(&heights, 0, 60.), 1);
        assert_eq!(page_at_top(&heights, 2, 90.), 2);
        assert_eq!(page_at_top(&heights, 7, 0.), 2);
        assert_eq!(page_at_top(&[], 0, 0.), 0);
    }

    #[test]
    fn pages_in_view_are_drawn_before_those_around_them() {
        assert_eq!(draw_order(4..6, 10).collect::<Vec<_>>(), [4, 5, 6, 3]);
        assert_eq!(draw_order(0..1, 1).collect::<Vec<_>>(), [0]);
    }
}
