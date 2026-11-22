use super::layout::{self, Canvas, DetailsCard, EDGE_GAP, Frame, Obstacles, ResizeDirection};
use super::{Browsers, PageFit, PageView};
use crate::m;
use crate::panel::RightPanel;
use crate::state::Workspace;
use gpui_kit::*;
use hoshi_ui::LaneRoom;
use hoshi_ui::prelude::*;
use std::time::Duration;

const DEFAULT_PAGE: layout::Size = layout::Size {
    width: 1280.,
    height: 800.,
};
/// How far a press on the handle moves before it counts as a drag.
const TAP_SLOP: f64 = 6.;
const PILL_INSET: f32 = 8.;
const RADIUS: f32 = 12.;

/// What the floating browser keeps clear of inside the chat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Obstacle {
    /// Goes right above the composer, in its column, taking no room.
    Composer,
    /// Goes inside the details card, filling it.
    Details,
}

/// Where the obstacles painted this frame; cleared as the player renders, filled as they
/// paint, read as the player paints after them.
#[derive(Default)]
struct ChatObstacles {
    composer: Option<Bounds<Pixels>>,
    details: Option<Bounds<Pixels>>,
}

impl Global for ChatObstacles {}

/// Marks an obstacle of the floating browser.
pub fn obstacle(kind: Obstacle) -> impl IntoElement {
    let marker = canvas(
        move |bounds, _, cx| {
            let obstacles = cx.default_global::<ChatObstacles>();
            match kind {
                Obstacle::Composer => obstacles.composer = Some(bounds),
                Obstacle::Details => obstacles.details = Some(bounds),
            }
        },
        |_, _, _, _| {},
    );
    match kind {
        Obstacle::Composer => marker.w_full().h_0().flex_none(),
        Obstacle::Details => marker.absolute().top_0().left_0().size_full(),
    }
}

/// The chat's size and what lies over it, in its own coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ChatLayout {
    container: layout::Size,
    composer_height: f64,
    details_card: Option<DetailsCard>,
}

impl ChatLayout {
    fn measure(chat: Bounds<Pixels>, obstacles: &ChatObstacles) -> Self {
        let f = |pixels: Pixels| f64::from(f32::from(pixels));
        Self {
            container: layout::Size {
                width: f(chat.size.width),
                height: f(chat.size.height),
            },
            composer_height: obstacles
                .composer
                .filter(|composer| composer.size.width > px(0.))
                .map_or(0., |composer| f(chat.bottom() - composer.top())),
            details_card: obstacles
                .details
                .filter(|details| details.size.width > px(0.))
                .map(|details| DetailsCard {
                    left: f(details.left() - chat.left()),
                    right: f(details.right() - chat.left()),
                    bottom: f(details.bottom() - chat.top()),
                }),
        }
    }
}

struct Gesture {
    start: Point<Pixels>,
    frame: Frame,
    direction: Option<ResizeDirection>,
    moved: bool,
}

/// The open thread's browser page floating over the chat, where it opens when the agent
/// opens one: moved by its handle, resized by its edges, and sent to the side panel from
/// there.
pub struct MiniBrowser {
    workspace: Entity<Workspace>,
    panel: Entity<RightPanel>,
    browsers: Entity<Browsers>,
    /// While it shows; the page streams as long as it exists.
    page: Option<Entity<PageView>>,
    chat: ChatLayout,
    canvas: Option<Canvas>,
    room: Option<LaneRoom>,
    gesture: Option<Gesture>,
    handle_hovered: bool,
    /// `HOSHI_NATIVE_MINI`: an address opened floating once a thread is open.
    dev_page: Option<String>,
    _subscriptions: [Subscription; 3],
}

