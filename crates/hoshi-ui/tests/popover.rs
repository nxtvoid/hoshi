//! The popover and its menu in a headless window: opening and closing by every way Base UI has,
//! the keyboard, nesting and placement near the window's edges.

use gpui_kit::component::theme::ThemeMode;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, InteractiveElement as _, IntoElement, Modifiers,
    MouseButton, ParentElement as _, Pixels, Point, Render, StatefulInteractiveElement as _,
    Styled as _, TestAppContext, VisualTestContext, WeakEntity, Window, WindowBounds,
    WindowOptions, div, point, px, size,
};
use hoshi_ui::{
    Dialog, DialogButton, DialogLayer, MenuOption, Popover, PopoverChange, PopoverChangeReason,
    PopoverContext, PopoverMenu,
};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Content {
    PopoverMenu,
    Plain,
    Nested,
    Tall,
    Submenu,
}

struct Harness {
    content: Content,
    controlled: bool,
    open: bool,
    hover: bool,
    shown: bool,
    trigger_at: Point<Pixels>,
    changes: Vec<PopoverChange>,
    picked: Vec<&'static str>,
    under_clicks: usize,
}

fn pick(
    this: &WeakEntity<Harness>,
    name: &'static str,
) -> impl Fn(&mut Window, &mut gpui_kit::App) + 'static {
    let this = this.clone();
    move |_, cx| {
        this.update(cx, |harness, _| harness.picked.push(name)).ok();
    }
}

fn menu(menu: PopoverMenu, this: &WeakEntity<Harness>) -> PopoverMenu {
    menu.heading("Pick")
        .item(MenuOption::new("alpha", "Alpha").on_select(pick(this, "alpha")))
        .item(
            MenuOption::new("beta", "Beta")
                .disabled(true)
                .on_select(pick(this, "beta")),
        )
        .item(MenuOption::new("gamma", "Gamma").on_select(pick(this, "gamma")))
        .separator()
        .item(MenuOption::radio("delta", "Delta", false).on_select(pick(this, "delta")))
}

fn plain(_: &PopoverContext, this: &WeakEntity<Harness>) -> impl IntoElement + use<> {
    let this = this.clone();
    div()
        .id("plain-content")
        .debug_selector(|| "plain-content".into())
        .h(px(60.))
        .on_click(move |_, _, cx| {
            this.update(cx, |harness, _| harness.picked.push("plain"))
                .ok();
        })
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        let content = self.content;
        let popover = Popover::new("subject")
            .when(self.controlled, |popover| {
                popover.open(self.open).on_open_change(cx.listener(
                    |this, change: &PopoverChange, _, cx| {
                        this.changes.push(*change);
                        this.open = change.open;
                        cx.notify();
                    },
                ))
            })
            .open_on_hover(self.hover)
            .trigger(|open| {
                div()
                    .id("trigger")
                    .debug_selector(|| "trigger".into())
                    .w(px(80.))
                    .h(px(24.))
                    .child(if open { "Open" } else { "Closed" })
            });
        let popover = match content {
            Content::PopoverMenu => popover.menu(move |m, _, _| menu(m, &this)),
            Content::Tall => popover.menu(move |m, _, _| {
                m.items((0..60usize).map(|index| {
                    MenuOption::new(("row", index), format!("Row {index}"))
                        .on_select(pick(&this, "row"))
                }))
            }),
            Content::Submenu => popover.menu(move |m, _, _| {
                let inner = this.clone();
                m.item(MenuOption::new("alpha", "Alpha").on_select(pick(&this, "alpha")))
                    .submenu("more", "More", None, move |sub, _, _| {
                        sub.item(MenuOption::new("deep", "Deep").on_select(pick(&inner, "deep")))
                    })
            }),
            Content::Plain => popover.content(move |popover, _, _| plain(popover, &this)),
            Content::Nested => popover.content(move |popover, _, _| {
                let inner = this.clone();
                div().child(plain(popover, &this)).child(
                    Popover::new("inner")
                        .side(hoshi_ui::Side::Right)
                        .trigger(|_| {
                            div()
                                .id("inner-trigger")
                                .debug_selector(|| "inner-trigger".into())
                                .w(px(60.))
                                .h(px(20.))
                        })
                        .content(move |_, _, _| {
                            let inner = inner.clone();
                            div()
                                .id("inner-content")
                                .debug_selector(|| "inner-content".into())
                                .h(px(40.))
                                .on_click(move |_, _, cx| {
                                    inner
                                        .update(cx, |harness, _| harness.picked.push("inner"))
                                        .ok();
                                })
                        }),
                )
            }),
        };
        div()
            .size_full()
            .child(
                div()
                    .id("under")
                    .debug_selector(|| "under".into())
                    .absolute()
                    .left(px(300.))
                    .top(px(300.))
                    .size(px(60.))
                    .on_click(cx.listener(|this, _, _, _| this.under_clicks += 1)),
            )
            .when(self.shown, |root| {
                root.child(
                    div()
                        .absolute()
                        .left(self.trigger_at.x)
                        .top(self.trigger_at.y)
                        .child(popover),
                )
            })
    }
}

