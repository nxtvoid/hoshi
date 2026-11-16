use crate::ClickHandler;
use crate::theme::ActiveTheme;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// A row of a sidebar list: an optional leading mark, a label that truncates, and a
/// selected look.
#[derive(IntoElement)]
pub struct NavItem {
    id: ElementId,
    label: SharedString,
    leading: Option<AnyElement>,
    selected: bool,
    on_click: Option<ClickHandler>,
}

impl NavItem {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            leading: None,
            selected: false,
            on_click: None,
        }
    }

    pub fn leading(mut self, leading: impl IntoElement) -> Self {
        self.leading = Some(leading.into_any_element());
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
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

impl RenderOnce for NavItem {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi();
        div()
            .id(self.id)
            .flex()
            .items_center()
            .gap_2()
            .h_7()
            .px_2()
            .rounded_md()
            .text_sm()
            .cursor_pointer()
            .text_color(colors.sidebar_foreground)
            .when(self.selected, |row| {
                row.bg(colors.sidebar_accent)
                    .text_color(colors.sidebar_accent_foreground)
            })
            .hover(|row| row.bg(colors.sidebar_accent))
            .when_some(self.on_click, |row, handler| row.on_click(handler))
            .children(self.leading)
            .child(div().flex_1().truncate().child(self.label))
    }
}
