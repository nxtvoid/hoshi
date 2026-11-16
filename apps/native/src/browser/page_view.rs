use super::{Browsers, key_input, page_bounds, page_point, page_scroll};
use gpui_kit::*;
use hoshi_core::BrowserInput;
use hoshi_ui::prelude::*;
use std::time::Duration;

/// Wheel turns closer than this go to the page as one.
const WHEEL_BATCH: Duration = Duration::from_millis(16);
const AGENT_MARKER: f32 = 32.;

/// How the frame takes the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageFit {
    /// Across the whole width when the page fills the view, else whole and centered.
    Engine,
    /// Always across the whole width, from the top.
    Width,
}

/// The view's bounds in the window changed.
pub struct ViewportChanged;

struct Wheel {
    point: (f64, f64),
    delta: (f64, f64),
}

/// The thread's page as it last painted, which takes clicks, scrolls and keys. It streams
/// while viewed.
pub struct PageView {
    thread_id: String,
    browsers: Entity<Browsers>,
    fit: PageFit,
    selector: &'static str,
    radius: Pixels,
    scrolls: bool,
    focus: FocusHandle,
    viewing: bool,
    viewport: Option<Bounds<Pixels>>,
    /// The window's scale, which the page paints at.
    scale: Option<f32>,
    pressed: bool,
    wheel: Option<Wheel>,
    _subscriptions: [Subscription; 2],
}

impl EventEmitter<ViewportChanged> for PageView {}

impl PageView {
    pub fn new(
        thread_id: String,
        browsers: Entity<Browsers>,
        fit: PageFit,
        selector: &'static str,
        cx: &mut Context<Self>,
    ) -> Self {
        let changes = cx.observe(&browsers, |_, _, cx| cx.notify());
        let release = cx.on_release(|this, cx| {
            if this.viewing {
                let thread_id = this.thread_id.clone();
                this.browsers
                    .update(cx, |browsers, _| browsers.view(&thread_id, false));
            }
        });
        browsers.update(cx, |browsers, _| browsers.view(&thread_id, true));
        Self {
            thread_id,
            browsers,
            fit,
            selector,
            radius: px(0.),
            scrolls: true,
            focus: cx.focus_handle(),
            viewing: true,
            viewport: None,
            scale: None,
            pressed: false,
            wheel: None,
            _subscriptions: [changes, release],
        }
    }

    /// Rounds the frame's corners.
    pub fn set_radius(&mut self, radius: Pixels) {
        self.radius = radius;
    }

    /// Whether the wheel scrolls the page; when not, it scrolls what holds the view.
    pub fn set_scrolls(&mut self, scrolls: bool) {
        self.scrolls = scrolls;
    }

    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    /// Starts or stops streaming the page to this view.
    pub fn set_viewing(&mut self, viewing: bool, cx: &mut Context<Self>) {
        if self.viewing == viewing {
            return;
        }
        self.viewing = viewing;
        let thread_id = self.thread_id.clone();
        self.browsers
            .update(cx, |browsers, _| browsers.view(&thread_id, viewing));
    }

    pub fn viewport(&self) -> Option<Bounds<Pixels>> {
        self.viewport
    }

    fn fill(&self, cx: &App) -> bool {
        match self.fit {
            PageFit::Width => true,
            PageFit::Engine => self
                .browsers
                .read(cx)
                .page(&self.thread_id)
                .and_then(|page| page.fitted)
                .unwrap_or(true),
        }
    }

    fn set_viewport(&mut self, bounds: Bounds<Pixels>, scale: f32, cx: &mut Context<Self>) {
        if self.scale != Some(scale) {
            self.scale = Some(scale);
            let thread_id = self.thread_id.clone();
            self.browsers
                .update(cx, |browsers, _| browsers.scale(&thread_id, scale));
        }
        if self.viewport != Some(bounds) {
            self.viewport = Some(bounds);
            cx.emit(ViewportChanged);
            cx.notify();
        }
    }

    /// Where the frame shows in the window, while there is one.
    fn shown_page(&self, cx: &App) -> Option<(Bounds<Pixels>, u32)> {
        let viewport = self.viewport?;
        let frame = self.browsers.read(cx).frame(&self.thread_id)?;
        let size = (frame.width, frame.height);
        Some((page_bounds(viewport, size, self.fill(cx)), frame.width))
    }

