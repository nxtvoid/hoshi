//! A popover's menu: items the arrows, Home, End and typing move through, radio and checkbox
//! items, headings, separators and submenus, like Base UI's Menu.

use super::navigation::{NavItem, Navigation};
use super::state::{Edge, PopoverState};
use super::{
    Align, MenuDown, MenuEnd, MenuEnter, MenuHome, MenuLeft, MenuRight, MenuSpace, MenuUp, Popover,
    PopoverChangeReason, PopoverContext, Side,
};
use crate::assets::IconName;
use crate::components::{MenuHeading, Tooltip, TooltipTrigger};
use crate::theme::ActiveTheme;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::rc::Rc;
use std::time::Instant;

type SelectHandler = Rc<dyn Fn(&mut Window, &mut App)>;
pub(super) type MenuBuilder = Box<dyn FnOnce(PopoverMenu, &mut Window, &mut App) -> PopoverMenu>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ItemKind {
    Plain,
    Radio,
    Checkbox,
}

/// One item of a [`PopoverMenu`]: an action, or a radio or checkbox choice.
pub struct MenuOption {
    id: ElementId,
    label: SharedString,
    icon: Option<IconName>,
    leading: Option<AnyElement>,
    description: Option<SharedString>,
    trailing: Vec<AnyElement>,
    shortcut: Option<SharedString>,
    accelerator: Option<char>,
    kind: ItemKind,
    checked: bool,
    disabled: bool,
    destructive: bool,
    close_on_click: Option<bool>,
    on_select: Option<SelectHandler>,
    tooltip: Option<Tooltip>,
    mono: bool,
}

impl MenuOption {
    /// An action; choosing it closes the menu.
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            leading: None,
            description: None,
            trailing: Vec::new(),
            shortcut: None,
            accelerator: None,
            kind: ItemKind::Plain,
            checked: false,
            disabled: false,
            destructive: false,
            close_on_click: None,
            on_select: None,
            tooltip: None,
            mono: false,
        }
    }

    /// Shown on hover, also while disabled, as to say why.
    pub fn tooltip(mut self, tooltip: impl Into<Tooltip>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// The label in the small mono font, for names such as a branch's.
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    /// One choice of a group, checked when it is the current one; choosing it keeps the menu
    /// open.
    pub fn radio(id: impl Into<ElementId>, label: impl Into<SharedString>, checked: bool) -> Self {
        Self {
            kind: ItemKind::Radio,
            checked,
            ..Self::new(id, label)
        }
    }

    /// An option turned on or off; choosing it keeps the menu open.
    pub fn checkbox(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        checked: bool,
    ) -> Self {
        Self {
            kind: ItemKind::Checkbox,
            checked,
            ..Self::new(id, label)
        }
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Drawn where the icon goes, for a mark an icon name can't give, such as a brand's.
    pub fn leading(mut self, element: impl IntoElement) -> Self {
        self.leading = Some(element.into_any_element());
        self
    }

    /// The keys that do the same, muted at the end of the row.
    pub fn shortcut(mut self, keys: impl Into<SharedString>) -> Self {
        self.shortcut = Some(keys.into());
        self
    }

    /// The key that chooses it at once while the menu is open, ignoring case, in place of
    /// typing to find it.
    pub fn accelerator(mut self, key: char) -> Self {
        self.accelerator = Some(key);
        self
    }

    /// A muted line under the label.
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Shown after the label, such as a tag.
    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing.push(element.into_any_element());
        self
    }

    /// Shown, but skipped by the keyboard and not chosen.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// In the destructive color, for what can't be undone.
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    /// Whether choosing it closes the menu: yes for an action, no for a radio or checkbox.
    pub fn close_on_click(mut self, close: bool) -> Self {
        self.close_on_click = Some(close);
        self
    }

    pub fn on_select(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }

    fn closes(&self) -> bool {
        self.close_on_click.unwrap_or(self.kind == ItemKind::Plain)
    }
}

impl FluentBuilder for MenuOption {}

impl FluentBuilder for PopoverMenu {}

struct Submenu {
    id: ElementId,
    label: SharedString,
    icon: Option<IconName>,
    build: MenuBuilder,
}

enum Entry {
    Item(Box<MenuOption>),
    Submenu(Submenu),
    Heading(SharedString),
    Separator,
    Element(AnyElement),
}

