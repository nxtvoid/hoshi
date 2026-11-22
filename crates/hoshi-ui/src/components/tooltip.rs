//! A short label that floats next to its trigger on hover or keyboard focus, with Base UI's
//! delays, delay group, placement and dismissal. Any element with an id takes one through
//! [`WithTooltip::with_tooltip`]; [`crate::Button::tooltip`] wraps its own.

mod machine;

use super::placement::{Align, Placement, Side, place};
use crate::theme::ActiveTheme as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use machine::{Effect, Machine, Timing};
use std::cell::{Cell, RefCell};
use std::f32::consts::SQRT_2;
use std::rc::{Rc, Weak};
use std::time::Duration;

/// Paints above popovers and dialogs (`gpui_kit::component`'s popups draw at 100).
pub const TOOLTIP_PRIORITY: usize = 200;

const SIDE_OFFSET: f32 = 4.;
const COLLISION_PADDING: f32 = 5.;
const ARROW: f32 = 10.;
const ARROW_PADDING: f32 = 5.;
const FADE: Duration = Duration::from_millis(150);

/// The delays every tooltip shares unless it sets its own, like Base UI's `Tooltip.Provider`.
/// Install another with `cx.set_global(TooltipProvider { .. })`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooltipProvider {
    /// How long the pointer rests on a trigger before its tooltip opens.
    pub delay: Duration,
    pub close_delay: Duration,
    /// How long after a tooltip closes the next trigger still opens its own at once.
    pub timeout: Duration,
}

impl Default for TooltipProvider {
    fn default() -> Self {
        Self {
            delay: Duration::from_millis(600),
            close_delay: Duration::ZERO,
            timeout: Duration::from_millis(400),
        }
    }
}

impl Global for TooltipProvider {}

impl TooltipProvider {
    pub fn global(cx: &App) -> Self {
        cx.try_global::<Self>().copied().unwrap_or_default()
    }
}

type Content = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// What a trigger shows and where. Text converts into one: `.tooltip("Settings")`.
#[derive(Clone)]
pub struct Tooltip {
    content: Content,
    shortcut: Option<SharedString>,
    side: Side,
    align: Align,
    side_offset: Pixels,
    align_offset: Pixels,
    delay: Option<Duration>,
    close_delay: Option<Duration>,
    max_width: AbsoluteLength,
    disabled: bool,
    focus: Option<FocusHandle>,
}

impl Tooltip {
    pub fn new(text: impl Into<SharedString>) -> Self {
        let text = text.into();
        Self::element(move |_, _| text.clone())
    }

    /// Content of any shape, built only while the tooltip shows.
    pub fn element<E: IntoElement>(build: impl Fn(&mut Window, &mut App) -> E + 'static) -> Self {
        Self {
            content: Rc::new(move |window, cx| build(window, cx).into_any_element()),
            shortcut: None,
            side: Side::Top,
            align: Align::Center,
            side_offset: px(SIDE_OFFSET),
            align_offset: px(0.),
            delay: None,
            close_delay: None,
            max_width: rems(20.).into(),
            disabled: false,
            focus: None,
        }
    }

    /// The keys that do the same, shown after the content.
    pub fn shortcut(mut self, keys: impl Into<SharedString>) -> Self {
        self.shortcut = Some(keys.into());
        self
    }

    pub fn side(mut self, side: Side) -> Self {
        self.side = side;
        self
    }

    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// The gap to the trigger; 4px by default.
    pub fn side_offset(mut self, offset: impl Into<Pixels>) -> Self {
        self.side_offset = offset.into();
        self
    }

    pub fn align_offset(mut self, offset: impl Into<Pixels>) -> Self {
        self.align_offset = offset.into();
        self
    }

    /// Overrides the provider's open delay for this trigger.
    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    pub fn close_delay(mut self, delay: Duration) -> Self {
        self.close_delay = Some(delay);
        self
    }

    /// Longer content wraps at this width; 20rem by default.
    pub fn max_width(mut self, width: impl Into<AbsoluteLength>) -> Self {
        self.max_width = width.into();
        self
    }

    /// Never opens, and closes if it shows.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Opens while this handle holds keyboard focus, as a trigger focused with Tab.
    pub fn focus(mut self, handle: &FocusHandle) -> Self {
        self.focus = Some(handle.clone());
        self
    }

    fn timing(&self, provider: TooltipProvider) -> Timing {
        Timing {
            delay: self.delay.unwrap_or(provider.delay),
            close_delay: self.close_delay.unwrap_or(provider.close_delay),
            timeout: provider.timeout,
        }
    }
}

