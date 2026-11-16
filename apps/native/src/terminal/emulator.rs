//! The screen a shell's output draws, emulated by `alacritty_terminal`.

use super::links;
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle, Processor};
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Instant;

const SCROLLBACK: usize = 5000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub columns: usize,
    pub lines: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// A web address on screen: the cells it covers on each visible row it spans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub url: String,
    /// Rows of the screen as shown, with the columns the address takes on each.
    pub spans: Vec<(usize, Range<usize>)>,
}

impl Link {
    pub fn covers(&self, line: usize, column: usize) -> bool {
        self.spans
            .iter()
            .any(|(row, columns)| *row == line && columns.contains(&column))
    }
}

#[derive(Clone, Default)]
pub struct Listener(Rc<RefCell<Vec<Event>>>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        self.0.borrow_mut().push(event);
    }
}

pub struct Emulator {
    term: Term<Listener>,
    parser: Processor,
    events: Listener,
    size: GridSize,
}

impl Emulator {
    pub fn new(size: GridSize) -> Self {
        let events = Listener::default();
        let config = Config {
            scrolling_history: SCROLLBACK,
            default_cursor_style: CursorStyle {
                shape: CursorShape::Block,
                blinking: true,
            },
            ..Config::default()
        };
        Self {
            term: Term::new(config, &size, events.clone()),
            parser: Processor::new(),
            events,
            size,
        }
    }

    pub fn term(&self) -> &Term<Listener> {
        &self.term
    }

    pub fn size(&self) -> GridSize {
        self.size
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    /// The cursor blinks unless the program asked for a steady one.
    pub fn cursor_blinks(&self) -> bool {
        self.term.cursor_style().blinking
    }

    /// When a synchronized update the program began must be drawn even if it never ends it.
    pub fn sync_deadline(&self) -> Option<Instant> {
        self.parser.sync_timeout().sync_timeout()
    }

    /// Draws what a synchronized update buffered so far.
    pub fn end_sync(&mut self) {
        if self.sync_deadline().is_some() {
            self.parser.stop_sync(&mut self.term);
        }
    }

    /// What the emulation asked of the app since the last call: answers to send the shell,
    /// text to copy.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut *self.events.0.borrow_mut())
    }

    /// `false` when the size is the same.
    pub fn resize(&mut self, size: GridSize) -> bool {
        if size == self.size {
            return false;
        }
        self.size = size;
        self.term.resize(size);
        true
    }

