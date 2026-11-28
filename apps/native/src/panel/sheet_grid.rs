//! A table drawn as a spreadsheet: lettered columns and numbered rows that stay in place as it
//! scrolls, cells picked one at a time or as a range, and the range copied as tab-separated
//! text. Only the cells in view are built.

use crate::edit_menu::{EditAction, EditMenu, RunEdit, enabled_edit_items};
use crate::m;
use crate::toast::{self, Toast};
use gpui_kit::component::scroll::{ScrollableElement as _, ScrollbarAxis};
use gpui_kit::*;
use hoshi_engine::table::{Grid, Kind, Value};
use hoshi_ui::prelude::*;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

const ROW_HEIGHT: f32 = 22.;
const HEADER_HEIGHT: f32 = 22.;
/// Pixels a character of the table's text takes, near enough to size columns before drawing.
const CHAR_WIDTH: f32 = 7.;
const CELL_PADDING: f32 = 20.;
const MIN_COLUMN: f32 = 48.;
const MAX_COLUMN: f32 = 360.;
/// Rows kept split into cells; the rest are split again when they come back into view.
const KEPT_ROWS: usize = 400;
/// Characters of a cell drawn; the rest is copied but not shown.
pub const MAX_SHOWN_CHARS: usize = 2000;
/// The most cells copied at once.
const MAX_COPIED_CELLS: usize = 1_000_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub row: usize,
    pub column: usize,
}

/// The cells picked: from `anchor`, where the pick started, to `focus`, which keys move.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Cell,
    pub focus: Cell,
}

impl Selection {
    pub fn at(cell: Cell) -> Self {
        Self {
            anchor: cell,
            focus: cell,
        }
    }

    pub fn rows(&self) -> Range<usize> {
        self.anchor.row.min(self.focus.row)..self.anchor.row.max(self.focus.row) + 1
    }

    pub fn columns(&self) -> Range<usize> {
        self.anchor.column.min(self.focus.column)..self.anchor.column.max(self.focus.column) + 1
    }

    pub fn contains(&self, cell: Cell) -> bool {
        self.rows().contains(&cell.row) && self.columns().contains(&cell.column)
    }
}

/// How a key moves the selection over a sheet of `rows` by `columns`, `page` rows a screen:
/// with `extend` only its focus moves, and `jump` goes to the sheet's edge. `None` for keys
/// that don't move it.
pub fn moved(
    selection: Selection,
    key: &str,
    extend: bool,
    jump: bool,
    (rows, columns): (usize, usize),
    page: usize,
) -> Option<Selection> {
    let (last_row, last_column) = (rows.saturating_sub(1), columns.saturating_sub(1));
    let Cell { row, column } = selection.focus;
    let focus = match key {
        "up" if jump => Cell { row: 0, column },
        "up" => Cell {
            row: row.saturating_sub(1),
            column,
        },
        "down" if jump => Cell {
            row: last_row,
            column,
        },
        "down" => Cell {
            row: (row + 1).min(last_row),
            column,
        },
        "left" if jump => Cell { row, column: 0 },
        "left" => Cell {
            row,
            column: column.saturating_sub(1),
        },
        "right" if jump => Cell {
            row,
            column: last_column,
        },
        "right" => Cell {
            row,
            column: (column + 1).min(last_column),
        },
        "pageup" => Cell {
            row: row.saturating_sub(page.max(1)),
            column,
        },
        "pagedown" => Cell {
            row: (row + page.max(1)).min(last_row),
            column,
        },
        "home" if jump => Cell::default(),
        "home" => Cell { row, column: 0 },
        "end" if jump => Cell {
            row: last_row,
            column: last_column,
        },
        "end" => Cell {
            row,
            column: last_column,
        },
        _ => return None,
    };
    Some(if extend {
        Selection {
            anchor: selection.anchor,
            focus,
        }
    } else {
        Selection::at(focus)
    })
}

