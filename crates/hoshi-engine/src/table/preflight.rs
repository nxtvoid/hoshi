//! How many cells an XLS or ODS workbook spans, measured before it is read: their readers load
//! every sheet as the workbook opens, so a sheet too large to hold must be caught first.

use quick_xml::Reader;
use quick_xml::errors::IllFormedError;
use quick_xml::events::{BytesStart, Event};
use std::cell::Cell;
use std::io::{BufReader, Read};
use std::path::Path;
use std::rc::Rc;

/// The cells the sheets of the XLS or ODS at `path` span together, each from its first used
/// cell to its last; `None` when it can't be measured, so its reader still tries, and
/// `u64::MAX` when a part of it is too large to read.
pub fn cells(path: &Path, extension: &str) -> Option<u64> {
    match extension {
        "xls" => xls_cells(&std::fs::read(path).ok()?),
        "ods" => ods_cells(path),
        _ => None,
    }
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// The sectors a compound file chains from `start` in `table`, stopping at its end mark or
/// at the first sector it reaches again.
fn chain(table: &[u32], start: u32) -> Vec<u32> {
    let mut chained = Vec::new();
    let mut seen = vec![false; table.len()];
    let mut at = start as usize;
    while at < table.len() && !seen[at] {
        seen[at] = true;
        chained.push(at as u32);
        at = table[at] as usize;
    }
    chained
}

fn words(bytes: &[u8]) -> impl Iterator<Item = u32> + '_ {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| u32::from_le_bytes(*word))
}

/// The `Workbook` stream of an XLS: an OLE compound file, read as `[MS-CFB]` lays it out.
fn workbook_stream(file: &[u8]) -> Option<Vec<u8>> {
    const SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    if file.get(..8)? != SIGNATURE {
        return None;
    }
    let size = match u16_at(file, 0x1E)? {
        9 => 512,
        12 => 4096,
        _ => return None,
    };
    let mini_size = 1usize.checked_shl(u32::from(u16_at(file, 0x20)?))?;
    let cutoff = u32_at(file, 0x38)? as usize;
    let sector = |id: u32| {
        let start = (id as usize).checked_add(1)?.checked_mul(size)?;
        file.get(start..start.checked_add(size)?)
    };
    let sectors = (file.len() / size).saturating_sub(1);
    let mut taken = vec![false; sectors];
    let mut fat_sectors = Vec::new();
    let mut take = |ids: &[u8]| {
        for id in words(ids) {
            if (id as usize) < sectors && !taken[id as usize] {
                taken[id as usize] = true;
                fat_sectors.push(id);
            }
        }
    };
    take(file.get(0x4C..0x4C + 109 * 4)?);
    let mut difat = u32_at(file, 0x44)?;
    let mut read_difat = vec![false; sectors];
    while (difat as usize) < sectors && !read_difat[difat as usize] {
        read_difat[difat as usize] = true;
        let data = sector(difat)?;
        let entries = size / 4 - 1;
        take(&data[..entries * 4]);
        difat = u32_at(data, entries * 4)?;
    }
    let fat: Vec<u32> = fat_sectors
        .iter()
        .filter_map(|id| sector(*id))
        .flat_map(words)
        .collect();
    let read = |table: &[u32], start: u32| -> Vec<u8> {
        chain(table, start)
            .into_iter()
            .filter_map(sector)
            .flatten()
            .copied()
            .collect()
    };
    let directory = read(&fat, u32_at(file, 0x30)?);
    let entries: Vec<&[u8]> = directory
        .as_chunks::<128>()
        .0
        .iter()
        .map(|entry| entry.as_slice())
        .collect();
    let root = entries.first()?;
    let stream = entries.iter().find(|entry| {
        let length = u16_at(entry, 64).unwrap_or(0) as usize;
        let name: Vec<u16> = entry[..length.min(64)]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|unit| u16::from_le_bytes(*unit))
            .take_while(|unit| *unit != 0)
            .collect();
        entry[66] == 2
            && matches!(
                String::from_utf16_lossy(&name).as_str(),
                "Workbook" | "Book"
            )
    })?;
    let (start, length) = (u32_at(stream, 116)?, u32_at(stream, 120)? as usize);
    let mut bytes = if length < cutoff {
        let mini_fat: Vec<u32> = words(&read(&fat, u32_at(file, 0x3C)?)).collect();
        let mini_stream = read(&fat, u32_at(root, 116)?);
        chain(&mini_fat, start)
            .into_iter()
            .filter_map(|id| {
                let at = (id as usize).checked_mul(mini_size)?;
                mini_stream.get(at..at.checked_add(mini_size)?)
            })
            .flatten()
            .copied()
            .collect()
    } else {
        read(&fat, start)
    };
    bytes.truncate(length);
    Some(bytes)
}

