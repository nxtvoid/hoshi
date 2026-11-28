//! Tables read from files for the panel to show: CSV and TSV text, and Excel and OpenDocument
//! workbooks one sheet at a time. A table is read whole off the UI thread; a row becomes values
//! only when it is shown.

mod delimited;
mod workbook;

use calamine::{Data, Range};
use delimited::Delimited;
use std::io::Read;
use std::path::Path;

/// Most of a text file read; the rest is left out.
pub const MAX_TEXT_BYTES: u64 = 256 * 1024 * 1024;
/// The largest workbook file opened.
pub const MAX_WORKBOOK_BYTES: u64 = 128 * 1024 * 1024;
/// The most cells a sheet spans from A1; a sheet past it is refused rather than held.
const MAX_CELLS: u64 = 20_000_000;
/// Rows looked at to size columns.
const SAMPLE_ROWS: usize = 500;
/// Characters a column grows to at most.
const MAX_WIDTH: usize = 48;
/// Characters given to a true or false cell, whose word the app picks.
const LOGICAL_WIDTH: usize = 9;

const DELIMITED: &[&str] = &["csv", "tsv"];
const WORKBOOKS: &[&str] = &["xlsx", "xlsm", "xlsb", "xls", "ods"];

fn extension(path: &str) -> Option<String> {
    let name = path.rsplit(['/', '\\']).next()?;
    let (_, extension) = name.rsplit_once('.')?;
    Some(extension.to_ascii_lowercase())
}

/// Whether the file at `path` is one a table shows.
pub fn is_table(path: &str) -> bool {
    extension(path).is_some_and(|extension| {
        DELIMITED.contains(&extension.as_str()) || WORKBOOKS.contains(&extension.as_str())
    })
}

/// Why a table can't be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableFailure {
    /// A workbook larger than [`MAX_WORKBOOK_BYTES`], or a sheet with too many cells to hold.
    TooLarge,
    /// The file couldn't be read, for the reason given.
    Read(String),
}

/// What a table file is, as its view tells it apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Format {
    /// Text with `separator` between cells; `truncated` when it was larger than
    /// [`MAX_TEXT_BYTES`] and only its start was read.
    Delimited { separator: char, truncated: bool },
    /// A workbook, shown one of its `sheets` at a time.
    Workbook { sheets: Vec<String> },
}

/// What a cell holds, as a spreadsheet tells it apart to align it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Text,
    Number,
    Date,
    /// True or false; its text is empty, as each language names them its own way.
    Logical(bool),
}

/// A cell: its text and what kind of value it is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Value {
    pub text: String,
    pub kind: Kind,
}

pub struct TableFile {
    pub format: Format,
    pub grid: Grid,
}

enum Cells {
    Text(Delimited),
    Sheet(Range<Data>),
}

/// The cells of a table or sheet.
pub struct Grid {
    cells: Cells,
    rows: usize,
    columns: usize,
    widths: Vec<usize>,
}

/// Reads the table at `path`; a workbook's first visible sheet.
pub fn open(path: &Path) -> Result<TableFile, TableFailure> {
    let extension = extension(&path.to_string_lossy()).unwrap_or_default();
    if WORKBOOKS.contains(&extension.as_str()) {
        let mut book = open_workbook(path)?;
        let sheets = workbook::sheets(&book);
        let range = match sheets.first() {
            Some(first) => workbook::sheet(&mut book, first)?,
            None => Range::default(),
        };
        return Ok(TableFile {
            format: Format::Workbook { sheets },
            grid: Grid::new(Cells::Sheet(range)),
        });
    }
    let (text, truncated) = read_text(path, MAX_TEXT_BYTES)?;
    let separator = if extension == "tsv" {
        b'\t'
    } else {
        delimited::csv_separator(&text)
    };
    let table = Delimited::new(text, separator);
    Ok(TableFile {
        format: Format::Delimited {
            separator: table.separator(),
            truncated,
        },
        grid: Grid::new(Cells::Text(table)),
    })
}

/// Reads one sheet of the workbook at `path`.
pub fn open_sheet(path: &Path, name: &str) -> Result<Grid, TableFailure> {
    let mut book = open_workbook(path)?;
    workbook::sheet(&mut book, name).map(|range| Grid::new(Cells::Sheet(range)))
}

fn open_workbook(path: &Path) -> Result<workbook::Book, TableFailure> {
    let size = std::fs::metadata(path)
        .map_err(|error| TableFailure::Read(error.to_string()))?
        .len();
    if size > MAX_WORKBOOK_BYTES {
        return Err(TableFailure::TooLarge);
    }
    workbook::open(path)
}

