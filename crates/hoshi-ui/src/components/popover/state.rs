//! What an open popover remembers between frames, and the window-wide list of open popovers
//! that outside presses, Escape and nesting are decided against.

use super::navigation::Navigation;
use super::{OpenChangeHandler, PopoverChange, PopoverChangeReason, PopoverFocus};
use crate::components::placement::Side;
use gpui_kit::base::{DeferredPopover, GlobalState};
use gpui_kit::*;
use std::cell::Cell;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

/// A press on a popover opened by hover keeps it open when it comes this soon after.
const PATIENT_CLICK: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Popover,
    Menu,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Method {
    Pointer,
    Keyboard,
    Hover,
    Programmatic,
}

/// The item a menu opened from the keyboard highlights first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Edge {
    First,
    Last,
}

/// How the next opening was asked for.
#[derive(Clone, Copy, Debug)]
struct Pending {
    method: Method,
    edge: Option<Edge>,
    drag: bool,
}

impl Default for Pending {
    fn default() -> Self {
        Self {
            method: Method::Programmatic,
            edge: None,
            drag: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct AnchorSnapshot {
    pub bounds: Bounds<Pixels>,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Hover {
    pub delay: Duration,
    pub close_delay: Duration,
}

#[derive(Default)]
pub(super) struct MenuState {
    pub nav: Navigation,
    pub edge: Option<Edge>,
    /// The item whose submenu is open.
    pub submenu: Option<usize>,
    /// Bumped to move the keyboard into the open submenu.
    pub submenu_focus: u64,
    /// The open submenu was opened from the keyboard.
    pub submenu_keyboard: bool,
    /// Opened by pressing the trigger, with the button still down: letting go over an item
    /// picks it.
    pub drag_open: bool,
    pub reveal: Option<usize>,
}

/// What a popover's render settles every frame.
pub(super) struct Config {
    pub kind: Kind,
    pub modal: bool,
    pub initial_focus: PopoverFocus,
    pub return_focus: Option<FocusHandle>,
    pub on_open_change: Option<OpenChangeHandler>,
    pub hover: Option<Hover>,
    pub parent: Option<EntityId>,
    pub parent_menu: Option<WeakEntity<PopoverState>>,
    pub focus_request: u64,
}

pub(super) struct PopoverState {
    pub open: bool,
    pub controlled: bool,
    initialized: bool,
    pub kind: Kind,
    pub modal: bool,
    pub popup_focus: FocusHandle,
    pub trigger_focus: FocusHandle,
    pub return_focus: Option<FocusHandle>,
    initial_focus: PopoverFocus,
    pub on_open_change: Option<OpenChangeHandler>,
    pub hover: Option<Hover>,
    pub parent: Option<EntityId>,
    pub parent_menu: Option<WeakEntity<PopoverState>>,
    focus_request: u64,
    pending: Pending,
    pub method: Method,
    pub opened_at: Option<Instant>,
    pub anchor: Rc<Cell<Option<AnchorSnapshot>>>,
    pub side: Side,
    pub pressing: bool,
    pub press_outside: bool,
    pub exit_point: Option<Point<Pixels>>,
    hover_open: Option<Task<()>>,
    hover_close: Option<Task<()>>,
    pub menu: MenuState,
    pub scroll: ScrollHandle,
    window: AnyWindowHandle,
    token: Option<Rc<()>>,
    deferred: Option<DeferredPopover>,
    subscriptions: Vec<Subscription>,
    pub release: Option<Subscription>,
}

impl PopoverState {
    pub fn new(open: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            open,
            controlled: false,
            initialized: false,
            kind: Kind::Popover,
            modal: false,
            popup_focus: cx.focus_handle(),
            trigger_focus: cx.focus_handle().tab_stop(true),
            return_focus: None,
            initial_focus: PopoverFocus::Popup,
            on_open_change: None,
            hover: None,
            parent: None,
            parent_menu: None,
            focus_request: 0,
            pending: Pending::default(),
            method: Method::Programmatic,
            opened_at: None,
            anchor: Rc::new(Cell::new(None)),
            side: Side::Bottom,
            pressing: false,
            press_outside: false,
            exit_point: None,
            hover_open: None,
            hover_close: None,
            menu: MenuState::default(),
            scroll: ScrollHandle::new(),
            window: window.window_handle(),
            token: None,
            deferred: None,
            subscriptions: Vec::new(),
            release: None,
        }
    }

    /// Takes what this frame's render asked for; a controlled popover follows `open`.
    pub fn configure(
        &mut self,
        config: Config,
        open: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.kind = config.kind;
        self.modal = config.modal;
        self.initial_focus = config.initial_focus;
        self.return_focus = config.return_focus;
        self.on_open_change = config.on_open_change;
        self.hover = config.hover;
        self.parent = config.parent;
        self.parent_menu = config.parent_menu;
        self.controlled = open.is_some();
        let first = !self.initialized;
        self.initialized = true;
        match open {
            Some(open) if open != self.open || (first && open) => {
                self.open = !open;
                self.transition(open, window, cx);
            }
            None if first && self.open => {
                self.open = false;
                self.transition(true, window, cx);
            }
            _ => {}
        }
        if config.focus_request != self.focus_request {
            self.focus_request = config.focus_request;
            if self.open {
                self.method = Method::Keyboard;
                self.menu.edge = Some(Edge::First);
                self.popup_focus.focus(window, cx);
            }
        }
    }

    /// Asks for the popover to open or close. A controlled popover only tells its owner.
    pub fn request(
        &mut self,
        open: bool,
        reason: PopoverChangeReason,
        method: Method,
        edge: Option<Edge>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if open == self.open {
            return;
        }
        self.pending = Pending {
            method,
            edge,
            drag: false,
        };
        if !self.controlled {
            self.transition(open, window, cx);
        }
        if let Some(callback) = self.on_open_change.clone() {
            callback(&PopoverChange { open, reason }, window, cx);
        }
        cx.notify();
    }

    pub fn close(
        &mut self,
        reason: PopoverChangeReason,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request(false, reason, Method::Programmatic, None, window, cx);
    }

    /// A press or keyboard click on the trigger.
    pub fn trigger_press(&mut self, keyboard: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            let patient = self
                .opened_at
                .is_some_and(|at| at.elapsed() < PATIENT_CLICK);
            if self.method == Method::Hover && !keyboard && patient {
                self.method = Method::Pointer;
                self.hover_close = None;
                cx.notify();
                return;
            }
            self.request(
                false,
                PopoverChangeReason::TriggerPress,
                Method::Pointer,
                None,
                window,
                cx,
            );
        } else if keyboard {
            self.request(
                true,
                PopoverChangeReason::TriggerPress,
                Method::Keyboard,
                Some(Edge::First),
                window,
                cx,
            );
        } else {
            self.request(
                true,
                PopoverChangeReason::TriggerPress,
                Method::Pointer,
                None,
                window,
                cx,
            );
            if self.kind == Kind::Menu {
                if self.open {
                    self.menu.drag_open = true;
                } else {
                    self.pending.drag = true;
                }
            }
        }
    }

    pub fn trigger_hover(&mut self, hovered: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hover) = self.hover else {
            return;
        };
        if hovered {
            self.hover_close = None;
            if self.open {
                return;
            }
            self.hover_open = Some(cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(hover.delay).await;
                this.update_in(cx, |state, window, cx| {
                    state.hover_open = None;
                    state.request(
                        true,
                        PopoverChangeReason::TriggerHover,
                        Method::Hover,
                        None,
                        window,
                        cx,
                    );
                })
                .ok();
            }));
        } else {
            self.hover_open = None;
            if self.open && self.method == Method::Hover {
                self.exit_point = Some(window.mouse_position());
            }
        }
    }

    /// The pointer moved while the popover is open by hover: it stays while the pointer is on
    /// its way, else it closes after the close delay.
    pub fn track_pointer(
        &mut self,
        safe: bool,
        over_popup: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(hover) = self.hover else {
            return;
        };
        if self.method != Method::Hover {
            return;
        }
        if safe {
            self.hover_close = None;
            if over_popup {
                self.exit_point = None;
            }
            return;
        }
        if self.hover_close.is_some() {
            return;
        }
        if hover.close_delay.is_zero() {
            self.close(PopoverChangeReason::TriggerHover, window, cx);
            return;
        }
        self.hover_close = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(hover.close_delay).await;
            this.update_in(cx, |state, window, cx| {
                state.hover_close = None;
                state.close(PopoverChangeReason::TriggerHover, window, cx);
            })
            .ok();
        }));
    }

    fn transition(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if open == self.open {
            return;
        }
        self.open = open;
        let Pending { method, edge, drag } = std::mem::take(&mut self.pending);
        self.hover_open = None;
        self.hover_close = None;
        self.press_outside = false;
        self.exit_point = None;
        if open {
            self.method = method;
            self.opened_at = Some(Instant::now());
            self.menu = MenuState {
                edge,
                drag_open: drag,
                ..MenuState::default()
            };
            let id = cx.entity_id();
            let weak = cx.weak_entity();
            self.token = Some(OpenPopovers::register(
                cx,
                id,
                self.window,
                self.parent,
                weak,
            ));
            GlobalState::init(cx);
            self.deferred = Some(GlobalState::register_deferred_popover(cx));
            self.watch(window, cx);
            if method != Method::Hover {
                let target = match (&self.initial_focus, self.kind) {
                    (_, Kind::Menu) | (PopoverFocus::Popup, _) => Some(self.popup_focus.clone()),
                    (PopoverFocus::Handle(handle), _) => Some(handle.clone()),
                    (PopoverFocus::None, _) => None,
                };
                if let Some(target) = target {
                    target.focus(window, cx);
                }
            }
        } else {
            let had_focus =
                self.popup_focus.contains_focused(window, cx) || window.focused(cx).is_none();
            self.token = None;
            self.deferred = None;
            self.subscriptions.clear();
            self.menu = MenuState::default();
            self.opened_at = None;
            self.anchor.set(None);
            if had_focus && self.method != Method::Hover {
                self.return_focus
                    .clone()
                    .unwrap_or_else(|| self.trigger_focus.clone())
                    .focus(window, cx);
            }
        }
    }

    /// Closes on Escape, wherever the keyboard is, and when the keyboard leaves for something
    /// else.
    fn watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        let focus_out = window.on_focus_out(&self.popup_focus, cx, {
            let this = this.clone();
            move |_, window, cx| {
                this.update(cx, |state, cx| state.focus_left(window, cx))
                    .ok();
            }
        });
        self.subscriptions = vec![focus_out];
    }

    fn focus_left(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open || self.pressing || self.method == Method::Hover {
            return;
        }
        let Some(focused) = window.focused(cx) else {
            return;
        };
        if focused == self.trigger_focus
            || self.return_focus.as_ref() == Some(&focused)
            || self.popup_focus.contains_focused(window, cx)
        {
            return;
        }
        self.close(PopoverChangeReason::FocusOut, window, cx);
    }

    /// Closes this popover and every menu it is a submenu of.
    pub fn close_tree(
        &mut self,
        reason: PopoverChangeReason,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parent = self.parent_menu.clone();
        self.close(reason, window, cx);
        if let Some(parent) = parent.and_then(|parent| parent.upgrade()) {
            parent.update(cx, |parent, cx| parent.close_tree(reason, window, cx));
        }
    }

    /// The trigger of the outermost menu this one belongs to.
    pub fn root_trigger(&self, cx: &App) -> FocusHandle {
        match self
            .parent_menu
            .as_ref()
            .and_then(|parent| parent.upgrade())
        {
            Some(parent) => parent.read(cx).root_trigger(cx),
            None => self
                .return_focus
                .clone()
                .unwrap_or_else(|| self.trigger_focus.clone()),
        }
    }
}

