use crate::theme::{ActiveTheme, Colors};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    Hunk,
    Added,
    Removed,
    Context,
    Note,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
    /// In the old file; `None` for added lines and hunk headers.
    pub old_line: Option<u32>,
    /// In the new file; `None` for removed lines and hunk headers.
    pub new_line: Option<u32>,
}

/// One file's `git diff` output as numbered lines, without the file headers.
pub fn parse_unified_diff(text: &str) -> Vec<DiffLine> {
    let mut lines = Vec::new();
    let (mut old_line, mut new_line) = (0u32, 0u32);
    let mut in_hunk = false;
    for raw in text.split('\n') {
        if let Some((old, new)) = hunk_start(raw) {
            in_hunk = true;
            (old_line, new_line) = (old, new);
            lines.push(DiffLine {
                kind: DiffLineKind::Hunk,
                text: raw.to_owned(),
                old_line: None,
                new_line: None,
            });
            continue;
        }
        if !in_hunk {
            continue;
        }
        let line = |kind, text: &str, old_line, new_line| DiffLine {
            kind,
            text: text.to_owned(),
            old_line,
            new_line,
        };
        if let Some(text) = raw.strip_prefix('+') {
            lines.push(line(DiffLineKind::Added, text, None, Some(new_line)));
            new_line += 1;
        } else if let Some(text) = raw.strip_prefix('-') {
            lines.push(line(DiffLineKind::Removed, text, Some(old_line), None));
            old_line += 1;
        } else if let Some(text) = raw.strip_prefix(' ') {
            lines.push(line(
                DiffLineKind::Context,
                text,
                Some(old_line),
                Some(new_line),
            ));
            old_line += 1;
            new_line += 1;
        } else if raw.starts_with('\\') {
            lines.push(line(DiffLineKind::Note, raw, None, None));
        } else if raw.starts_with("diff --git") {
            in_hunk = false;
        }
    }
    lines
}

/// `@@ -12,3 +14,5 @@` → `(12, 14)`.
fn hunk_start(line: &str) -> Option<(u32, u32)> {
    let ranges = line.strip_prefix("@@ -")?;
    let (ranges, _) = ranges.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    let start = |range: &str| range.split(',').next()?.parse().ok();
    Some((start(old)?, start(new)?))
}

/// A unified diff with old and new line numbers, added and removed lines tinted.
#[derive(IntoElement)]
pub struct DiffView {
    id: ElementId,
    lines: Vec<DiffLine>,
    max_height: Pixels,
}

impl DiffView {
    pub fn new(id: impl Into<ElementId>, lines: Vec<DiffLine>) -> Self {
        Self {
            id: id.into(),
            lines,
            max_height: px(384.),
        }
    }

    pub fn max_height(mut self, max_height: Pixels) -> Self {
        self.max_height = max_height;
        self
    }
}

impl RenderOnce for DiffView {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let mono = cx.theme().mono_font_family.clone();
        let size = cx.theme().mono_font_size;
        let wrap = crate::theme::fonts(cx).word_wrap;
        let colors = cx.hoshi().clone();
        let lines = self
            .lines
            .into_iter()
            .map(|line| render_line(line, wrap, None, &colors));
        div()
            .id(self.id)
            .debug_selector(|| "diff-view".into())
            .max_h(self.max_height)
            .overflow_y_scroll()
            .when(!wrap, |view| view.overflow_x_scroll())
            .py_1()
            .font_family(mono)
            .text_size(size)
            .map(|view| {
                if wrap {
                    view.children(lines)
                } else {
                    view.child(div().flex().flex_col().min_w_full().children(lines))
                }
            })
    }
}

/// One line of a [`DiffView`] on its own, for lists that only build the rows in view.
#[derive(IntoElement)]
pub struct DiffLineRow {
    line: DiffLine,
    selectable: Option<(ElementId, u64)>,
}