/// The file's text, up to its last whole line within `max` bytes when it is larger.
fn read_text(path: &Path, max: u64) -> Result<(String, bool), TableFailure> {
    let unreadable = |error: std::io::Error| TableFailure::Read(error.to_string());
    let file = std::fs::File::open(path).map_err(unreadable)?;
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(unreadable)?;
    let truncated = bytes.len() as u64 > max;
    if truncated {
        bytes.truncate(max as usize);
        let end = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |at| at + 1);
        bytes.truncate(end);
    }
    Ok((delimited::decode(bytes), truncated))
}

/// Whether a cell's text reads as a number: digits with grouping or decimal marks, a sign,
/// a currency or a percent, or a number as code writes it.
fn looks_numeric(text: &str) -> bool {
    let text = text.trim();
    if text.parse::<f64>().is_ok_and(f64::is_finite) {
        return true;
    }
    let text = text.strip_prefix(['-', '+']).unwrap_or(text);
    let text = text.trim_start_matches(['$', '€', '£']).trim_start();
    let text = text.strip_suffix('%').unwrap_or(text).trim_end();
    let text = text.trim_end_matches(['$', '€', '£']).trim_end();
    text.bytes().any(|byte| byte.is_ascii_digit())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.' || byte == b',')
}

/// Whether a cell's text starts with an ISO date, as `2026-01-04` or `2026-01-04 10:30`.
fn looks_like_date(text: &str) -> bool {
    let text = text.trim();
    let date = text.get(..10).is_some_and(|date| {
        date.char_indices().all(|(index, char)| match index {
            4 | 7 => char == '-',
            _ => char.is_ascii_digit(),
        })
    });
    date && text[10..]
        .chars()
        .next()
        .is_none_or(|next| next == ' ' || next == 'T')
}

/// What a cell of a text table holds, read from its text.
fn text_kind(text: &str) -> Kind {
    if looks_like_date(text) {
        Kind::Date
    } else if looks_numeric(text) {
        Kind::Number
    } else {
        Kind::Text
    }
}

impl Grid {
    fn new(cells: Cells) -> Self {
        let (rows, columns) = match &cells {
            Cells::Text(table) => (table.rows(), table.columns()),
            Cells::Sheet(range) => workbook::size(range),
        };
        let mut grid = Self {
            cells,
            rows,
            columns,
            widths: Vec::new(),
        };
        grid.measure();
        grid
    }

    /// Sizes each column to its widest sampled cell.
    fn measure(&mut self) {
        let mut widths = vec![1; self.columns];
        for row in 0..self.rows.min(SAMPLE_ROWS) {
            for (column, value) in self.row(row).iter().enumerate().take(self.columns) {
                let width = match value.kind {
                    Kind::Logical(_) => LOGICAL_WIDTH,
                    _ => value.text.chars().count(),
                };
                widths[column] = widths[column].max(width.min(MAX_WIDTH));
            }
        }
        self.widths = widths;
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn columns(&self) -> usize {
        self.columns
    }

    /// The cells of row `index`; a row of a text table may have fewer than
    /// [`Self::columns`].
    pub fn row(&self, index: usize) -> Vec<Value> {
        match &self.cells {
            Cells::Text(table) => table
                .row(index)
                .into_iter()
                .map(|text| Value {
                    kind: text_kind(&text),
                    text,
                })
                .collect(),
            Cells::Sheet(range) => (0..self.columns)
                .map(|column| workbook::value(range, index, column))
                .collect(),
        }
    }

    /// How many characters wide the column's text runs, up to a limit.
    pub fn width(&self, column: usize) -> usize {
        self.widths.get(column).copied().unwrap_or(1)
    }

    /// Whether [`Self::lines`] answers without reading the whole text first.
    pub fn lines_indexed(&self) -> bool {
        match &self.cells {
            Cells::Text(table) => table.lines_indexed(),
            Cells::Sheet(_) => true,
        }
    }

    /// The lines of a text table's source, indexed the first time they are asked for; none
    /// for a sheet.
    pub fn lines(&self) -> usize {
        match &self.cells {
            Cells::Text(table) => table.lines(),
            Cells::Sheet(_) => 0,
        }
    }

    pub fn line(&self, index: usize) -> &str {
        match &self.cells {
            Cells::Text(table) => table.line(index),
            Cells::Sheet(_) => "",
        }
    }
}

#[cfg(test)]
mod tests;