impl From<SharedString> for Tooltip {
    fn from(text: SharedString) -> Self {
        Self::new(text)
    }
}

impl From<String> for Tooltip {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

impl From<&'static str> for Tooltip {
    fn from(text: &'static str) -> Self {
        Self::new(text)
    }
}

pub trait WithTooltip: IntoElement + Sized {
    /// Shows `tooltip` for this element, which needs an id (`div().id(..)`); wrap others in
    /// [`TooltipTrigger::new`].
    fn with_tooltip(self, tooltip: impl Into<Tooltip>) -> TooltipTrigger {
        let element = self.into_element();
        let id = element.id();
        debug_assert!(id.is_some(), "a tooltip trigger needs an element id");
        TooltipTrigger::new(id.unwrap_or("tooltip-trigger".into()), element, tooltip)
    }
}

impl<E: IntoElement> WithTooltip for E {}

/// An element that shows a tooltip. It lays out exactly as its child.
pub struct TooltipTrigger {
    id: ElementId,
    child: AnyElement,
    tooltip: Tooltip,
}

impl TooltipTrigger {
    pub fn new(
        id: impl Into<ElementId>,
        child: impl IntoElement,
        tooltip: impl Into<Tooltip>,
    ) -> Self {
        Self {
            id: id.into(),
            child: child.into_any_element(),
            tooltip: tooltip.into(),
        }
    }
}

impl IntoElement for TooltipTrigger {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TooltipTrigger {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        self.child.prepaint(window, cx);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let Some(id) = id else {
            return hitbox;
        };
        let shared = shared(cx);
        let key = Key {
            window: window.window_handle(),
            element: id.clone(),
        };
        let now = cx.background_executor().now();
        let timing = self.tooltip.timing(TooltipProvider::global(cx));
        let active = window.is_window_active();
        let focused = self
            .tooltip
            .focus
            .as_ref()
            .is_some_and(|handle| handle.is_focused(window));
        let (was_focused, was_active) =
            window.with_element_state(id, |state: Option<TriggerState>, _| {
                let mut state = state.unwrap_or_else(|| TriggerState {
                    focused: false,
                    window_active: active,
                    _mounted: Mounted {
                        key: key.clone(),
                        shared: Rc::downgrade(&shared),
                    },
                });
                let previous = (state.focused, state.window_active);
                state.focused = focused;
                state.window_active = active;
                (previous, state)
            });

        let mut effects = Vec::new();
        let shown = {
            let mut state = shared.borrow_mut();
            if self.tooltip.disabled {
                if state.machine.involves(&key) {
                    effects.push(state.machine.unmount(&key));
                }
            } else {
                if focused && !was_focused && window.last_input_was_keyboard() {
                    effects.push(state.machine.focus(key.clone(), now, timing));
                } else if !focused && was_focused {
                    effects.push(state.machine.blur(&key, now));
                }
                let by_pointer = state
                    .machine
                    .shown()
                    .filter(|shown| shown.key == key)
                    .map(|shown| !shown.by_focus);
                let visible = hitbox.bounds.intersect(&hitbox.content_mask.bounds);
                if was_active && !active && by_pointer.is_some() {
                    effects.push(state.machine.dismiss(now));
                } else if by_pointer == Some(true) && !visible.contains(&window.mouse_position()) {
                    state.hovered.retain(|hovered| *hovered != key);
                    effects.push(state.machine.leave(&key, now, timing));
                }
            }
            state
                .machine
                .shown()
                .filter(|shown| shown.key == key)
                .map(|shown| (shown.instant, shown.generation))
        };
        for effect in effects {
            apply(effect, &shared, window, cx, true);
        }
        if let Some((instant, generation)) = shown {
            self.defer_popup(bounds, instant, generation, window, cx);
        }
        hitbox
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
        let Some(id) = id else {
            return;
        };
        let shared = shared(cx);
        let key = Key {
            window: window.window_handle(),
            element: id.clone(),
        };
        let timing = self.tooltip.timing(TooltipProvider::global(cx));
        let disabled = self.tooltip.disabled;