/// The open popovers of every window, so a press inside a nested popup is not outside its
/// parent, and Escape closes only the innermost.
#[derive(Default)]
pub(super) struct OpenPopovers {
    entries: Vec<Entry>,
    rendering: Vec<EntityId>,
    escape: Option<Subscription>,
}

impl Global for OpenPopovers {}

struct Entry {
    id: EntityId,
    window: AnyWindowHandle,
    parent: Option<EntityId>,
    trigger: Option<Bounds<Pixels>>,
    popup: Option<Bounds<Pixels>>,
    state: WeakEntity<PopoverState>,
    alive: Weak<()>,
}

impl OpenPopovers {
    fn global(cx: &mut App) -> &mut Self {
        if !cx.has_global::<Self>() {
            cx.set_global(Self::default());
        }
        cx.global_mut::<Self>()
    }

    fn live(cx: &App) -> impl Iterator<Item = &Entry> {
        cx.try_global::<Self>()
            .into_iter()
            .flat_map(|open| open.entries.iter())
            .filter(|entry| entry.alive.strong_count() > 0)
    }

    fn register(
        cx: &mut App,
        id: EntityId,
        window: AnyWindowHandle,
        parent: Option<EntityId>,
        state: WeakEntity<PopoverState>,
    ) -> Rc<()> {
        if Self::global(cx).escape.is_none() {
            let escape = cx.intercept_keystrokes(Self::escape);
            Self::global(cx).escape = Some(escape);
        }
        let token = Rc::new(());
        let open = Self::global(cx);
        open.entries
            .retain(|entry| entry.alive.strong_count() > 0 && entry.id != id);
        open.entries.push(Entry {
            id,
            window,
            parent,
            trigger: None,
            popup: None,
            state,
            alive: Rc::downgrade(&token),
        });
        token
    }