struct Setup {
    content: Content,
    controlled: bool,
    hover: bool,
    trigger_at: Point<Pixels>,
}

impl Setup {
    fn new(content: Content) -> Self {
        Self {
            content,
            controlled: true,
            hover: false,
            trigger_at: point(px(100.), px(100.)),
        }
    }
}

fn open_window(cx: &mut TestAppContext, setup: Setup) -> (VisualTestContext, Entity<Harness>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        hoshi_ui::theme::apply(ThemeMode::Light, cx);
        cx.set_reduce_motion(true);
    });
    let (window, view) = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(800.), px(400.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| {
            cx.new(|_| Harness {
                content: setup.content,
                controlled: setup.controlled,
                open: false,
                hover: setup.hover,
                shown: true,
                trigger_at: setup.trigger_at,
                changes: Vec::new(),
                picked: Vec::new(),
                under_clicks: 0,
            })
        })
        .expect("test window")
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    redraw(&mut cx);
    (cx, view)
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    for _ in 0..2 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
}

fn center(cx: &mut VisualTestContext, selector: &'static str) -> Point<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not on screen"))
        .center()
}

fn click_at(cx: &mut VisualTestContext, position: Point<Pixels>) {
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
    redraw(cx);
    cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
    redraw(cx);
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let position = center(cx, selector);
    click_at(cx, position);
}

fn press(cx: &mut VisualTestContext, keys: &str) {
    cx.simulate_keystrokes(keys);
    redraw(cx);
}

fn popup(cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
    cx.debug_bounds("popover-popup")
}

fn reasons(view: &Entity<Harness>, cx: &mut VisualTestContext) -> Vec<(bool, PopoverChangeReason)> {
    view.read_with(cx, |harness, _| {
        harness
            .changes
            .iter()
            .map(|change| (change.open, change.reason))
            .collect()
    })
}

fn picked(view: &Entity<Harness>, cx: &mut VisualTestContext) -> Vec<&'static str> {
    view.read_with(cx, |harness, _| harness.picked.clone())
}

#[gpui_kit::test]
fn a_press_on_the_trigger_opens_and_closes_the_menu(cx: &mut TestAppContext) {
    let (mut cx, view) = open_window(cx, Setup::new(Content::PopoverMenu));
    assert!(popup(&mut cx).is_none());

    click(&mut cx, "trigger");
    let bounds = popup(&mut cx).expect("the menu opens");
    let trigger = cx.debug_bounds("trigger").unwrap();
    assert_eq!(
        bounds.top(),
        trigger.bottom() + px(4.),
        "under the trigger, 4px off"
    );
    assert_eq!(
        bounds.left(),
        trigger.left(),
        "a menu lines up with the trigger's start"
    );

    click(&mut cx, "trigger");
    assert!(popup(&mut cx).is_none());
    assert_eq!(
        reasons(&view, &mut cx),
        vec![
            (true, PopoverChangeReason::TriggerPress),
            (false, PopoverChangeReason::TriggerPress)
        ]
    );
}

#[gpui_kit::test]
fn an_outside_press_closes_a_menu_without_reaching_what_is_under_it(cx: &mut TestAppContext) {
    let (mut cx, view) = open_window(cx, Setup::new(Content::PopoverMenu));
    click(&mut cx, "trigger");
    click(&mut cx, "under");
    assert!(popup(&mut cx).is_none());
    assert_eq!(
        reasons(&view, &mut cx).last(),
        Some(&(false, PopoverChangeReason::OutsidePress))
    );
    assert_eq!(view.read_with(&cx, |harness, _| harness.under_clicks), 0);

    click(&mut cx, "under");
    assert_eq!(view.read_with(&cx, |harness, _| harness.under_clicks), 1);
}