/// `text` on one line, its line breaks as spaces, and cut at [`MAX_SHOWN_CHARS`].
pub fn one_line(text: &str) -> String {
    let mut line = String::with_capacity(text.len().min(MAX_SHOWN_CHARS));
    let mut chars = text.chars().peekable();
    for _ in 0..MAX_SHOWN_CHARS {
        let Some(char) = chars.next() else {
            break;
        };
        match char {
            '\r' => {
                chars.next_if_eq(&'\n');
                line.push(' ');
            }
            '\n' => line.push(' '),
            char => line.push(char),
        }
    }
    line
}

/// The words true and false read as, in the language shown.
#[derive(Clone)]
struct Logicals {
    yes: String,
    no: String,
}

impl Logicals {
    fn shown() -> Self {
        Self {
            yes: m::sheet_true(),
            no: m::sheet_false(),
        }
    }

    /// A cell's text, with true and false named.
    fn text(&self, value: Value) -> String {
        match value.kind {
            Kind::Logical(true) => self.yes.clone(),
            Kind::Logical(false) => self.no.clone(),
            _ => value.text,
        }
    }
}

/// A cell as drawn.
struct Shown {
    text: SharedString,
    align: Align,
}

/// The rows' cells as a spreadsheet pastes them: tabs between cells, a line per row, and
/// quotes around a cell that holds a tab, a line break or a quote.
pub fn tab_separated(rows: impl IntoIterator<Item = Vec<String>>) -> String {
    let mut text = String::new();
    for (index, row) in rows.into_iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        for (column, cell) in row.iter().enumerate() {
            if column > 0 {
                text.push('\t');
            }
            if cell.contains(['\t', '\n', '\r', '"']) {
                text.push('"');
                text.push_str(&cell.replace('"', "\"\""));
                text.push('"');
            } else {
                text.push_str(cell);
            }
        }
    }
    text
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Align {
    Left,
    Center,
    Right,
}

/// Where a spreadsheet puts a value by default: numbers and dates right, true and false
/// centered, text left.
fn align(kind: Kind) -> Align {
    match kind {
        Kind::Number | Kind::Date => Align::Right,
        Kind::Logical(_) => Align::Center,
        Kind::Text => Align::Left,
    }
}

/// Where row `row` starts, below the header, with the rows scrolled `scrolled` pixels up.
/// Counted in `f64`: a long table runs past where `f32` holds whole pixels.
fn row_top(row: usize, scrolled: f32) -> f32 {
    (row as f64 * f64::from(ROW_HEIGHT) - f64::from(scrolled)) as f32
}

/// The row at `y` pixels below the header, with the rows scrolled `scrolled` pixels up.
fn row_at(y: f32, scrolled: f32) -> usize {
    ((f64::from(y) + f64::from(scrolled)) / f64::from(ROW_HEIGHT))
        .floor()
        .max(0.) as usize
}

/// `scrolled` held between the start and how far `content` scrolls in `viewport`.
fn within(scrolled: f32, content: f32, viewport: f32) -> f32 {
    scrolled.clamp(0., (content - viewport).max(0.))
}

/// What a press landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Corner,
    Column(usize),
    Row(usize),
    Cell(Cell),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drag {
    Cells,
    Rows,
    Columns,
}

pub struct SheetGrid {
    grid: Arc<Grid>,
    /// Where each column starts from the first one's left edge, and where the last one ends.
    lefts: Vec<f32>,
    gutter: f32,
    scroll: ScrollHandle,
    viewport: Size<Pixels>,
    selection: Selection,
    drag: Option<Drag>,
    rows: HashMap<usize, Rc<Vec<Shown>>>,
    focus: FocusHandle,
    menu: Option<(Point<Pixels>, usize)>,
    menus: usize,
    copying: Option<Task<()>>,
}

impl SheetGrid {
    pub fn new(grid: Arc<Grid>, cx: &mut Context<Self>) -> Self {
        let mut lefts = Vec::with_capacity(grid.columns() + 1);
        let mut left = 0.;
        lefts.push(left);
        for column in 0..grid.columns() {
            left += (grid.width(column) as f32 * CHAR_WIDTH + CELL_PADDING)
                .clamp(MIN_COLUMN, MAX_COLUMN);
            lefts.push(left);
        }
        let digits = grid.rows().max(1).to_string().len();
        Self {
            gutter: (digits as f32 * CHAR_WIDTH + 16.).max(36.),
            grid,
            lefts,
            scroll: ScrollHandle::new(),
            viewport: Size::default(),
            selection: Selection::default(),
            drag: None,
            rows: HashMap::new(),
            focus: cx.focus_handle(),
            menu: None,
            menus: 0,
            copying: None,
        }
    }

