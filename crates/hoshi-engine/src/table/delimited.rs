//! Comma, semicolon and tab separated text: indexed once by row, and by line when its source
//! is shown, each row split into cells only when it is shown.

use encoding_rs::{Encoding, UTF_8, WINDOWS_1252};
use std::sync::OnceLock;

pub struct Delimited {
    text: String,
    separator: u8,
    /// Where each row starts; a quoted cell may hold line breaks, so rows aren't lines.
    rows: Vec<usize>,
    /// Where the last row ends: blank lines after it are no rows.
    end: usize,
    lines: OnceLock<Vec<usize>>,
    columns: usize,
}

/// The text of a file's bytes: UTF-8, or UTF-16 with its byte order mark, or else
/// Windows-1252, as spreadsheets save CSV on Windows. UTF-8 keeps the bytes' memory.
pub fn decode(mut bytes: Vec<u8>) -> String {
    if let Some((encoding, bom)) = Encoding::for_bom(&bytes) {
        if encoding != UTF_8 {
            return encoding
                .decode_without_bom_handling(&bytes[bom..])
                .0
                .into_owned();
        }
        bytes.drain(..bom);
    }
    match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => WINDOWS_1252
            .decode_without_bom_handling(error.as_bytes())
            .0
            .into_owned(),
    }
}

/// The separator of a `.csv`: a semicolon when the first two records use it consistently and
/// commas don't, as files saved where the comma is the decimal mark do.
pub fn csv_separator(text: &str) -> u8 {
    let mut commas = [0usize; 2];
    let mut semicolons = [0usize; 2];
    let mut record = 0;
    let mut second = false;
    let mut quoted = false;
    let mut field_start = true;
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if quoted {
            if byte == b'"' {
                if bytes.get(index + 1) == Some(&b'"') {
                    index += 1;
                } else {
                    quoted = false;
                }
            }
            index += 1;
            continue;
        }
        match byte {
            b'"' if field_start => quoted = true,
            b'\n' | b'\r' => {
                if record == 1 {
                    break;
                }
                if byte == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
                    index += 1;
                }
                record = 1;
                field_start = true;
                index += 1;
                continue;
            }
            b',' => commas[record] += 1,
            b';' => semicolons[record] += 1,
            _ => {}
        }
        second |= record == 1;
        field_start = byte == b',' || byte == b';';
        index += 1;
    }
    if semicolons[0] <= commas[0] {
        return b',';
    }
    if !second {
        return b';';
    }
    let consistent_commas = commas[0] > 0 && commas[1] == commas[0];
    if semicolons[1] == semicolons[0] && !consistent_commas {
        b';'
    } else {
        b','
    }
}

impl Delimited {
    pub fn new(mut text: String, separator: u8) -> Self {
        if text.starts_with('\u{feff}') {
            text.drain(..'\u{feff}'.len_utf8());
        }
        let (mut rows, columns) = index_rows(text.as_bytes(), separator);
        let mut end = text.len();
        while let Some(&start) = rows.last()
            && without_break(&text[start..end]).is_empty()
        {
            rows.pop();
            end = start;
        }
        Self {
            text,
            separator,
            rows,
            end,
            lines: OnceLock::new(),
            columns,
        }
    }

    pub fn separator(&self) -> char {
        char::from(self.separator)
    }

    pub fn rows(&self) -> usize {
        self.rows.len()
    }

    pub fn columns(&self) -> usize {
        self.columns
    }

    pub fn row(&self, index: usize) -> Vec<String> {
        let Some(&start) = self.rows.get(index) else {
            return Vec::new();
        };
        let end = self.rows.get(index + 1).copied().unwrap_or(self.end);
        split(without_break(&self.text[start..end]), self.separator)
    }

    fn line_starts(&self) -> &[usize] {
        self.lines.get_or_init(|| index_lines(self.text.as_bytes()))
    }

    pub fn lines_indexed(&self) -> bool {
        self.lines.get().is_some()
    }

    pub fn lines(&self) -> usize {
        self.line_starts().len()
    }

    pub fn line(&self, index: usize) -> &str {
        let lines = self.line_starts();
        let Some(&start) = lines.get(index) else {
            return "";
        };
        let end = lines.get(index + 1).copied().unwrap_or(self.text.len());
        without_break(&self.text[start..end])
    }
}

fn without_break(row: &str) -> &str {
    row.strip_suffix("\r\n")
        .or_else(|| row.strip_suffix('\n'))
        .or_else(|| row.strip_suffix('\r'))
        .unwrap_or(row)
}

/// Where each row starts, and how many cells the widest row has.
fn index_rows(bytes: &[u8], separator: u8) -> (Vec<usize>, usize) {
    if bytes.is_empty() {
        return (Vec::new(), 0);
    }
    let mut rows = vec![0];
    let mut quoted = false;
    let mut field_start = true;
    let mut cells = 1;
    let mut widest = 0;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if quoted {
            if byte == b'"' {
                if bytes.get(index + 1) == Some(&b'"') {
                    index += 1;
                } else {
                    quoted = false;
                }
            }
            index += 1;
            continue;
        }
        if byte == b'"' && field_start {
            quoted = true;
            field_start = false;
        } else if byte == separator {
            cells += 1;
            field_start = true;
        } else if byte == b'\n' || byte == b'\r' {
            if byte == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
                index += 1;
            }
            widest = widest.max(cells);
            cells = 1;
            field_start = true;
            if index + 1 < bytes.len() {
                rows.push(index + 1);
            }
        } else {
            field_start = false;
        }
        index += 1;
    }
    let last_open = rows.last().is_some_and(|start| *start < bytes.len())
        && !matches!(bytes.last(), Some(b'\n' | b'\r'));
    if last_open {
        widest = widest.max(cells);
    }
    (rows, widest)
}