        window.on_mouse_event({
            let (shared, key, hitbox) = (shared.clone(), key.clone(), hitbox.clone());
            move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture {
                    return;
                }
                let hovered = hitbox.is_hovered(window);
                let now = cx.background_executor().now();
                let effect = {
                    let mut state = shared.borrow_mut();
                    if hovered == state.hovered.contains(&key) {
                        return;
                    }
                    if !hovered {
                        state.hovered.retain(|hovered| *hovered != key);
                        state.machine.leave(&key, now, timing)
                    } else {
                        state.hovered.push(key.clone());
                        if disabled || event.pressed_button.is_some() {
                            return;
                        }
                        state.machine.hover(key.clone(), now, timing)
                    }
                };
                apply(effect, &shared, window, cx, false);
            }
        });
        window.on_mouse_event({
            let (shared, key) = (shared.clone(), key.clone());
            move |_: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || !shared.borrow().machine.involves(&key) {
                    return;
                }
                let now = cx.background_executor().now();
                let effect = shared.borrow_mut().machine.dismiss(now);
                apply(effect, &shared, window, cx, false);
            }
        });
        window.on_mouse_event({
            let (shared, key) = (shared.clone(), key.clone());
            move |_: &ScrollEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || !shared.borrow().machine.involves(&key) {
                    return;
                }
                let now = cx.background_executor().now();
                let effect = {
                    let mut state = shared.borrow_mut();
                    state.hovered.clear();
                    state.machine.scroll(now)
                };
                apply(effect, &shared, window, cx, false);
            }
        });
        window.on_mouse_event(move |_: &MouseExitEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture {
                return;
            }
            let now = cx.background_executor().now();
            let effect = {
                let mut state = shared.borrow_mut();
                if !state.hovered.contains(&key) {
                    return;
                }
                state.hovered.retain(|hovered| *hovered != key);
                state.machine.leave(&key, now, timing)
            };
            apply(effect, &shared, window, cx, false);
        });
    }
}

impl TooltipTrigger {
    fn defer_popup(
        &self,
        anchor: Bounds<Pixels>,
        instant: bool,
        generation: u64,
        window: &mut Window,
        cx: &mut App,
    ) {
        let arrow = Rc::new(Cell::new(None));
        let content = (self.tooltip.content)(window, cx);
        let popup = popup(content, &self.tooltip, arrow.clone(), cx);
        let mut popup = if instant {
            popup.into_any_element()
        } else {
            popup
                .with_animation(
                    ElementId::NamedInteger("tooltip".into(), generation),
                    Animation::new(FADE).with_easing(ease_out),
                    |popup, delta| popup.opacity(delta),
                )
                .into_any_element()
        };
        let size = popup.layout_as_root(AvailableSpace::min_size(), window, cx);
        let placed = place(
            anchor,
            size,
            window.viewport_size(),
            Placement {
                side: self.tooltip.side,
                align: self.tooltip.align,
                side_offset: self.tooltip.side_offset,
                align_offset: self.tooltip.align_offset,
                collision_padding: px(COLLISION_PADDING),
                arrow: px(ARROW),
                arrow_padding: px(ARROW_PADDING),
                fallback_axis: true,
            },
        );
        arrow.set(Some((placed.side, placed.arrow_center)));
        window.defer_draw(popup, placed.origin, TOOLTIP_PRIORITY, None);
    }
}

/// The web's `TooltipContent`: `bg-foreground text-background rounded-md px-3 py-1.5 text-xs`.
fn popup(
    content: AnyElement,
    tooltip: &Tooltip,
    arrow: Rc<Cell<Option<(Side, Pixels)>>>,
    cx: &App,
) -> Div {
    let colors = cx.hoshi();
    let (background, foreground) = (colors.foreground, colors.background);
    let key_background = colors
        .background
        .opacity(if cx.theme().is_dark() { 0.1 } else { 0.2 });
    div()
        .debug_selector(|| "tooltip".into())
        .relative()
        .flex()
        .items_center()
        .gap(rems(0.375))
        .max_w(tooltip.max_width)
        .pl(rems(0.75))
        .pr(if tooltip.shortcut.is_some() {
            rems(0.375)
        } else {
            rems(0.75)
        })
        .py(rems(0.375))
        .rounded(rems(0.5))
        .bg(background)
        .text_color(foreground)
        .text_size(rems(0.75))
        .line_height(rems(1.))
        .font_weight(FontWeight::NORMAL)
        .whitespace_normal()
        .child(div().min_w_0().child(content))
        .when_some(tooltip.shortcut.clone(), |this, keys| {
            this.child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .h(rems(1.25))
                    .min_w(rems(1.25))
                    .px(rems(0.25))
                    .rounded(rems(0.375))
                    .bg(key_background)
                    .font_weight(FontWeight::MEDIUM)
                    .child(keys),
            )
        })
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    if let Some((side, center)) = arrow.get() {
                        paint_arrow(bounds, side, center, background, window);
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
}