    /// Positive goes back in history.
    pub fn scroll_lines(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// Starts a selection at a cell of the screen as shown: one click selects characters,
    /// two words and three lines.
    pub fn select_from(&mut self, line: usize, column: usize, right_half: bool, clicks: usize) {
        let kind = match clicks {
            0 | 1 => SelectionType::Simple,
            2 => SelectionType::Semantic,
            _ => SelectionType::Lines,
        };
        let point = self.grid_point(line, column);
        self.term.selection = Some(Selection::new(kind, point, side(right_half)));
    }

    pub fn select_to(&mut self, line: usize, column: usize, right_half: bool) {
        let point = self.grid_point(line, column);
        if let Some(selection) = &mut self.term.selection {
            selection.update(point, side(right_half));
        }
    }

    /// Selects the whole history and screen.
    pub fn select_all(&mut self) {
        let grid = self.term.grid();
        let start = Point::new(grid.topmost_line(), Column(0));
        let end = Point::new(grid.bottommost_line(), grid.last_column());
        let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
        selection.update(end, Side::Right);
        self.term.selection = Some(selection);
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    pub fn selected_text(&self) -> Option<String> {
        self.term
            .selection_to_string()
            .filter(|text| !text.is_empty())
    }

    /// The web address under a cell of the screen as shown, across the rows it wraps over: an
    /// OSC 8 hyperlink's target, or an address written out.
    pub fn link_at(&self, line: usize, column: usize) -> Option<Link> {
        if line >= self.size.lines || column >= self.size.columns {
            return None;
        }
        let grid = self.term.grid();
        let offset = self.display_offset() as i32;
        let last_column = Column(self.size.columns - 1);
        let wraps = |row: Line| grid[row][last_column].flags.contains(Flags::WRAPLINE);
        let hovered = Line(line as i32 - offset);
        let mut first = hovered;
        while first > self.term.topmost_line() && wraps(first - 1) {
            first -= 1;
        }
        let mut text = Vec::new();
        let mut cells = Vec::new();
        let mut row = first;
        loop {
            for column in 0..self.size.columns {
                let cell = &grid[row][Column(column)];
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    continue;
                }
                text.push(cell.c);
                cells.push((row, column));
            }
            if !wraps(row) || row >= self.term.bottommost_line() {
                break;
            }
            row += 1;
        }
        let at = cells
            .iter()
            .position(|&(row, other)| row == hovered && other == column)?;
        let hyperlink = |index: usize| {
            let (row, column) = cells[index];
            grid[row][Column(column)].hyperlink()
        };
        let found = match hyperlink(at).filter(|link| links::is_web_address(link.uri())) {
            Some(target) => {
                let same = |index: usize| hyperlink(index).as_ref() == Some(&target);
                let start = (0..at).rev().take_while(|&index| same(index)).last();
                let end = (at..cells.len()).take_while(|&index| same(index)).last();
                links::Found {
                    url: target.uri().to_owned(),
                    chars: start.unwrap_or(at)..end.unwrap_or(at) + 1,
                }
            }
            None => links::find_links(&text)
                .into_iter()
                .find(|link| link.chars.contains(&at))?,
        };
        let mut spans: Vec<(usize, Range<usize>)> = Vec::new();
        for &(row, column) in &cells[found.chars] {
            let shown = row.0 + offset;
            if shown < 0 || shown as usize >= self.size.lines {
                continue;
            }
            let shown = shown as usize;
            match spans.last_mut() {
                Some((last, columns)) if *last == shown => columns.end = column + 1,
                _ => spans.push((shown, column..column + 1)),
            }
        }
        Some(Link {
            url: found.url,
            spans,
        })
    }

    fn grid_point(&self, line: usize, column: usize) -> Point {
        let line = line.min(self.size.lines.saturating_sub(1));
        let column = column.min(self.size.columns.saturating_sub(1));
        Point::new(
            Line(line as i32 - self.display_offset() as i32),
            Column(column),
        )
    }
}

fn side(right_half: bool) -> Side {
    if right_half { Side::Right } else { Side::Left }
}

#[cfg(test)]
mod tests {
    use super::{Emulator, GridSize};
    use alacritty_terminal::event::Event;
    use alacritty_terminal::index::{Column, Line, Point};

