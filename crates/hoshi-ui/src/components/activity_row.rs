use crate::components::{Status, StatusDot};
use crate::theme::ActiveTheme;
use gpui_kit::*;

/// One muted line for something the agent did: a status mark and its name.
#[derive(IntoElement)]
pub struct ActivityRow {
    label: SharedString,
    status: Status,
}

impl ActivityRow {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            status: Status::Muted,
        }
    }

    pub fn status(mut self, status: Status) -> Self {
        self.status = status;
        self
    }
}

impl RenderOnce for ActivityRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .text_color(cx.hoshi().muted_foreground)
            .child(StatusDot::new(Some(self.status)))
            .child(div().truncate().child(self.label))
    }
}
