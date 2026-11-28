use super::{
    Format, Grid, Kind, MAX_WORKBOOK_BYTES, TableFailure, is_table, looks_numeric, open,
    open_sheet, read_text, text_kind,
};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hoshi-table-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn texts(grid: &Grid, row: usize) -> Vec<String> {
    grid.row(row).into_iter().map(|value| value.text).collect()
}

fn kinds(grid: &Grid, row: usize) -> Vec<Kind> {
    grid.row(row).into_iter().map(|value| value.kind).collect()
}

fn workbook(path: &Path) {
    let parts = [
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        ),
        (
            "xl/workbook.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="January" sheetId="1" r:id="rId1"/><sheet name="Hidden" sheetId="2" state="hidden" r:id="rId2"/><sheet name="Summary" sheetId="3" r:id="rId3"/><sheet name="Chart" sheetId="4" r:id="rId4"/><sheet name="Wide" sheetId="5" r:id="rId5"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet3.xml"/><Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet" Target="chartsheets/sheet1.xml"/><Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet4.xml"/></Relationships>"#,
        ),
        (
            "xl/sharedStrings.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>product</t></si><si><t>units</t></si><si><t>Oak Table</t></si></sst>"#,
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="C1" t="s"><v>1</v></c></row><row r="2"><c r="A2" t="s"><v>2</v></c><c r="C2"><v>3</v></c></row><row r="3"><c r="C3"><v>0.30000000000000004</v></c></row></sheetData></worksheet>"#,
        ),
        (
            "xl/worksheets/sheet2.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#,
        ),
        (
            "xl/worksheets/sheet3.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="2"><c r="B2" t="b"><v>1</v></c></row></sheetData></worksheet>"#,
        ),
        (
            "xl/chartsheets/sheet1.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><chartsheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"/>"#,
        ),
        (
            "xl/worksheets/sheet4.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>1</v></c></row><row r="1048576"><c r="XFD1048576"><v>2</v></c></row></sheetData></worksheet>"#,
        ),
    ];
    let mut writer = zip::ZipWriter::new(File::create(path).unwrap());
    for (name, content) in parts {
        writer
            .start_file(name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(content.as_bytes()).unwrap();
    }
    writer.finish().unwrap();
}

#[test]
fn tables_are_told_by_their_extension() {
    assert!(is_table("/home/dev/shop/sales.CSV"));
    assert!(is_table(r"C:\dev\shop\report.xlsx"));
    assert!(is_table("/home/dev/shop/list.tsv"));
    assert!(!is_table("/home/dev/shop/csv/readme.md"));
    assert!(!is_table("/home/dev/shop/xlsx"));
}

#[test]
fn a_csv_with_semicolons_is_read_with_its_numbers_aligned() {
    let path = folder("csv").join("sales.csv");
    std::fs::write(
        &path,
        "date;product;total\n2026-01-04;Oak Table;1.188,00\n2026-01-05;Lamp;-632,50\n2026-01-06;Chair;\n",
    )
    .unwrap();

    let table = open(&path).unwrap();

    assert_eq!(
        table.format,
        Format::Delimited {
            separator: ';',
            truncated: false
        }
    );
    let grid = table.grid;
    assert_eq!((grid.rows(), grid.columns()), (4, 3));
    assert_eq!(texts(&grid, 1), ["2026-01-04", "Oak Table", "1.188,00"]);
    assert_eq!(kinds(&grid, 1), [Kind::Date, Kind::Text, Kind::Number]);
    assert_eq!(kinds(&grid, 0), [Kind::Text; 3]);
    assert_eq!(grid.width(1), "Oak Table".len());
    assert!(!grid.lines_indexed());
    assert_eq!(grid.lines(), 4);
    assert!(grid.lines_indexed());
}

#[test]
fn a_tsv_splits_on_tabs_even_with_commas_in_it() {
    let path = folder("tsv").join("list.tsv");
    std::fs::write(&path, "name\tnote\nAda\tone, two\n").unwrap();

    let grid = open(&path).unwrap().grid;

    assert_eq!(texts(&grid, 1), ["Ada", "one, two"]);
}

#[test]
fn a_workbook_lists_its_visible_sheets_and_shows_cells_where_they_sit() {
    let path = folder("xlsx").join("sales.xlsx");
    workbook(&path);

    let table = open(&path).unwrap();

    assert_eq!(
        table.format,
        Format::Workbook {
            sheets: vec!["January".into(), "Summary".into(), "Wide".into()]
        }
    );
    let grid = table.grid;
    assert_eq!((grid.rows(), grid.columns()), (3, 3));
    assert_eq!(texts(&grid, 0), ["product", "", "units"]);
    assert_eq!(texts(&grid, 2), ["", "", "0.3"]);
    assert_eq!(kinds(&grid, 2), [Kind::Text, Kind::Text, Kind::Number]);
    assert_eq!(grid.lines(), 0);

    let summary = open_sheet(&path, "Summary").unwrap();
    assert_eq!((summary.rows(), summary.columns()), (2, 2));
    assert_eq!(kinds(&summary, 1), [Kind::Text, Kind::Logical(true)]);
    assert_eq!(summary.width(1), 9);
}

#[test]
fn a_sheet_reaching_the_last_cell_is_refused_before_it_is_held() {
    let path = folder("wide").join("wide.xlsx");
    workbook(&path);

    assert_eq!(
        open_sheet(&path, "Wide").err(),
        Some(TableFailure::TooLarge)
    );
}

#[test]
fn a_workbook_too_large_to_open_is_refused() {
    let path = folder("huge").join("huge.xlsx");
    let file = File::create(&path).unwrap();
    file.set_len(MAX_WORKBOOK_BYTES + 1).unwrap();

    assert_eq!(open(&path).err(), Some(TableFailure::TooLarge));
}

#[test]
fn a_text_larger_than_the_limit_is_cut_at_its_last_whole_line() {
    let path = folder("cut").join("long.csv");
    std::fs::write(&path, "a,b\n1,2\n3,4\n").unwrap();

    assert_eq!(read_text(&path, 10).unwrap(), ("a,b\n1,2\n".into(), true));
    assert_eq!(
        read_text(&path, 12).unwrap(),
        ("a,b\n1,2\n3,4\n".into(), false)
    );
}

#[test]
fn a_file_that_is_not_a_workbook_fails_to_open() {
    let path = folder("broken").join("broken.xlsx");
    std::fs::write(&path, "not a zip").unwrap();

    assert!(open(&path).is_err());
}

#[test]
fn numbers_may_carry_marks_signs_currencies_and_percents() {
    for text in ["1.188,00", "-632.5", "$ 12", "15%", "1,000,000", "€3"] {
        assert!(looks_numeric(text), "{text}");
    }
    for text in ["2026-01-04", "v1.2", "N/A", "", "-", "12 units", "inf"] {
        assert!(!looks_numeric(text), "{text}");
    }
    assert!(looks_numeric("1e5"));
}

#[test]
fn a_text_cell_is_a_date_only_when_it_starts_with_a_whole_one() {
    assert_eq!(text_kind("2026-01-04"), Kind::Date);
    assert_eq!(text_kind("2026-01-04 10:30:00"), Kind::Date);
    assert_eq!(text_kind("2026-01-04T10:30"), Kind::Date);
    assert_eq!(text_kind("2026-01-045"), Kind::Text);
    assert_eq!(text_kind("TRUE"), Kind::Text);
    assert_eq!(text_kind("P-10001"), Kind::Text);
}