#[gpui_kit::test]
fn an_outside_press_closes_a_popover_and_reaches_what_is_under_it(cx: &mut TestAppContext) {
    let (mut cx, view) = open_window(cx, Setup::new(Content::Plain));
    click(&mut cx, "trigger");
    assert!(popup(&mut cx).is_some());
    click(&mut cx, "plain-content");
    assert!(popup(&mut cx).is_some(), "a press inside keeps it open");

    click(&mut cx, "under");
    assert!(popup(&mut cx).is_none());
    assert_eq!(view.read_with(&cx, |harness, _| harness.under_clicks), 1);
    assert_eq!(
        reasons(&view, &mut cx).last(),
        Some(&(false, PopoverChangeReason::OutsidePress))
    );
}

#[gpui_kit::test]
fn escape_closes_and_gives_the_keyboard_back_to_the_trigger(cx: &mut TestAppContext) {
    let (mut cx, view) = open_window(cx, Setup::new(Content::PopoverMenu));
    click(&mut cx, "trigger");
    press(&mut cx, "escape");
    assert!(popup(&mut cx).is_none());
    assert_eq!(
        reasons(&view, &mut cx).last(),
        Some(&(false, PopoverChangeReason::EscapeKey))
    );

    press(&mut cx, "down");
    assert!(
        popup(&mut cx).is_some(),
        "the trigger has the keyboard again"
    );
    press(&mut cx, "enter");
    assert_eq!(
        picked(&view, &mut cx),
        vec!["alpha"],
        "opened from the keyboard on the first"
    );
}

#[gpui_kit::test]
fn an_item_runs_and_closes_the_menu_and_a_radio_item_keeps_it_open(cx: &mut TestAppContext) {
    let (mut cx, view) = open_window(cx, Setup::new(Content::PopoverMenu));
    click(&mut cx, "trigger");
    click(&mut cx, "menu-item-alpha");
    assert_eq!(picked(&view, &mut cx), vec!["alpha"]);
    assert!(popup(&mut cx).is_none());
    assert_eq!(
        reasons(&view, &mut cx).last(),
        Some(&(false, PopoverChangeReason::ItemPress))
    );

    click(&mut cx, "trigger");
    click(&mut cx, "menu-item-beta");
    assert_eq!(
        picked(&view, &mut cx),
        vec!["alpha"],
        "a disabled item does nothing"
    );
    click(&mut cx, "menu-item-delta");
    assert_eq!(picked(&view, &mut cx), vec!["alpha", "delta"]);
    assert!(popup(&mut cx).is_some());
}

#[gpui_kit::test]
fn the_arrows_skip_disabled_items_and_enter_picks(cx: &mut TestAppContext) {
    let (mut cx, view) = open_window(cx, Setup::new(Content::PopoverMenu));
    click(&mut cx, "trigger");
    press(&mut cx, "down");
    press(&mut cx, "down");
    press(&mut cx, "enter");
    assert_eq!(picked(&view, &mut cx), vec!["gamma"]);
    assert!(popup(&mut cx).is_none());

    press(&mut cx, "up");
    assert!(
        popup(&mut cx).is_some(),
        "up on the trigger opens on the last item"
    );
    press(&mut cx, "enter");
    assert_eq!(picked(&view, &mut cx), vec!["gamma", "delta"]);
    press(&mut cx, "home");
    press(&mut cx, "up");
    press(&mut cx, "space");
    assert_eq!(
        picked(&view, &mut cx),
        vec!["gamma", "delta", "delta"],
        "up from the first wraps to the last"
    );
}

#[gpui_kit::test]
fn typing_jumps_to_the_item_that_starts_with_it(cx: &mut TestAppContext) {
    let (mut cx, view) = open_window(cx, Setup::new(Content::PopoverMenu));
    click(&mut cx, "trigger");
    press(&mut cx, "g");
    press(&mut cx, "enter");
    assert_eq!(picked(&view, &mut cx), vec!["gamma"]);
}

#[gpui_kit::test]
fn the_pointer_highlights_the_item_under_it(cx: &mut TestAppContext) {
    let (mut cx, view) = open_window(cx, Setup::new(Content::PopoverMenu));
    click(&mut cx, "trigger");
    let gamma = center(&mut cx, "menu-item-gamma");
    cx.simulate_mouse_move(gamma, None, Modifiers::default());
    redraw(&mut cx);
    press(&mut cx, "enter");
    assert_eq!(picked(&view, &mut cx), vec!["gamma"]);
}

#[gpui_kit::test]
fn an_uncontrolled_popover_opens_and_closes_itself(cx: &mut TestAppContext) {
    let setup = Setup {
        controlled: false,
        ..Setup::new(Content::Plain)
    };
    let (mut cx, _) = open_window(cx, setup);
    click(&mut cx, "trigger");
    assert!(popup(&mut cx).is_some());
    press(&mut cx, "escape");
    assert!(popup(&mut cx).is_none());
    click(&mut cx, "trigger");
    assert!(popup(&mut cx).is_some());
    click(&mut cx, "trigger");
    assert!(popup(&mut cx).is_none());
}

