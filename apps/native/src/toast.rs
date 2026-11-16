//! A short notice floating over the bottom of the main area, one at a time: what just happened,
//! with an action such as Undo and the key that runs it. It goes away by itself.

use crate::m;
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_ui::prelude::*;
use std::rc::Rc;
use std::time::Duration;

/// How long a notice shows.
pub const TOAST_DURATION: Duration = Duration::from_secs(5);

type ToastHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// A key that runs a notice's action, with how it reads.
#[derive(Clone)]
pub struct Shortcut {
    pub label: SharedString,
    pub matches: fn(&Keystroke, &App) -> bool,
}

/// Ctrl+Z, or ⌘Z on macOS.
pub fn undo_shortcut(cx: &App) -> Shortcut {
    let label = if cx.platform_info().is_mac() {
        m::undo_shortcut_cmd()
    } else {
        m::undo_shortcut_ctrl()
    };
    Shortcut {
        label: label.into(),
        matches: |keystroke, cx| {
            keystroke.key == "z" && keystroke.modifiers == Modifiers::secondary_key(cx)
        },
    }
}

pub struct ToastAction {
    pub label: SharedString,
    pub shortcut: Option<Shortcut>,
    pub run: ToastHandler,
}

pub struct Toast {
    pub icon: IconName,
    pub message: SharedString,
    pub action: Option<ToastAction>,
}

struct Shown {
    toast: Toast,
    /// The action's key runs it, even in a text field, until another key is pressed.
    claims_shortcut: bool,
}

/// The window's notices: the one shown now, if any.
pub struct Toasts {
    shown: Option<Shown>,
    serial: usize,
    _expiry: Option<Task<()>>,
    _keys: Subscription,
}

struct ToastLayer(Entity<Toasts>);

impl Global for ToastLayer {}

/// Makes the window's notice layer, which [`show`] then shows notices in.
pub fn install(cx: &mut App) -> Entity<Toasts> {
    let toasts = cx.new(Toasts::new);
    cx.set_global(ToastLayer(toasts.clone()));
    toasts
}

/// Shows `toast` in place of the one shown, when the window has a notice layer.
pub fn show(toast: Toast, cx: &mut App) {
    if let Some(layer) = cx.try_global::<ToastLayer>() {
        let toasts = layer.0.clone();
        toasts.update(cx, |toasts, cx| toasts.show(toast, cx));
    }
}

impl Toasts {
    fn new(cx: &mut Context<Self>) -> Self {
        let this = cx.weak_entity();
        let keys = cx.intercept_keystrokes(move |event, window, cx| {
            let Some(toasts) = this.upgrade() else {
                return;
            };
            if event.keystroke.is_modifier_key() {
                return;
            }
            let run = toasts.update(cx, |toasts, cx| toasts.take_for_key(&event.keystroke, cx));
            if let Some(run) = run {
                cx.stop_propagation();
                run(window, cx);
            }
        });
        Self {
            shown: None,
            serial: 0,
            _expiry: None,
            _keys: keys,
        }
    }

    pub fn show(&mut self, toast: Toast, cx: &mut Context<Self>) {
        self.serial += 1;
        let serial = self.serial;
        let claims_shortcut = toast
            .action
            .as_ref()
            .is_some_and(|action| action.shortcut.is_some());
        self.shown = Some(Shown {
            toast,
            claims_shortcut,
        });
        self._expiry = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_DURATION).await;
            this.update(cx, |this, cx| {
                if this.serial == serial {
                    this.dismiss(cx);
                }
            })
            .ok();
        }));
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.shown.take().is_some() {
            self._expiry = None;
            cx.notify();
        }
    }

    /// The action to run for `keystroke`, which then ends the notice; any other key gives the
    /// shortcut back to whatever has focus.
    fn take_for_key(
        &mut self,
        keystroke: &Keystroke,
        cx: &mut Context<Self>,
    ) -> Option<ToastHandler> {
        let shown = self.shown.as_mut()?;
        if !shown.claims_shortcut {
            return None;
        }
        let action = shown.toast.action.as_ref()?;
        let shortcut = action.shortcut.as_ref()?;
        if !(shortcut.matches)(keystroke, cx) {
            shown.claims_shortcut = false;
            return None;
        }
        let run = action.run.clone();
        self.dismiss(cx);
        Some(run)
    }

    fn run_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(run) = self
            .shown
            .as_ref()
            .and_then(|shown| shown.toast.action.as_ref())
            .map(|action| action.run.clone())
        else {
            return;
        };
        self.dismiss(cx);
        run(window, cx);
    }
}