    /// How many rows are split into cells now.
    #[cfg(test)]
    pub fn built_rows(&self) -> usize {
        self.rows.len()
    }

    fn row(&mut self, index: usize) -> Rc<Vec<Shown>> {
        if let Some(row) = self.rows.get(&index) {
            return row.clone();
        }
        if self.rows.len() >= KEPT_ROWS {
            self.rows.clear();
        }
        let logicals = Logicals::shown();
        let row: Rc<Vec<Shown>> = Rc::new(
            self.grid
                .row(index)
                .into_iter()
                .map(|value| Shown {
                    align: align(value.kind),
                    text: one_line(&logicals.text(value)).into(),
                })
                .collect(),
        );
        self.rows.insert(index, row.clone());
        row
    }

    fn size(&self) -> (usize, usize) {
        (self.grid.rows(), self.grid.columns())
    }

    fn content_width(&self) -> f32 {
        self.lefts.last().copied().unwrap_or(0.)
    }

    /// How far the cells are scrolled, right and down. A wheel moves the offset past the ends
    /// until the next layout pulls it back, so it is held inside them here.
    fn scrolled(&self) -> (f32, f32) {
        let offset = self.scroll.offset();
        let (rows, _) = self.size();
        (
            within(
                -f32::from(offset.x),
                self.gutter + self.content_width(),
                f32::from(self.viewport.width),
            ),
            within(
                -f32::from(offset.y),
                HEADER_HEIGHT + rows as f32 * ROW_HEIGHT,
                f32::from(self.viewport.height),
            ),
        )
    }

    /// The width and height the cells have, between the row numbers and column letters.
    fn cells_area(&self) -> (f32, f32) {
        (
            (f32::from(self.viewport.width) - self.gutter).max(0.),
            (f32::from(self.viewport.height) - HEADER_HEIGHT).max(0.),
        )
    }

    fn column_at(&self, x: f32) -> usize {
        let columns = self.grid.columns();
        self.lefts[..columns]
            .partition_point(|left| *left <= x)
            .saturating_sub(1)
            .min(columns.saturating_sub(1))
    }

    /// What `position` is over; a cell past the edges when it is outside, so a drag there
    /// reaches the cells out of view.
    fn hit(&self, position: Point<Pixels>) -> Hit {
        let origin = self.scroll.bounds().origin;
        let (x, y) = (
            f32::from(position.x - origin.x),
            f32::from(position.y - origin.y),
        );
        let (scrolled_x, scrolled_y) = self.scrolled();
        let (rows, _) = self.size();
        let column = self.column_at(x - self.gutter + scrolled_x);
        let row = row_at(y - HEADER_HEIGHT, scrolled_y).min(rows.saturating_sub(1));
        let inside = x >= 0. && y >= 0.;
        match (inside && x < self.gutter, inside && y < HEADER_HEIGHT) {
            (true, true) => Hit::Corner,
            (true, false) => Hit::Row(row),
            (false, true) => Hit::Column(column),
            (false, false) => Hit::Cell(Cell { row, column }),
        }
    }

    fn select(&mut self, selection: Selection, cx: &mut Context<Self>) {
        if self.selection != selection {
            self.selection = selection;
            cx.notify();
        }
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        let (rows, columns) = self.size();
        self.select(
            Selection {
                anchor: Cell::default(),
                focus: Cell {
                    row: rows.saturating_sub(1),
                    column: columns.saturating_sub(1),
                },
            },
            cx,
        );
    }