#[gpui_kit::test]
fn a_press_in_a_nested_popover_keeps_its_parent_open(cx: &mut TestAppContext) {
    let (mut cx, view) = open_window(cx, Setup::new(Content::Nested));
    click(&mut cx, "trigger");
    click(&mut cx, "inner-trigger");
    assert!(cx.debug_bounds("inner-content").is_some());
    click(&mut cx, "inner-content");
    assert_eq!(picked(&view, &mut cx), vec!["inner"]);
    assert!(
        cx.debug_bounds("plain-content").is_some(),
        "the parent stays"
    );

    press(&mut cx, "escape");
    assert!(
        cx.debug_bounds("inner-content").is_none(),
        "escape closes the inner one"
    );
    assert!(cx.debug_bounds("plain-content").is_some());
    press(&mut cx, "escape");
    assert!(cx.debug_bounds("plain-content").is_none());
    assert_eq!(
        reasons(&view, &mut cx),
        vec![
            (true, PopoverChangeReason::TriggerPress),
            (false, PopoverChangeReason::EscapeKey)
        ]
    );
}

#[gpui_kit::test]
fn near_the_window_edge_the_menu_flips_and_stays_inside(cx: &mut TestAppContext) {
    let setup = Setup {
        trigger_at: point(px(740.), px(360.)),
        ..Setup::new(Content::PopoverMenu)
    };
    let (mut cx, _) = open_window(cx, setup);
    click(&mut cx, "trigger");
    let bounds = popup(&mut cx).expect("open");
    let trigger = cx.debug_bounds("trigger").unwrap();
    assert!(
        bounds.bottom() <= trigger.top() - px(4.),
        "flipped above: {bounds:?}"
    );
    assert!(
        bounds.right() <= px(795.),
        "kept 5px from the right edge: {bounds:?}"
    );
}

#[gpui_kit::test]
fn a_menu_taller_than_the_room_is_cut_to_it_and_scrolls(cx: &mut TestAppContext) {
    let (mut cx, view) = open_window(cx, Setup::new(Content::Tall));
    click(&mut cx, "trigger");
    let bounds = popup(&mut cx).expect("open");
    let trigger = cx.debug_bounds("trigger").unwrap();
    assert_eq!(bounds.top(), trigger.bottom() + px(4.));
    assert_eq!(
        bounds.bottom(),
        px(395.),
        "as tall as the room under the trigger"
    );

    press(&mut cx, "end");
    let last = cx.debug_bounds("menu-item-row-59").expect("rendered");
    assert!(
        last.bottom() <= bounds.bottom(),
        "the last row scrolled into view: {last:?} in {bounds:?}"
    );
    press(&mut cx, "enter");
    assert_eq!(picked(&view, &mut cx), vec!["row"]);
}

#[gpui_kit::test]
fn hover_opens_after_the_delay_and_leaving_closes(cx: &mut TestAppContext) {
    let setup = Setup {
        hover: true,
        ..Setup::new(Content::Plain)
    };
    let (mut cx, view) = open_window(cx, setup);
    let trigger = center(&mut cx, "trigger");
    cx.simulate_mouse_move(trigger, None, Modifiers::default());
    redraw(&mut cx);
    cx.executor().advance_clock(Duration::from_millis(299));
    redraw(&mut cx);
    assert!(popup(&mut cx).is_none());
    cx.executor().advance_clock(Duration::from_millis(1));
    redraw(&mut cx);
    let bounds = popup(&mut cx).expect("open after 300ms");

    cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
    redraw(&mut cx);
    assert!(
        popup(&mut cx).is_some(),
        "the pointer may move into the popup"
    );

    cx.simulate_mouse_move(point(px(700.), px(50.)), None, Modifiers::default());
    redraw(&mut cx);
    assert!(popup(&mut cx).is_none());
    assert_eq!(
        reasons(&view, &mut cx),
        vec![
            (true, PopoverChangeReason::TriggerHover),
            (false, PopoverChangeReason::TriggerHover)
        ]
    );
}

#[gpui_kit::test]
fn a_trigger_that_goes_away_closes_its_popover(cx: &mut TestAppContext) {
    let (mut cx, view) = open_window(cx, Setup::new(Content::PopoverMenu));
    click(&mut cx, "trigger");
    view.update(&mut cx, |harness, cx| {
        harness.shown = false;
        cx.notify();
    });
    redraw(&mut cx);
    assert!(popup(&mut cx).is_none());
    assert_eq!(
        reasons(&view, &mut cx).last(),
        Some(&(false, PopoverChangeReason::TriggerGone))
    );
    assert!(!view.read_with(&cx, |harness, _| harness.open));
}