/// What a menu's keyboard and pointer need of each item.
struct ItemMeta {
    child_index: usize,
    enabled: bool,
    closes: bool,
    submenu: bool,
    accelerator: Option<char>,
    on_select: Option<SelectHandler>,
}

/// The content of a [`Popover::menu`].
pub struct PopoverMenu {
    popover: PopoverContext,
    entries: Vec<Entry>,
    footer: Vec<AnyElement>,
    list_max_height: Option<Pixels>,
    loop_focus: bool,
}

impl PopoverMenu {
    pub(super) fn new(popover: PopoverContext) -> Self {
        Self {
            popover,
            entries: Vec::new(),
            footer: Vec::new(),
            list_max_height: None,
            loop_focus: true,
        }
    }

    /// The popover the menu is in, to close it from a custom row.
    pub fn context(&self) -> PopoverContext {
        self.popover.clone()
    }

    pub fn item(mut self, item: MenuOption) -> Self {
        self.entries.push(Entry::Item(Box::new(item)));
        self
    }

    pub fn items(mut self, items: impl IntoIterator<Item = MenuOption>) -> Self {
        self.entries
            .extend(items.into_iter().map(|item| Entry::Item(Box::new(item))));
        self
    }

    /// The small muted title of the group that follows.
    pub fn heading(mut self, text: impl Into<SharedString>) -> Self {
        self.entries.push(Entry::Heading(text.into()));
        self
    }

    pub fn separator(mut self) -> Self {
        self.entries.push(Entry::Separator);
        self
    }

    /// A row of its own in the list, which the keyboard does not stop on.
    pub fn child(mut self, element: impl IntoElement) -> Self {
        self.entries
            .push(Entry::Element(element.into_any_element()));
        self
    }

    /// Shown under the list, which scrolls without it.
    pub fn footer(mut self, element: impl IntoElement) -> Self {
        self.footer.push(element.into_any_element());
        self
    }

    /// An item that opens another menu beside this one, on hover, click, Enter or the right
    /// arrow.
    pub fn submenu(
        mut self,
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        icon: Option<IconName>,
        build: impl FnOnce(PopoverMenu, &mut Window, &mut App) -> PopoverMenu + 'static,
    ) -> Self {
        self.entries.push(Entry::Submenu(Submenu {
            id: id.into(),
            label: label.into(),
            icon,
            build: Box::new(build),
        }));
        self
    }

    /// Caps the list's height; it scrolls past it.
    pub fn max_list_height(mut self, height: Pixels) -> Self {
        self.list_max_height = Some(height);
        self
    }

    /// Whether the arrows wrap around the ends. On by default.
    pub fn loop_focus(mut self, loop_focus: bool) -> Self {
        self.loop_focus = loop_focus;
        self
    }