    pub fn place(cx: &mut App, id: EntityId, trigger: Bounds<Pixels>, popup: Bounds<Pixels>) {
        if let Some(entry) = Self::global(cx)
            .entries
            .iter_mut()
            .find(|entry| entry.id == id && entry.alive.strong_count() > 0)
        {
            entry.trigger = Some(trigger);
            entry.popup = Some(popup);
        }
    }

    /// Whether `point` is on the popover's trigger or popup, or on a popup nested in it.
    pub fn is_inside(cx: &App, id: EntityId, point: Point<Pixels>) -> bool {
        let entries: Vec<&Entry> = Self::live(cx).collect();
        let Some(own) = entries.iter().find(|entry| entry.id == id) else {
            return false;
        };
        let window = own.window;
        let hit = |entry: &Entry| {
            entry.popup.is_some_and(|bounds| bounds.contains(&point))
                || (entry.id == id && entry.trigger.is_some_and(|bounds| bounds.contains(&point)))
        };
        let mut family = vec![id];
        let mut index = 0;
        while let Some(&current) = family.get(index) {
            for entry in &entries {
                if entry.parent == Some(current)
                    && entry.window == window
                    && !family.contains(&entry.id)
                {
                    family.push(entry.id);
                }
            }
            index += 1;
        }
        entries
            .iter()
            .filter(|entry| family.contains(&entry.id))
            .any(|entry| hit(entry))
    }