#[gpui_kit::test]
fn the_pointer_crosses_the_gap_to_a_submenu_without_closing_it(cx: &mut TestAppContext) {
    let (mut cx, _) = open_window(cx, Setup::new(Content::Submenu));
    click(&mut cx, "trigger");
    let more = cx.debug_bounds("menu-item-more").expect("the submenu row");
    cx.simulate_mouse_move(more.center(), None, Modifiers::none());
    cx.executor().advance_clock(Duration::from_millis(400));
    redraw(&mut cx);
    let deep = cx
        .debug_bounds("menu-item-deep")
        .expect("the submenu opens");
    assert!(
        deep.left() - more.right() > px(10.),
        "a gap between the menu and its submenu"
    );
    let mut x = more.center().x;
    while x < deep.center().x {
        cx.simulate_mouse_move(point(x, more.center().y), None, Modifiers::none());
        redraw(&mut cx);
        x += px(2.);
    }
    assert!(cx.debug_bounds("menu-item-deep").is_some());
}

#[gpui_kit::test]
fn the_right_arrow_opens_a_submenu_and_its_item_closes_both(cx: &mut TestAppContext) {
    let (mut cx, view) = open_window(cx, Setup::new(Content::Submenu));
    click(&mut cx, "trigger");
    press(&mut cx, "down");
    press(&mut cx, "down");
    press(&mut cx, "right");
    let alpha = cx
        .debug_bounds("menu-item-alpha")
        .expect("the parent stays");
    let deep = cx
        .debug_bounds("menu-item-deep")
        .expect("the submenu opens");
    assert!(deep.left() > alpha.right(), "beside its parent");

    press(&mut cx, "left");
    assert!(
        cx.debug_bounds("menu-item-deep").is_none(),
        "left closes only the submenu"
    );
    assert!(popup(&mut cx).is_some());

    press(&mut cx, "enter");
    press(&mut cx, "enter");
    assert_eq!(picked(&view, &mut cx), vec!["deep"]);
    assert!(
        popup(&mut cx).is_none(),
        "picking in a submenu closes the whole menu"
    );
    assert_eq!(
        reasons(&view, &mut cx).last(),
        Some(&(false, PopoverChangeReason::ItemPress))
    );
}

struct InDialog {
    outside: gpui_kit::FocusHandle,
    first: gpui_kit::FocusHandle,
}

impl Render for InDialog {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(div().id("outside").track_focus(&self.outside).size(px(40.)))
            .child(
                Dialog::new("dialog")
                    .open(true)
                    .title("Rename")
                    .close_button(false)
                    .child(
                        DialogButton::new("first")
                            .label("First")
                            .track_focus(&self.first),
                    )
                    .footer(
                        Popover::new("in-dialog")
                            .trigger(|_| {
                                div()
                                    .id("dialog-trigger")
                                    .debug_selector(|| "dialog-trigger".into())
                                    .w(px(80.))
                                    .h(px(24.))
                            })
                            .content(|_, _, _| div().h(px(40.))),
                    ),
            )
    }
}

#[gpui_kit::test]
fn tab_past_a_popover_at_the_end_of_a_dialog_stays_in_the_dialog(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        DialogLayer::init(cx);
        hoshi_ui::theme::apply(ThemeMode::Light, cx);
        cx.set_reduce_motion(true);
    });
    let (window, view) = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(800.), px(400.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| {
            cx.new(|cx| InDialog {
                outside: cx.focus_handle().tab_stop(true),
                first: cx.focus_handle(),
            })
        })
        .expect("test window")
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    redraw(&mut cx);
    let (outside, first) =
        view.read_with(&cx, |view, _| (view.outside.clone(), view.first.clone()));

    click(&mut cx, "dialog-trigger");
    assert!(popup(&mut cx).is_some());
    let escaped = cx.update(|window, cx| {
        window.dispatch_keystroke(gpui_kit::Keystroke::parse("tab").unwrap(), cx);
        outside.is_focused(window)
    });
    assert!(
        !escaped,
        "Tab stepped within the dialog's trap, not out of it"
    );
    redraw(&mut cx);

    assert!(
        popup(&mut cx).is_none(),
        "tab past the popover's end closes it"
    );
    assert!(
        cx.update(|window, _| first.is_focused(window)),
        "it wrapped to the dialog's first stop"
    );
}
