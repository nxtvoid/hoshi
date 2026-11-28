//! Excel and OpenDocument workbooks: the names of the sheets a reader sees, and the values of
//! one sheet's cells.

use super::{Kind, MAX_CELLS, TableFailure, Value};
use calamine::{
    Cell, Data, DataRef, Range, Reader, SheetType, SheetVisible, Sheets, open_workbook_auto,
};
use std::fmt::Display;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

pub type Book = Sheets<BufReader<File>>;

pub fn open(path: &Path) -> Result<Book, TableFailure> {
    open_workbook_auto(path).map_err(unreadable)
}

fn unreadable(error: impl Display) -> TableFailure {
    TableFailure::Read(error.to_string())
}

/// The visible worksheets of `book`, in its order; charts and macros are left out.
pub fn sheets(book: &Book) -> Vec<String> {
    book.sheets_metadata()
        .iter()
        .filter(|sheet| sheet.visible == SheetVisible::Visible && sheet.typ == SheetType::WorkSheet)
        .map(|sheet| sheet.name.clone())
        .collect()
}

/// The cells of sheet `name`; refused, before they are all held, once they span more than
/// [`MAX_CELLS`] from A1.
pub fn sheet(book: &mut Book, name: &str) -> Result<Range<Data>, TableFailure> {
    let range = match book {
        Sheets::Xlsx(xlsx) => {
            let mut cells = xlsx.worksheet_cells_reader(name).map_err(unreadable)?;
            sparse(|| cells.next_cell())?
        }
        Sheets::Xlsb(xlsb) => {
            let mut cells = xlsb.worksheet_cells_reader(name).map_err(unreadable)?;
            sparse(|| cells.next_cell())?
        }
        book => book.worksheet_range(name).map_err(unreadable)?,
    };
    let (rows, columns) = size(&range);
    if too_large(rows as u64, columns as u64) {
        return Err(TableFailure::TooLarge);
    }
    Ok(range)
}

fn too_large(rows: u64, columns: u64) -> bool {
    rows.saturating_mul(columns) > MAX_CELLS
}

/// The cells `next` reads one by one, held only while they fit.
fn sparse<'a, E: Display>(
    mut next: impl FnMut() -> Result<Option<Cell<DataRef<'a>>>, E>,
) -> Result<Range<Data>, TableFailure> {
    let mut cells = Vec::new();
    let (mut rows, mut columns) = (0u64, 0u64);
    while let Some(cell) = next().map_err(unreadable)? {
        if *cell.get_value() == DataRef::Empty {
            continue;
        }
        let (row, column) = cell.get_position();
        rows = rows.max(u64::from(row) + 1);
        columns = columns.max(u64::from(column) + 1);
        if too_large(rows, columns) {
            return Err(TableFailure::TooLarge);
        }
        cells.push(Cell::new(
            (row, column),
            Data::from(cell.get_value().clone()),
        ));
    }
    Ok(Range::from_sparse(cells))
}

/// The rows and columns from A1 to the last cell with a value, so they keep their numbers
/// and letters.
pub fn size(range: &Range<Data>) -> (usize, usize) {
    range.end().map_or((0, 0), |(row, column)| {
        (row as usize + 1, column as usize + 1)
    })
}

pub fn value(range: &Range<Data>, row: usize, column: usize) -> Value {
    let data = u32::try_from(row)
        .ok()
        .zip(u32::try_from(column).ok())
        .and_then(|position| range.get_value(position));
    data.map_or_else(Value::default, |data| Value {
        text: text(data),
        kind: kind(data),
    })
}

fn kind(data: &Data) -> Kind {
    match data {
        Data::Int(_) | Data::Float(_) | Data::DurationIso(_) => Kind::Number,
        Data::DateTime(date) if date.is_duration() => Kind::Number,
        Data::DateTime(_) | Data::DateTimeIso(_) => Kind::Date,
        Data::Bool(value) => Kind::Logical(*value),
        Data::String(_) | Data::Error(_) | Data::Empty => Kind::Text,
    }
}

/// A cell as Excel's General format shows it: whole numbers without decimals, others without
/// the float's trailing noise, dates as ISO dates. True and false have no text: the app names
/// them.
fn text(data: &Data) -> String {
    match data {
        Data::Empty | Data::Bool(_) => String::new(),
        Data::String(text) | Data::DateTimeIso(text) | Data::DurationIso(text) => text.clone(),
        Data::Int(number) => number.to_string(),
        Data::Float(number) => float_text(*number),
        Data::Error(error) => error.to_string(),
        Data::DateTime(date) => {
            if date.is_duration() {
                return date.as_duration().map_or_else(
                    || float_text(date.as_f64()),
                    |duration| {
                        let seconds = duration.num_seconds();
                        format!(
                            "{}:{:02}:{:02}",
                            seconds / 3600,
                            seconds % 3600 / 60,
                            seconds % 60
                        )
                    },
                );
            }
            match date.as_datetime() {
                Some(moment) if moment.time() == chrono::NaiveTime::MIN => {
                    moment.format("%Y-%m-%d").to_string()
                }
                Some(moment) => moment.format("%Y-%m-%d %H:%M:%S").to_string(),
                None => float_text(date.as_f64()),
            }
        }
    }
}

fn float_text(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        return format!("{number:.0}");
    }
    let text = format!("{number:.10}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::{float_text, kind, text, too_large};
    use crate::table::Kind;
    use calamine::Data;

    #[test]
    fn numbers_read_as_a_spreadsheet_shows_them() {
        assert_eq!(float_text(12.0), "12");
        assert_eq!(float_text(0.1 + 0.2), "0.3");
        assert_eq!(float_text(-1.25), "-1.25");
        assert_eq!(text(&Data::Int(7)), "7");
        assert_eq!(text(&Data::Empty), "");
    }

    #[test]
    fn a_cell_keeps_what_kind_of_value_it_holds() {
        assert_eq!(kind(&Data::Float(1.5)), Kind::Number);
        assert_eq!(kind(&Data::Bool(false)), Kind::Logical(false));
        assert_eq!(text(&Data::Bool(true)), "");
        assert_eq!(kind(&Data::DateTimeIso("2026-01-04".into())), Kind::Date);
        assert_eq!(kind(&Data::String("12".into())), Kind::Text);
    }

    #[test]
    fn a_sheet_reaching_the_last_cell_is_too_large() {
        assert!(too_large(1_048_576, 16_384));
        assert!(!too_large(100_000, 50));
    }
}