    fn page_point(&self, position: Point<Pixels>, cx: &App) -> Option<(f64, f64)> {
        let (shown, frame_width) = self.shown_page(cx)?;
        page_point(shown, frame_width, position)
    }

    fn send(&self, input: BrowserInput, cx: &App) {
        self.browsers.read(cx).input(&self.thread_id, input);
    }

    fn scroll(&mut self, point: (f64, f64), delta: (f64, f64), cx: &mut Context<Self>) {
        let batching = self.wheel.is_some();
        let before = self.wheel.take().map_or((0., 0.), |wheel| wheel.delta);
        self.wheel = Some(Wheel {
            point,
            delta: (before.0 + delta.0, before.1 + delta.1),
        });
        if batching {
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(WHEEL_BATCH).await;
            this.update(cx, |this, cx| {
                if let Some(Wheel { point, delta }) = this.wheel.take() {
                    this.send(
                        BrowserInput::Scroll {
                            x: point.0,
                            y: point.1,
                            delta_x: delta.0,
                            delta_y: delta.1,
                        },
                        cx,
                    );
                }
            })
            .ok();
        })
        .detach();
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.browsers.read(cx).page(&self.thread_id).is_none() {
            return;
        }
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let paste = (modifiers.control || modifiers.platform)
            && !modifiers.alt
            && keystroke.key.eq_ignore_ascii_case("v");
        let input = if paste {
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .filter(|text| !text.is_empty())
                .map(|text| BrowserInput::Text { text })
        } else {
            key_input(keystroke)
        };
        if let Some(input) = input {
            self.send(input, cx);
            cx.stop_propagation();
        }
    }
}

impl Render for PageView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let browsers = self.browsers.read(cx);
        let page = browsers.page(&self.thread_id);
        let agent_point = page
            .and_then(|page| page.page.agent_point)
            .filter(|_| browsers.agent_active(&self.thread_id));
        let image = page
            .and(browsers.frame(&self.thread_id))
            .map(|frame| (frame.image.clone(), frame.width));
        let raised = !self.fill(cx);
        let shown = self.shown_page(cx);
        let (viewport, shown_scale) = (self.viewport, self.scale);

        let this = cx.weak_entity();
        let measure = canvas(
            move |bounds, window, cx| {
                let scale = window.scale_factor();
                if viewport != Some(bounds) || shown_scale != Some(scale) {
                    cx.defer(move |cx| {
                        this.update(cx, |this, cx| this.set_viewport(bounds, scale, cx))
                            .ok();
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        let page_view = match (image, shown, viewport) {
            (Some((image, frame_width)), Some((shown, _)), Some(viewport)) => {
                let offset = shown.origin - viewport.origin;
                let marker = agent_point.map(|point| {
                    let scale = f32::from(shown.size.width) / frame_width as f32;
                    let half = AGENT_MARKER / 2.;
                    div()
                        .absolute()
                        .left(offset.x + px(point.x as f32 * scale - half))
                        .top(offset.y + px(point.y as f32 * scale - half))
                        .size(px(AGENT_MARKER))
                        .rounded_full()
                        .border_2()
                        .border_color(colors.primary)
                });
                Some(
                    div()
                        .absolute()
                        .left(offset.x)
                        .top(offset.y)
                        .w(shown.size.width)
                        .h(shown.size.height)
                        .when(raised, |page| page.shadow_sm())
                        .child(img(image).size_full().rounded(self.radius))
                        .into_any_element(),
                )
                .into_iter()
                .chain(marker.map(IntoElement::into_any_element))
                .collect::<Vec<_>>()
            }
            _ => Vec::new(),
        };

        let selector = self.selector;
        div()
            .id(selector)
            .debug_selector(move || selector.into())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.focus, cx);
                    this.pressed = true;
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    if !std::mem::take(&mut this.pressed) {
                        return;
                    }
                    if let Some((x, y)) = this.page_point(event.position, cx) {
                        this.send(BrowserInput::Click { x, y }, cx);
                    }
                }),
            )
            .on_scroll(cx.listener(|this, event: &ScrollEvent, window, cx| {
                if !this.scrolls {
                    return;
                }
                let Some(point) = this.page_point(event.position, cx) else {
                    return;
                };
                let delta = page_scroll(event.delta.pixel_delta(window.line_height()));
                this.scroll(point, delta, cx);
                cx.stop_propagation();
            }))
            .relative()
            .size_full()
            .overflow_hidden()
            .child(measure)
            .children(page_view)
    }
}
