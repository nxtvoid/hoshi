//! A popup anchored to a trigger, with Base UI's Popover behavior, and its Menu content.
//!
//! The popup is drawn above everything, placed against the trigger every frame (flipped,
//! shifted and bounded by the window), closed by an outside press, Escape, an item or the
//! keyboard leaving, and it hands the keyboard back to the trigger when it closes.

mod host;
mod menu;
mod navigation;
mod position;
mod state;

pub use menu::{MenuOption, PopoverMenu};

use menu::MenuBuilder;

use crate::components::PressSelectsNothing as _;
use crate::components::placement::{Align, Placement, Side};
use crate::theme::ActiveTheme;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use host::PopupHost;
use state::{AnchorSnapshot, Config, Edge, Hover, Kind, Method, OpenPopovers, PopoverState};
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Another element a popup is placed against instead of its trigger, like Base UI's `anchor`:
/// put [`PopoverAnchor::area`] inside that element and pass the anchor to [`Popover::anchor`].
#[derive(Clone, Default)]
pub struct PopoverAnchor(Rc<Cell<Option<Bounds<Pixels>>>>);

impl PopoverAnchor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Measures the element it is a child of, which it covers.
    pub fn area(&self) -> impl IntoElement {
        let bounds = self.0.clone();
        canvas(
            move |measured, _, _| bounds.set(Some(measured)),
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
}

/// Above dialogs (`DIALOG_PRIORITY`, 50), below tooltips (`TOOLTIP_PRIORITY`, 200).
pub const POPOVER_PRIORITY: usize = gpui_kit::base::POPUP_PRIORITY;

const ENTER: Duration = Duration::from_millis(150);
const POPUP_CONTEXT: &str = "HoshiPopover";
const MENU_CONTEXT: &str = "HoshiMenu";
const TRIGGER_CONTEXT: &str = "HoshiPopoverTrigger";

gpui_kit::actions!(
    hoshi_popover,
    [
        PopupTab,
        PopupTabBack,
        TriggerDown,
        TriggerUp,
        TriggerTab,
        MenuDown,
        MenuUp,
        MenuHome,
        MenuEnd,
        MenuEnter,
        MenuSpace,
        MenuRight,
        MenuLeft,
    ]
);

struct KeysBound;

impl Global for KeysBound {}

/// Binds the popover's keys. Popovers call it themselves the first time they render.
pub fn init(cx: &mut App) {
    if cx.has_global::<KeysBound>() {
        return;
    }
    cx.set_global(KeysBound);
    cx.bind_keys([
        KeyBinding::new("tab", PopupTab, Some(POPUP_CONTEXT)),
        KeyBinding::new("shift-tab", PopupTabBack, Some(POPUP_CONTEXT)),
        KeyBinding::new("down", TriggerDown, Some(TRIGGER_CONTEXT)),
        KeyBinding::new("up", TriggerUp, Some(TRIGGER_CONTEXT)),
        KeyBinding::new("tab", TriggerTab, Some(TRIGGER_CONTEXT)),
        KeyBinding::new("down", MenuDown, Some(MENU_CONTEXT)),
        KeyBinding::new("up", MenuUp, Some(MENU_CONTEXT)),
        KeyBinding::new("home", MenuHome, Some(MENU_CONTEXT)),
        KeyBinding::new("end", MenuEnd, Some(MENU_CONTEXT)),
        KeyBinding::new("enter", MenuEnter, Some(MENU_CONTEXT)),
        KeyBinding::new("space", MenuSpace, Some(MENU_CONTEXT)),
        KeyBinding::new("right", MenuRight, Some(MENU_CONTEXT)),
        KeyBinding::new("left", MenuLeft, Some(MENU_CONTEXT)),
    ]);
}

/// Whether a popover's popup covers `position`: a press there is not on what is below it.
/// A dialog that closes on presses outside itself checks this first.
pub fn popover_at(position: Point<Pixels>, window: &Window, cx: &App) -> bool {
    OpenPopovers::any_at(window, position, cx)
}

/// Whether any popover is open in `window`.
pub fn any_popover_open(window: &Window, cx: &App) -> bool {
    OpenPopovers::any_open(window, cx)
}

/// Why a popover opens or closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopoverChangeReason {
    TriggerPress,
    TriggerHover,
    OutsidePress,
    EscapeKey,
    ItemPress,
    ClosePress,
    FocusOut,
    ListNavigation,
    /// The trigger stopped being drawn while the popup was open.
    TriggerGone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopoverChange {
    pub open: bool,
    pub reason: PopoverChangeReason,
}

type OpenChangeHandler = Rc<dyn Fn(&PopoverChange, &mut Window, &mut App)>;

/// Where the keyboard goes when a popover opens. Menus always take it into their list.
#[derive(Clone, Default)]
pub enum PopoverFocus {
    /// The popup itself, so Escape and Tab work from the start.
    #[default]
    Popup,
    /// A control inside the popup, such as a search field.
    Handle(FocusHandle),
    /// Nowhere: the keyboard stays where it was.
    None,
}

/// What the content of an open popover can do with it.
#[derive(Clone)]
pub struct PopoverContext {
    state: Entity<PopoverState>,
}

impl PopoverContext {
    /// Closes the popover, as its close button would.
    pub fn close(&self, window: &mut Window, cx: &mut App) {
        self.close_with(PopoverChangeReason::ClosePress, window, cx);
    }

    pub fn close_with(&self, reason: PopoverChangeReason, window: &mut Window, cx: &mut App) {
        self.state
            .update(cx, |state, cx| state.close(reason, window, cx));
    }
}

type TriggerBuilder = Box<dyn FnOnce(bool) -> AnyElement>;

type ContentBuilder = Box<dyn FnOnce(&PopoverContext, &mut Window, &mut App) -> AnyElement>;

enum Content {
    Plain(ContentBuilder),
    Menu(MenuBuilder),
}

/// A trigger and the popup it opens.
///
/// Controlled with [`Popover::open`] and [`Popover::on_open_change`], or left to itself with
/// [`Popover::default_open`]. Styling the popover (`.w_96()`, `.p_0()`) styles its popup.
#[derive(IntoElement)]
pub struct Popover {
    id: ElementId,
    open: Option<bool>,
    default_open: bool,
    on_open_change: Option<OpenChangeHandler>,
    side: Side,
    align: Option<Align>,
    side_offset: Pixels,
    align_offset: Pixels,
    collision_padding: Pixels,
    modal: Option<bool>,
    initial_focus: PopoverFocus,
    open_on_hover: bool,
    delay: Option<Duration>,
    close_delay: Duration,
    trigger: Option<TriggerBuilder>,
    content: Option<Content>,
    style: StyleRefinement,
    trigger_focusable: bool,
    return_focus: Option<FocusHandle>,
    parent_menu: Option<WeakEntity<PopoverState>>,
    focus_request: u64,
    trigger_width: bool,
    anchor: Option<PopoverAnchor>,
    focus_ring: Option<Pixels>,
}

impl Popover {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            open: None,
            default_open: false,
            on_open_change: None,
            side: Side::Bottom,
            align: None,
            side_offset: px(4.),
            align_offset: px(0.),
            collision_padding: px(5.),
            modal: None,
            initial_focus: PopoverFocus::Popup,
            open_on_hover: false,
            delay: None,
            close_delay: Duration::ZERO,
            trigger: None,
            content: None,
            style: StyleRefinement::default(),
            trigger_focusable: true,
            return_focus: None,
            parent_menu: None,
            focus_request: 0,
            trigger_width: false,
            anchor: None,
            focus_ring: None,
        }
    }

    /// Rings the trigger, whose corners have `radius`, while the keyboard is on it.
    pub fn focus_ring(mut self, radius: Pixels) -> Self {
        self.focus_ring = Some(radius);
        self
    }

    /// Makes the popover controlled: it is open exactly when `open` is.
    pub fn open(mut self, open: bool) -> Self {
        self.open = Some(open);
        self
    }

    /// Whether an uncontrolled popover starts open.
    pub fn default_open(mut self, open: bool) -> Self {
        self.default_open = open;
        self
    }

    /// Told when the popover asks to open or close, and why.
    pub fn on_open_change(
        mut self,
        handler: impl Fn(&PopoverChange, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_change = Some(Rc::new(handler));
        self
    }

    pub fn side(mut self, side: Side) -> Self {
        self.side = side;
        self
    }

    /// Center by default; a menu's default is the start.
    pub fn align(mut self, align: Align) -> Self {
        self.align = Some(align);
        self
    }

    pub fn side_offset(mut self, offset: Pixels) -> Self {
        self.side_offset = offset;
        self
    }

    pub fn align_offset(mut self, offset: Pixels) -> Self {
        self.align_offset = offset;
        self
    }

    pub fn collision_padding(mut self, padding: Pixels) -> Self {
        self.collision_padding = padding;
        self
    }

    /// A modal popup blocks the rest of the window: an outside press only closes it. Menus
    /// are modal by default, popovers are not.
    pub fn modal(mut self, modal: bool) -> Self {
        self.modal = Some(modal);
        self
    }

    pub fn initial_focus(mut self, focus: PopoverFocus) -> Self {
        self.initial_focus = focus;
        self
    }

    /// Opens while the pointer rests on the trigger, after [`Popover::delay`].
    pub fn open_on_hover(mut self, open_on_hover: bool) -> Self {
        self.open_on_hover = open_on_hover;
        self
    }

    /// How long the pointer rests on the trigger before it opens: 300 ms, 100 for a menu.
    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    /// How long a popover opened by hover stays after the pointer leaves.
    pub fn close_delay(mut self, delay: Duration) -> Self {
        self.close_delay = delay;
        self
    }

    /// Makes the popup at least as wide as the trigger, like a select's list.
    pub fn match_trigger_width(mut self) -> Self {
        self.trigger_width = true;
        self
    }

    /// Places the popup against another element than the trigger.
    pub fn anchor(mut self, anchor: PopoverAnchor) -> Self {
        self.anchor = Some(anchor);
        self
    }

    /// Builds the trigger; it gets whether the popover is open, to show it.
    pub fn trigger<E: IntoElement>(mut self, trigger: impl FnOnce(bool) -> E + 'static) -> Self {
        self.trigger = Some(Box::new(move |open| trigger(open).into_any_element()));
        self
    }

    /// Builds what the popup shows, only while it is open.
    pub fn content<E: IntoElement>(
        mut self,
        content: impl FnOnce(&PopoverContext, &mut Window, &mut App) -> E + 'static,
    ) -> Self {
        self.content = Some(Content::Plain(Box::new(move |popover, window, cx| {
            content(popover, window, cx).into_any_element()
        })));
        self
    }

    /// Makes the popup a menu: a list the arrows, Home, End and typing move through.
    pub fn menu(
        mut self,
        menu: impl FnOnce(PopoverMenu, &mut Window, &mut App) -> PopoverMenu + 'static,
    ) -> Self {
        self.content = Some(Content::Menu(Box::new(menu)));
        self
    }

    fn submenu_of(
        mut self,
        parent: WeakEntity<PopoverState>,
        parent_focus: FocusHandle,
        focus_request: u64,
    ) -> Self {
        self.parent_menu = Some(parent);
        self.return_focus = Some(parent_focus);
        self.trigger_focusable = false;
        self.focus_request = focus_request;
        self
    }
}

