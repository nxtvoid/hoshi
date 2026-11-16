use crate::theme::ActiveTheme;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// Text in the mono font that wraps and scrolls past `max_height`, with an optional muted
/// line under it, such as how much was left out.
#[derive(IntoElement)]
pub struct CodeBlock {
    id: ElementId,
    text: SharedString,
    footer: Option<SharedString>,
    max_height: Pixels,
    muted: bool,
    divided: bool,
}

impl CodeBlock {
    pub fn new(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            footer: None,
            max_height: px(320.),
            muted: false,
            divided: false,
        }
    }

    pub fn footer(mut self, footer: Option<SharedString>) -> Self {
        self.footer = footer;
        self
    }

    pub fn max_height(mut self, max_height: Pixels) -> Self {
        self.max_height = max_height;
        self
    }

    pub fn muted(mut self, muted: bool) -> Self {
        self.muted = muted;
        self
    }

    /// A rule above it, for a block that follows another.
    pub fn divided(mut self, divided: bool) -> Self {
        self.divided = divided;
        self
    }
}

impl RenderOnce for CodeBlock {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let mono = cx.theme().mono_font_family.clone();
        let colors = cx.hoshi();
        let selector = format!("code-block-{}", self.id);
        let color = if self.muted {
            colors.muted_foreground
        } else {
            colors.foreground.opacity(0.85)
        };
        let size = cx.theme().mono_font_size;
        let wrap = crate::theme::fonts(cx).word_wrap;
        div()
            .id(self.id)
            .debug_selector(|| selector)
            .max_h(self.max_height)
            .overflow_y_scroll()
            .when(!wrap, |block| block.overflow_x_scroll().whitespace_nowrap())
            .px_3()
            .py_2()
            .font_family(mono)
            .text_size(size)
            .text_color(color)
            .when(self.divided, |block| {
                block.border_t_1().border_color(colors.border.opacity(0.6))
            })
            .child(self.text)
            .when_some(self.footer, |block, footer| {
                block.child(
                    div()
                        .pt_1()
                        .text_color(colors.muted_foreground)
                        .italic()
                        .child(footer),
                )
            })
    }
}

/// The first `max_lines` lines of `text`, at most `max_chars` of it, and how many lines were
/// left out.
pub fn head_lines(text: &str, max_lines: usize, max_chars: usize) -> (&str, usize) {
    let mut end = text.len();
    if let Some((index, _)) = text.match_indices('\n').nth(max_lines.saturating_sub(1)) {
        end = index;
    }
    if let Some((index, _)) = text.char_indices().nth(max_chars) {
        end = end.min(index);
    }
    let shown = &text[..end];
    let total = text.trim_end_matches('\n').lines().count();
    let kept = shown.lines().count();
    (shown, total.saturating_sub(kept))
}

#[cfg(test)]
mod tests {
    use super::head_lines;

    #[test]
    fn keeps_short_text_whole() {
        assert_eq!(head_lines("a\nb\n", 5, 100), ("a\nb\n", 0));
    }

    #[test]
    fn cuts_at_the_line_cap_and_counts_the_rest() {
        assert_eq!(head_lines("1\n2\n3\n4\n5", 2, 100), ("1\n2", 3));
    }

    #[test]
    fn cuts_a_long_line_at_the_char_cap() {
        let (shown, hidden) = head_lines("ééééé\nnext", 10, 3);
        assert_eq!(shown, "ééé");
        assert_eq!(hidden, 1);
    }
}
