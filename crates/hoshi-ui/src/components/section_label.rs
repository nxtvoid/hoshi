use crate::theme::ActiveTheme;
use gpui_kit::*;

/// The small muted heading above a group of rows.
#[derive(IntoElement)]
pub struct SectionLabel {
    label: SharedString,
    action: Option<AnyElement>,
}

impl SectionLabel {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            action: None,
        }
    }

    /// A small button at the end of the heading, such as "new thread".
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }
}

impl RenderOnce for SectionLabel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap_1()
            .pl_2()
            .pr_1()
            .pt_3()
            .pb_1()
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(cx.hoshi().muted_foreground)
            .child(div().flex_1().min_w_0().truncate().child(self.label))
            .children(self.action)
    }
}
