//! The pieces of a settings page, like the web's `SettingsLayout`.

use super::status_dot::{Status, StatusDot};
use crate::ClickHandler;
use crate::assets::IconName;
use crate::theme::ActiveTheme;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::rc::Rc;

/// A titled card of [`SettingsRow`]s, with a rule between rows.
#[derive(IntoElement)]
pub struct SettingsGroup {
    title: Option<SharedString>,
    description: Option<SharedString>,
    action: Option<AnyElement>,
    rows: Vec<AnyElement>,
}

impl SettingsGroup {
    pub fn new() -> Self {
        Self {
            title: None,
            description: None,
            action: None,
            rows: Vec::new(),
        }
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// A muted line under the title.
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// At the end of the title's line, such as a refresh button.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }
}

impl Default for SettingsGroup {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for SettingsGroup {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.rows.extend(elements);
    }
}

impl RenderOnce for SettingsGroup {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi();
        let rule = colors.border.opacity(0.5);
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .when_some(self.title, |this, title| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_4()
                        .px_4()
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .children(self.action),
                )
            })
            .when_some(self.description, |this, description| {
                this.child(
                    div()
                        .mt(px(-6.))
                        .px_4()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(description),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .rounded_xl()
                    .border_1()
                    .border_color(colors.border.opacity(0.6))
                    .bg(colors.card.opacity(0.4))
                    .children(self.rows.into_iter().enumerate().map(|(index, row)| {
                        div()
                            .when(index > 0, |this| this.border_t_1().border_color(rule))
                            .child(row)
                    })),
            )
    }
}

/// One setting: its title and description on the left, its control on the right, and anything
/// that needs the whole width below.
#[derive(IntoElement)]
pub struct SettingsRow {
    title: AnyElement,
    description: Option<AnyElement>,
    action: Option<AnyElement>,
    control: Option<AnyElement>,
    children: Vec<AnyElement>,
}

impl SettingsRow {
    pub fn new(title: impl IntoElement) -> Self {
        Self {
            title: title.into_any_element(),
            description: None,
            action: None,
            control: None,
            children: Vec::new(),
        }
    }

    pub fn description(mut self, description: impl IntoElement) -> Self {
        self.description = Some(description.into_any_element());
        self
    }

    /// Beside the title, such as a reset button.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }

    pub fn control(mut self, control: impl IntoElement) -> Self {
        self.control = Some(control.into_any_element());
        self
    }
}

impl ParentElement for SettingsRow {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for SettingsRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi();
        div()
            .px_4()
            .py_3()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_8()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .min_h_5()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(self.title)
                                    .children(self.action),
                            )
                            .when_some(self.description, |this, description| {
                                this.child(
                                    div()
                                        .max_w(px(672.))
                                        .text_xs()
                                        .text_color(colors.muted_foreground)
                                        .child(description),
                                )
                            }),
                    )
                    .when_some(self.control, |this, control| {
                        this.child(
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_end()
                                .gap_2()
                                .child(control),
                        )
                    }),
            )
            .children(self.children)
    }
}

/// A list beside the detail of its selected item, inside a [`SettingsGroup`]. The detail's rows
/// come as children, under its heading.
#[derive(IntoElement)]
pub struct SettingsSplit {
    items: Vec<AnyElement>,
    list_end: Option<AnyElement>,
    heading: Option<AnyElement>,
    rows: Vec<AnyElement>,
    height: Option<Pixels>,
}

impl SettingsSplit {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            list_end: None,
            heading: None,
            rows: Vec::new(),
            height: None,
        }
    }

    /// The same height whatever is selected; each side scrolls what doesn't fit.
    pub fn height(mut self, height: Pixels) -> Self {
        self.height = Some(height);
        self
    }

    pub fn items(mut self, items: impl IntoIterator<Item = SettingsListItem>) -> Self {
        self.items
            .extend(items.into_iter().map(IntoElement::into_any_element));
        self
    }

    /// Under the list's items, such as a row that adds one.
    pub fn list_end(mut self, element: impl IntoElement) -> Self {
        self.list_end = Some(element.into_any_element());
        self
    }

    pub fn heading(mut self, heading: impl IntoElement) -> Self {
        self.heading = Some(heading.into_any_element());
        self
    }
}

impl Default for SettingsSplit {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for SettingsSplit {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.rows.extend(elements);
    }
}