impl Styled for Popover {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

fn enter_progress(opened_at: Option<Instant>, window: &Window, cx: &App) -> f32 {
    let Some(at) = opened_at else {
        return 1.;
    };
    if cx.reduce_motion() {
        return 1.;
    }
    let t = at.elapsed().as_secs_f32() / ENTER.as_secs_f32();
    if t >= 1. {
        return 1.;
    }
    window.request_animation_frame();
    1. - (1. - t).powi(3)
}

impl RenderOnce for Popover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        init(cx);
        let kind = match self.content {
            Some(Content::Menu(_)) => Kind::Menu,
            _ => Kind::Popover,
        };
        let submenu = self.parent_menu.is_some();
        let modal = self.modal.unwrap_or(kind == Kind::Menu && !submenu);
        let controlled = self.open;
        let default_open = self.default_open;
        let state = window.use_keyed_state(self.id.clone(), cx, |window, cx| {
            PopoverState::new(controlled.is_none() && default_open, window, cx)
        });
        if state.read(cx).release.is_none() {
            let release = window.observe_release(&state, cx, |state, window, cx| {
                if state.open
                    && let Some(callback) = state.on_open_change.clone()
                {
                    let change = PopoverChange {
                        open: false,
                        reason: PopoverChangeReason::TriggerGone,
                    };
                    callback(&change, window, cx);
                }
            });
            state.update(cx, |state, _| state.release = Some(release));
        }
        let hover = self.open_on_hover.then(|| Hover {
            delay: self.delay.unwrap_or(match kind {
                Kind::Menu => Duration::from_millis(100),
                Kind::Popover => Duration::from_millis(300),
            }),
            close_delay: self.close_delay,
        });
        let config = Config {
            kind,
            modal,
            initial_focus: self.initial_focus,
            return_focus: self.return_focus,
            on_open_change: self.on_open_change,
            hover,
            parent: OpenPopovers::rendering_parent(cx),
            parent_menu: self.parent_menu,
            focus_request: self.focus_request,
        };
        state.update(cx, |state, cx| {
            state.configure(config, controlled, window, cx)
        });