fn index_lines(bytes: &[u8]) -> Vec<usize> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![0];
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\n' || byte == b'\r' {
            if byte == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
                index += 1;
            }
            if index + 1 < bytes.len() {
                lines.push(index + 1);
            }
        }
        index += 1;
    }
    lines
}

/// The cells of one row: a cell that starts with a quote runs to its closing quote, with `""`
/// for a quote inside it.
fn split(row: &str, separator: u8) -> Vec<String> {
    let separator = char::from(separator);
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut field_start = true;
    let mut chars = row.chars().peekable();
    while let Some(char) = chars.next() {
        if quoted {
            if char == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cell.push('"');
                } else {
                    quoted = false;
                }
            } else {
                cell.push(char);
            }
            continue;
        }
        if char == '"' && field_start {
            quoted = true;
            field_start = false;
        } else if char == separator {
            cells.push(std::mem::take(&mut cell));
            field_start = true;
        } else {
            cell.push(char);
            field_start = false;
        }
    }
    cells.push(cell);
    cells
}

#[cfg(test)]
mod tests {
    use super::{Delimited, csv_separator, decode};

    fn rows(table: &Delimited) -> Vec<Vec<String>> {
        (0..table.rows()).map(|index| table.row(index)).collect()
    }

    #[test]
    fn quoted_cells_keep_their_separators_quotes_and_line_breaks() {
        let table = Delimited::new(
            "name,notes\r\n\"Shop, Inc.\",\"said \"\"hi\"\"\nthen left\"\r\nplain,\r\n".into(),
            b',',
        );
        assert_eq!(
            rows(&table),
            [
                vec!["name", "notes"],
                vec!["Shop, Inc.", "said \"hi\"\nthen left"],
                vec!["plain", ""],
            ]
        );
        assert_eq!(table.columns(), 2);
        assert_eq!(table.lines(), 4);
        assert_eq!(table.line(2), "then left\"");
    }

    #[test]
    fn the_widest_row_sets_the_columns_and_a_last_row_may_lack_its_break() {
        let table = Delimited::new("a\tb\n1\t2\t3\n4".into(), b'\t');
        assert_eq!(table.columns(), 3);
        assert_eq!(table.rows(), 3);
        assert_eq!(table.row(2), ["4"]);
        assert!(table.row(3).is_empty());
    }

    #[test]
    fn old_mac_line_breaks_end_rows_too() {
        let table = Delimited::new("a,b\r1,2\r3,4".into(), b',');
        assert_eq!(table.rows(), 3);
        assert_eq!(table.row(1), ["1", "2"]);
        assert_eq!(table.lines(), 3);
        assert_eq!(table.line(2), "3,4");
    }

    #[test]
    fn blank_lines_at_the_end_are_not_rows_but_those_between_rows_are() {
        let table = Delimited::new("a,b\n\n1,2\n\n\r\n".into(), b',');
        assert_eq!(table.rows(), 3);
        assert_eq!(table.row(1), [""]);
        assert_eq!(table.row(2), ["1", "2"]);
    }

    #[test]
    fn a_quote_inside_an_unquoted_cell_is_text() {
        let table = Delimited::new("5\" screen,ok".into(), b',');
        assert_eq!(table.row(0), ["5\" screen", "ok"]);
    }

    #[test]
    fn an_empty_file_has_no_rows() {
        let table = Delimited::new(String::new(), b',');
        assert_eq!((table.rows(), table.columns(), table.lines()), (0, 0, 0));
    }

    #[test]
    fn the_byte_order_mark_is_not_part_of_the_first_cell() {
        let table = Delimited::new("\u{feff}id,name".into(), b',');
        assert_eq!(table.row(0), ["id", "name"]);
    }

    #[test]
    fn semicolons_win_only_when_they_are_consistent_and_commas_are_not() {
        assert_eq!(csv_separator("date;total\n2026-01-04;1,5\n"), b';');
        assert_eq!(csv_separator("a;b;c\n"), b';');
        assert_eq!(csv_separator("a,b\n1,2\n"), b',');
        assert_eq!(csv_separator("\"x;y\",z\n1,2\n"), b',');
        assert_eq!(csv_separator("a,b;c\nd,e;f\n"), b',');
        assert_eq!(csv_separator(""), b',');
    }

    #[test]
    fn text_is_read_as_utf8_utf16_or_windows_1252() {
        assert_eq!(decode("región".into()), "región");
        assert_eq!(decode(b"regi\xf3n".to_vec()), "región");
        assert_eq!(decode(b"\xff\xfeo\0k\0".to_vec()), "ok");
        assert_eq!(decode(b"\xef\xbb\xbfid".to_vec()), "id");
    }
}