/// Base UI's arrow as the web draws it: a 10px square turned 45deg with 2px corners, its center
/// 2px inside the popup's edge (1px on the left and right sides).
fn paint_arrow(
    bounds: Bounds<Pixels>,
    side: Side,
    center: Pixels,
    color: Hsla,
    window: &mut Window,
) {
    let inset = if side.is_vertical() { 2. } else { 1. };
    let reach = ARROW / 2. * SQRT_2 - inset;
    let corner = 2. / SQRT_2;
    let at = |along: f32, out: f32| {
        let along = px(along) + center;
        let out = px(out);
        match side {
            Side::Top => point(bounds.left() + along, bounds.bottom() + out),
            Side::Bottom => point(bounds.left() + along, bounds.top() - out),
            Side::Left => point(bounds.right() + out, bounds.top() + along),
            Side::Right => point(bounds.left() - out, bounds.top() + along),
        }
    };
    let mut path = PathBuilder::fill();
    path.move_to(at(-reach - 1., -1.));
    path.line_to(at(-corner, reach - corner));
    path.curve_to(at(corner, reach - corner), at(0., reach));
    path.line_to(at(reach + 1., -1.));
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// Tailwind's `ease-out`, near enough.
fn ease_out(delta: f32) -> f32 {
    1. - (1. - delta).powi(3)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    window: AnyWindowHandle,
    element: GlobalElementId,
}

#[derive(Default)]
struct Shared {
    machine: Machine<Key>,
    /// The triggers under the pointer, so moves within one do not reopen what a press closed.
    hovered: Vec<Key>,
    _escape: Option<Subscription>,
}

struct Tooltips(Rc<RefCell<Shared>>);

impl Global for Tooltips {}

fn shared(cx: &mut App) -> Rc<RefCell<Shared>> {
    if let Some(tooltips) = cx.try_global::<Tooltips>() {
        return tooltips.0.clone();
    }
    let shared = Rc::new(RefCell::new(Shared::default()));
    let weak = Rc::downgrade(&shared);
    let escape = cx.intercept_keystrokes(move |event, window, cx| {
        if event.keystroke.key != "escape" {
            return;
        }
        let Some(shared) = weak.upgrade() else {
            return;
        };
        let now = cx.background_executor().now();
        let effect = shared.borrow_mut().machine.dismiss(now);
        apply(effect, &shared, window, cx, false);
    });
    shared.borrow_mut()._escape = Some(escape);
    cx.set_global(Tooltips(shared.clone()));
    shared
}

/// Redraws the windows whose tooltip changed and starts the timer the machine asked for.
fn apply(
    effect: Effect<Key>,
    shared: &Rc<RefCell<Shared>>,
    window: &mut Window,
    cx: &mut App,
    drawing: bool,
) {
    let here = window.window_handle();
    let mut redraw = false;
    for key in [effect.shown, effect.hidden].into_iter().flatten() {
        if key.window == here {
            redraw = true;
        } else {
            cx.defer(move |cx| {
                key.window.update(cx, |_, window, _| window.refresh()).ok();
            });
        }
    }
    if redraw {
        if drawing {
            window.on_next_frame(|window, _| window.refresh());
        } else {
            window.refresh();
        }
    }
    if let Some((delay, generation)) = effect.timer {
        let shared = Rc::downgrade(shared);
        window
            .spawn(cx, async move |cx| {
                cx.background_executor().timer(delay).await;
                cx.update(|window, cx| {
                    let Some(shared) = shared.upgrade() else {
                        return;
                    };
                    let now = cx.background_executor().now();
                    let timeout = TooltipProvider::global(cx).timeout;
                    let effect = shared.borrow_mut().machine.fire(generation, now, timeout);
                    apply(effect, &shared, window, cx, false);
                })
                .ok();
            })
            .detach();
    }
}

struct TriggerState {
    focused: bool,
    window_active: bool,
    _mounted: Mounted,
}

/// Lives in the trigger's element state, which GPUI drops the first frame the trigger is not
/// drawn: its tooltip closes with it.
struct Mounted {
    key: Key,
    shared: Weak<RefCell<Shared>>,
}

impl Drop for Mounted {
    fn drop(&mut self) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        if let Ok(mut state) = shared.try_borrow_mut() {
            state.hovered.retain(|hovered| *hovered != self.key);
            state.machine.unmount(&self.key);
        }
    }
}

#[cfg(test)]
mod tests;
