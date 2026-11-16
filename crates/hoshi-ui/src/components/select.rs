use super::placement::{Align, Side};
use super::popover::{MenuOption, Popover};
use crate::assets::IconName;
use crate::theme::{ActiveTheme, Colors};
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::rc::Rc;

type SelectHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// One choice of a [`Select`].
pub struct SelectOption {
    id: ElementId,
    label: SharedString,
    description: Option<SharedString>,
    selected: bool,
    on_select: Option<SelectHandler>,
}

impl SelectOption {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, selected: bool) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: None,
            selected,
            on_select: None,
        }
    }

    /// A muted line under the label.
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn on_select(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }
}

/// A button that shows the current choice and opens a menu of the others, like the web's
/// `Select`.
#[derive(IntoElement)]
pub struct Select {
    id: ElementId,
    value: SharedString,
    options: Vec<SelectOption>,
    disabled: bool,
    min_width: Pixels,
    menu_width: Option<Pixels>,
    align: Align,
}

impl Select {
    /// `value` is the label the trigger shows.
    pub fn new(id: impl Into<ElementId>, value: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            value: value.into(),
            options: Vec::new(),
            disabled: false,
            min_width: px(160.),
            menu_width: None,
            align: Align::Start,
        }
    }

    pub fn option(mut self, option: SelectOption) -> Self {
        self.options.push(option);
        self
    }

    pub fn options(mut self, options: impl IntoIterator<Item = SelectOption>) -> Self {
        self.options.extend(options);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn min_width(mut self, width: Pixels) -> Self {
        self.min_width = width;
        self
    }

    /// The menu's width; by default it fits its options.
    pub fn menu_width(mut self, width: Pixels) -> Self {
        self.menu_width = Some(width);
        self
    }

    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }
}

fn trigger(
    id: ElementId,
    value: SharedString,
    min_width: Pixels,
    open: bool,
    colors: &Colors,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(|| "select-trigger".into())
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .gap_1p5()
        .h_8()
        .min_w(min_width)
        .pl_2p5()
        .pr_2()
        .rounded_lg()
        .border_1()
        .border_color(colors.input)
        .when(open, |this| this.bg(colors.muted))
        .text_sm()
        .whitespace_nowrap()
        .child(div().min_w_0().truncate().child(value))
        .child(
            Icon::new(IconName::ChevronDown)
                .size_4()
                .flex_none()
                .text_color(colors.muted_foreground),
        )
}

impl RenderOnce for Select {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            value,
            options,
            disabled,
            min_width,
            menu_width,
            align,
        } = self;
        if disabled {
            return trigger(id, value, min_width, false, cx.hoshi())
                .opacity(0.5)
                .into_any_element();
        }
        let colors = cx.hoshi().clone();
        let trigger_id = id.clone();
        Popover::new(id)
            .focus_ring(px(10.))
            .match_trigger_width()
            .side(Side::Bottom)
            .align(align)
            .when_some(menu_width, |this, width| this.w(width))
            .trigger(move |open| {
                let hover = colors.muted;
                trigger(trigger_id, value, min_width, open, &colors)
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
            })
            .menu(move |menu, _, _| {
                menu.max_list_height(px(320.))
                    .items(options.into_iter().map(|option| {
                        let mut item = MenuOption::radio(option.id, option.label, option.selected)
                            .close_on_click(true);
                        if let Some(description) = option.description {
                            item = item.description(description);
                        }
                        if let Some(handler) = option.on_select {
                            item = item.on_select(move |window, cx| handler(window, cx));
                        }
                        item
                    }))
            })
            .into_any_element()
    }
}
