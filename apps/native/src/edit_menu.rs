//! The right-click menu over text: what it offers where it opens, as the web's
//! `lib/edit-menu.ts` decides, and its items.

use crate::m;
use gpui_kit::base::TextSelection;
use gpui_kit::*;
use hoshi_ui::prelude::{EditLabels, EditLabelsProvider, MenuOption, PopoverMenu, context_menu};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditAction {
    Cut,
    Copy,
    Paste,
    SelectAll,
}

pub const EDIT_ACTIONS: [EditAction; 4] = [
    EditAction::Cut,
    EditAction::Copy,
    EditAction::Paste,
    EditAction::SelectAll,
];

/// Where a right-click landed: text the user can edit, the terminal, or anything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditTarget {
    Text,
    Terminal,
    ReadOnly,
}

/// Which items the menu enables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditMenu {
    pub cut: bool,
    pub copy: bool,
    pub paste: bool,
    pub select_all: bool,
}

impl EditMenu {
    pub fn enables(&self, action: EditAction) -> bool {
        match action {
            EditAction::Cut => self.cut,
            EditAction::Copy => self.copy,
            EditAction::Paste => self.paste,
            EditAction::SelectAll => self.select_all,
        }
    }
}

/// What the menu enables there; `None` when it has nothing to offer.
pub fn edit_menu(target: EditTarget, selected: bool) -> Option<EditMenu> {
    if target == EditTarget::ReadOnly && !selected {
        return None;
    }
    Some(EditMenu {
        cut: target == EditTarget::Text && selected,
        copy: selected,
        paste: target != EditTarget::ReadOnly,
        select_all: target != EditTarget::ReadOnly,
    })
}

pub fn edit_shortcut(action: EditAction, mac: bool) -> String {
    let key = match action {
        EditAction::Cut => 'X',
        EditAction::Copy => 'C',
        EditAction::Paste => 'V',
        EditAction::SelectAll => 'A',
    };
    if mac {
        format!("⌘{key}")
    } else {
        format!("Ctrl {key}")
    }
}

fn label(action: EditAction) -> String {
    match action {
        EditAction::Cut => m::edit_cut(),
        EditAction::Copy => m::edit_copy(),
        EditAction::Paste => m::edit_paste(),
        EditAction::SelectAll => m::edit_select_all(),
    }
}

/// Gives the text fields' own menus the app's labels.
pub fn label_text_fields(cx: &mut App) {
    cx.set_global(EditLabelsProvider(|| EditLabels {
        cut: m::edit_cut().into(),
        copy: m::edit_copy().into(),
        paste: m::edit_paste().into(),
        select_all: m::edit_select_all().into(),
    }));
}

pub type RunEdit = Rc<dyn Fn(EditAction, &mut Window, &mut App)>;

/// The four edit items, enabled as `enabled` says, each running `run`.
pub fn edit_items(menu: PopoverMenu, enabled: EditMenu, run: RunEdit) -> PopoverMenu {
    menu.items(
        EDIT_ACTIONS
            .into_iter()
            .map(|action| edit_item(action, enabled.enables(action), run.clone())),
    )
}

/// Only the items `enabled` turns on, for a view nothing can be typed into.
pub fn enabled_edit_items(menu: PopoverMenu, enabled: EditMenu, run: RunEdit) -> PopoverMenu {
    menu.items(
        EDIT_ACTIONS
            .into_iter()
            .filter(|action| enabled.enables(*action))
            .map(|action| edit_item(action, true, run.clone())),
    )
}

fn edit_item(action: EditAction, enabled: bool, run: RunEdit) -> MenuOption {
    MenuOption::new(("edit-menu", action as usize), label(action))
        .shortcut(edit_shortcut(action, cfg!(target_os = "macos")))
        .disabled(!enabled)
        .on_select(move |window, cx| run(action, window, cx))
}

/// The menu over read-only text, such as the transcript's: right-clicking a selection there
/// offers to copy it. Inputs and the terminal open their own.
pub struct ReadOnlyMenu {
    /// Where it opened, which opening it is, and the text selected then: a press on an item
    /// clears the window's selection before the item runs.
    open: Option<(Point<Pixels>, usize, SharedString)>,
    opened: usize,
}

impl ReadOnlyMenu {
    pub fn new() -> Self {
        Self {
            open: None,
            opened: 0,
        }
    }

    fn right_click(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = TextSelection::has_selection(window, cx)
            && TextSelection::is_selectable_at(position, window, cx);
        if edit_menu(EditTarget::ReadOnly, selected).is_none() {
            return;
        }
        let text = crate::markdown::selected_text(window, cx);
        self.opened += 1;
        self.open = Some((position, self.opened, text.into()));
        cx.notify();
    }
}

