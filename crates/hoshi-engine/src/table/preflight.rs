//! How many cells an XLS or ODS workbook spans, measured before it is read: their readers load
//! every sheet as the workbook opens, so a sheet too large to hold must be caught first.

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use std::io::BufReader;
use std::path::Path;

/// The cells the sheets of the XLS or ODS at `path` span together, each from its first used
/// cell to its last; `None` when it can't be measured, so its reader still tries.
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
/// at a loop.
fn chain(table: &[u32], start: u32) -> Vec<u32> {
    let mut chained = Vec::new();
    let mut at = start;
    while (at as usize) < table.len() && chained.len() < table.len() {
        chained.push(at);
        at = table[at as usize];
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
    const LAST_ID: u32 = 0xFFFF_FFFA;
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
    let mut fat_sectors: Vec<u32> = words(file.get(0x4C..0x4C + 109 * 4)?)
        .filter(|id| *id < LAST_ID)
        .collect();
    let mut difat = u32_at(file, 0x44)?;
    let mut seen = 0;
    while difat < LAST_ID && seen < file.len() / size {
        let data = sector(difat)?;
        let entries = size / 4 - 1;
        fat_sectors.extend(words(&data[..entries * 4]).filter(|id| *id < LAST_ID));
        difat = u32_at(data, entries * 4)?;
        seen += 1;
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

/// The cells an XLS's worksheets span, as each sheet's dimensions record gives them.
fn xls_cells(file: &[u8]) -> Option<u64> {
    const EOF: u16 = 0x000A;
    const BOUND_SHEET: u16 = 0x0085;
    const DIMENSIONS: u16 = 0x0200;
    let stream = workbook_stream(file)?;
    let record = |at: usize| {
        let kind = u16_at(&stream, at)?;
        let length = u16_at(&stream, at + 2)? as usize;
        let data = stream.get(at + 4..at + 4 + length)?;
        Some((kind, data, at + 4 + length))
    };
    let mut sheets = Vec::new();
    let mut at = 0;
    loop {
        let (kind, data, next) = record(at)?;
        match kind {
            BOUND_SHEET if data.get(5) == Some(&0) => sheets.push(u32_at(data, 0)? as usize),
            EOF => break,
            _ => {}
        }
        at = next;
    }
    let mut cells = 0u64;
    for start in sheets {
        let mut at = start;
        loop {
            let (kind, data, next) = record(at)?;
            if kind == DIMENSIONS {
                let (rows, columns) = if data.len() >= 14 {
                    (
                        u32_at(data, 4)?.saturating_sub(u32_at(data, 0)?),
                        u16_at(data, 10)?.saturating_sub(u16_at(data, 8)?),
                    )
                } else {
                    (
                        u32::from(u16_at(data, 2)?.saturating_sub(u16_at(data, 0)?)),
                        u16_at(data, 6)?.saturating_sub(u16_at(data, 4)?),
                    )
                };
                cells = cells.saturating_add(u64::from(rows) * u64::from(columns));
                break;
            }
            if kind == EOF {
                break;
            }
            at = next;
        }
    }
    Some(cells)
}

fn repeated(element: &BytesStart<'_>, name: &str) -> u64 {
    element
        .try_get_attribute(name)
        .ok()
        .flatten()
        .and_then(|attribute| attribute.value.parse().ok())
        .unwrap_or(1)
}

fn has_value(element: &BytesStart<'_>) -> bool {
    element
        .try_get_attribute("office:value-type")
        .ok()
        .flatten()
        .is_some()
}

/// The cells an ODS's tables span from A1 to their last cell with something in it, counting
/// rows and cells repeated.
fn ods_cells(path: &Path) -> Option<u64> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    let content = archive.by_name("content.xml").ok()?;
    let mut reader = Reader::from_reader(BufReader::new(content));
    let mut buffer = Vec::new();
    let mut cells = 0u64;
    let (mut row, mut rows, mut columns) = (0u64, 0u64, 0u64);
    let (mut column, mut repeats, mut filled) = (0u64, 1u64, false);
    loop {
        let event = reader.read_event_into(&mut buffer).ok()?;
        match &event {
            Event::Start(element) | Event::Empty(element) => {
                let empty = matches!(event, Event::Empty(_));
                match element.name().as_ref() {
                    "table:table" => (row, rows, columns) = (0, 0, 0),
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
                        if !empty || has_value(element) {
                            filled = true;
                            columns = columns.max(column.saturating_add(times));
                        }
                        column = column.saturating_add(times);
                    }
                    _ => {}
                }
            }
            Event::End(element) => match element.name().as_ref() {
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
    use super::{ods_cells, xls_cells};
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn record(kind: u16, data: &[u8]) -> Vec<u8> {
        let mut bytes = kind.to_le_bytes().to_vec();
        bytes.extend((data.len() as u16).to_le_bytes());
        bytes.extend(data);
        bytes
    }

    /// A BIFF8 workbook stream whose worksheets span `sheets` rows by columns each.
    fn biff(sheets: &[(u32, u16)]) -> Vec<u8> {
        let bof = record(0x0809, &[0; 16]);
        let bound = |at: u32| {
            let mut data = at.to_le_bytes().to_vec();
            data.extend([0, 0, 1, 0, b'S']);
            record(0x0085, &data)
        };
        let globals_length = bof.len() + sheets.len() * bound(0).len() + 4;
        let mut sheet_streams = Vec::new();
        let mut offsets = Vec::new();
        for (rows, columns) in sheets {
            offsets.push((globals_length + sheet_streams.len()) as u32);
            let mut dimensions = 0u32.to_le_bytes().to_vec();
            dimensions.extend(rows.to_le_bytes());
            dimensions.extend(0u16.to_le_bytes());
            dimensions.extend(columns.to_le_bytes());
            dimensions.extend(0u16.to_le_bytes());
            sheet_streams.extend(&bof);
            sheet_streams.extend(record(0x0200, &dimensions));
            sheet_streams.extend(record(0x000A, &[]));
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
        let one = compound(biff(&[(65_536, 256)]));
        let two = compound(biff(&[(65_536, 256), (10, 4)]));

        assert_eq!(xls_cells(&one), Some(65_536 * 256));
        assert_eq!(xls_cells(&two), Some(65_536 * 256 + 40));
        assert_eq!(xls_cells(b"not a compound file"), None);
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
            r#"<table:table><table:table-row><table:table-cell office:value-type="float"/><table:table-cell><text:p>b</text:p></table:table-cell><table:table-cell table:number-columns-repeated="1000"/></table:table-row><table:table-row table:number-rows-repeated="1048000"><table:table-cell table:number-columns-repeated="1024"/></table:table-row></table:table>"#,
        );
        assert_eq!(ods_cells(&small), Some(2));

        let huge = ods(
            &dir,
            r#"<table:table><table:table-row table:number-rows-repeated="1048576"><table:table-cell office:value-type="float" table:number-columns-repeated="16384"/></table:table-row></table:table>"#,
        );
        assert_eq!(ods_cells(&huge), Some(1_048_576 * 16_384));
    }
}
