use gpui_kit::component::theme::ThemeMode;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, ParentElement as _, Pixels, Point, Render,
    StatefulInteractiveElement as _, Styled as _, Task, TestAppContext, VisualTestContext, Window,
    WindowBounds, WindowOptions, div, point, px, size,
};
use hoshi_ui::theme;
use hoshi_ui::{
    ConfirmChoice, ConfirmRequest, Dialog, DialogButton, DialogHandle, DialogLayer, InitialFocus,
    OpenChange, OpenChangeReason, Popover,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

struct Screen {
    open: bool,
    nested_open: bool,
    alert: bool,
    veto: bool,
    mounted: bool,
    changes: Rc<RefCell<Vec<(&'static str, OpenChange)>>>,
    background_clicks: Rc<Cell<usize>>,
    opener: FocusHandle,
    first: FocusHandle,
    second: FocusHandle,
    nested_button: FocusHandle,
}

impl Screen {
    fn new(alert: bool, cx: &mut Context<Self>) -> Self {
        Self {
            open: false,
            nested_open: false,
            alert,
            veto: false,
            mounted: true,
            changes: Rc::default(),
            background_clicks: Rc::default(),
            opener: cx.focus_handle().tab_stop(true),
            first: cx.focus_handle(),
            second: cx.focus_handle(),
            nested_button: cx.focus_handle(),
        }
    }

    fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.open = open;
        cx.notify();
    }
}

impl Render for Screen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let clicks = self.background_clicks.clone();
        let changes = self.changes.clone();
        let nested_changes = self.changes.clone();
        div()
            .size_full()
            .child(
                div()
                    .id("opener")
                    .track_focus(&self.opener)
                    .size(px(100.))
                    .on_click(move |_, _, _| clicks.set(clicks.get() + 1)),
            )
            .when(self.mounted, |this| {
                this.child(
                    Dialog::new("dialog")
                        .open(self.open)
                        .when(self.alert, |this| this.alert())
                        .on_open_change(cx.listener(move |this, change: &OpenChange, _, cx| {
                            changes.borrow_mut().push(("dialog", *change));
                            if !this.veto {
                                this.set_open(change.open, cx);
                            }
                        }))
                        .title("Rename")
                        .description("Pick a new name")
                        .child(
                            DialogButton::new("first")
                                .label("First")
                                .track_focus(&self.first)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.nested_open = true;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Popover::new("popover")
                                .w(px(560.))
                                .trigger(|_| {
                                    div()
                                        .id("popover-trigger")
                                        .debug_selector(|| "popover-trigger".into())
                                        .w(px(80.))
                                        .h(px(24.))
                                })
                                .content(|_, _, _| {
                                    div()
                                        .id("popover-content")
                                        .debug_selector(|| "popover-content".into())
                                        .h(px(60.))
                                }),
                        )
                        .child(
                            Dialog::new("nested")
                                .open(self.nested_open)
                                .on_open_change(cx.listener(
                                    move |this, change: &OpenChange, _, cx| {
                                        nested_changes.borrow_mut().push(("nested", *change));
                                        this.nested_open = change.open;
                                        cx.notify();
                                    },
                                ))
                                .title("Nested")
                                .footer(
                                    DialogButton::new("nested-ok")
                                        .label("OK")
                                        .track_focus(&self.nested_button)
                                        .close(),
                                ),
                        )
                        .footer(
                            DialogButton::new("second")
                                .label("Second")
                                .track_focus(&self.second)
                                .close(),
                        ),
                )
            })
    }
}

fn window(cx: &mut TestAppContext, alert: bool) -> (Entity<Screen>, VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        DialogLayer::init(cx);
        theme::apply(ThemeMode::Dark, cx);
        cx.set_reduce_motion(true);
    });
    let (window, screen) = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(800.), px(600.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| cx.new(|cx| Screen::new(alert, cx))).unwrap()
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.run_until_parked();
    let opener = screen.read_with(&cx, |screen, _| screen.opener.clone());
    cx.update(|window, cx| opener.focus(window, cx));
    cx.run_until_parked();
    (screen, cx)
}