impl MiniBrowser {
    pub fn new(
        workspace: Entity<Workspace>,
        panel: Entity<RightPanel>,
        cx: &mut Context<Self>,
    ) -> Self {
        let browsers = workspace.read(cx).browsers.clone();
        let subscriptions = [
            cx.observe(&workspace, |this, _, cx| this.sync(cx)),
            cx.observe(&panel, |this, _, cx| this.sync(cx)),
            cx.observe(&browsers, |this, _, cx| this.sync(cx)),
        ];
        let mut this = Self {
            workspace,
            panel,
            browsers,
            page: None,
            chat: ChatLayout::default(),
            canvas: None,
            room: None,
            gesture: None,
            handle_hovered: false,
            dev_page: std::env::var("HOSHI_NATIVE_MINI").ok(),
            _subscriptions: subscriptions,
        };
        this.sync(cx);
        this
    }

    /// What the chat column leaves the player while it moves the chat aside.
    pub fn lane_room(&self) -> Option<LaneRoom> {
        self.room
    }

    fn shown_thread(&self, cx: &App) -> Option<String> {
        let thread_id = self.workspace.read(cx).open_thread_id(cx)?;
        let browsers = self.browsers.read(cx);
        let panel = self.panel.read(cx);
        let floats = browsers.mini(&thread_id).is_some() && browsers.page(&thread_id).is_some();
        let hidden = panel.shows_browser(&thread_id) || panel.is_maximized(&thread_id);
        (floats && !hidden).then_some(thread_id)
    }

    fn source(&self, thread_id: &str, cx: &App) -> layout::Size {
        self.browsers
            .read(cx)
            .frame(thread_id)
            .filter(|frame| frame.width > 0 && frame.height > 0)
            .map_or(DEFAULT_PAGE, |frame| layout::Size {
                width: frame.width.into(),
                height: frame.height.into(),
            })
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        if let Some(address) = self.dev_page.take() {
            match self.workspace.read(cx).open_thread_id(cx) {
                Some(thread_id) => self.browsers.update(cx, |browsers, cx| {
                    browsers.open(&thread_id, &address, cx);
                    browsers.open_mini(&thread_id, cx);
                }),
                None => self.dev_page = Some(address),
            }
        }
        let shown = self.shown_thread(cx);
        match (&shown, &self.page) {
            (Some(id), Some(page)) if page.read(cx).thread_id() == id => {}
            (Some(id), _) => {
                let (id, browsers) = (id.clone(), self.browsers.clone());
                self.page = Some(cx.new(|cx| {
                    let mut page =
                        PageView::new(id, browsers, PageFit::Width, "mini-browser-page", cx);
                    page.set_radius(px(RADIUS));
                    page
                }));
            }
            (None, _) => {
                self.page = None;
                self.gesture = None;
                self.handle_hovered = false;
            }
        }
        let container = self.chat.container;
        self.canvas = shown.as_deref().and_then(|id| {
            let placement = self.browsers.read(cx).mini(id)?;
            (container.width > 0.).then(|| {
                layout::resolve_canvas(
                    container,
                    placement,
                    self.source(id, cx),
                    self.chat.composer_height,
                    self.chat.details_card,
                )
            })
        });
        self.room = self
            .canvas
            .and_then(|canvas| layout::lane_room(&canvas, container))
            .map(|(left, right)| LaneRoom {
                right: px(right as f32),
                gutter: px(left as f32),
            });
        cx.notify();
    }

    fn set_chat(&mut self, chat: ChatLayout, cx: &mut Context<Self>) {
        if self.chat != chat {
            self.chat = chat;
            self.sync(cx);
        }
    }

    fn begin(
        &mut self,
        position: Point<Pixels>,
        direction: Option<ResizeDirection>,
        cx: &mut Context<Self>,
    ) {
        gpui_kit::base::GlobalState::suppress_text_selection(cx);
        cx.stop_propagation();
        let Some(canvas) = self.canvas else {
            return;
        };
        self.gesture = Some(Gesture {
            start: position,
            frame: canvas.frame,
            direction,
            moved: false,
        });
        cx.notify();
    }

