use super::{Opened, decode, encode, read, serve_request};
use crate::table::TableFailure;
use calamine::{CellErrorType, Data, ExcelDateTime, ExcelDateTimeType, Range};
use std::io::Write;
use std::path::PathBuf;
use zip::write::SimpleFileOptions;

/// The reading process the other tests start; it does nothing when the tests run.
#[test]
fn reads_a_workbook_when_asked() {
    if std::env::var_os(super::PATH).is_some() {
        std::process::exit(serve_request(
            |limit| {
                let _ = super::ALLOCATOR.set_limit(limit);
            },
            || super::ALLOCATOR.allocated(),
        ));
    }
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hoshi-reader-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn record(kind: u16, data: &[u8]) -> Vec<u8> {
    let mut bytes = kind.to_le_bytes().to_vec();
    bytes.extend((data.len() as u16).to_le_bytes());
    bytes.extend(data);
    bytes
}

fn substream(records: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = record(
        0x0809,
        &[0x00, 0x06, 0x10, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    );
    bytes.extend(records.concat());
    bytes.extend(record(0x000A, &[]));
    bytes
}

/// A BIFF8 workbook stream with a sheet holding each of `sheets`' records.
fn biff(sheets: &[Vec<Vec<u8>>]) -> Vec<u8> {
    let bof = record(
        0x0809,
        &[0x00, 0x06, 0x05, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    );
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

fn round_trip(opened: Result<Opened, TableFailure>) -> Result<Opened, TableFailure> {
    let mut bytes = Vec::new();
    encode(&mut bytes, &opened).unwrap();
    decode(&mut bytes.as_slice())
}

#[test]
fn what_the_reader_sends_back_reads_as_it_was() {
    let mut range = Range::new((1, 1), (2, 6));
    let cells = [
        Data::Int(-7),
        Data::Float(0.25),
        Data::String("Lisbon".into()),
        Data::Bool(true),
        Data::DateTime(ExcelDateTime::new(
            46_026.5,
            ExcelDateTimeType::DateTime,
            false,
        )),
        Data::DateTime(ExcelDateTime::new(
            1.25,
            ExcelDateTimeType::TimeDelta,
            false,
        )),
        Data::DateTimeIso("2026-01-04".into()),
        Data::DurationIso("PT1H".into()),
        Data::Error(CellErrorType::Ref),
    ];
    for (at, cell) in cells.iter().enumerate() {
        let at = at as u32;
        range.set_value((1 + at / 6, 1 + at % 6), cell.clone());
    }
    let sheets = vec!["January".to_owned(), "Summary".to_owned()];

    let (read_sheets, read_range) = round_trip(Ok((sheets.clone(), range.clone()))).unwrap();

    assert_eq!(read_sheets, sheets);
    assert_eq!(read_range, range);
    assert_eq!(
        round_trip(Ok((Vec::new(), Range::default()))).unwrap().1,
        Range::default()
    );
    assert_eq!(
        round_trip(Err(TableFailure::TooLarge)),
        Err(TableFailure::TooLarge)
    );
    assert_eq!(
        round_trip(Err(TableFailure::Read("broken".into()))),
        Err(TableFailure::Read("broken".into()))
    );
}

#[test]
fn a_date_or_duration_too_far_to_tell_reads_as_its_number() {
    let mut range = Range::new((0, 0), (0, 2));
    let far = [
        ExcelDateTime::new(-1e300, ExcelDateTimeType::TimeDelta, false),
        ExcelDateTime::new(1e300, ExcelDateTimeType::DateTime, false),
        ExcelDateTime::new(f64::NAN, ExcelDateTimeType::DateTime, false),
    ];
    for (column, date) in far.into_iter().enumerate() {
        range.set_value((0, column as u32), Data::DateTime(date));
    }

    let (_, read) = round_trip(Ok((Vec::new(), range))).unwrap();

    assert_eq!(read.get_value((0, 0)), Some(&Data::Float(-1e300)));
    assert_eq!(read.get_value((0, 1)), Some(&Data::Float(1e300)));
    assert!(matches!(read.get_value((0, 2)), Some(Data::Float(value)) if value.is_nan()));
}

#[test]
fn a_date_counted_from_1904_reads_as_the_same_day() {
    let date = ExcelDateTime::new(44_564., ExcelDateTimeType::DateTime, false);
    let mut range = Range::new((0, 0), (0, 0));
    range.set_value((0, 0), Data::DateTime(date));

    let (_, read) = round_trip(Ok((Vec::new(), range))).unwrap();

    let Some(Data::DateTime(read)) = read.get_value((0, 0)) else {
        panic!("{read:?}");
    };
    assert_eq!(read.as_datetime(), date.as_datetime());
}

#[test]
fn a_workbook_whose_reader_runs_out_of_memory_is_too_large() {
    let path = folder("memory").join("huge.xls");
    let mut dimensions = 0u32.to_le_bytes().to_vec();
    dimensions.extend(4_000_000_000u32.to_le_bytes());
    dimensions.extend([0, 0, 0, 1, 0, 0]);
    std::fs::write(&path, compound(biff(&[vec![record(0x0200, &dimensions)]]))).unwrap();

    assert_eq!(read(&path, None).err(), Some(TableFailure::TooLarge));
}

/// An ODS whose content holds `tables`, cut off there.
fn ods(path: &std::path::Path, tables: &str) {
    let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    writer
        .start_file("mimetype", SimpleFileOptions::default())
        .unwrap();
    writer
        .write_all(b"application/vnd.oasis.opendocument.spreadsheet")
        .unwrap();
    writer
        .start_file("META-INF/manifest.xml", SimpleFileOptions::default())
        .unwrap();
    writer
        .write_all(br#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/></manifest:manifest>"#)
        .unwrap();
    writer
        .start_file("content.xml", SimpleFileOptions::default())
        .unwrap();
    write!(
        writer,
        r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet>{tables}"#
    )
    .unwrap();
    writer.finish().unwrap();
}

/// An ODS of one string cell with `spaces` spaces, repeated across `columns` columns.
fn spaced(path: &std::path::Path, spaces: &str, columns: u32) {
    ods(
        path,
        &format!(
            r#"<table:table table:name="Spaces"><table:table-row><table:table-cell office:value-type="string" table:number-columns-repeated="{columns}"><text:p>a<text:s text:c="{spaces}"/>b</text:p></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#
        ),
    );
}

#[test]
fn a_workbook_whose_reader_never_ends_is_too_large() {
    let path = folder("endless").join("endless.ods");
    ods(&path, r#"<table:table table:name="Open">"#);

    assert_eq!(read(&path, None).err(), Some(TableFailure::TooLarge));
}

#[test]
fn spaces_an_ods_cell_expands_to_are_held_to_the_limits() {
    let dir = folder("spaces");
    let few = dir.join("few.ods");
    spaced(&few, "3", 2);
    let escaped = dir.join("escaped.ods");
    spaced(&escaped, "&#50;147483647", 1);
    let repeated = dir.join("repeated.ods");
    spaced(&repeated, "1048576", 4096);
    let under_memory = dir.join("under_memory.ods");
    spaced(&under_memory, "600000", 2);

    let (sheets, range) = read(&few, None).unwrap();
    assert_eq!(sheets, ["Spaces"]);
    assert_eq!(range.get_value((0, 1)), Some(&Data::String("a   b".into())));
    assert_eq!(read(&escaped, None).err(), Some(TableFailure::TooLarge));
    assert_eq!(read(&repeated, None).err(), Some(TableFailure::TooLarge));
    assert_eq!(
        read(&under_memory, None).err(),
        Some(TableFailure::TooLarge)
    );
}

#[test]
fn a_sheet_with_more_text_than_the_limit_is_too_large_either_side() {
    let mut range = Range::new((0, 0), (0, 1));
    range.set_value((0, 0), Data::String("a".repeat(super::MAX_TEXT / 2 + 1)));
    range.set_value((0, 1), Data::String("b".repeat(super::MAX_TEXT / 2 + 1)));
    let held = Ok((Vec::new(), range));

    assert_eq!(round_trip(held.clone()), Err(TableFailure::TooLarge));

    let mut frame = vec![0, 0, 0, 0, 0, 1];
    frame.extend([0u8; 16]);
    frame.push(3);
    frame.extend(((super::MAX_TEXT + 1) as u32).to_le_bytes());
    assert_eq!(decode(&mut frame.as_slice()), Err(TableFailure::TooLarge));
}

#[test]
fn a_file_its_reader_refuses_fails_to_read() {
    let path = folder("refused").join("refused.xls");
    std::fs::write(&path, "not a workbook").unwrap();

    assert!(matches!(read(&path, None), Err(TableFailure::Read(_))));
}

#[test]
fn the_sheet_is_read_past_whatever_the_program_wrote_before_it() {
    let mut output: Vec<u8> = b"running 1 test\n\0hoshi\0".to_vec();
    output.extend(super::MARK);
    output.extend(b"rest");
    let mut input = output.as_slice();

    super::past_mark(&mut input).unwrap();

    assert_eq!(input, b"rest");
    assert!(super::past_mark(&mut &b"no mark here"[..]).is_err());
}