    fn press(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.menu = None;
        let (rows, columns) = self.size();
        let (last_row, last_column) = (rows.saturating_sub(1), columns.saturating_sub(1));
        let anchor = self.selection.anchor;
        let extend = event.modifiers.shift;
        let (selection, drag) = match self.hit(event.position) {
            Hit::Corner => {
                self.select_all(cx);
                return;
            }
            Hit::Column(column) => (
                Selection {
                    anchor: if extend {
                        Cell { row: 0, ..anchor }
                    } else {
                        Cell { row: 0, column }
                    },
                    focus: Cell {
                        row: last_row,
                        column,
                    },
                },
                Drag::Columns,
            ),
            Hit::Row(row) => (
                Selection {
                    anchor: if extend {
                        Cell {
                            column: 0,
                            ..anchor
                        }
                    } else {
                        Cell { row, column: 0 }
                    },
                    focus: Cell {
                        row,
                        column: last_column,
                    },
                },
                Drag::Rows,
            ),
            Hit::Cell(cell) if extend => (
                Selection {
                    anchor,
                    focus: cell,
                },
                Drag::Cells,
            ),
            Hit::Cell(cell) => (Selection::at(cell), Drag::Cells),
        };
        self.drag = Some(drag);
        self.select(selection, cx);
    }

    fn drag_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.drag else {
            return;
        };
        let (rows, columns) = self.size();
        let anchor = self.selection.anchor;
        let cell = match self.hit(position) {
            Hit::Cell(cell) => cell,
            Hit::Row(row) => Cell { row, column: 0 },
            Hit::Column(column) => Cell { row: 0, column },
            Hit::Corner => Cell::default(),
        };
        let focus = match drag {
            Drag::Cells => cell,
            Drag::Rows => Cell {
                row: cell.row,
                column: columns.saturating_sub(1),
            },
            Drag::Columns => Cell {
                row: rows.saturating_sub(1),
                column: cell.column,
            },
        };
        self.select(Selection { anchor, focus }, cx);
        self.reveal(focus, cx);
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            cx.notify();
        }
    }

    fn right_press(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        if let Hit::Cell(cell) = self.hit(event.position)
            && !self.selection.contains(cell)
        {
            self.select(Selection::at(cell), cx);
        }
        self.menus += 1;
        self.menu = Some((event.position, self.menus));
        cx.notify();
    }

    /// Scrolls just enough to show `cell` whole.
    fn reveal(&mut self, cell: Cell, cx: &mut Context<Self>) {
        let (width, height) = self.cells_area();
        if width <= 0. || height <= 0. {
            return;
        }
        let (mut x, mut y) = self.scrolled();
        let (left, right) = (self.lefts[cell.column], self.lefts[cell.column + 1]);
        if right > x + width {
            x = right - width;
        }
        if left < x {
            x = left;
        }
        let (top, bottom) = (
            cell.row as f32 * ROW_HEIGHT,
            (cell.row + 1) as f32 * ROW_HEIGHT,
        );
        if bottom > y + height {
            y = bottom - height;
        }
        if top < y {
            y = top;
        }
        if (x, y) != self.scrolled() {
            self.scroll.set_offset(point(px(-x), px(-y)));
            cx.notify();
        }
    }

    fn key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let command = keystroke.modifiers.secondary(cx);
        if command && keystroke.key == "c" {
            self.copy(cx);
        } else if command && keystroke.key == "a" {
            self.select_all(cx);
        } else {
            let page = (self.cells_area().1 / ROW_HEIGHT).floor() as usize;
            let Some(selection) = moved(
                self.selection,
                &keystroke.key,
                keystroke.modifiers.shift,
                command,
                self.size(),
                page,
            ) else {
                return;
            };
            self.select(selection, cx);
            self.reveal(selection.focus, cx);
        }
        cx.stop_propagation();
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let (grid, selection) = (self.grid.clone(), self.selection);
        let cells = selection
            .rows()
            .len()
            .saturating_mul(selection.columns().len());
        if cells > MAX_COPIED_CELLS {
            toast::show(
                Toast {
                    icon: IconName::CircleAlert,
                    message: m::sheet_copy_too_large().into(),
                    action: None,
                },
                cx,
            );
            return;
        }
        let logicals = Logicals::shown();
        self.copying = Some(cx.spawn(async move |_, cx| {
            let text = cx
                .background_spawn(async move {
                    tab_separated(selection.rows().map(|row| {
                        let mut cells = grid.row(row);
                        selection
                            .columns()
                            .map(|column| {
                                cells
                                    .get_mut(column)
                                    .map(|value| logicals.text(std::mem::take(value)))
                                    .unwrap_or_default()
                            })
                            .collect()
                    }))
                })
                .await;
            cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(text)));
        }));
    }

    fn close_menu(&mut self, serial: usize, cx: &mut Context<Self>) {
        if self.menu.is_some_and(|(_, open)| open == serial) {
            self.menu = None;
            cx.notify();
        }
    }

    fn menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (position, serial) = self.menu?;
        let this = cx.weak_entity();
        let closing = this.clone();
        let run: RunEdit = Rc::new(move |action, _, cx| {
            this.update(cx, |this, cx| match action {
                EditAction::Copy => this.copy(cx),
                EditAction::SelectAll => this.select_all(cx),
                EditAction::Cut | EditAction::Paste => {}
            })
            .ok();
        });
        let enabled = EditMenu {
            cut: false,
            copy: true,
            paste: false,
            select_all: true,
        };
        Some(
            context_menu(
                SharedString::from(format!("sheet-menu-{serial}")),
                position,
                move |_, cx| {
                    closing
                        .update(cx, |this, cx| this.close_menu(serial, cx))
                        .ok();
                },
                move |menu, _, _| enabled_edit_items(menu, enabled, run),
            )
            .into_any_element(),
        )
    }
}