    pub(super) fn render_into(
        self,
        surface: Stateful<Div>,
        state: &Entity<PopoverState>,
        _: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        let colors = cx.hoshi().clone();
        let mono = gpui_kit::component::Theme::global(cx)
            .mono_font_family
            .clone();
        let mut nav = Vec::new();
        let mut metas = Vec::new();
        for (child_index, entry) in self.entries.iter().enumerate() {
            match entry {
                Entry::Item(item) => {
                    nav.push(NavItem {
                        label: item.label.to_string(),
                        enabled: !item.disabled,
                    });
                    metas.push(ItemMeta {
                        child_index,
                        enabled: !item.disabled,
                        closes: item.closes(),
                        submenu: false,
                        accelerator: item.accelerator,
                        on_select: item.on_select.clone(),
                    });
                }
                Entry::Submenu(submenu) => {
                    nav.push(NavItem {
                        label: submenu.label.to_string(),
                        enabled: true,
                    });
                    metas.push(ItemMeta {
                        child_index,
                        enabled: true,
                        closes: false,
                        submenu: true,
                        accelerator: None,
                        on_select: None,
                    });
                }
                _ => {}
            }
        }
        let nav = Rc::new(nav);
        let metas = Rc::new(metas);

        let (highlighted, submenu_open, submenu_focus, popup_focus, scroll, reveal) =
            state.update(cx, |state, _| {
                let menu = &mut state.menu;
                if let Some(edge) = menu.edge.take() {
                    match edge {
                        Edge::First => menu.nav.first(&nav),
                        Edge::Last => menu.nav.last(&nav),
                    };
                    menu.reveal = menu.nav.highlighted;
                }
                if menu.nav.highlighted.is_some_and(|index| index >= nav.len()) {
                    menu.nav.highlighted = None;
                }
                let focus = if menu.submenu_keyboard {
                    menu.submenu_focus
                } else {
                    0
                };
                (
                    menu.nav.highlighted,
                    menu.submenu,
                    focus,
                    state.popup_focus.clone(),
                    state.scroll.clone(),
                    menu.reveal.take(),
                )
            });
        if let Some(meta) = reveal.and_then(|index| metas.get(index)) {
            scroll.scroll_to_item(meta.child_index);
        }

        let mut index = 0;
        let rows: Vec<AnyElement> = self
            .entries
            .into_iter()
            .map(|entry| match entry {
                Entry::Heading(text) => MenuHeading::new(text).into_any_element(),
                Entry::Separator => div()
                    .h_px()
                    .my_1()
                    .mx(px(-4.))
                    .bg(colors.border)
                    .into_any_element(),
                Entry::Element(element) => element,
                Entry::Item(item) => {
                    let current = index;
                    index += 1;
                    item_row(
                        *item,
                        current,
                        highlighted == Some(current),
                        state,
                        &metas,
                        &colors,
                        &mono,
                    )
                }
                Entry::Submenu(submenu) => {
                    let current = index;
                    index += 1;
                    let open = submenu_open == Some(current);
                    submenu_row(
                        submenu,
                        current,
                        highlighted == Some(current) || open,
                        open,
                        if open { submenu_focus } else { 0 },
                        state,
                        &popup_focus,
                        &colors,
                    )
                }
            })
            .collect();

        let wrap = self.loop_focus;
        let list = div()
            .id("menu-list")
            .flex()
            .flex_col()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .when_some(self.list_max_height, |list, height| list.max_h(height))
            .children(rows);

        surface
            .on_action({
                let (state, nav) = (state.clone(), nav.clone());
                move |_: &MenuDown, _, cx| {
                    step(&state, &nav, |nav, items| nav.next(items, wrap), cx)
                }
            })
            .on_action({
                let (state, nav) = (state.clone(), nav.clone());
                move |_: &MenuUp, _, cx| {
                    step(&state, &nav, |nav, items| nav.previous(items, wrap), cx)
                }
            })
            .on_action({
                let (state, nav) = (state.clone(), nav.clone());
                move |_: &MenuHome, _, cx| step(&state, &nav, |nav, items| nav.first(items), cx)
            })
            .on_action({
                let (state, nav) = (state.clone(), nav.clone());
                move |_: &MenuEnd, _, cx| step(&state, &nav, |nav, items| nav.last(items), cx)
            })
            .on_action({
                let (state, metas) = (state.clone(), metas.clone());
                move |_: &MenuEnter, window, cx| activate_highlighted(&state, &metas, window, cx)
            })
            .on_action({
                let (state, metas, nav) = (state.clone(), metas.clone(), nav.clone());
                move |_: &MenuSpace, window, cx| {
                    if state.read(cx).menu.nav.is_typing(Instant::now()) {
                        type_ahead(&state, &nav, ' ', cx);
                    } else {
                        activate_highlighted(&state, &metas, window, cx);
                    }
                }
            })
            .on_action({
                let (state, metas) = (state.clone(), metas.clone());
                move |_: &MenuRight, window, cx| {
                    let highlighted = state.read(cx).menu.nav.highlighted;
                    if let Some(index) = highlighted
                        && metas.get(index).is_some_and(|meta| meta.submenu)
                    {
                        activate(&state, &metas, index, true, window, cx);
                    }
                }
            })
            .on_action({
                let state = state.clone();
                move |_: &MenuLeft, window, cx| {
                    state.update(cx, |state, cx| {
                        if state.parent_menu.is_some() {
                            state.close(PopoverChangeReason::ListNavigation, window, cx);
                        }
                    })
                }
            })
            .on_key_down({
                let (state, nav, metas) = (state.clone(), nav.clone(), metas.clone());
                move |event, window, cx| {
                    let keystroke = &event.keystroke;
                    let modifiers = &keystroke.modifiers;
                    if modifiers.control
                        || modifiers.alt
                        || modifiers.platform
                        || modifiers.function
                    {
                        return;
                    }
                    let Some(character) = keystroke.key_char.as_deref().and_then(single_char)
                    else {
                        return;
                    };
                    if character.is_control() || character.is_whitespace() {
                        return;
                    }
                    let chosen = metas.iter().position(|meta| {
                        meta.enabled
                            && meta
                                .accelerator
                                .is_some_and(|key| key.to_lowercase().eq(character.to_lowercase()))
                    });
                    match chosen {
                        Some(index) => activate(&state, &metas, index, true, window, cx),
                        None => type_ahead(&state, &nav, character, cx),
                    }
                    cx.stop_propagation();
                }
            })
            .child(list)
            .children(self.footer)
    }
}

fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

fn step(
    state: &Entity<PopoverState>,
    nav: &[NavItem],
    move_to: impl FnOnce(&mut Navigation, &[NavItem]) -> Option<usize>,
    cx: &mut App,
) {
    state.update(cx, |state, cx| {
        move_to(&mut state.menu.nav, nav);
        state.menu.reveal = state.menu.nav.highlighted;
        cx.notify();
    });
}

fn type_ahead(state: &Entity<PopoverState>, nav: &[NavItem], character: char, cx: &mut App) {
    state.update(cx, |state, cx| {
        if state
            .menu
            .nav
            .type_char(nav, character, Instant::now())
            .is_some()
        {
            state.menu.reveal = state.menu.nav.highlighted;
            cx.notify();
        }
    });
}

fn activate_highlighted(
    state: &Entity<PopoverState>,
    metas: &[ItemMeta],
    window: &mut Window,
    cx: &mut App,
) {
    if let Some(index) = state.read(cx).menu.nav.highlighted {
        activate(state, metas, index, true, window, cx);
    }
}

/// Chooses an item: runs it and closes the menu when it closes, or opens its submenu.
fn activate(
    state: &Entity<PopoverState>,
    metas: &[ItemMeta],
    index: usize,
    keyboard: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(meta) = metas.get(index).filter(|meta| meta.enabled) else {
        return;
    };
    if meta.submenu {
        state.update(cx, |state, cx| {
            let menu = &mut state.menu;
            menu.nav.highlighted = Some(index);
            menu.submenu = Some(index);
            if keyboard {
                menu.submenu_keyboard = true;
                menu.submenu_focus += 1;
            }
            cx.notify();
        });
        return;
    }
    if let Some(handler) = meta.on_select.clone() {
        handler(window, cx);
    }
    if meta.closes {
        state.update(cx, |state, cx| {
            state.close_tree(PopoverChangeReason::ItemPress, window, cx)
        });
    }
}