    fn follow(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(thread_id) = self
            .page
            .as_ref()
            .map(|page| page.read(cx).thread_id().to_owned())
        else {
            return;
        };
        let source = self.source(&thread_id, cx);
        let container = self.chat.container;
        let Some(gesture) = &mut self.gesture else {
            return;
        };
        let delta = layout::Point {
            x: f64::from(f32::from(position.x - gesture.start.x)),
            y: f64::from(f32::from(position.y - gesture.start.y)),
        };
        if !gesture.moved && delta.x.hypot(delta.y) < TAP_SLOP {
            return;
        }
        gesture.moved = true;
        let start = gesture.frame;
        match gesture.direction {
            None => {
                let position = layout::clamp_position(
                    layout::Point {
                        x: start.x + delta.x,
                        y: start.y + delta.y,
                    },
                    container,
                    start.size(),
                    Obstacles::default(),
                    EDGE_GAP,
                );
                self.browsers.update(cx, |browsers, cx| {
                    browsers.move_mini(&thread_id, position, cx)
                });
            }
            Some(direction) => {
                let next = layout::resize_frame(
                    start,
                    direction,
                    delta,
                    source,
                    container,
                    Obstacles::default(),
                );
                self.browsers.update(cx, |browsers, cx| {
                    browsers.resize_mini(&thread_id, next.width, next.position(), cx)
                });
            }
        }
    }

    fn end(&mut self, cx: &mut Context<Self>) {
        if self.gesture.take().is_some() {
            cx.notify();
        }
    }

    fn handle(&self, thread_id: &str, colors: &Colors, cx: &mut Context<Self>) -> Stateful<Div> {
        let agent_active = self.browsers.read(cx).agent_active(thread_id);
        let handle = div()
            .id("mini-browser-handle")
            .debug_selector(|| "mini-browser-handle".into())
            .absolute()
            .top(px(PILL_INSET))
            .right(px(PILL_INSET))
            .cursor(if self.gesture.is_some() {
                CursorStyle::ClosedHand
            } else {
                CursorStyle::OpenHand
            })
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.handle_hovered = *hovered;
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.begin(event.position, None, cx)
                }),
            );
        if !self.handle_hovered {
            let dot = div()
                .absolute()
                .top_0()
                .right_0()
                .size_2()
                .rounded_full()
                .shadow_sm()
                .border_1()
                .border_color(colors.background.opacity(0.7))
                .bg(if agent_active {
                    colors.primary
                } else {
                    colors.foreground.opacity(0.25)
                });
            let dot = if agent_active {
                dot.with_animation(
                    "mini-browser-pulse",
                    hoshi_ui::looping(Duration::from_secs(2))
                        .with_easing(pulsating_between(0.5, 1.)),
                    |dot, delta| dot.opacity(delta),
                )
                .into_any_element()
            } else {
                dot.into_any_element()
            };
            return handle.size_3().child(dot);
        }
        let thread = thread_id.to_owned();
        let docked = thread.clone();
        let buttons = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                Button::new("mini-browser-open-in-panel")
                    .icon(IconName::PanelRight)
                    .size(ButtonSize::IconSm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip(Tooltip::new(m::mini_browser_open_in_panel()).side(Side::Top))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.browsers
                            .update(cx, |browsers, cx| browsers.dock(&docked, cx));
                    })),
            )
            .child(
                Button::new("mini-browser-close")
                    .icon(IconName::X)
                    .size(ButtonSize::IconSm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip(Tooltip::new(m::mini_browser_close()).side(Side::Top))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.browsers
                            .update(cx, |browsers, cx| browsers.close_mini(&thread, cx));
                    })),
            );
        handle.child(
            div()
                .flex()
                .items_center()
                .p(px(2.))
                .rounded_lg()
                .border_1()
                .border_color(colors.border)
                .bg(colors.popover.opacity(0.9))
                .shadow_lg()
                .child(buttons),
        )
    }

    fn edges(&self, cx: &mut Context<Self>) -> Vec<Div> {
        ResizeDirection::ALL
            .into_iter()
            .map(|direction| {
                let edge = div().absolute().occlude().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.begin(event.position, Some(direction), cx)
                    }),
                );
                let (edge, cursor) = match direction {
                    ResizeDirection::North => (
                        edge.left_0().right_0().top(px(-4.)).h_2(),
                        CursorStyle::ResizeUpDown,
                    ),
                    ResizeDirection::South => (
                        edge.left_0().right_0().bottom(px(-4.)).h_2(),
                        CursorStyle::ResizeUpDown,
                    ),
                    ResizeDirection::West => (
                        edge.top_0().bottom_0().left(px(-4.)).w_2(),
                        CursorStyle::ResizeLeftRight,
                    ),
                    ResizeDirection::East => (
                        edge.top_0().bottom_0().right(px(-4.)).w_2(),
                        CursorStyle::ResizeLeftRight,
                    ),
                    ResizeDirection::Northwest => (
                        edge.top(px(-8.)).left(px(-8.)).size_4(),
                        CursorStyle::ResizeUpLeftDownRight,
                    ),
                    ResizeDirection::Northeast => (
                        edge.top(px(-8.)).right(px(-8.)).size_4(),
                        CursorStyle::ResizeUpRightDownLeft,
                    ),
                    ResizeDirection::Southwest => (
                        edge.bottom(px(-8.)).left(px(-8.)).size_4(),
                        CursorStyle::ResizeUpRightDownLeft,
                    ),
                    ResizeDirection::Southeast => (
                        edge.bottom(px(-8.)).right(px(-8.)).size_4(),
                        CursorStyle::ResizeUpLeftDownRight,
                    ),
                };
                edge.cursor(cursor)
            })
            .collect()
    }

    /// Follows the pointer anywhere in the window while a move or resize lasts.
    fn tracking(cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let moving = this.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble && event.dragging() {
                        moving
                            .update(cx, |this, cx| this.follow(event.position, cx))
                            .ok();
                    }
                });
                let ending = this.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble {
                        ending.update(cx, |this, cx| this.end(cx)).ok();
                    }
                });
            },
        )
        .absolute()
        .size_0()
    }
}

