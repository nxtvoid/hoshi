use crate::theme::ActiveTheme;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// The rounded box a prompt is typed in.
#[derive(IntoElement)]
pub struct InputFrame {
    children: Vec<AnyElement>,
    roomy: bool,
    drop_group: Option<SharedString>,
}

impl InputFrame {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            roomy: false,
            drop_group: None,
        }
    }

    /// More padding, for a prompt that stands alone on the page.
    pub fn roomy(mut self, roomy: bool) -> Self {
        self.roomy = roomy;
        self
    }

    /// Rings the frame while files are dragged over the element named `group`, which takes
    /// them when they are dropped.
    pub fn file_drop_group(mut self, group: impl Into<SharedString>) -> Self {
        self.drop_group = Some(group.into());
        self
    }
}

impl Default for InputFrame {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for InputFrame {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for InputFrame {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi();
        div()
            .rounded_xl()
            .border_1()
            .border_color(colors.border)
            .bg(colors.card)
            .map(|this| if self.roomy { this.p_2() } else { this.p_1() })
            .when_some(self.drop_group, |this, group| {
                let ring = colors.ring;
                this.group_drag_over::<ExternalPaths>(group, move |style| {
                    style.border_color(ring).bg(ring.opacity(0.08))
                })
            })
            .children(self.children)
    }
}