impl Render for SheetGrid {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let (rows, columns) = self.size();
        let (scrolled_x, scrolled_y) = self.scrolled();
        let (width, height) = self.cells_area();
        let shown_rows =
            row_at(0., scrolled_y).min(rows)..(row_at(height, scrolled_y) + 1).min(rows);
        let shown_columns = if columns == 0 || width <= 0. {
            0..0
        } else {
            self.column_at(scrolled_x)
                ..self.lefts[..columns].partition_point(|left| *left < scrolled_x + width)
        };
        let selection = self.selection;
        let (picked_rows, picked_columns) = (selection.rows(), selection.columns());
        let picked = colors.link.opacity(0.08);
        let picked_header = colors.link.opacity(0.12);

        let mut cells = Vec::new();
        for row in shown_rows.clone() {
            let shown = self.row(row);
            let top = row_top(row, scrolled_y);
            for column in shown_columns.clone() {
                let cell = Cell { row, column };
                let (text, alignment) = shown
                    .get(column)
                    .map_or((SharedString::default(), Align::Left), |shown| {
                        (shown.text.clone(), shown.align)
                    });
                let left = self.lefts[column] - scrolled_x;
                let wide = self.lefts[column + 1] - self.lefts[column];
                cells.push(
                    div()
                        .debug_selector(move || format!("sheet-cell-{row}-{column}"))
                        .absolute()
                        .left(px(left))
                        .top(px(top))
                        .w(px(wide))
                        .h(px(ROW_HEIGHT))
                        .flex()
                        .items_center()
                        .px(px(6.))
                        .overflow_hidden()
                        .border_r_1()
                        .border_b_1()
                        .border_color(colors.border)
                        .when(
                            selection.contains(cell) && cell != selection.focus,
                            |cell| cell.bg(picked),
                        )
                        .when(!text.is_empty(), |cell| {
                            cell.child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .map(|text| match alignment {
                                        Align::Left => text,
                                        Align::Center => text.text_center(),
                                        Align::Right => text.text_right(),
                                    })
                                    .child(text),
                            )
                        }),
                );
            }
        }
        let outline = (rows > 0 && columns > 0).then(|| {
            let (left, right) = (
                self.lefts[picked_columns.start],
                self.lefts[picked_columns.end],
            );
            div()
                .absolute()
                .left(px(left - scrolled_x - 1.))
                .top(px(row_top(picked_rows.start, scrolled_y) - 1.))
                .w(px(right - left + 1.))
                .h(px(picked_rows.len() as f32 * ROW_HEIGHT + 1.))
                .border_2()
                .border_color(colors.link)
        });
        let letters = shown_columns.clone().map(|column| {
            let picked = picked_columns.contains(&column);
            div()
                .absolute()
                .left(px(self.lefts[column] - scrolled_x))
                .top_0()
                .w(px(self.lefts[column + 1] - self.lefts[column]))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .border_r_1()
                .border_color(colors.border)
                .when(picked, |letter| {
                    letter.bg(picked_header).text_color(colors.foreground)
                })
                .child(super::file_surface::column_letters(column))
        });
        let numbers = shown_rows.clone().map(|row| {
            let picked = picked_rows.contains(&row);
            div()
                .absolute()
                .left_0()
                .top(px(row_top(row, scrolled_y)))
                .w_full()
                .h(px(ROW_HEIGHT))
                .flex()
                .items_center()
                .justify_end()
                .pr(px(6.))
                .border_b_1()
                .border_color(colors.border)
                .when(picked, |number| {
                    number.bg(picked_header).text_color(colors.foreground)
                })
                .child((row + 1).to_string())
        });

        let viewport = self.viewport;
        let this = cx.weak_entity();
        let measure = canvas(
            move |bounds, _, cx| {
                if viewport != bounds.size {
                    cx.defer(move |cx| {
                        this.update(cx, |this, cx| {
                            this.viewport = bounds.size;
                            cx.notify();
                        })
                        .ok();
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let dragging = self.drag.is_some();
        let this = cx.weak_entity();
        let drag_listener = canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                if !dragging {
                    return;
                }
                let (moving, releasing) = (this.clone(), this.clone());
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase.bubble() {
                        moving
                            .update(cx, |this, cx| {
                                if event.pressed_button == Some(MouseButton::Left) {
                                    this.drag_to(event.position, cx)
                                } else {
                                    this.end_drag(cx)
                                }
                            })
                            .ok();
                    }
                });
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase.bubble() {
                        releasing.update(cx, |this, cx| this.end_drag(cx)).ok();
                    }
                });
            },
        )
        .absolute()
        .size_0();
        let gutter = px(self.gutter);
        let menu = self.menu(cx);

        div()
            .id("sheet-grid")
            .debug_selector(|| "sheet-grid".into())
            .track_focus(&self.focus)
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .text_xs()
            .text_color(colors.foreground)
            .on_key_down(cx.listener(|this, event, _, cx| this.key(event, cx)))
            .child(
                div()
                    .id("sheet-scroll")
                    .absolute()
                    .size_full()
                    .overflow_scroll()
                    .track_scroll(&self.scroll)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event, window, cx| this.press(event, window, cx)),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, event, window, cx| this.right_press(event, window, cx)),
                    )
                    .child(
                        div()
                            .w(px(self.gutter + self.content_width()))
                            .h(px(HEADER_HEIGHT + rows as f32 * ROW_HEIGHT)),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left(gutter)
                    .top(px(HEADER_HEIGHT))
                    .right_0()
                    .bottom_0()
                    .overflow_hidden()
                    .children(cells)
                    .children(outline),
            )
            .child(
                div()
                    .absolute()
                    .left(gutter)
                    .top_0()
                    .right_0()
                    .h(px(HEADER_HEIGHT))
                    .overflow_hidden()
                    .bg(colors.muted)
                    .border_b_1()
                    .border_color(colors.border)
                    .text_color(colors.muted_foreground)
                    .children(letters),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(HEADER_HEIGHT))
                    .w(gutter)
                    .bottom_0()
                    .overflow_hidden()
                    .bg(colors.muted)
                    .border_r_1()
                    .border_color(colors.border)
                    .text_color(colors.muted_foreground)
                    .children(numbers),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .w(gutter)
                    .h(px(HEADER_HEIGHT))
                    .bg(colors.muted)
                    .border_r_1()
                    .border_b_1()
                    .border_color(colors.border),
            )
            .child(measure)
            .child(drag_listener)
            .scrollbar(&self.scroll, ScrollbarAxis::Both)
            .children(menu)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Align, Cell, MAX_SHOWN_CHARS, Selection, align, moved, one_line, row_at, row_top,
        tab_separated, within,
    };
    use hoshi_engine::table::Kind;

    fn cell(row: usize, column: usize) -> Cell {
        Cell { row, column }
    }

    #[test]
    fn keys_move_the_pick_and_shift_stretches_it() {
        let size = (100, 5);
        let at = Selection::at(cell(3, 2));
        let step = |key, extend, jump| moved(at, key, extend, jump, size, 20).unwrap();

        assert_eq!(step("down", false, false), Selection::at(cell(4, 2)));
        assert_eq!(step("left", false, true), Selection::at(cell(3, 0)));
        assert_eq!(
            step("right", true, false),
            Selection {
                anchor: cell(3, 2),
                focus: cell(3, 3)
            }
        );
        assert_eq!(step("pagedown", false, false), Selection::at(cell(23, 2)));
        assert_eq!(step("end", false, true), Selection::at(cell(99, 4)));
        assert_eq!(step("home", false, true), Selection::at(cell(0, 0)));
        assert_eq!(moved(at, "x", false, false, size, 20), None);
    }

    #[test]
    fn a_pick_stays_inside_the_sheet() {
        let size = (3, 2);
        let corner = Selection::at(cell(2, 1));

        assert_eq!(moved(corner, "down", false, false, size, 10), Some(corner));
        assert_eq!(moved(corner, "right", false, false, size, 10), Some(corner));
        assert_eq!(
            moved(Selection::at(cell(0, 0)), "up", false, false, size, 10),
            Some(Selection::at(cell(0, 0)))
        );
        assert_eq!(
            moved(corner, "pagedown", false, false, size, 10),
            Some(corner)
        );
    }

    #[test]
    fn a_range_reads_the_same_from_either_corner() {
        let range = Selection {
            anchor: cell(5, 3),
            focus: cell(2, 1),
        };

        assert_eq!(range.rows(), 2..6);
        assert_eq!(range.columns(), 1..4);
        assert!(range.contains(cell(2, 3)));
        assert!(!range.contains(cell(6, 1)));
    }

    #[test]
    fn copied_cells_paste_into_a_spreadsheet_as_they_were() {
        let text = tab_separated([
            vec!["Name".to_owned(), "Note".to_owned()],
            vec!["Test User".to_owned(), "said \"hi\"\tthen left".to_owned()],
            vec![String::new(), "two\nlines".to_owned()],
        ]);

        assert_eq!(
            text,
            "Name\tNote\nTest User\t\"said \"\"hi\"\"\tthen left\"\n\t\"two\nlines\""
        );
    }

    #[test]
    fn a_wheel_past_the_ends_leaves_the_cells_at_them() {
        assert_eq!(within(-120., 2000., 500.), 0.);
        assert_eq!(within(1800., 2000., 500.), 1500.);
        assert_eq!(within(40., 300., 500.), 0.);
        assert_eq!(within(700., 2000., 500.), 700.);
    }

    #[test]
    fn values_line_up_where_a_spreadsheet_puts_them() {
        assert_eq!(align(Kind::Number), Align::Right);
        assert_eq!(align(Kind::Date), Align::Right);
        assert_eq!(align(Kind::Logical(false)), Align::Center);
        assert_eq!(align(Kind::Text), Align::Left);
    }

    #[test]
    fn a_cell_is_drawn_on_one_line_and_cut_when_long() {
        assert_eq!(
            one_line("said hi\nthen\r\nleft\rbye"),
            "said hi then left bye"
        );
        assert_eq!(one_line(&"é".repeat(5000)).chars().count(), MAX_SHOWN_CHARS);
    }

    #[test]
    fn rows_far_down_a_long_table_stay_a_row_apart() {
        let scrolled = 10_000_000. * 22.;
        let row = row_at(0., scrolled);
        assert_eq!(row, 10_000_000);
        assert_eq!(row_top(row + 1, scrolled) - row_top(row, scrolled), 22.);
        assert_eq!(row_at(30., 0.), 1);
    }
}