impl Render for Toasts {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div().relative().h_0().w_full();
        let Some(shown) = &self.shown else {
            return root;
        };
        let colors = cx.hoshi().clone();
        let toast = &shown.toast;
        let link = colors.link;
        let notice = div()
            .id("toast")
            .debug_selector(|| "toast".into())
            .occlude()
            .flex()
            .items_center()
            .gap(px(10.))
            .px_3()
            .py_2()
            .rounded(px(10.))
            .border_1()
            .border_color(colors.border)
            .bg(colors.popover)
            .text_color(colors.popover_foreground)
            .shadow_md()
            .text_sm()
            .child(
                Icon::new(toast.icon)
                    .size_4()
                    .flex_none()
                    .text_color(colors.status_working),
            )
            .child(toast.message.clone())
            .when_some(toast.action.as_ref(), |this, action| {
                this.child(
                    div()
                        .id("toast-action")
                        .debug_selector(|| "toast-action".into())
                        .cursor_pointer()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(link)
                        .hover(|style| style.underline())
                        .on_click(cx.listener(|this, _, window, cx| this.run_action(window, cx)))
                        .child(action.label.clone()),
                )
                .when_some(action.shortcut.as_ref(), |this, shortcut| {
                    this.child(Kbd::new(shortcut.label.clone()))
                })
            });
        root.child(
            div()
                .absolute()
                .bottom_3()
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(notice),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Toast, ToastAction, Toasts, install, show, undo_shortcut};
    use gpui_kit::component::input::{Input, InputState};
    use gpui_kit::component::theme::ThemeMode;
    use gpui_kit::{
        AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
        TestAppContext, VisualTestContext, Window, WindowOptions, div,
    };
    use hoshi_ui::prelude::IconName;
    use std::cell::Cell;
    use std::rc::Rc;

    struct Shell {
        toasts: Entity<Toasts>,
        field: Entity<InputState>,
    }

    impl Render for Shell {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(Input::new(&self.field))
                .child(div().flex_1())
                .child(self.toasts.clone())
        }
    }

    fn harness(cx: &mut TestAppContext) -> (VisualTestContext, Entity<InputState>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            hoshi_ui::theme::apply(ThemeMode::Light, cx);
        });
        let (window, field) = cx.update(|cx| {
            let toasts = install(cx);
            let mut field = None;
            let (window, _) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                let input = cx.new(|cx| InputState::new(window, cx));
                field = Some(input.clone());
                cx.new(|_| Shell {
                    toasts,
                    field: input,
                })
            })
            .unwrap();
            (window, field.unwrap())
        });
        let mut cx = VisualTestContext::from_window(window, cx);
        cx.run_until_parked();
        field.update_in(&mut cx, |field, window, cx| field.focus(window, cx));
        (cx, field)
    }

    fn notice(cx: &mut VisualTestContext, runs: &Rc<Cell<usize>>, message: &'static str) {
        let counted = runs.clone();
        cx.update(|_, cx| {
            let shortcut = undo_shortcut(cx);
            show(
                Toast {
                    icon: IconName::AlarmClock,
                    message: message.into(),
                    action: Some(ToastAction {
                        label: "Undo".into(),
                        shortcut: Some(shortcut),
                        run: Rc::new(move |_, _| counted.set(counted.get() + 1)),
                    }),
                },
                cx,
            )
        });
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn the_shortcut_runs_the_action_until_another_key_is_pressed(cx: &mut TestAppContext) {
        let (mut cx, field) = harness(cx);
        let runs = Rc::new(Cell::new(0));
        notice(&mut cx, &runs, "first");
        assert!(cx.debug_bounds("toast").is_some());
        cx.simulate_keystrokes("ctrl-z");
        cx.run_until_parked();
        assert_eq!(runs.get(), 1);
        assert!(cx.debug_bounds("toast").is_none());

        notice(&mut cx, &runs, "second");
        cx.simulate_input("a");
        cx.simulate_keystrokes("ctrl-z");
        cx.run_until_parked();
        assert_eq!(runs.get(), 1, "after typing, the field keeps its undo");
        assert!(cx.debug_bounds("toast").is_some());
        let text = field.read_with(&cx, |field, _| field.value().to_string());
        assert_eq!(text, "");
    }

    #[gpui_kit::test]
    fn a_new_notice_replaces_the_shown_one_and_each_goes_away_by_itself(cx: &mut TestAppContext) {
        let (mut cx, _) = harness(cx);
        let runs = Rc::new(Cell::new(0));
        notice(&mut cx, &runs, "first");
        cx.executor().advance_clock(super::TOAST_DURATION / 2);
        notice(&mut cx, &runs, "second");
        cx.executor()
            .advance_clock(super::TOAST_DURATION / 2 + std::time::Duration::from_millis(100));
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("toast").is_some(),
            "the second keeps its own time"
        );
        let action = cx.debug_bounds("toast-action").unwrap();
        cx.simulate_click(action.center(), gpui_kit::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(runs.get(), 1);

        notice(&mut cx, &runs, "third");
        cx.executor().advance_clock(super::TOAST_DURATION);
        cx.run_until_parked();
        assert!(cx.debug_bounds("toast").is_none());
    }
}
