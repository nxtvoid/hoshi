//! The visible screen as runs to paint: only the rows on screen, cells of one look merged.

use super::colors::Palette;
use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, point_to_viewport};
use alacritty_terminal::vte::ansi::CursorShape;
use gpui_kit::Hsla;

#[derive(Clone, Debug, PartialEq)]
pub struct Fill {
    pub column: usize,
    pub width: usize,
    pub color: Hsla,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub column: usize,
    pub text: String,
    pub color: Hsla,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

impl Text {
    fn same_look(&self, other: &Text) -> bool {
        self.color == other.color
            && self.bold == other.bold
            && self.italic == other.italic
            && self.underline == other.underline
            && self.strike == other.strike
    }

    fn end(&self) -> usize {
        self.column + self.text.chars().count()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub fills: Vec<Fill>,
    pub highlights: Vec<Fill>,
    pub texts: Vec<Text>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorLook {
    Block,
    Hollow,
    Beam,
    Underline,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cursor {
    pub line: usize,
    pub column: usize,
    pub width: usize,
    pub look: CursorLook,
    pub color: Hsla,
}

/// How the cursor shows: solid with the keyboard, hollow without, or blinked off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorState {
    Focused,
    Blurred,
    BlinkedOff,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub rows: Vec<Row>,
    pub cursor: Option<Cursor>,
}

impl Frame {
    pub fn read<T: EventListener>(term: &Term<T>, palette: &Palette, state: CursorState) -> Self {
        let content = term.renderable_content();
        let lines = term.grid().screen_lines();
        let mut rows = vec![Row::default(); lines];
        let cursor_point = point_to_viewport(content.display_offset, content.cursor.point);
        let look = match content.cursor.shape {
            CursorShape::Hidden => None,
            _ if state == CursorState::BlinkedOff => None,
            _ if state == CursorState::Blurred => Some(CursorLook::Hollow),
            CursorShape::Block => Some(CursorLook::Block),
            CursorShape::HollowBlock => Some(CursorLook::Hollow),
            CursorShape::Beam => Some(CursorLook::Beam),
            CursorShape::Underline => Some(CursorLook::Underline),
        };
        let mut cursor = None;

        for indexed in content.display_iter {
            let Some(point) = point_to_viewport(content.display_offset, indexed.point) else {
                continue;
            };
            let Some(row) = rows.get_mut(point.line) else {
                continue;
            };
            let cell = indexed.cell;
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let width = if cell.flags.contains(Flags::WIDE_CHAR) {
                2
            } else {
                1
            };
            let (mut color, mut fill) = palette.cell(cell.fg, cell.bg, cell.flags, content.colors);
            if Some(point) == cursor_point
                && let Some(look) = look
            {
                cursor = Some(Cursor {
                    line: point.line,
                    column: point.column.0,
                    width,
                    look,
                    color: palette.cursor,
                });
                if look == CursorLook::Block {
                    color = fill;
                    fill = palette.cursor;
                }
            }
            if fill != palette.background {
                push_fill(&mut row.fills, point.column.0, width, fill);
            }
            if content
                .selection
                .is_some_and(|selection| selection.contains(indexed.point))
            {
                push_fill(
                    &mut row.highlights,
                    point.column.0,
                    width,
                    palette.selection,
                );
            }
            let character = if cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
                ' '
            } else {
                cell.c
            };
            let mut text = String::from(character);
            if let Some(zerowidth) = cell.zerowidth() {
                text.extend(zerowidth);
            }
            push_text(
                &mut row.texts,
                Text {
                    column: point.column.0,
                    text,
                    color,
                    bold: cell.flags.contains(Flags::BOLD),
                    italic: cell.flags.contains(Flags::ITALIC),
                    underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
                    strike: cell.flags.contains(Flags::STRIKEOUT),
                },
            );
        }
        for row in &mut rows {
            for text in &mut row.texts {
                let trimmed = text.text.trim_end_matches(' ').len();
                text.text.truncate(trimmed);
            }
            row.texts.retain(|text| !text.text.is_empty());
        }
        Self { rows, cursor }
    }

    #[cfg(test)]
    /// The row as plain text, spaces where nothing is drawn.
    pub fn row_text(&self, line: usize) -> String {
        let mut line_text = String::new();
        let Some(row) = self.rows.get(line) else {
            return line_text;
        };
        let mut column = 0;
        for text in &row.texts {
            while column < text.column {
                line_text.push(' ');
                column += 1;
            }
            line_text.push_str(&text.text);
            column = text.end();
        }
        line_text
    }
}

fn push_fill(fills: &mut Vec<Fill>, column: usize, width: usize, color: Hsla) {
    if let Some(last) = fills.last_mut()
        && last.color == color
        && last.column + last.width == column
    {
        last.width += width;
        return;
    }
    fills.push(Fill {
        column,
        width,
        color,
    });
}

/// Ascii cells of one look join the run before them; any other character starts its own,
/// so a glyph from a fallback font can't push the rest of the row off the grid.
fn push_text(texts: &mut Vec<Text>, text: Text) {
    let blank = text.text == " " && !text.underline && !text.strike;
    if let Some(last) = texts.last_mut()
        && last.end() == text.column
        && last.text.is_ascii()
        && text.text.is_ascii()
        && (blank || last.same_look(&text))
    {
        last.text.push_str(&text.text);
        return;
    }
    if blank {
        return;
    }
    texts.push(text);
}

#[cfg(test)]
mod tests {
    use super::{CursorLook, CursorState, Frame};
    use crate::terminal::colors::Palette;
    use crate::terminal::emulator::{Emulator, GridSize};
    use gpui_kit::component::theme::ThemeMode;