        let (open, trigger_focus, opened_at, anchor) = {
            let state = state.read(cx);
            (
                state.open,
                state.trigger_focus.clone(),
                state.opened_at,
                state.anchor.clone(),
            )
        };
        let trigger = self
            .trigger
            .map(|trigger| trigger(open))
            .unwrap_or_else(|| div().into_any_element());

        let ring = self
            .focus_ring
            .map(|radius| (radius, crate::theme::focus_ring(cx.hoshi())));
        let mut root = div()
            .id(self.id.clone())
            .flex()
            .flex_none()
            .press_selects_nothing()
            .when_some(ring, |this, (radius, ring)| {
                this.rounded(radius)
                    .focus_visible(move |style| style.shadow(ring))
            })
            .when(self.trigger_focusable, |this| {
                this.track_focus(&trigger_focus)
                    .key_context(TRIGGER_CONTEXT)
                    .on_action({
                        let state = state.clone();
                        move |_: &TriggerDown, window, cx| {
                            open_from_arrow(&state, Edge::First, window, cx)
                        }
                    })
                    .on_action({
                        let state = state.clone();
                        move |_: &TriggerUp, window, cx| {
                            open_from_arrow(&state, Edge::Last, window, cx)
                        }
                    })
                    .on_action({
                        let state = state.clone();
                        move |_: &TriggerTab, window, cx| tab_into_popup(&state, window, cx)
                    })
            })
            .when(kind == Kind::Menu, |this| {
                let state = state.clone();
                this.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    window.prevent_default();
                    state.update(cx, |state, cx| state.trigger_press(false, window, cx));
                })
            })
            .on_click({
                let state = state.clone();
                move |event, window, cx| {
                    let keyboard = event.is_keyboard();
                    if kind == Kind::Menu && !keyboard {
                        return;
                    }
                    state.update(cx, |state, cx| state.trigger_press(keyboard, window, cx));
                }
            })
            .when(hover.is_some(), |this| {
                let state = state.clone();
                this.on_hover(move |hovered, window, cx| {
                    state.update(cx, |state, cx| state.trigger_hover(*hovered, window, cx))
                })
            })
            .child(trigger);

        if !open {
            return root;
        }

        let popover = PopoverContext {
            state: state.clone(),
        };
        let colors = cx.hoshi().clone();
        let progress = enter_progress(opened_at, window, cx);
        let (popup_focus, scroll) = {
            let state = state.read(cx);
            (state.popup_focus.clone(), state.scroll.clone())
        };
        let mut surface = div()
            .id("popover-popup")
            .debug_selector(|| "popover-popup".into())
            .track_focus(&popup_focus)
            .key_context(match kind {
                Kind::Menu => "HoshiPopover HoshiMenu",
                Kind::Popover => POPUP_CONTEXT,
            })
            .on_action({
                let state = state.clone();
                move |_: &PopupTab, window, cx| tab_in_popup(&state, true, window, cx)
            })
            .on_action({
                let state = state.clone();
                move |_: &PopupTabBack, window, cx| tab_in_popup(&state, false, window, cx)
            })
            .flex()
            .flex_col()
            .max_w_full()
            .max_h_full()
            .rounded_lg()
            .border_1()
            .border_color(colors.foreground.opacity(0.1))
            .bg(colors.popover)
            .text_color(colors.popover_foreground)
            .text_sm()
            .shadow_md()
            .opacity(progress)
            .map(|this| match kind {
                Kind::Menu => this.min_w_32().p_1().overflow_hidden(),
                Kind::Popover => this
                    .w_72()
                    .p_2p5()
                    .gap_2p5()
                    .overflow_y_scroll()
                    .track_scroll(&scroll),
            });
        if self.trigger_width
            && let Some(anchor) = state.read(cx).anchor.get()
        {
            surface = surface.min_w(anchor.bounds.size.width);
        }
        surface.style().refine(&self.style);
        let surface = match self.content {
            Some(Content::Menu(build)) => build(PopoverMenu::new(popover), window, cx)
                .render_into(surface, &state, window, cx),
            Some(Content::Plain(build)) => surface.child(build(&popover, window, cx)),
            None => surface,
        };

        let placement = Placement {
            side: self.side,
            align: self.align.unwrap_or(match kind {
                Kind::Menu => Align::Start,
                Kind::Popover => Align::Center,
            }),
            side_offset: self.side_offset,
            align_offset: self.align_offset,
            collision_padding: self.collision_padding,
            fallback_axis: kind == Kind::Popover || submenu,
            ..Placement::default()
        };
        let host = PopupHost::new(
            state.clone(),
            surface.into_any_element(),
            placement,
            modal,
            progress,
        );
        let external = self.anchor;
        root = root
            .child(
                canvas(
                    move |bounds, window, _| {
                        let visible = !window.content_mask().bounds.intersect(&bounds).is_empty();
                        let bounds = external
                            .as_ref()
                            .and_then(|anchor| anchor.0.get())
                            .unwrap_or(bounds);
                        anchor.set(Some(AnchorSnapshot { bounds, visible }));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(deferred(host).with_priority(POPOVER_PRIORITY));
        root
    }
}

fn open_from_arrow(state: &Entity<PopoverState>, edge: Edge, window: &mut Window, cx: &mut App) {
    let (kind, open, focused) = {
        let state = state.read(cx);
        (
            state.kind,
            state.open,
            state.trigger_focus.is_focused(window),
        )
    };
    if kind != Kind::Menu || open || !focused {
        cx.propagate();
        return;
    }
    state.update(cx, |state, cx| {
        state.request(
            true,
            PopoverChangeReason::TriggerPress,
            Method::Keyboard,
            Some(edge),
            window,
            cx,
        )
    });
}

/// Tab on the trigger of an open popover goes into its popup, which comes next in the order.
fn tab_into_popup(state: &Entity<PopoverState>, window: &mut Window, cx: &mut App) {
    let (kind, open, focused, popup) = {
        let state = state.read(cx);
        (
            state.kind,
            state.open,
            state.trigger_focus.is_focused(window),
            state.popup_focus.clone(),
        )
    };
    if kind != Kind::Popover || !open || !focused {
        cx.propagate();
        return;
    }
    popup.focus(window, cx);
    step_focus(true, window, cx);
    if !popup.contains_focused(window, cx) {
        popup.focus(window, cx);
    }
}

/// Tab inside a popup. A menu closes and the keyboard moves on from its trigger. A popover
/// steps through its own controls; past its last one it closes and the keyboard moves on from
/// the trigger, before its first one it goes back to the trigger. A modal popover keeps the
/// keyboard inside.
fn tab_in_popup(state: &Entity<PopoverState>, forward: bool, window: &mut Window, cx: &mut App) {
    let (kind, modal, popup, trigger, root_trigger) = {
        let state = state.read(cx);
        (
            state.kind,
            state.modal,
            state.popup_focus.clone(),
            state
                .return_focus
                .clone()
                .unwrap_or_else(|| state.trigger_focus.clone()),
            state.root_trigger(cx),
        )
    };
    if kind == Kind::Menu {
        root_trigger.focus(window, cx);
        if forward {
            step_focus(true, window, cx);
        }
        state.update(cx, |state, cx| {
            state.close_tree(PopoverChangeReason::FocusOut, window, cx)
        });
        return;
    }
    let advance = |window: &mut Window, cx: &mut App| step_focus(forward, window, cx);
    advance(window, cx);
    if popup.contains_focused(window, cx) {
        return;
    }
    if modal {
        for _ in 0..64 {
            advance(window, cx);
            if popup.contains_focused(window, cx) {
                return;
            }
        }
        popup.focus(window, cx);
        return;
    }
    trigger.focus(window, cx);
    if forward {
        step_focus(true, window, cx);
        state.update(cx, |state, cx| {
            state.close(PopoverChangeReason::FocusOut, window, cx)
        });
    }
}

/// Moves the keyboard to the next or previous tab stop without leaving the focus trap that holds
/// it now, such as a dialog's, the way gpui-kit's root moves it on Tab.
fn step_focus(forward: bool, window: &mut Window, cx: &mut App) {
    let trap = gpui_kit::base::active_focus_trap(window, cx);
    let advance = |window: &mut Window, cx: &mut App| {
        if forward {
            window.focus_next(cx)
        } else {
            window.focus_prev(cx)
        }
    };
    advance(window, cx);
    let Some(trap) = trap else {
        return;
    };
    let mut visited = Vec::new();
    while !trap.contains_focused(window, cx) {
        let focused = window.focused(cx);
        if visited.contains(&focused) {
            trap.focus(window, cx);
            return;
        }
        visited.push(focused);
        advance(window, cx);
    }
}