const BOF: u16 = 0x0809;
const EOF: u16 = 0x000A;

/// One record of a BIFF stream at `at`: its kind, its data, and where the next one starts.
fn record(stream: &[u8], at: usize) -> Option<(u16, &[u8], usize)> {
    let kind = u16_at(stream, at)?;
    let length = u16_at(stream, at + 2)? as usize;
    let data = stream.get(at + 4..at + 4 + length)?;
    Some((kind, data, at + 4 + length))
}

/// The cells an XLS's sheets span: each sheet as many as its reader sets aside for it, the
/// larger of what its dimensions record declares and what its cells reach.
fn xls_cells(file: &[u8]) -> Option<u64> {
    const BOUND_SHEET: u16 = 0x0085;
    let stream = workbook_stream(file)?;
    let mut sheets = Vec::new();
    let mut at = 0;
    loop {
        let (kind, data, next) = record(&stream, at)?;
        match kind {
            BOUND_SHEET => sheets.push(u32_at(data, 0)? as usize),
            EOF => break,
            _ => {}
        }
        at = next;
    }
    Some(sheets.into_iter().fold(0u64, |cells, start| {
        cells.saturating_add(sheet_cells(&stream, start))
    }))
}

/// Takes the cells of `row` from column `first` to `last` into the box `reached` spans, and
/// counts them.
fn reach(reached: &mut Option<[u16; 4]>, row: u16, first: u16, last: u16) -> u64 {
    let [top, bottom, left, right] = reached.get_or_insert([row, row, first, last]);
    (*top, *bottom) = ((*top).min(row), (*bottom).max(row));
    (*left, *right) = ((*left).min(first), (*right).max(last));
    u64::from(last - first) + 1
}

/// The cells of the sheet whose records start at `start`, skipping the charts and other
/// streams nested in it, as its reader does: the most of the box its cells span, the cells
/// read, and what a dimensions record has it set aside past those read before it.
fn sheet_cells(stream: &[u8], start: usize) -> u64 {
    const DIMENSIONS: u16 = 0x0200;
    const MUL_RK: u16 = 0x00BD;
    const FORMULA: u16 = 0x0006;
    const STRING: u16 = 0x0207;
    const CELLS: [u16; 6] = [0x00D6, 0x00FD, 0x0203, 0x0204, 0x0205, 0x027E];
    let (mut read, mut reserved) = (0u64, 0u64);
    let mut formula = (0, 0);
    let mut reached = None;
    let (mut at, mut nested, mut opened) = (start, 0usize, false);
    while let Some((kind, data, next)) = record(stream, at) {
        at = next;
        if nested > 0 {
            match kind {
                BOF => nested += 1,
                EOF => nested -= 1,
                _ => {}
            }
            continue;
        }
        match kind {
            BOF if opened => nested += 1,
            BOF => opened = true,
            EOF => break,
            DIMENSIONS => {
                let declared = declared_cells(data).unwrap_or(0);
                reserved = reserved.max(read.saturating_add(declared));
            }
            FORMULA => {
                if let (Some(row), Some(column)) = (u16_at(data, 0), u16_at(data, 2)) {
                    formula = (row, column);
                    read += reach(&mut reached, row, column, column);
                }
            }
            STRING => read += reach(&mut reached, formula.0, formula.1, formula.1),
            MUL_RK if data.len() >= 6 => {
                if let (Some(row), Some(first), Some(last)) = (
                    u16_at(data, 0),
                    u16_at(data, 2),
                    u16_at(data, data.len() - 2),
                ) {
                    read += reach(&mut reached, row, first, last.max(first));
                }
            }
            kind if CELLS.contains(&kind) => {
                if let (Some(row), Some(column)) = (u16_at(data, 0), u16_at(data, 2)) {
                    read += reach(&mut reached, row, column, column);
                }
            }
            _ => {}
        }
    }
    let spanned = reached.map_or(0, |[top, bottom, left, right]| {
        (u64::from(bottom - top) + 1) * (u64::from(right - left) + 1)
    });
    spanned.max(read).max(reserved)
}