    fn palette() -> Palette {
        Palette::new(&hoshi_ui::theme::colors(ThemeMode::Dark))
    }

    fn emulator(output: &str) -> Emulator {
        let mut emulator = Emulator::new(GridSize {
            columns: 20,
            lines: 4,
        });
        emulator.feed(output.as_bytes());
        emulator
    }

    #[test]
    fn rows_read_back_what_was_printed() {
        let emulator = emulator("hello world\r\n  indented");
        let frame = Frame::read(emulator.term(), &palette(), CursorState::Focused);
        assert_eq!(frame.rows.len(), 4);
        assert_eq!(frame.row_text(0), "hello world");
        assert_eq!(frame.row_text(1), "  indented");
        assert_eq!(frame.row_text(2), "");
        assert_eq!(frame.rows[0].texts.len(), 1);
    }

    #[test]
    fn colored_cells_split_runs_and_fill_their_background() {
        let palette = palette();
        let emulator = emulator("ab\x1b[31mcd\x1b[0m \x1b[44mef\x1b[0m");
        let frame = Frame::read(emulator.term(), &palette, CursorState::Focused);
        let row = &frame.rows[0];
        assert_eq!(frame.row_text(0), "abcd ef");
        assert_eq!(row.texts[1].text, "cd");
        assert_eq!(row.texts[1].color, palette.ansi[1]);
        assert_eq!(row.fills.len(), 2);
        assert_eq!((row.fills[0].column, row.fills[0].width), (5, 2));
        assert_eq!(row.fills[0].color, palette.ansi[4]);
    }

    #[test]
    fn wide_characters_take_two_cells() {
        let emulator = emulator("a漢b");
        let frame = Frame::read(emulator.term(), &palette(), CursorState::Focused);
        let row = &frame.rows[0];
        assert_eq!(row.texts.len(), 3);
        assert_eq!(row.texts[1].column, 1);
        assert_eq!(row.texts[2].column, 3);
    }

    #[test]
    fn the_cursor_is_a_block_when_focused_and_hollow_otherwise() {
        let palette = palette();
        let emulator = emulator("ab");
        let focused = Frame::read(emulator.term(), &palette, CursorState::Focused);
        let cursor = focused.cursor.unwrap();
        assert_eq!((cursor.line, cursor.column), (0, 2));
        assert_eq!(cursor.look, CursorLook::Block);
        assert_eq!(focused.rows[0].fills[0].color, palette.cursor);
        let blurred = Frame::read(emulator.term(), &palette, CursorState::Blurred);
        assert_eq!(blurred.cursor.unwrap().look, CursorLook::Hollow);
        assert!(blurred.rows[0].fills.is_empty());
        let hidden = self::emulator("ab\x1b[?25l");
        assert_eq!(
            Frame::read(hidden.term(), &palette, CursorState::Focused).cursor,
            None
        );
    }

    #[test]
    fn scrolling_back_shows_older_lines() {
        let mut emulator = emulator("1\r\n2\r\n3\r\n4\r\n5\r\n6");
        let frame = Frame::read(emulator.term(), &palette(), CursorState::Focused);
        assert_eq!(frame.row_text(0), "3");
        emulator.scroll_lines(2);
        let frame = Frame::read(emulator.term(), &palette(), CursorState::Focused);
        assert_eq!(frame.row_text(0), "1");
        assert_eq!(frame.row_text(3), "4");
        assert_eq!(frame.cursor, None);
    }
}