fn open(screen: &Entity<Screen>, cx: &mut VisualTestContext) {
    screen.update(cx, |screen, cx| screen.set_open(true, cx));
    cx.run_until_parked();
}

fn focused(cx: &mut VisualTestContext, handle: &FocusHandle) -> bool {
    cx.update(|window, _| handle.is_focused(window))
}

fn open_count(cx: &mut VisualTestContext) -> usize {
    cx.update(|window, cx| DialogLayer::open_count(window, cx))
}

fn changes(screen: &Entity<Screen>, cx: &mut VisualTestContext) -> Vec<(&'static str, OpenChange)> {
    screen.read_with(cx, |screen, _| screen.changes.borrow().clone())
}

fn closed_by(reason: OpenChangeReason) -> OpenChange {
    OpenChange {
        open: false,
        reason,
    }
}

fn press(cx: &mut VisualTestContext, key: &'static str) {
    let keystroke = Keystroke::parse(key).unwrap();
    cx.simulate_event(KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(KeyUpEvent { keystroke });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn opening_moves_focus_to_the_first_tab_stop_and_escape_returns_it(cx: &mut TestAppContext) {
    let (screen, mut cx) = window(cx, false);
    let (opener, first) = screen.read_with(&cx, |s, _| (s.opener.clone(), s.first.clone()));

    open(&screen, &mut cx);
    assert_eq!(open_count(&mut cx), 1);
    assert!(focused(&mut cx, &first), "focus moves into the popup");
    assert!(cx.debug_bounds("hoshi-dialog-popup-0").is_some());

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(
        changes(&screen, &mut cx),
        [("dialog", closed_by(OpenChangeReason::EscapeKey))]
    );
    assert_eq!(open_count(&mut cx), 0);
    assert!(focused(&mut cx, &opener), "focus returns to the opener");
    assert!(cx.debug_bounds("hoshi-dialog-popup-0").is_none());
}

#[gpui_kit::test]
fn a_controlled_dialog_stays_open_until_its_owner_closes_it(cx: &mut TestAppContext) {
    let (screen, mut cx) = window(cx, false);
    let first = screen.read_with(&cx, |s, _| s.first.clone());
    screen.update(&mut cx, |screen, _| screen.veto = true);
    open(&screen, &mut cx);

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(
        changes(&screen, &mut cx),
        [("dialog", closed_by(OpenChangeReason::EscapeKey))]
    );
    assert_eq!(open_count(&mut cx), 1, "the owner kept it open");
    assert!(focused(&mut cx, &first));
}

#[gpui_kit::test]
fn tab_is_trapped_inside_the_dialog(cx: &mut TestAppContext) {
    let (screen, mut cx) = window(cx, false);
    let (opener, first, second) = screen.read_with(&cx, |s, _| {
        (s.opener.clone(), s.first.clone(), s.second.clone())
    });
    open(&screen, &mut cx);
    assert!(focused(&mut cx, &first));

    cx.simulate_keystrokes("tab");
    assert!(
        !focused(&mut cx, &opener) && !focused(&mut cx, &first) && !focused(&mut cx, &second),
        "the popover trigger comes next"
    );
    cx.simulate_keystrokes("tab");
    assert!(focused(&mut cx, &second));
    cx.simulate_keystrokes("tab");
    assert!(
        !focused(&mut cx, &opener) && !focused(&mut cx, &first) && !focused(&mut cx, &second),
        "the corner close button is the last stop"
    );
    cx.simulate_keystrokes("tab");
    assert!(
        focused(&mut cx, &first),
        "Tab wraps around inside the dialog"
    );
    cx.simulate_keystrokes("shift-tab");
    assert!(
        !focused(&mut cx, &opener),
        "Shift+Tab doesn't leave it either"
    );
    cx.simulate_keystrokes("shift-tab");
    assert!(focused(&mut cx, &second));
}

#[gpui_kit::test]
fn a_backdrop_press_closes_a_dialog_and_never_reaches_the_app(cx: &mut TestAppContext) {
    let (screen, mut cx) = window(cx, false);
    open(&screen, &mut cx);

    cx.simulate_click(point(px(20.), px(20.)), Modifiers::default());
    cx.run_until_parked();
    let clicks = screen.read_with(&cx, |s, _| s.background_clicks.get());
    assert_eq!(clicks, 0, "the opener under the backdrop stays inert");
    assert_eq!(
        changes(&screen, &mut cx),
        [("dialog", closed_by(OpenChangeReason::OutsidePress))]
    );
    assert_eq!(open_count(&mut cx), 0);

    cx.simulate_click(point(px(20.), px(20.)), Modifiers::default());
    let clicks = screen.read_with(&cx, |s, _| s.background_clicks.get());
    assert_eq!(clicks, 1, "the app takes clicks again once it closed");
}

#[gpui_kit::test]
fn a_press_inside_the_popup_does_not_close_it(cx: &mut TestAppContext) {
    let (screen, mut cx) = window(cx, false);
    open(&screen, &mut cx);
    let popup = cx.debug_bounds("hoshi-dialog-popup-0").unwrap();

    cx.simulate_click(popup.origin + point(px(4.), px(4.)), Modifiers::default());
    cx.run_until_parked();
    assert!(changes(&screen, &mut cx).is_empty());
    assert_eq!(open_count(&mut cx), 1);
}

#[gpui_kit::test]
fn an_alert_dialog_ignores_the_backdrop_but_not_escape(cx: &mut TestAppContext) {
    let (screen, mut cx) = window(cx, true);
    open(&screen, &mut cx);

    cx.simulate_click(point(px(20.), px(20.)), Modifiers::default());
    cx.run_until_parked();
    assert!(changes(&screen, &mut cx).is_empty());
    assert_eq!(open_count(&mut cx), 1);
    let clicks = screen.read_with(&cx, |s, _| s.background_clicks.get());
    assert_eq!(clicks, 0);

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(
        changes(&screen, &mut cx),
        [("dialog", closed_by(OpenChangeReason::EscapeKey))]
    );
}

#[gpui_kit::test]
fn a_close_button_reports_a_close_press(cx: &mut TestAppContext) {
    let (screen, mut cx) = window(cx, false);
    let opener = screen.read_with(&cx, |s, _| s.opener.clone());
    open(&screen, &mut cx);
    cx.simulate_keystrokes("tab tab");

    press(&mut cx, "enter");
    assert_eq!(
        changes(&screen, &mut cx),
        [("dialog", closed_by(OpenChangeReason::ClosePress))]
    );
    assert!(focused(&mut cx, &opener));
}

#[gpui_kit::test]
fn escape_closes_only_the_nested_dialog_on_top(cx: &mut TestAppContext) {
    let (screen, mut cx) = window(cx, false);
    let (first, nested_button) =
        screen.read_with(&cx, |s, _| (s.first.clone(), s.nested_button.clone()));
    open(&screen, &mut cx);

    press(&mut cx, "enter");
    cx.run_until_parked();
    assert_eq!(open_count(&mut cx), 2);
    assert!(focused(&mut cx, &nested_button));
    let nested = cx.debug_bounds("hoshi-dialog-popup-1").unwrap();
    let parent = cx.debug_bounds("hoshi-dialog-popup-0").unwrap();

    cx.simulate_click(parent.origin + point(px(4.), px(4.)), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        changes(&screen, &mut cx),
        [("nested", closed_by(OpenChangeReason::OutsidePress))],
        "the parent is inert: a press on it is outside the nested dialog"
    );
    assert!(focused(&mut cx, &first), "focus returns into the parent");
    assert!(nested.size.width > px(0.));

    press(&mut cx, "enter");
    cx.run_until_parked();
    assert_eq!(open_count(&mut cx), 2);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(open_count(&mut cx), 1, "Escape closes only the top dialog");
    assert!(focused(&mut cx, &first));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(open_count(&mut cx), 0);
}

#[gpui_kit::test]
fn closing_a_parent_closes_the_dialogs_opened_from_it(cx: &mut TestAppContext) {
    let (screen, mut cx) = window(cx, false);
    let opener = screen.read_with(&cx, |s, _| s.opener.clone());
    open(&screen, &mut cx);
    press(&mut cx, "enter");
    assert_eq!(open_count(&mut cx), 2);

    screen.update(&mut cx, |screen, cx| screen.set_open(false, cx));
    cx.run_until_parked();
    assert_eq!(open_count(&mut cx), 0);
    assert!(focused(&mut cx, &opener));
}

fn ask(cx: &mut VisualTestContext, request: ConfirmRequest) -> Task<bool> {
    cx.update(|window, cx| DialogLayer::confirm(request, window, cx))
}

fn answer(cx: &mut VisualTestContext, task: Task<bool>) -> bool {
    let answer = Rc::new(Cell::new(None));
    let slot = answer.clone();
    TestAppContext::update(cx, |cx| {
        cx.spawn(async move |_| slot.set(Some(task.await))).detach()
    });
    cx.run_until_parked();
    answer.get().expect("the confirm resolved")
}

fn delete() -> ConfirmRequest {
    ConfirmRequest::new("Delete thread?", "Delete")
        .description("This can't be undone.")
        .cancel_label("Cancel")
        .destructive()
}

#[gpui_kit::test]
fn enter_confirms_because_the_confirm_button_has_focus(cx: &mut TestAppContext) {
    let (screen, mut cx) = window(cx, false);
    let opener = screen.read_with(&cx, |s, _| s.opener.clone());
    let task = ask(&mut cx, delete());
    cx.run_until_parked();
    assert_eq!(open_count(&mut cx), 1);

    press(&mut cx, "enter");
    assert!(answer(&mut cx, task));
    assert_eq!(open_count(&mut cx), 0);
    assert!(focused(&mut cx, &opener));
}

#[gpui_kit::test]
fn cancel_escape_and_the_backdrop_answer_no(cx: &mut TestAppContext) {
    let (_, mut cx) = window(cx, false);

    let task = ask(&mut cx, delete());
    cx.run_until_parked();
    cx.simulate_keystrokes("shift-tab");
    press(&mut cx, "enter");
    assert!(!answer(&mut cx, task), "the cancel button");

    let task = ask(&mut cx, delete());
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    assert!(!answer(&mut cx, task), "Escape");

    let task = ask(&mut cx, delete());
    cx.run_until_parked();
    cx.simulate_click(point(px(20.), px(20.)), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(open_count(&mut cx), 1, "a confirm is an alert dialog");
    cx.simulate_keystrokes("escape");
    assert!(!answer(&mut cx, task));
}

#[gpui_kit::test]
fn a_choice_tells_the_alternative_from_confirming_and_cancelling(cx: &mut TestAppContext) {
    let (_, mut cx) = window(cx, false);
    let rewind = || {
        ConfirmRequest::new("Edit from here?", "Revert and keep changes")
            .cancel_label("Cancel")
            .alternative("Revert files too", true)
    };
    let mut choose = |keys: &[&'static str]| {
        let task = cx.update(|window, cx| DialogLayer::choose(rewind(), window, cx));
        cx.run_until_parked();
        for key in keys {
            press(&mut cx, key);
        }
        let answer = Rc::new(Cell::new(None));
        let slot = answer.clone();
        TestAppContext::update(&cx, |cx| {
            cx.spawn(async move |_| slot.set(Some(task.await))).detach()
        });
        cx.run_until_parked();
        answer.get().expect("the choice resolved")
    };
    assert_eq!(choose(&["enter"]), Some(ConfirmChoice::Confirm));
    assert_eq!(
        choose(&["shift-tab", "enter"]),
        Some(ConfirmChoice::Alternative)
    );
    assert_eq!(choose(&["shift-tab", "shift-tab", "enter"]), None);
    assert_eq!(choose(&["escape"]), None);
}

#[gpui_kit::test]
fn a_second_confirm_waits_for_the_first(cx: &mut TestAppContext) {
    let (_, mut cx) = window(cx, false);
    let first = ask(&mut cx, delete());
    let second = ask(&mut cx, ConfirmRequest::new("Archive?", "Archive"));
    cx.run_until_parked();
    assert_eq!(open_count(&mut cx), 1);

    press(&mut cx, "enter");
    assert!(answer(&mut cx, first));
    assert_eq!(open_count(&mut cx), 1, "the queued one shows next");
    cx.simulate_keystrokes("escape");
    assert!(!answer(&mut cx, second));
    assert_eq!(open_count(&mut cx), 0);
}

#[gpui_kit::test]
fn a_confirm_resolves_no_when_its_window_closes(cx: &mut TestAppContext) {
    let (_, mut cx) = window(cx, false);
    let task = ask(&mut cx, delete());
    cx.run_until_parked();

    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
    assert!(!answer(&mut cx, task));
}

#[gpui_kit::test]
fn an_opened_dialog_reports_its_imperative_close(cx: &mut TestAppContext) {
    let (screen, mut cx) = window(cx, false);
    let opener = screen.read_with(&cx, |s, _| s.opener.clone());
    let reasons = Rc::new(RefCell::new(Vec::new()));
    let handle: Rc<RefCell<Option<DialogHandle>>> = Rc::default();
    let opened = cx.update(|window, cx| {
        let reasons = reasons.clone();
        let handle = handle.clone();
        DialogLayer::open(window, cx, move |dialog, _, _| {
            let reasons = reasons.clone();
            let handle = handle.clone();
            dialog
                .title("Opened")
                .initial_focus(InitialFocus::Popup)
                .on_open_change(move |change, window, cx| {
                    reasons.borrow_mut().push(change.reason);
                    if let Some(handle) = handle.borrow().as_ref() {
                        handle.close(window, cx);
                    }
                })
        })
    });
    *handle.borrow_mut() = Some(opened.clone());
    let handle = opened;
    cx.run_until_parked();
    assert!(cx.update(|_, cx| handle.is_open(cx)));
    assert!(!focused(&mut cx, &opener));

    cx.update(|window, cx| handle.close(window, cx));
    cx.run_until_parked();
    assert!(!cx.update(|_, cx| handle.is_open(cx)));
    assert_eq!(*reasons.borrow(), [OpenChangeReason::Imperative]);
    assert!(focused(&mut cx, &opener));
}

#[gpui_kit::test]
fn reopening_while_it_fades_out_keeps_one_dialog(cx: &mut TestAppContext) {
    let (screen, mut cx) = window(cx, false);
    cx.update(|_, cx| cx.set_reduce_motion(false));
    open(&screen, &mut cx);
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();

    screen.update(&mut cx, |screen, cx| screen.set_open(false, cx));
    cx.run_until_parked();
    assert_eq!(open_count(&mut cx), 0);
    assert!(
        cx.debug_bounds("hoshi-dialog-popup-0").is_some(),
        "it stays mounted while it fades out"
    );

    open(&screen, &mut cx);
    assert_eq!(open_count(&mut cx), 1);
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    assert_eq!(open_count(&mut cx), 1, "the old exit doesn't unmount it");
    assert!(cx.debug_bounds("hoshi-dialog-popup-1").is_none());

    screen.update(&mut cx, |screen, cx| screen.set_open(false, cx));
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    assert!(cx.debug_bounds("hoshi-dialog-popup-0").is_none());
}

#[gpui_kit::test]
fn a_dialog_its_owner_stops_rendering_unmounts(cx: &mut TestAppContext) {
    let (screen, mut cx) = window(cx, false);
    let opener = screen.read_with(&cx, |s, _| s.opener.clone());
    open(&screen, &mut cx);
    assert_eq!(open_count(&mut cx), 1);

    screen.update(&mut cx, |screen, cx| {
        screen.mounted = false;
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(open_count(&mut cx), 0);
    assert!(changes(&screen, &mut cx).is_empty());
    assert!(focused(&mut cx, &opener));
}

fn open_popover(cx: &mut VisualTestContext) -> Bounds<Pixels> {
    let trigger = cx.debug_bounds("popover-trigger").unwrap();
    cx.simulate_click(trigger.center(), Modifiers::default());
    cx.run_until_parked();
    cx.debug_bounds("popover-popup")
        .expect("the popover opened")
}

#[gpui_kit::test]
fn a_press_on_a_popover_past_the_dialog_keeps_both_open(cx: &mut TestAppContext) {
    let (screen, mut cx) = window(cx, false);
    open(&screen, &mut cx);
    let panel = cx.debug_bounds("hoshi-dialog-popup-0").unwrap();
    let popup = open_popover(&mut cx);
    let outside_panel = point(popup.right() - px(8.), popup.center().y);
    assert!(
        popup.contains(&outside_panel) && !panel.contains(&outside_panel),
        "the popover hangs past the dialog: {popup:?} {panel:?}"
    );

    cx.simulate_click(outside_panel, Modifiers::default());
    cx.run_until_parked();
    assert!(changes(&screen, &mut cx).is_empty());
    assert_eq!(open_count(&mut cx), 1);
    assert!(cx.debug_bounds("popover-popup").is_some());
}

#[gpui_kit::test]
fn a_backdrop_press_with_a_popover_open_closes_only_the_popover(cx: &mut TestAppContext) {
    let (screen, mut cx) = window(cx, false);
    open(&screen, &mut cx);
    open_popover(&mut cx);

    cx.simulate_click(point(px(20.), px(580.)), Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("popover-popup").is_none());
    assert!(changes(&screen, &mut cx).is_empty());
    assert_eq!(open_count(&mut cx), 1);

    cx.simulate_click(point(px(20.), px(580.)), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        changes(&screen, &mut cx),
        [("dialog", closed_by(OpenChangeReason::OutsidePress))]
    );
}

#[gpui_kit::test]
fn escape_closes_the_popover_first_then_the_dialog(cx: &mut TestAppContext) {
    let (screen, mut cx) = window(cx, false);
    open(&screen, &mut cx);
    open_popover(&mut cx);

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("popover-popup").is_none());
    assert!(changes(&screen, &mut cx).is_empty());
    assert_eq!(open_count(&mut cx), 1);

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(
        changes(&screen, &mut cx),
        [("dialog", closed_by(OpenChangeReason::EscapeKey))]
    );
}

#[gpui_kit::test]
fn tab_from_a_popover_stays_inside_the_dialog(cx: &mut TestAppContext) {
    let (screen, mut cx) = window(cx, false);
    let opener = screen.read_with(&cx, |s, _| s.opener.clone());
    open(&screen, &mut cx);
    open_popover(&mut cx);

    for _ in 0..6 {
        cx.simulate_keystrokes("tab");
        cx.run_until_parked();
        assert!(!focused(&mut cx, &opener), "Tab never reaches the app");
        assert!(cx.update(|window, cx| window.focused(cx).is_some()));
    }
    assert_eq!(open_count(&mut cx), 1);
    assert!(changes(&screen, &mut cx).is_empty());
}

#[gpui_kit::test]
fn focus_moved_out_of_a_modal_dialog_comes_back(cx: &mut TestAppContext) {
    let (screen, mut cx) = window(cx, false);
    let (opener, first) = screen.read_with(&cx, |s, _| (s.opener.clone(), s.first.clone()));
    open(&screen, &mut cx);

    cx.update(|window, cx| opener.focus(window, cx));
    cx.run_until_parked();
    assert!(focused(&mut cx, &first));
}

#[gpui_kit::test]
fn the_first_click_after_escape_reaches_the_window(cx: &mut TestAppContext) {
    let (screen, mut cx) = window(cx, false);
    cx.update(|_, cx| cx.set_reduce_motion(false));
    open(&screen, &mut cx);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_click(point(px(50.), px(50.)), Modifiers::none());
    cx.run_until_parked();
    let clicks = screen.read_with(&cx, |screen, _| screen.background_clicks.get());
    assert_eq!(clicks, 1);
}