    /// Escape closes the innermost open popover of the window, wherever the keyboard is.
    fn escape(event: &KeystrokeEvent, window: &mut Window, cx: &mut App) {
        let keystroke = &event.keystroke;
        if keystroke.key != "escape" || keystroke.modifiers.modified() {
            return;
        }
        let handle = window.window_handle();
        let entries: Vec<&Entry> = Self::live(cx)
            .filter(|entry| entry.window == handle)
            .collect();
        let innermost = entries
            .iter()
            .rev()
            .find(|entry| !entries.iter().any(|other| other.parent == Some(entry.id)))
            .and_then(|entry| entry.state.upgrade());
        let Some(state) = innermost else {
            return;
        };
        state.update(cx, |state, cx| {
            state.close(PopoverChangeReason::EscapeKey, window, cx)
        });
        cx.stop_propagation();
    }

    pub fn begin_render(cx: &mut App, id: EntityId) {
        Self::global(cx).rendering.push(id);
    }

    pub fn end_render(cx: &mut App) {
        Self::global(cx).rendering.pop();
    }

    pub fn rendering_parent(cx: &App) -> Option<EntityId> {
        cx.try_global::<Self>()
            .and_then(|open| open.rendering.last().copied())
    }

    pub fn any_at(window: &Window, point: Point<Pixels>, cx: &App) -> bool {
        let handle = window.window_handle();
        Self::live(cx).any(|entry| {
            entry.window == handle && entry.popup.is_some_and(|bounds| bounds.contains(&point))
        })
    }

    pub fn any_open(window: &Window, cx: &App) -> bool {
        let handle = window.window_handle();
        Self::live(cx).any(|entry| entry.window == handle)
    }
}