impl RenderOnce for SettingsSplit {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let rule = cx.hoshi().border.opacity(0.5);
        let headed = self.heading.is_some();
        div()
            .flex()
            .when_some(self.height, |this, height| this.h(height))
            .child(
                div()
                    .id("settings-split-list")
                    .overflow_y_scroll()
                    .flex_none()
                    .w(px(224.))
                    .p(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .border_r_1()
                    .border_color(rule)
                    .children(self.items)
                    .children(self.list_end),
            )
            .child(
                div()
                    .id("settings-split-detail")
                    .overflow_y_scroll()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .when_some(self.heading, |this, heading| {
                        this.child(
                            div()
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_4()
                                .pt(px(14.))
                                .pb_3()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(heading),
                        )
                    })
                    .children(self.rows.into_iter().enumerate().map(|(index, row)| {
                        div()
                            .flex_none()
                            .when(headed || index > 0, |this| {
                                this.border_t_1().border_color(rule)
                            })
                            .child(row)
                    })),
            )
    }
}

/// One item of a [`SettingsSplit`]'s list: a mark, a name, a status line under it and a
/// control at the end. Clicking it selects it; the control handles its own clicks.
#[derive(IntoElement)]
pub struct SettingsListItem {
    id: ElementId,
    title: SharedString,
    subtitle: SharedString,
    leading: Option<AnyElement>,
    status: Option<Status>,
    control: Option<AnyElement>,
    selected: bool,
    dimmed: bool,
    on_click: Option<ClickHandler>,
}

impl SettingsListItem {
    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        subtitle: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            subtitle: subtitle.into(),
            leading: None,
            status: None,
            control: None,
            selected: false,
            dimmed: false,
            on_click: None,
        }
    }

    pub fn leading(mut self, leading: impl IntoElement) -> Self {
        self.leading = Some(leading.into_any_element());
        self
    }

    /// A dot before the status line, for a state that needs attention.
    pub fn status(mut self, status: Option<Status>) -> Self {
        self.status = status;
        self
    }

    pub fn control(mut self, control: impl IntoElement) -> Self {
        self.control = Some(control.into_any_element());
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Fades the mark and the name, as for something turned off.
    pub fn dimmed(mut self, dimmed: bool) -> Self {
        self.dimmed = dimmed;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for SettingsListItem {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi();
        let hover = colors.muted.opacity(0.6);
        let faded = if self.dimmed { 0.5 } else { 1. };
        let selector = format!("settings-item-{}", self.id);
        div()
            .id(self.id)
            .debug_selector(|| selector)
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(10.))
            .py_2()
            .rounded_lg()
            .cursor_pointer()
            .when(self.selected, |this| this.bg(colors.muted))
            .when(!self.selected, |this| {
                this.hover(move |style| style.bg(hover))
            })
            .when_some(self.on_click, |this, handler| this.on_click(handler))
            .children(
                self.leading
                    .map(|leading| div().flex_none().opacity(faded).child(leading)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .opacity(faded)
                            .child(self.title),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .min_w_0()
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .when_some(self.status, |this, status| {
                                this.child(StatusDot::new(Some(status)))
                            })
                            .child(div().min_w_0().truncate().child(self.subtitle)),
                    ),
            )
            .children(self.control)
    }
}

type ToggleHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// Folds rows few people need at the end of a [`SettingsGroup`]. The caller keeps whether it is
/// open.
#[derive(IntoElement)]
pub struct SettingsAdvanced {
    id: ElementId,
    label: SharedString,
    open: bool,
    on_toggle: Option<ToggleHandler>,
    rows: Vec<AnyElement>,
}

impl SettingsAdvanced {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            open: false,
            on_toggle: None,
            rows: Vec::new(),
        }
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn on_toggle(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(handler));
        self
    }
}

impl ParentElement for SettingsAdvanced {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.rows.extend(elements);
    }
}

impl RenderOnce for SettingsAdvanced {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi();
        let rule = colors.border.opacity(0.5);
        let foreground = colors.foreground;
        let open = self.open;
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .id(self.id)
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .min_h_10()
                    .px_4()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors.muted_foreground)
                    .cursor_pointer()
                    .hover(move |style| style.text_color(foreground))
                    .when_some(self.on_toggle, |this, handler| {
                        this.on_click(move |_, window, cx| handler(window, cx))
                    })
                    .child(
                        Icon::new(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size_3p5()
                        .flex_none(),
                    )
                    .child(self.label),
            )
            .when(open, |this| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .border_t_1()
                        .border_color(rule)
                        .children(self.rows.into_iter().enumerate().map(|(index, row)| {
                            div()
                                .when(index > 0, |this| this.border_t_1().border_color(rule))
                                .child(row)
                        })),
                )
            })
    }
}