/// The cells a dimensions record declares, counted as its reader counts them: a first row
/// past the last wraps around to a count too large to hold.
fn declared_cells(data: &[u8]) -> Option<u64> {
    let (first_row, end_row, first_column, end_column) = match data.len() {
        10 => (
            u32::from(u16_at(data, 0)?),
            u32::from(u16_at(data, 2)?),
            u32::from(u16_at(data, 4)?),
            u32::from(u16_at(data, 6)?),
        ),
        14 | 16 => (
            u32_at(data, 0)?,
            u32_at(data, 4)?,
            u32::from(u16_at(data, 8)?),
            u32::from(u16_at(data, 10)?),
        ),
        _ => return None,
    };
    if end_row == 0 || end_column == 0 {
        return Some(1);
    }
    let first_column = if first_column > 0xFF || end_column < first_column {
        0
    } else {
        first_column
    };
    Some(
        u64::from(end_row.wrapping_sub(first_row))
            * u64::from(end_column.wrapping_sub(first_column)),
    )
}

/// The most `content.xml` may hold in one tag or text: far past any real cell's.
const MAX_EVENT: u64 = 16 * 1024 * 1024;
/// The most text an ODS's cells may hold, each cell's as many times as it is repeated.
const MAX_TEXT: u64 = 512 * 1024 * 1024;

/// `inner`, failing once more than [`MAX_EVENT`] bytes are read with `read` not set back to
/// zero, and then leaving `read` at `u64::MAX`.
struct Capped<R> {
    inner: R,
    read: Rc<Cell<u64>>,
}

impl<R: Read> Read for Capped<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.inner.read(buffer)?;
        let read = self.read.get().saturating_add(count as u64);
        if read > MAX_EVENT {
            self.read.set(u64::MAX);
            return Err(std::io::Error::other("an XML event is too large"));
        }
        self.read.set(read);
        Ok(count)
    }
}

/// A count attribute, its character references read as its reader reads them.
fn repeated(element: &BytesStart<'_>, name: &str) -> u64 {
    element
        .try_get_attribute(name)
        .ok()
        .flatten()
        .and_then(|attribute| {
            quick_xml::escape::unescape(&attribute.value)
                .ok()?
                .parse()
                .ok()
        })
        .unwrap_or(1)
}

/// The text a cell's attributes give it: its value, as text, and its formula.
fn attributes_text(element: &BytesStart<'_>) -> u64 {
    element
        .attributes()
        .flatten()
        .filter(|attribute| {
            matches!(
                attribute.key.as_ref(),
                "office:string-value" | "office:date-value" | "office:time-value" | "table:formula"
            )
        })
        .map(|attribute| attribute.value.len() as u64)
        .sum()
}

/// Whether the reader keeps the cell: with a value, a type or a formula. Text without a type
/// it leaves out.
fn has_value(element: &BytesStart<'_>) -> bool {
    element.attributes().flatten().any(|attribute| {
        matches!(
            attribute.key.as_ref(),
            "office:value-type"
                | "office:value"
                | "office:string-value"
                | "office:date-value"
                | "office:time-value"
                | "office:boolean-value"
                | "table:formula"
        )
    })
}