    #[test]
    fn a_cursor_position_query_is_answered() {
        let mut emulator = Emulator::new(GridSize {
            columns: 10,
            lines: 3,
        });
        emulator.feed(b"ab\x1b[6n");
        let answers: Vec<String> = emulator
            .take_events()
            .into_iter()
            .filter_map(|event| match event {
                Event::PtyWrite(text) => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(answers, vec!["\x1b[1;3R".to_owned()]);
        assert!(emulator.take_events().is_empty());
    }

    #[test]
    fn select_all_takes_the_history_and_the_screen() {
        let mut emulator = Emulator::new(GridSize {
            columns: 20,
            lines: 2,
        });
        emulator.feed(
            b"one
two
three",
        );
        emulator.select_all();
        let text = emulator.selected_text().unwrap_or_default();
        assert!(text.contains("one"), "selected: {text:?}");
        assert!(text.contains("three"), "selected: {text:?}");
    }

    #[test]
    fn a_dragged_selection_reads_back_its_text() {
        let mut emulator = Emulator::new(GridSize {
            columns: 20,
            lines: 3,
        });
        emulator.feed(b"first line\r\nsecond line");
        emulator.select_from(0, 6, false, 1);
        emulator.select_to(1, 5, true);
        assert_eq!(emulator.selected_text().as_deref(), Some("line\nsecond"));
        emulator.select_from(1, 8, false, 2);
        assert_eq!(emulator.selected_text().as_deref(), Some("line"));
        emulator.clear_selection();
        assert_eq!(emulator.selected_text(), None);
    }

    #[test]
    fn a_link_is_found_across_the_rows_it_wraps_over() {
        let mut emulator = Emulator::new(GridSize {
            columns: 12,
            lines: 4,
        });
        emulator.feed(b"go https://example.com/x now\r\nnext");
        let link = emulator.link_at(1, 2).expect("a link under the cell");
        assert_eq!(link.url, "https://example.com/x");
        assert_eq!(link.spans, vec![(0, 3..12), (1, 0..12)]);
        assert!(link.covers(0, 3));
        assert_eq!(emulator.link_at(0, 0), None);
        assert_eq!(emulator.link_at(3, 0), None);
    }

    #[test]
    fn an_osc_8_hyperlink_opens_its_target_from_any_of_its_cells() {
        let mut emulator = Emulator::new(GridSize {
            columns: 20,
            lines: 3,
        });
        emulator.feed(b"see \x1b]8;;https://example.com/docs\x1b\\the docs\x1b]8;;\x1b\\ now");
        let link = emulator
            .link_at(0, 6)
            .expect("the hyperlink under the cell");
        assert_eq!(link.url, "https://example.com/docs");
        assert_eq!(link.spans, vec![(0, 4..12)]);
        assert_eq!(emulator.link_at(0, 13), None);
    }

    #[test]
    fn an_osc_8_hyperlink_to_anything_but_the_web_stays_text() {
        let mut emulator = Emulator::new(GridSize {
            columns: 20,
            lines: 3,
        });
        emulator.feed(b"\x1b]8;;file:///C:/run.exe\x1b\\click me\x1b]8;;\x1b\\");
        assert_eq!(emulator.link_at(0, 2), None);
    }

    #[test]
    fn a_synchronized_update_is_drawn_once_it_ends_or_is_cut_short() {
        let mut emulator = Emulator::new(GridSize {
            columns: 10,
            lines: 2,
        });
        emulator.feed(b"\x1b[?2026hhidden");
        assert!(emulator.sync_deadline().is_some());
        let first = emulator.term().bounds_to_string(
            Point::new(Line(0), Column(0)),
            Point::new(Line(0), Column(9)),
        );
        assert!(first.trim().is_empty(), "{first:?}");
        emulator.end_sync();
        assert_eq!(emulator.sync_deadline(), None);
        let drawn = emulator.term().bounds_to_string(
            Point::new(Line(0), Column(0)),
            Point::new(Line(0), Column(9)),
        );
        assert!(drawn.starts_with("hidden"), "{drawn:?}");
    }

    #[test]
    fn the_cursor_blinks_until_the_program_asks_for_a_steady_one() {
        let mut emulator = Emulator::new(GridSize {
            columns: 10,
            lines: 2,
        });
        assert!(emulator.cursor_blinks());
        emulator.feed(b"\x1b[2 q");
        assert!(!emulator.cursor_blinks());
        emulator.feed(b"\x1b[0 q");
        assert!(emulator.cursor_blinks());
    }

    #[test]
    fn resizing_keeps_the_text() {
        let mut emulator = Emulator::new(GridSize {
            columns: 10,
            lines: 3,
        });
        emulator.feed(b"hello");
        assert!(emulator.resize(GridSize {
            columns: 30,
            lines: 5,
        }));
        assert!(!emulator.resize(emulator.size()));
        let text = emulator.term().bounds_to_string(
            Point::new(Line(0), Column(0)),
            Point::new(Line(0), Column(29)),
        );
        assert!(text.starts_with("hello"), "{text:?}");
    }
}
