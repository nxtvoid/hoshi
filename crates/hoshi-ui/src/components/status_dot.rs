use crate::theme::{ActiveTheme, Colors};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Working,
    Waiting,
    Error,
    Done,
    Muted,
}

impl Status {
    pub fn color(self, colors: &Colors) -> Hsla {
        match self {
            Self::Working => colors.status_working,
            Self::Waiting => colors.status_waiting,
            Self::Error => colors.status_error,
            Self::Done => colors.status_done,
            Self::Muted => colors.muted_foreground,
        }
    }
}

/// A small round mark; without a status it keeps its room and shows nothing.
#[derive(IntoElement)]
pub struct StatusDot {
    status: Option<Status>,
}

impl StatusDot {
    pub fn new(status: Option<Status>) -> Self {
        Self { status }
    }
}

impl RenderOnce for StatusDot {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = self.status.map(|status| status.color(cx.hoshi()));
        div()
            .size_1p5()
            .flex_none()
            .rounded_full()
            .when_some(color, |dot, color| dot.bg(color))
    }
}
