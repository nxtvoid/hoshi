use crate::theme::ActiveTheme;
use gpui_kit::*;

/// What the user wrote, set to the right of the lane.
#[derive(IntoElement)]
pub struct MessageBubble {
    children: Vec<AnyElement>,
}

impl MessageBubble {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }
}

impl Default for MessageBubble {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for MessageBubble {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for MessageBubble {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi();
        div().flex().flex_col().items_end().child(
            div()
                .max_w(relative(0.8))
                .px_3()
                .py_2()
                .rounded_lg()
                .bg(colors.secondary)
                .text_color(colors.secondary_foreground)
                .text_sm()
                .children(self.children),
        )
    }
}
