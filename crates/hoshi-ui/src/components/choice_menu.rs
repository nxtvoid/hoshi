use crate::ClickHandler;
use crate::theme::ActiveTheme;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// A floating list of choices, opened above what it belongs to: a heading, the choices, and
/// a status under them, or alone while there are none.
#[derive(IntoElement)]
pub struct ChoiceMenu {
    id: ElementId,
    heading: Option<SharedString>,
    status: Option<SharedString>,
    items: Vec<Choice>,
    scroll: Option<ScrollHandle>,
}

impl ChoiceMenu {
    pub fn new(id: impl Into<ElementId>, items: impl IntoIterator<Item = Choice>) -> Self {
        Self {
            id: id.into(),
            heading: None,
            status: None,
            items: items.into_iter().collect(),
            scroll: None,
        }
    }

    pub fn heading(mut self, heading: impl Into<SharedString>) -> Self {
        self.heading = Some(heading.into());
        self
    }

    pub fn status(mut self, status: Option<impl Into<SharedString>>) -> Self {
        self.status = status.map(Into::into);
        self
    }

    /// Scrolls the list with `scroll`, so its owner can keep the active choice in view.
    pub fn track_scroll(mut self, scroll: &ScrollHandle) -> Self {
        self.scroll = Some(scroll.clone());
        self
    }
}

impl RenderOnce for ChoiceMenu {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi();
        let list = (!self.items.is_empty()).then(|| {
            div()
                .id(self.id)
                .max_h_72()
                .overflow_y_scroll()
                .px_1()
                .pb_1()
                .when_some(self.scroll, |list, scroll| list.track_scroll(&scroll))
                .children(self.items)
        });
        div()
            .absolute()
            .bottom_full()
            .left_0()
            .right_0()
            .mb_2()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded_xl()
            .border_1()
            .border_color(colors.border)
            .bg(colors.popover)
            .text_color(colors.popover_foreground)
            .shadow_lg()
            .when_some(self.heading, |menu, heading| {
                menu.child(
                    div()
                        .px_3()
                        .pt_2()
                        .pb_1()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(colors.muted_foreground)
                        .child(heading),
                )
            })
            .children(list)
            .when_some(self.status, |menu, status| {
                menu.child(
                    div()
                        .px_3()
                        .pt_1()
                        .pb_2p5()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(status),
                )
            })
    }
}

/// A choice of a [`ChoiceMenu`]: an icon, its label, a code-like hint, and a muted detail.
#[derive(IntoElement)]
pub struct Choice {
    id: ElementId,
    icon: Option<AnyElement>,
    label: SharedString,
    hint: Option<SharedString>,
    detail: Option<SharedString>,
    active: bool,
    on_click: Option<ClickHandler>,
}

impl Choice {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon: None,
            label: label.into(),
            hint: None,
            detail: None,
            active: false,
            on_click: None,
        }
    }

    pub fn icon(mut self, icon: impl IntoElement) -> Self {
        self.icon = Some(icon.into_any_element());
        self
    }

    pub fn hint(mut self, hint: Option<impl Into<SharedString>>) -> Self {
        self.hint = hint.map(Into::into);
        self
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// The one the keyboard is on.
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
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

impl RenderOnce for Choice {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi();
        let mono = cx.theme().mono_font_family.clone();
        div()
            .id(self.id)
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1p5()
            .rounded_lg()
            .text_sm()
            .cursor_pointer()
            .when(self.active, |row| {
                row.bg(colors.accent).text_color(colors.accent_foreground)
            })
            .hover(|row| row.bg(colors.accent))
            .when_some(self.on_click, |row, handler| row.on_click(handler))
            .when_some(self.icon, |row, icon| {
                row.child(
                    div()
                        .flex_none()
                        .text_color(colors.muted_foreground)
                        .child(icon),
                )
            })
            .child(
                div()
                    .flex_none()
                    .max_w(relative(0.5))
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.label),
            )
            .when_some(self.hint, |row, hint| {
                row.child(
                    div()
                        .flex_none()
                        .max_w(relative(0.33))
                        .truncate()
                        .font_family(mono)
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(hint),
                )
            })
            .when_some(self.detail, |row, detail| {
                row.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(detail),
                )
            })
    }
}
