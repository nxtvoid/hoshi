use crate::theme::ActiveTheme;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

/// Lines added and removed: `+12 −3`.
#[derive(IntoElement)]
pub struct DiffStat {
    additions: u32,
    deletions: u32,
}

impl DiffStat {
    pub fn new(additions: u32, deletions: u32) -> Self {
        Self {
            additions,
            deletions,
        }
    }
}

impl RenderOnce for DiffStat {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let mono = cx.theme().mono_font_family.clone();
        let colors = cx.hoshi();
        div()
            .flex()
            .flex_none()
            .gap_1()
            .font_family(mono)
            .text_xs()
            .child(
                div()
                    .text_color(colors.pr_open)
                    .child(format!("+{}", self.additions)),
            )
            .child(
                div()
                    .text_color(colors.status_error)
                    .child(format!("−{}", self.deletions)),
            )
    }
}