impl DiffLineRow {
    pub fn new(line: DiffLine) -> Self {
        Self {
            line,
            selectable: None,
        }
    }

    /// Lets the window's text selection take the line's text, without its numbers and sign.
    /// `id` must be stable while the line shows; `order` places it among the other lines.
    pub fn selectable(mut self, id: impl Into<ElementId>, order: u64) -> Self {
        self.selectable = Some((id.into(), order));
        self
    }
}

impl RenderOnce for DiffLineRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let mono = cx.theme().mono_font_family.clone();
        let size = cx.theme().mono_font_size;
        let wrap = crate::theme::fonts(cx).word_wrap;
        let colors = cx.hoshi().clone();
        render_line(self.line, wrap, self.selectable, &colors)
            .font_family(mono)
            .text_size(size)
            .when(!wrap, |row| row.min_w_full())
    }
}

fn render_line(
    line: DiffLine,
    wrap: bool,
    selectable: Option<(ElementId, u64)>,
    colors: &Colors,
) -> Div {
    let number = |number: Option<u32>| {
        div()
            .w_10()
            .flex_none()
            .text_right()
            .text_color(colors.muted_foreground)
            .child(number.map(|number| number.to_string()).unwrap_or_default())
    };
    let (sign, sign_color) = match line.kind {
        DiffLineKind::Added => ("+", colors.pr_open),
        DiffLineKind::Removed => ("−", colors.status_error),
        _ => ("", colors.muted_foreground),
    };
    let text = match selectable {
        Some((id, order)) => gpui_kit::base::SelectableText::new(id, line.text)
            .document_order(order)
            .into_any_element(),
        None if line.text.is_empty() => " ".into_any_element(),
        None => line.text.into_any_element(),
    };
    div()
        .flex()
        .map(|row| match line.kind {
            DiffLineKind::Hunk => row
                .bg(colors.muted.opacity(0.6))
                .text_color(colors.muted_foreground),
            DiffLineKind::Added => row.bg(colors.diff_added),
            DiffLineKind::Removed => row.bg(colors.diff_removed),
            DiffLineKind::Note => row.text_color(colors.muted_foreground).italic(),
            DiffLineKind::Context => row,
        })
        .child(number(line.old_line).pr_1())
        .child(number(line.new_line).pr_2())
        .child(div().w_3().flex_none().text_color(sign_color).child(sign))
        .child(
            div()
                .flex_1()
                .pr_3()
                .map(|cell| {
                    if wrap {
                        cell.min_w_0()
                    } else {
                        cell.flex_none().whitespace_nowrap()
                    }
                })
                .child(text),
        )
}

#[cfg(test)]
mod tests {
    use super::{DiffLine, DiffLineKind, parse_unified_diff};

    #[test]
    fn numbers_each_side_and_drops_the_file_headers() {
        let diff = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -3,3 +3,3 @@ fn main\n keep\n-old\n+new\n\\ No newline at end of file";
        let lines = parse_unified_diff(diff);
        let shape: Vec<_> = lines
            .iter()
            .map(|line| (line.kind, line.old_line, line.new_line))
            .collect();
        assert_eq!(
            shape,
            [
                (DiffLineKind::Hunk, None, None),
                (DiffLineKind::Context, Some(3), Some(3)),
                (DiffLineKind::Removed, Some(4), None),
                (DiffLineKind::Added, None, Some(4)),
                (DiffLineKind::Note, None, None),
            ]
        );
        assert_eq!(
            lines[2],
            DiffLine {
                kind: DiffLineKind::Removed,
                text: "old".into(),
                old_line: Some(4),
                new_line: None,
            }
        );
    }

    #[test]
    fn reads_a_hunk_without_counts() {
        let lines = parse_unified_diff("@@ -1 +1 @@\n-a\n+b");
        assert_eq!(lines[1].old_line, Some(1));
        assert_eq!(lines[2].new_line, Some(1));
    }
}