impl Render for ReadOnlyMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        let listener = canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let this = this.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase.bubble() && event.button == MouseButton::Right {
                        this.update(cx, |this, cx| this.right_click(event.position, window, cx))
                            .ok();
                    }
                });
            },
        )
        .absolute()
        .size_0();
        let menu = self.open.clone().map(|(position, serial, text)| {
            let closing = cx.weak_entity();
            let on_close = move |_: &mut Window, cx: &mut App| {
                closing
                    .update(cx, |this, cx| {
                        if this.open.as_ref().map(|(_, open, _)| *open) == Some(serial) {
                            this.open = None;
                            cx.notify();
                        }
                    })
                    .ok();
            };
            let enabled = edit_menu(EditTarget::ReadOnly, true).unwrap_or(EditMenu {
                cut: false,
                copy: false,
                paste: false,
                select_all: false,
            });
            let run: RunEdit = Rc::new(move |action, _, cx| {
                if action == EditAction::Copy && !text.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                }
            });
            context_menu(
                SharedString::from(format!("read-only-menu-{serial}")),
                position,
                on_close,
                move |menu, _, _| edit_items(menu, enabled, run),
            )
        });
        div().child(listener).children(menu)
    }
}

#[cfg(test)]
mod tests {
    use super::{EditAction, EditMenu, EditTarget, edit_menu, edit_shortcut};

    #[test]
    fn offers_nothing_on_plain_ui_without_a_selection() {
        assert_eq!(edit_menu(EditTarget::ReadOnly, false), None);
    }

    #[test]
    fn only_copies_a_selection_outside_an_editor() {
        assert_eq!(
            edit_menu(EditTarget::ReadOnly, true),
            Some(EditMenu {
                cut: false,
                copy: true,
                paste: false,
                select_all: false,
            })
        );
    }

    #[test]
    fn cuts_and_copies_only_with_a_selection_in_an_editor() {
        assert_eq!(
            edit_menu(EditTarget::Text, false),
            Some(EditMenu {
                cut: false,
                copy: false,
                paste: true,
                select_all: true,
            })
        );
        assert_eq!(
            edit_menu(EditTarget::Text, true),
            Some(EditMenu {
                cut: true,
                copy: true,
                paste: true,
                select_all: true,
            })
        );
    }

    #[test]
    fn never_cuts_from_the_terminal() {
        assert_eq!(
            edit_menu(EditTarget::Terminal, true),
            Some(EditMenu {
                cut: false,
                copy: true,
                paste: true,
                select_all: true,
            })
        );
    }

    #[test]
    fn the_shortcut_follows_the_platform() {
        assert_eq!(edit_shortcut(EditAction::Paste, true), "⌘V");
        assert_eq!(edit_shortcut(EditAction::SelectAll, false), "Ctrl A");
    }
}

#[cfg(test)]
mod ui_tests {
    use super::ReadOnlyMenu;
    use gpui_kit::component::text::TextView;
    use gpui_kit::component::theme::ThemeMode;
    use gpui_kit::{
        AppContext as _, Bounds, ClipboardItem, Context, Entity, IntoElement, Modifiers,
        MouseButton, ParentElement as _, Point, Render, Styled as _, TestAppContext,
        VisualTestContext, Window, WindowBounds, WindowOptions, div, point, px, size,
    };

    struct Page {
        menu: Entity<ReadOnlyMenu>,
    }

    impl Render for Page {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .p_4()
                .child(
                    TextView::markdown(
                        "text",
                        "First line of text.

Second line of text.",
                    )
                    .selectable(true),
                )
                .child(self.menu.clone())
        }
    }

    fn open(cx: &mut TestAppContext) -> VisualTestContext {
        cx.update(|cx| {
            gpui_kit::init(cx);
            hoshi_ui::theme::apply(ThemeMode::Dark, cx);
        });
        let window = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(600.), px(400.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |_, cx| {
                let menu = cx.new(|_| ReadOnlyMenu::new());
                cx.new(|_| Page { menu })
            })
            .expect("test window")
            .0
        });
        let mut cx = VisualTestContext::from_window(window, cx);
        redraw(&mut cx);
        cx
    }

    fn redraw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        for _ in 0..3 {
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
    }

    fn right_click(cx: &mut VisualTestContext, at: Point<gpui_kit::Pixels>) {
        cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::default());
        cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::default());
        redraw(cx);
    }

    #[gpui_kit::test]
    fn a_right_click_on_a_selection_copies_it(cx: &mut TestAppContext) {
        let mut cx = open(cx);
        cx.simulate_mouse_down(
            point(px(20.), px(25.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_move(
            point(px(200.), px(25.)),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        cx.simulate_mouse_up(
            point(px(200.), px(25.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        redraw(&mut cx);

        right_click(&mut cx, point(px(60.), px(25.)));
        let copy = cx
            .debug_bounds("menu-item-edit-menu-1")
            .expect("the menu offers copy");
        cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("before".into())));
        cx.simulate_click(copy.center(), Modifiers::default());
        redraw(&mut cx);

        let copied = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
        let copied = copied.unwrap_or_default();
        assert!(copied.starts_with("First line"), "copied: {copied:?}");
        assert!(
            cx.debug_bounds("popover-popup").is_none(),
            "the menu closes"
        );
    }

    #[gpui_kit::test]
    fn with_nothing_selected_a_right_click_opens_no_menu(cx: &mut TestAppContext) {
        let mut cx = open(cx);
        right_click(&mut cx, point(px(60.), px(25.)));
        assert!(cx.debug_bounds("popover-popup").is_none());
    }
}