fn highlight_on_pointer(
    row: Stateful<Div>,
    state: &Entity<PopoverState>,
    index: usize,
) -> Stateful<Div> {
    row.on_mouse_move({
        let state = state.clone();
        move |_, _, cx| {
            state.update(cx, |state, cx| {
                if state.menu.nav.highlighted != Some(index) {
                    state.menu.nav.highlighted = Some(index);
                    cx.notify();
                }
            })
        }
    })
    .on_hover({
        let state = state.clone();
        move |hovered, _, cx| {
            if *hovered {
                return;
            }
            state.update(cx, |state, cx| {
                if state.menu.nav.highlighted == Some(index) && state.menu.submenu != Some(index) {
                    state.menu.nav.highlighted = None;
                    cx.notify();
                }
            })
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn item_row(
    mut item: MenuOption,
    index: usize,
    highlighted: bool,
    state: &Entity<PopoverState>,
    metas: &Rc<Vec<ItemMeta>>,
    colors: &crate::theme::Colors,
    mono: &SharedString,
) -> AnyElement {
    let tooltip = item
        .tooltip
        .take()
        .map(|tooltip| (item.id.clone(), tooltip));
    let row = item_row_inner(item, index, highlighted, state, metas, colors, mono);
    match tooltip {
        Some((id, tooltip)) => TooltipTrigger::new(id, row, tooltip).into_any_element(),
        None => row,
    }
}

#[allow(clippy::too_many_arguments)]
fn item_row_inner(
    item: MenuOption,
    index: usize,
    highlighted: bool,
    state: &Entity<PopoverState>,
    metas: &Rc<Vec<ItemMeta>>,
    colors: &crate::theme::Colors,
    mono: &SharedString,
) -> AnyElement {
    let disabled = item.disabled;
    let described = item.description.is_some();
    let indicator = item.kind != ItemKind::Plain;
    let id = item.id.clone();
    let row = div()
        .id(item.id)
        .debug_selector(move || format!("menu-item-{id}"))
        .flex()
        .when(described, |row| row.items_start().py_1p5())
        .when(!described, |row| row.items_center().py_1())
        .gap_1p5()
        .rounded_md()
        .pl_1p5()
        .pr(if indicator { px(8.) } else { px(6.) })
        .text_sm()
        .when(item.destructive, |row| row.text_color(colors.destructive))
        .when(highlighted, |row| {
            if item.destructive {
                row.bg(colors.destructive.opacity(0.1))
            } else {
                row.bg(colors.accent).text_color(colors.accent_foreground)
            }
        })
        .when(disabled, |row| row.opacity(0.5))
        .when_some(item.icon, |row, icon| {
            row.child(
                Icon::new(icon)
                    .size_4()
                    .flex_none()
                    .when(described, |icon| icon.mt_0p5()),
            )
        })
        .children(item.leading)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child(
                            div()
                                .truncate()
                                .when(item.mono, |label| label.font_family(mono.clone()).text_xs())
                                .child(item.label),
                        )
                        .children(item.trailing),
                )
                .when_some(item.description, |column, description| {
                    column.child(
                        div()
                            .text_xs()
                            .text_color(if highlighted {
                                colors.accent_foreground
                            } else {
                                colors.muted_foreground
                            })
                            .child(description),
                    )
                }),
        )
        .when_some(item.shortcut, |row, keys| {
            row.child(
                div()
                    .flex_none()
                    .ml_auto()
                    .pl_2()
                    .text_xs()
                    .text_color(if highlighted {
                        colors.accent_foreground
                    } else {
                        colors.muted_foreground
                    })
                    .child(keys),
            )
        })
        .when(indicator, |row| {
            row.child(
                div()
                    .size_4()
                    .flex_none()
                    .ml_auto()
                    .when(described, |slot| slot.mt_0p5())
                    .when(item.checked, |slot| {
                        slot.child(Icon::new(IconName::Check).size_4())
                    }),
            )
        });
    if disabled {
        return row.into_any_element();
    }
    highlight_on_pointer(row, state, index)
        .on_click({
            let (state, metas) = (state.clone(), metas.clone());
            move |event, window, cx| {
                activate(&state, &metas, index, event.is_keyboard(), window, cx)
            }
        })
        .on_mouse_up(MouseButton::Left, {
            let (state, metas) = (state.clone(), metas.clone());
            move |_, window, cx| {
                if state.read(cx).menu.drag_open {
                    state.update(cx, |state, _| state.menu.drag_open = false);
                    activate(&state, &metas, index, false, window, cx);
                }
            }
        })
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn submenu_row(
    submenu: Submenu,
    index: usize,
    highlighted: bool,
    open: bool,
    focus_request: u64,
    state: &Entity<PopoverState>,
    popup_focus: &FocusHandle,
    colors: &crate::theme::Colors,
) -> AnyElement {
    let (accent, accent_foreground) = (colors.accent, colors.accent_foreground);
    let id = submenu.id.clone();
    let label = submenu.label;
    let icon = submenu.icon;
    let build = submenu.build;
    let parent = state.clone();
    Popover::new(submenu.id)
        .open(open)
        .on_open_change(move |change, _, cx| {
            parent.update(cx, |state, cx| {
                let menu = &mut state.menu;
                if change.open {
                    menu.submenu = Some(index);
                    menu.nav.highlighted = Some(index);
                } else if menu.submenu == Some(index) {
                    menu.submenu = None;
                    menu.submenu_keyboard = false;
                }
                cx.notify();
            })
        })
        .side(Side::Right)
        .align(Align::Start)
        .side_offset(px(0.))
        .align_offset(px(-3.))
        .open_on_hover(true)
        .submenu_of(state.downgrade(), popup_focus.clone(), focus_request)
        .trigger({
            let state = state.clone();
            move |_| {
                let row = div()
                    .id("submenu-trigger")
                    .debug_selector(move || format!("menu-item-{id}"))
                    .flex()
                    .flex_1()
                    .items_center()
                    .gap_1p5()
                    .rounded_md()
                    .px_1p5()
                    .py_1()
                    .text_sm()
                    .when(highlighted, |row| {
                        row.bg(accent).text_color(accent_foreground)
                    })
                    .when_some(icon, |row, icon| {
                        row.child(Icon::new(icon).size_4().flex_none())
                    })
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .child(Icon::new(IconName::ChevronRight).size_4().flex_none());
                highlight_on_pointer(row, &state, index)
            }
        })
        .menu(build)
        .into_any_element()
}