impl Render for MiniBrowser {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        cx.set_global(ChatObstacles::default());
        let this = cx.weak_entity();
        let chat = self.chat;
        let measure = canvas(
            move |bounds, _, cx| {
                let next = ChatLayout::measure(bounds, cx.default_global::<ChatObstacles>());
                if next != chat {
                    cx.defer(move |cx| {
                        this.update(cx, |this, cx| this.set_chat(next, cx)).ok();
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        let player = match (&self.page, self.canvas) {
            (Some(page), Some(canvas)) => {
                let colors = cx.hoshi().clone();
                let frame = canvas.frame;
                let thread_id = page.read(cx).thread_id().to_owned();
                let handle = self.handle(&thread_id, &colors, cx);
                let edges = self.edges(cx);
                let tracking = self.gesture.is_some().then(|| Self::tracking(cx));
                Some(
                    div()
                        .id("mini-browser")
                        .debug_selector(|| "mini-browser".into())
                        .occlude()
                        .absolute()
                        .left(px(frame.x as f32))
                        .top(px(frame.y as f32))
                        .w(px(frame.width as f32))
                        .h(px(frame.height as f32))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                            gpui_kit::base::GlobalState::suppress_text_selection(cx)
                        })
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full()
                                .overflow_hidden()
                                .rounded(px(RADIUS))
                                .bg(colors.muted)
                                .shadow_2xl()
                                .child(page.clone()),
                        )
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full()
                                .rounded(px(RADIUS))
                                .border_1()
                                .border_color(colors.border),
                        )
                        .child(handle)
                        .children(edges)
                        .children(tracking),
                )
            }
            _ => None,
        };

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(measure)
            .children(player)
    }
}

#[cfg(test)]
mod tests;