/// The cells an ODS's tables span from A1 to their last cell with something in it, counting
/// rows and cells repeated; `u64::MAX` when its cells' text, repeated, passes [`MAX_TEXT`], or
/// its XML can't be read through, as its reader may still read on past what stopped it.
fn ods_cells(path: &Path) -> Option<u64> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    let inner = archive.by_name("content.xml").ok()?;
    let read = Rc::new(Cell::new(0));
    let mut reader = Reader::from_reader(BufReader::new(Capped {
        inner,
        read: read.clone(),
    }));
    let config = reader.config_mut();
    config.check_end_names = false;
    config.check_comments = false;
    config.trim_text(false);
    let mut buffer = Vec::new();
    let mut cells = 0u64;
    let (mut row, mut rows, mut columns) = (0u64, 0u64, 0u64);
    let (mut column, mut repeats, mut filled) = (0u64, 1u64, false);
    let (mut text, mut held, mut cell) = (0u64, 0u64, None::<u64>);
    loop {
        let event = match reader.read_event_into(&mut buffer) {
            Ok(event) => event,
            Err(quick_xml::Error::IllFormed(IllFormedError::UnmatchedEndTag(_))) => {
                buffer.clear();
                continue;
            }
            Err(_) => return Some(u64::MAX),
        };
        read.set(0);
        let mut hold = |copies: u64, length: u64| {
            text = text.saturating_add(copies.saturating_mul(length));
            text > MAX_TEXT
        };
        match &event {
            Event::Start(element) | Event::Empty(element) => {
                let empty = matches!(event, Event::Empty(_));
                match element.name().as_ref() {
                    "table:table" => (row, rows, columns) = (0, 0, 0),
                    "text:s" => held = held.saturating_add(repeated(element, "text:c")),
                    "text:tab" | "text:line-break" => held = held.saturating_add(1),
                    "table:table-row" => {
                        let times = repeated(element, "table:number-rows-repeated");
                        if empty {
                            row = row.saturating_add(times);
                        } else {
                            (column, repeats, filled) = (0, times, false);
                        }
                    }
                    "table:table-cell" | "table:covered-table-cell" => {
                        let times = repeated(element, "table:number-columns-repeated");
                        if has_value(element) {
                            filled = true;
                            columns = columns.max(column.saturating_add(times));
                        }
                        column = column.saturating_add(times);
                        let copies = times.saturating_mul(repeats);
                        if has_value(element) && empty {
                            if hold(copies, attributes_text(element)) {
                                return Some(u64::MAX);
                            }
                        } else if has_value(element) {
                            (cell, held) = (Some(copies), attributes_text(element));
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(content) if cell.is_some() => {
                held = held.saturating_add(content.len() as u64);
            }
            Event::GeneralRef(_) if cell.is_some() => held = held.saturating_add(4),
            Event::End(element) => match element.name().as_ref() {
                "table:table-cell" | "table:covered-table-cell" => {
                    if let Some(copies) = cell.take()
                        && hold(copies, held)
                    {
                        return Some(u64::MAX);
                    }
                }
                "table:table-row" => {
                    row = row.saturating_add(repeats);
                    if filled {
                        rows = row;
                    }
                }
                "table:table" => cells = cells.saturating_add(rows.saturating_mul(columns)),
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Some(cells)
}

#[cfg(test)]
mod tests {
    use super::{chain, ods_cells, xls_cells};
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn record(kind: u16, data: &[u8]) -> Vec<u8> {
        let mut bytes = kind.to_le_bytes().to_vec();
        bytes.extend((data.len() as u16).to_le_bytes());
        bytes.extend(data);
        bytes
    }

    fn dimensions(rows: u32, columns: u16) -> Vec<u8> {
        let mut data = 0u32.to_le_bytes().to_vec();
        data.extend(rows.to_le_bytes());
        data.extend(0u16.to_le_bytes());
        data.extend(columns.to_le_bytes());
        data.extend(0u16.to_le_bytes());
        record(0x0200, &data)
    }

    fn number(row: u16, column: u16) -> Vec<u8> {
        let mut data = row.to_le_bytes().to_vec();
        data.extend(column.to_le_bytes());
        data.extend([0; 10]);
        record(0x0203, &data)
    }

    fn substream(records: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = record(0x0809, &[0; 16]);
        bytes.extend(records.concat());
        bytes.extend(record(0x000A, &[]));
        bytes
    }

    /// A BIFF8 workbook stream with a sheet holding each of `sheets`' records.
    fn biff(sheets: &[Vec<Vec<u8>>]) -> Vec<u8> {
        let bof = record(0x0809, &[0; 16]);
        let bound = |at: u32| {
            let mut data = at.to_le_bytes().to_vec();
            data.extend([0, 0, 1, 0, b'S']);
            record(0x0085, &data)
        };
        let globals_length = bof.len() + sheets.len() * bound(0).len() + 4;
        let mut sheet_streams = Vec::new();
        let mut offsets = Vec::new();
        for records in sheets {
            offsets.push((globals_length + sheet_streams.len()) as u32);
            sheet_streams.extend(substream(records));
        }
        let mut stream = bof.clone();
        for at in offsets {
            stream.extend(bound(at));
        }
        stream.extend(record(0x000A, &[]));
        stream.extend(sheet_streams);
        stream
    }

    /// An OLE compound file holding `workbook` as its `Workbook` stream, in regular sectors.
    fn compound(mut workbook: Vec<u8>) -> Vec<u8> {
        let length = workbook.len().max(4096);
        workbook.resize(length.div_ceil(512) * 512, 0);
        let sectors = workbook.len() / 512;
        let mut file = vec![0u8; 512];
        file[..8].copy_from_slice(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]);
        file[0x1A..0x1C].copy_from_slice(&3u16.to_le_bytes());
        file[0x1C..0x1E].copy_from_slice(&0xFFFEu16.to_le_bytes());
        file[0x1E..0x20].copy_from_slice(&9u16.to_le_bytes());
        file[0x20..0x22].copy_from_slice(&6u16.to_le_bytes());
        file[0x2C..0x30].copy_from_slice(&1u32.to_le_bytes());
        file[0x30..0x34].copy_from_slice(&1u32.to_le_bytes());
        file[0x38..0x3C].copy_from_slice(&4096u32.to_le_bytes());
        file[0x3C..0x40].copy_from_slice(&0xFFFF_FFFEu32.to_le_bytes());
        file[0x44..0x48].copy_from_slice(&0xFFFF_FFFEu32.to_le_bytes());
        for slot in 0..109 {
            let id: u32 = if slot == 0 { 0 } else { 0xFFFF_FFFF };
            file[0x4C + slot * 4..0x50 + slot * 4].copy_from_slice(&id.to_le_bytes());
        }
        let mut fat = vec![0xFFFF_FFFDu32, 0xFFFF_FFFE];
        fat.extend((0..sectors as u32).map(|at| {
            if at + 1 == sectors as u32 {
                0xFFFF_FFFE
            } else {
                at + 3
            }
        }));
        fat.resize(128, 0xFFFF_FFFF);
        file.extend(fat.iter().flat_map(|id| id.to_le_bytes()));
        let entry = |name: &str, kind: u8, start: u32, size: u32| {
            let mut entry = vec![0u8; 128];
            let units: Vec<u8> = name
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect();
            entry[..units.len()].copy_from_slice(&units);
            entry[64..66].copy_from_slice(&(units.len() as u16).to_le_bytes());
            entry[66] = kind;
            entry[116..120].copy_from_slice(&start.to_le_bytes());
            entry[120..124].copy_from_slice(&size.to_le_bytes());
            entry
        };
        file.extend(entry("Root Entry", 5, 0xFFFF_FFFE, 0));
        file.extend(entry("Workbook", 2, 2, length as u32));
        file.extend([0u8; 256]);
        file.extend(workbook);
        file
    }

    #[test]
    fn an_xls_spans_what_its_sheets_dimensions_say() {
        let one = compound(biff(&[vec![dimensions(65_536, 256)]]));
        let two = compound(biff(&[
            vec![dimensions(65_536, 256)],
            vec![dimensions(10, 4)],
        ]));

        assert_eq!(xls_cells(&one), Some(65_536 * 256));
        assert_eq!(xls_cells(&two), Some(65_536 * 256 + 40));
        assert_eq!(xls_cells(b"not a compound file"), None);
    }

    #[test]
    fn an_xls_spans_where_its_cells_are_whatever_its_dimensions_say() {
        let misleading = compound(biff(&[
            vec![dimensions(10, 4)],
            vec![
                dimensions(1, 1),
                substream(&[number(0, 0)]),
                number(0, 0),
                number(65_535, 255),
            ],
        ]));
        let backwards = compound(biff(&[vec![{
            let mut data = 9u32.to_le_bytes().to_vec();
            data.extend(5u32.to_le_bytes());
            data.extend([0, 0, 1, 0, 0, 0]);
            record(0x0200, &data)
        }]]));

        let shrunk = compound(biff(&[vec![dimensions(65_536, 256), dimensions(1, 1)]]));
        let string = compound(biff(&[vec![
            record(0x0207, &[1, 0, 0, b'a']),
            number(65_535, 255),
        ]]));

        assert_eq!(xls_cells(&misleading), Some(40 + 65_536 * 256));
        assert_eq!(xls_cells(&shrunk), Some(65_536 * 256));
        assert_eq!(xls_cells(&string), Some(65_536 * 256));

        let mut read_first = vec![number(0, 0); 1000];
        read_first.push(dimensions(100, 100));
        assert_eq!(xls_cells(&compound(biff(&[read_first]))), Some(11_000));
        assert!(xls_cells(&backwards).unwrap() > 4_000_000_000);
    }

    #[test]
    fn a_chain_of_sectors_ends_where_it_loops() {
        assert_eq!(chain(&[0], 0), [0]);
        assert_eq!(chain(&[1, 0, 7], 0), [0, 1]);
        assert_eq!(chain(&[1, 0xFFFF_FFFE], 0), [0, 1]);
    }

    #[test]
    fn a_compound_file_whose_tables_point_back_at_themselves_is_read_once() {
        let mut file = compound(biff(&[vec![dimensions(10, 4)]]));
        file[0x44..0x48].copy_from_slice(&0u32.to_le_bytes());
        file[0x48..0x4C].copy_from_slice(&1u32.to_le_bytes());
        for slot in 1..109 {
            file[0x4C + slot * 4..0x50 + slot * 4].copy_from_slice(&0u32.to_le_bytes());
        }
        file[512 + 127 * 4..512 + 128 * 4].copy_from_slice(&0u32.to_le_bytes());

        assert_eq!(xls_cells(&file), Some(40));
    }

    fn ods(dir: &std::path::Path, table: &str) -> std::path::PathBuf {
        let path = dir.join("book.ods");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        writer
            .start_file("content.xml", SimpleFileOptions::default())
            .unwrap();
        write!(
            writer,
            r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet>{table}</office:spreadsheet></office:body></office:document-content>"#
        )
        .unwrap();
        writer.finish().unwrap();
        path
    }

    #[test]
    fn an_ods_spans_its_cells_with_something_in_them_counting_repeats() {
        let dir = std::env::temp_dir().join(format!("hoshi-preflight-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let small = ods(
            &dir,
            r#"<table:table><table:table-row><table:table-cell office:value-type="float"/><table:table-cell office:value-type="string"><text:p>b</text:p></table:table-cell><table:table-cell><text:p>no type</text:p></table:table-cell><table:table-cell table:number-columns-repeated="1000"/></table:table-row><table:table-row table:number-rows-repeated="1048000"><table:table-cell table:number-columns-repeated="1024"/></table:table-row><table:table-row table:number-rows-repeated="20000"><table:table-cell table:number-columns-repeated="2000"></table:table-cell></table:table-row></table:table>"#,
        );
        assert_eq!(ods_cells(&small), Some(2));

        let formulas = ods(
            &dir,
            r#"<table:table><table:table-row table:number-rows-repeated="3"><table:table-cell table:formula="of:=1" table:number-columns-repeated="5"></table:table-cell></table:table-row></table:table>"#,
        );
        assert_eq!(ods_cells(&formulas), Some(15));

        let long = ods(
            &dir,
            &format!(
                r#"<table:table><table:table-row><table:table-cell office:value-type="string"><text:p>{}</text:p></table:table-cell></table:table-row></table:table>"#,
                "a".repeat(17 * 1024 * 1024)
            ),
        );
        assert_eq!(ods_cells(&long), Some(u64::MAX));

        let spaced = |count: &str, columns: u32, before: &str| {
            ods(
                &dir,
                &format!(
                    r#"<table:table><table:table-row table:number-rows-repeated="2"><table:table-cell office:value-type="string" table:number-columns-repeated="{columns}"><text:p>{before}a<text:s/>b<text:s text:c="{count}"/></text:p></table:table-cell></table:table-row></table:table>"#
                ),
            )
        };
        assert_eq!(ods_cells(&spaced("3", 1, "")), Some(2));
        assert_eq!(ods_cells(&spaced("1048576", 1, "")), Some(2));
        assert_eq!(ods_cells(&spaced("2147483647", 1, "")), Some(u64::MAX));
        assert_eq!(ods_cells(&spaced("&#50;147483647", 1, "")), Some(u64::MAX));
        assert_eq!(ods_cells(&spaced("1048576", 4096, "")), Some(u64::MAX));
        assert_eq!(
            ods_cells(&spaced("2147483647", 1, "<text:p></text:span>")),
            Some(u64::MAX)
        );
        assert_eq!(ods_cells(&spaced("3", 1, "</text:span>")), Some(2));
        let broken = ods(&dir, r#"<table:table><table:table-row a="></table:table>"#);
        assert_eq!(ods_cells(&broken), Some(u64::MAX));

        let huge = ods(
            &dir,
            r#"<table:table><table:table-row table:number-rows-repeated="1048576"><table:table-cell office:value-type="float" table:number-columns-repeated="16384"/></table:table-row></table:table>"#,
        );
        assert_eq!(ods_cells(&huge), Some(1_048_576 * 16_384));
    }
}
