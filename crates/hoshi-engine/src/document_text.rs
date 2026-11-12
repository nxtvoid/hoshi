//! The text of office documents, books and notebooks, written beside them as
//! `<file>.txt`, so an agent that can't read the document itself reads its text.

use roxmltree::{Document, Node};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

const EXTENSIONS: &[&str] = &[
    "docx", "xlsx", "pptx", "odt", "ods", "odp", "rtf", "epub", "ipynb",
];
/// Larger documents are left to the agent.
const MAX_INPUT_BYTES: u64 = 16 * 1024 * 1024;
/// Most a part of an archive may unpack to.
const MAX_PART_BYTES: u64 = 64 * 1024 * 1024;
/// Most text written for one document.
const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

fn extension(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    EXTENSIONS
        .contains(&extension.as_str())
        .then_some(extension)
}

/// Where the text of the document at `path` is, once written.
pub(crate) fn sidecar(path: &Path) -> Option<PathBuf> {
    extension(path)?;
    let mut name = path.file_name()?.to_os_string();
    name.push(".txt");
    let sidecar = path.with_file_name(name);
    sidecar.is_file().then_some(sidecar)
}

/// Writes the text of the document at `path` beside it; leaves nothing when it has none
/// or can't be read.
pub(crate) fn extract(path: &Path) {
    let Some(extension) = extension(path) else {
        return;
    };
    let small = std::fs::metadata(path).is_ok_and(|meta| meta.len() <= MAX_INPUT_BYTES);
    if !small {
        return;
    }
    let text = match text_of(path, &extension) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("hoshi: no text from a .{extension} document: {error}");
            return;
        }
    };
    let text = truncated(tidy(&text));
    if text.trim().is_empty() {
        return;
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".txt");
    let target = path.with_file_name(&name);
    name.push(".part");
    let pending = path.with_file_name(name);
    let written = std::fs::write(&pending, text).and_then(|()| std::fs::rename(&pending, &target));
    if written.is_err() {
        let _ = std::fs::remove_file(pending);
    }
}

fn text_of(path: &Path, extension: &str) -> Result<String, String> {
    match extension {
        "rtf" => Ok(rtf_text(
            &std::fs::read(path).map_err(|error| error.to_string())?,
        )),
        "ipynb" => notebook_text(&std::fs::read_to_string(path).map_err(|e| e.to_string())?),
        _ => {
            let file = File::open(path).map_err(|error| error.to_string())?;
            let mut archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
            match extension {
                "docx" => word_text(&mut archive),
                "pptx" => slides_text(&mut archive),
                "xlsx" => workbook_text(&mut archive),
                "odt" | "ods" | "odp" => open_document_text(&mut archive),
                "epub" => book_text(&mut archive),
                _ => Err("unknown document".into()),
            }
        }
    }
}

type Archive = zip::ZipArchive<File>;

fn part(archive: &mut Archive, name: &str) -> Result<String, String> {
    let entry = archive
        .by_name(name)
        .map_err(|error| format!("{name}: {error}"))?;
    let mut text = String::new();
    entry
        .take(MAX_PART_BYTES)
        .read_to_string(&mut text)
        .map_err(|error| format!("{name}: {error}"))?;
    Ok(text)
}

fn parse(xml: &str) -> Result<Document<'_>, String> {
    roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .map_err(|error| error.to_string())
}

/// Names of the archive's parts `prefix<n>.xml`, in the order of `n`.
fn numbered_parts(archive: &Archive, prefix: &str) -> Vec<String> {
    let mut parts: Vec<(u32, String)> = archive
        .file_names()
        .filter_map(|name| {
            let name = name.ok()?;
            let number = name.strip_prefix(prefix)?.strip_suffix(".xml")?;
            Some((number.parse().ok()?, name.into_owned()))
        })
        .collect();
    parts.sort();
    parts.into_iter().map(|(_, name)| name).collect()
}

/// What `on_element` writes for an element, before and after its children.
struct Writes {
    before: &'static str,
    after: &'static str,
}

/// How a format holds its text and marks its breaks.
struct Format {
    /// Whether text directly inside this element is the document's.
    holds_text: fn(Node) -> bool,
    /// What an element adds before and after its children.
    writes: fn(Node) -> Option<Writes>,
}

/// The text of `node`'s descendants, as `format` reads them.
fn walk(node: Node, out: &mut String, format: &Format) {
    for child in node.children() {
        if child.is_text() {
            if child.parent_element().is_some_and(format.holds_text) {
                out.push_str(child.text().unwrap_or_default());
            }
            continue;
        }
        if !child.is_element() {
            continue;
        }
        let writes = (format.writes)(child);
        if let Some(writes) = &writes {
            out.push_str(writes.before);
        }
        walk(child, out, format);
        if let Some(writes) = &writes {
            out.push_str(writes.after);
        }
    }
}

fn in_t(node: Node) -> bool {
    node.tag_name().name() == "t"
}

/// A paragraph ends its line, except the last one of a table cell, which the cell ends.
fn line_end(paragraph: Node, cell: &str) -> &'static str {
    let last_in_cell = paragraph
        .parent_element()
        .is_some_and(|parent| parent.tag_name().name() == cell)
        && paragraph.next_sibling_element().is_none();
    if last_in_cell { "" } else { "\n" }
}

fn writes(before: &'static str, after: &'static str) -> Option<Writes> {
    Some(Writes { before, after })
}

fn word_text(archive: &mut Archive) -> Result<String, String> {
    let xml = part(archive, "word/document.xml")?;
    let document = parse(&xml)?;
    let mut out = String::new();
    let format = Format {
        holds_text: in_t,
        writes: |node| match node.tag_name().name() {
            "p" => writes("", line_end(node, "tc")),
            "tab" => writes("\t", ""),
            "br" | "cr" => writes("\n", ""),
            "tc" => writes("", "\t"),
            "tr" => writes("", "\n"),
            _ => None,
        },
    };
    walk(document.root(), &mut out, &format);
    Ok(out)
}

fn drawing_text(xml: &str) -> Result<String, String> {
    let document = parse(xml)?;
    let mut out = String::new();
    let format = Format {
        holds_text: in_t,
        writes: |node| match node.tag_name().name() {
            "p" => writes("", "\n"),
            "br" => writes("\n", ""),
            "tab" => writes("\t", ""),
            _ => None,
        },
    };
    walk(document.root(), &mut out, &format);
    Ok(out)
}

fn slides_text(archive: &mut Archive) -> Result<String, String> {
    let mut out = String::new();
    for (index, name) in numbered_parts(archive, "ppt/slides/slide")
        .iter()
        .enumerate()
    {
        if out.len() > MAX_TEXT_BYTES {
            break;
        }
        let xml = part(archive, name)?;
        out.push_str(&format!("--- Slide {} ---\n", index + 1));
        out.push_str(&drawing_text(&xml)?);
        out.push('\n');
    }
    Ok(out)
}

/// Every `t` under `node`, as one string.
fn all_text(node: Node) -> String {
    node.descendants()
        .filter(|node| node.tag_name().name() == "t")
        .filter_map(|node| node.text())
        .collect()
}

const MAX_COLUMN: usize = 16384;

/// The zero-based column of a cell reference such as `C7`, or `None` past Excel's last column.
fn column(reference: &str) -> Option<usize> {
    let mut column: usize = 0;
    for letter in reference.chars().take_while(char::is_ascii_alphabetic) {
        column = column
            .checked_mul(26)?
            .checked_add(letter.to_ascii_uppercase() as usize - 'A' as usize + 1)?;
        if column > MAX_COLUMN {
            return None;
        }
    }
    Some(column.saturating_sub(1))
}

/// The workbook's sheets, by name and part, in the order the workbook lists them.
fn sheets(archive: &mut Archive) -> Result<Vec<(String, String)>, String> {
    let fallback = |archive: &Archive| {
        numbered_parts(archive, "xl/worksheets/sheet")
            .into_iter()
            .enumerate()
            .map(|(index, name)| (format!("Sheet {}", index + 1), name))
            .collect::<Vec<_>>()
    };
    let (Ok(workbook), Ok(relations)) = (
        part(archive, "xl/workbook.xml"),
        part(archive, "xl/_rels/workbook.xml.rels"),
    ) else {
        return Ok(fallback(archive));
    };
    let relations = parse(&relations)?;
    let targets: HashMap<&str, String> = relations
        .descendants()
        .filter(|node| node.tag_name().name() == "Relationship")
        .filter_map(|node| {
            let target = node.attribute("Target")?;
            let target = match target.strip_prefix('/') {
                Some(absolute) => absolute.to_owned(),
                None => format!("xl/{target}"),
            };
            Some((node.attribute("Id")?, target))
        })
        .collect();
    let workbook = parse(&workbook)?;
    let listed: Vec<(String, String)> = workbook
        .descendants()
        .filter(|node| node.tag_name().name() == "sheet")
        .filter_map(|node| {
            let id = node
                .attributes()
                .find(|attribute| attribute.name() == "id")?
                .value();
            Some((node.attribute("name")?.to_owned(), targets.get(id)?.clone()))
        })
        .collect();
    Ok(if listed.is_empty() {
        fallback(archive)
    } else {
        listed
    })
}

fn workbook_text(archive: &mut Archive) -> Result<String, String> {
    let shared: Vec<String> = match part(archive, "xl/sharedStrings.xml") {
        Ok(xml) => parse(&xml)?
            .descendants()
            .filter(|node| node.tag_name().name() == "si")
            .map(all_text)
            .collect(),
        Err(_) => Vec::new(),
    };
    let mut out = String::new();
    for (name, sheet) in sheets(archive)? {
        if out.len() > MAX_TEXT_BYTES {
            break;
        }
        let Ok(xml) = part(archive, &sheet) else {
            continue;
        };
        let document = parse(&xml)?;
        out.push_str(&format!("## {name}\n"));
        for row in document
            .descendants()
            .filter(|node| node.tag_name().name() == "row")
        {
            let mut cells: Vec<String> = Vec::new();
            for cell in row.children().filter(|node| node.tag_name().name() == "c") {
                let value = cell
                    .children()
                    .find(|node| node.tag_name().name() == "v")
                    .and_then(|node| node.text())
                    .unwrap_or_default();
                let text = match cell.attribute("t") {
                    Some("s") => value
                        .trim()
                        .parse::<usize>()
                        .ok()
                        .and_then(|index| shared.get(index).cloned())
                        .unwrap_or_default(),
                    Some("inlineStr") => all_text(cell),
                    Some("b") => if value == "1" { "TRUE" } else { "FALSE" }.to_owned(),
                    _ => value.to_owned(),
                };
                let at = cell.attribute("r").and_then(column).unwrap_or(cells.len());
                if at >= cells.len() {
                    cells.resize(at, String::new());
                    cells.push(text);
                } else {
                    cells.push(text);
                }
            }
            if cells.iter().any(|cell| !cell.is_empty()) {
                out.push_str(&cells.join("\t"));
                out.push('\n');
            }
        }
        out.push('\n');
    }
    Ok(out)
}

fn open_document_text(archive: &mut Archive) -> Result<String, String> {
    let xml = part(archive, "content.xml")?;
    let document = parse(&xml)?;
    let mut out = String::new();
    let format = Format {
        holds_text: |node| {
            node.tag_name()
                .namespace()
                .is_some_and(|namespace| namespace.ends_with(":text:1.0"))
        },
        writes: |node| match node.tag_name().name() {
            "p" | "h" => writes("", line_end(node, "table-cell")),
            "tab" => writes("\t", ""),
            "line-break" => writes("\n", ""),
            "s" => writes(" ", ""),
            "table-cell" => writes("", "\t"),
            "table-row" | "table" => writes("", "\n"),
            "page" => writes("--- Slide ---\n", "\n"),
            _ => None,
        },
    };
    walk(document.root(), &mut out, &format);
    Ok(out)
}

/// `path` relative to the folder of `base`, both inside an archive.
fn joined(base: &str, path: &str) -> String {
    let mut parts: Vec<&str> = base.split('/').collect();
    parts.pop();
    for segment in path.split('/') {
        match segment {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(segment),
        }
    }
    parts.join("/")
}

fn book_text(archive: &mut Archive) -> Result<String, String> {
    let container = part(archive, "META-INF/container.xml")?;
    let container = parse(&container)?;
    let package_path = container
        .descendants()
        .find(|node| node.tag_name().name() == "rootfile")
        .and_then(|node| node.attribute("full-path"))
        .ok_or("no package in the book")?
        .to_owned();
    let package = part(archive, &package_path)?;
    let package = parse(&package)?;
    let manifest: HashMap<&str, &str> = package
        .descendants()
        .filter(|node| node.tag_name().name() == "item")
        .filter_map(|node| Some((node.attribute("id")?, node.attribute("href")?)))
        .collect();
    let chapters: Vec<String> = package
        .descendants()
        .filter(|node| node.tag_name().name() == "itemref")
        .filter_map(|node| manifest.get(node.attribute("idref")?))
        .map(|href| joined(&package_path, &percent_decoded(href)))
        .collect();
    let mut out = String::new();
    for chapter in chapters {
        if out.len() > MAX_TEXT_BYTES {
            break;
        }
        if let Ok(html) = part(archive, &chapter) {
            out.push_str(&html_text(&html));
            out.push('\n');
        }
    }
    Ok(out)
}

fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'%')
            .then(|| text.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                out.push(byte);
                index += 3;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The text of an HTML page: its tags dropped, its blocks on lines of their own.
fn html_text(html: &str) -> String {
    let body = html.find("<body").map_or(html, |start| &html[start..]);
    let mut out = String::new();
    let mut rest = body;
    while let Some(start) = rest.find('<') {
        out.push_str(&entities_decoded(&rest[..start]));
        let Some(end) = rest[start..].find('>') else {
            rest = "";
            break;
        };
        let tag = &rest[start + 1..start + end];
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_ascii_lowercase();
        rest = &rest[start + end + 1..];
        if !tag.starts_with('/') && (name == "script" || name == "style") {
            let closing = format!("</{name}");
            rest = rest.find(&closing).map_or("", |at| &rest[at..]);
            continue;
        }
        let block = matches!(
            name.as_str(),
            "p" | "div"
                | "br"
                | "li"
                | "tr"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "blockquote"
                | "pre"
                | "section"
                | "article"
                | "table"
        );
        if block {
            out.push('\n');
        } else if name == "td" || name == "th" {
            out.push('\t');
        }
    }
    out.push_str(&entities_decoded(rest));
    out
}

fn entities_decoded(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let decoded = after.find(';').filter(|end| *end <= 10).and_then(|end| {
            let entity = &after[..end];
            let char = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| entity.strip_prefix('#').and_then(|n| n.parse().ok()))
                    .and_then(char::from_u32),
            }?;
            Some((char, end))
        });
        match decoded {
            Some((char, end)) => {
                out.push(char);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn notebook_text(json: &str) -> Result<String, String> {
    let notebook: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let language = notebook["metadata"]["kernelspec"]["language"]
        .as_str()
        .or_else(|| notebook["metadata"]["language_info"]["name"].as_str())
        .unwrap_or_default();
    let joined = |value: &serde_json::Value| match value {
        serde_json::Value::Array(lines) => lines.iter().filter_map(|line| line.as_str()).collect(),
        serde_json::Value::String(text) => text.clone(),
        _ => String::new(),
    };
    let mut out = String::new();
    for cell in notebook["cells"].as_array().into_iter().flatten() {
        let source = joined(&cell["source"]);
        match cell["cell_type"].as_str() {
            Some("code") => {
                out.push_str(&format!("```{language}\n{}\n```\n", source.trim_end()));
                for output in cell["outputs"].as_array().into_iter().flatten() {
                    let text = match output["output_type"].as_str() {
                        Some("stream") => joined(&output["text"]),
                        Some("execute_result" | "display_data") => {
                            joined(&output["data"]["text/plain"])
                        }
                        Some("error") => format!(
                            "{}: {}",
                            output["ename"].as_str().unwrap_or_default(),
                            output["evalue"].as_str().unwrap_or_default()
                        ),
                        _ => String::new(),
                    };
                    if !text.trim().is_empty() {
                        out.push_str(&format!("Output:\n{}\n", text.trim_end()));
                    }
                }
            }
            _ => out.push_str(source.trim_end()),
        }
        out.push_str("\n\n");
    }
    Ok(out)
}

/// Groups whose content is never the document's text.
const RTF_SKIPPED: &[&str] = &[
    "fonttbl",
    "colortbl",
    "stylesheet",
    "info",
    "pict",
    "object",
    "themedata",
    "colorschememapping",
    "latentstyles",
    "datastore",
    "listtable",
    "listoverridetable",
    "rsidtbl",
    "generator",
    "xmlnstbl",
    "header",
    "footer",
];

/// The text of an RTF document: its control words read for breaks and characters.
fn rtf_text(bytes: &[u8]) -> String {
    let mut out = String::new();
    // Per open group: whether it is skipped, and how many characters follow a `\u`.
    let mut groups: Vec<(bool, usize)> = vec![(false, 1)];
    let mut skip_chars = 0usize;
    let mut index = 0;
    let skipping = |groups: &[(bool, usize)]| groups.last().is_some_and(|group| group.0);
    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            b'{' => {
                let inherited = *groups.last().unwrap_or(&(false, 1));
                groups.push(inherited);
                index += 1;
            }
            b'}' => {
                if groups.len() > 1 {
                    groups.pop();
                }
                index += 1;
            }
            b'\\' => {
                index += 1;
                let Some(&next) = bytes.get(index) else {
                    break;
                };
                if next.is_ascii_alphabetic() {
                    let start = index;
                    while index < bytes.len() && bytes[index].is_ascii_alphabetic() {
                        index += 1;
                    }
                    let word = std::str::from_utf8(&bytes[start..index]).unwrap_or_default();
                    let number_start = index;
                    if bytes.get(index) == Some(&b'-') {
                        index += 1;
                    }
                    while index < bytes.len() && bytes[index].is_ascii_digit() {
                        index += 1;
                    }
                    let number: Option<i32> = std::str::from_utf8(&bytes[number_start..index])
                        .ok()
                        .and_then(|number| number.parse().ok());
                    if bytes.get(index) == Some(&b' ') {
                        index += 1;
                    }
                    if RTF_SKIPPED.contains(&word)
                        && let Some(group) = groups.last_mut()
                    {
                        group.0 = true;
                    }
                    if skipping(&groups) {
                        continue;
                    }
                    match word {
                        "par" | "line" | "row" | "sect" | "page" => out.push('\n'),
                        "tab" | "cell" => out.push('\t'),
                        "emdash" => out.push('—'),
                        "endash" => out.push('–'),
                        "bullet" => out.push('•'),
                        "lquote" => out.push('‘'),
                        "rquote" => out.push('’'),
                        "ldblquote" => out.push('“'),
                        "rdblquote" => out.push('”'),
                        "uc" => {
                            if let (Some(group), Some(count)) = (groups.last_mut(), number) {
                                group.1 = count.max(0) as usize;
                            }
                        }
                        "u" => {
                            if let Some(code) = number {
                                let code = if code < 0 { code + 65_536 } else { code };
                                out.extend(char::from_u32(code as u32));
                                skip_chars = groups.last().map_or(1, |group| group.1);
                            }
                        }
                        _ => {}
                    }
                    continue;
                }
                index += 1;
                match next {
                    b'*' => {
                        if let Some(group) = groups.last_mut() {
                            group.0 = true;
                        }
                    }
                    b'\'' => {
                        let hex = bytes.get(index..index + 2).unwrap_or_default();
                        index += hex.len();
                        if skip_chars > 0 {
                            skip_chars -= 1;
                        } else if !skipping(&groups)
                            && let Some(byte) = std::str::from_utf8(hex)
                                .ok()
                                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                        {
                            out.push(windows_1252(byte));
                        }
                    }
                    b'~' if !skipping(&groups) => out.push('\u{a0}'),
                    b'\\' | b'{' | b'}' if !skipping(&groups) => out.push(next as char),
                    b'\n' | b'\r' if !skipping(&groups) => out.push('\n'),
                    _ => {}
                }
            }
            b'\r' | b'\n' => index += 1,
            _ => {
                index += 1;
                if skip_chars > 0 {
                    skip_chars -= 1;
                } else if !skipping(&groups) {
                    out.push(windows_1252(byte));
                }
            }
        }
    }
    out
}

/// A byte as Windows-1252 reads it, the code page RTF files are mostly written in.
fn windows_1252(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    match byte {
        0x80..=0x9f => HIGH[(byte - 0x80) as usize],
        _ => byte as char,
    }
}

/// Lines without trailing blanks, and no more than one empty line in a row.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_owned()
}

fn truncated(mut text: String) -> String {
    if text.len() > MAX_TEXT_BYTES {
        let mut end = MAX_TEXT_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n[…]");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hoshi-doc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn archive(path: &Path, parts: &[(&str, &str)]) {
        let mut writer = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, content) in parts {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(content.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
    }

    fn extracted(path: &Path) -> String {
        extract(path);
        std::fs::read_to_string(sidecar(path).expect("no text written")).unwrap()
    }

    #[test]
    fn a_word_document_keeps_its_paragraphs_and_tables() {
        let path = folder("docx").join("brief.docx");
        archive(
            &path,
            &[(
                "word/document.xml",
                r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
                <w:p><w:r><w:t>Hello</w:t></w:r><w:r><w:tab/><w:t xml:space="preserve"> world</w:t></w:r></w:p>
                <w:tbl><w:tr><w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
                </w:body></w:document>"#,
            )],
        );

        assert_eq!(extracted(&path), "Hello\t world\nA\tB");
    }

    #[test]
    fn a_workbook_lists_each_sheet_by_name_with_its_cells() {
        let path = folder("xlsx").join("budget.xlsx");
        archive(
            &path,
            &[
                (
                    "xl/workbook.xml",
                    r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Costs" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
                ),
                (
                    "xl/_rels/workbook.xml.rels",
                    r#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/></Relationships>"#,
                ),
                (
                    "xl/sharedStrings.xml",
                    r#"<sst><si><t>Item</t></si><si><r><t>Cos</t></r><r><t>t</t></r></si></sst>"#,
                ),
                (
                    "xl/worksheets/sheet1.xml",
                    r#"<worksheet><sheetData>
                    <row r="1"><c r="A1" t="s"><v>0</v></c><c r="C1" t="s"><v>1</v></c></row>
                    <row r="2"><c r="A2" t="inlineStr"><is><t>Rent</t></is></c><c r="C2"><v>1200</v></c></row>
                    </sheetData></worksheet>"#,
                ),
            ],
        );

        assert_eq!(extracted(&path), "## Costs\nItem\t\tCost\nRent\t\t1200");
    }

    #[test]
    fn cell_columns_stop_at_the_last_excel_column() {
        assert_eq!(column("A1"), Some(0));
        assert_eq!(column("AB12"), Some(27));
        assert_eq!(column("XFD1"), Some(16383));
        assert_eq!(column("XFE1"), None);
        assert_eq!(column("ZZZZZZ1"), None);
        assert_eq!(column("ZZZZZZZZZZZZZZZZZZZZ1"), None);
    }

    #[test]
    fn a_book_that_repeats_a_chapter_stops_at_the_text_limit() {
        let path = folder("epub-loop").join("loop.epub");
        let chapter = format!("<html><body><p>{}</p></body></html>", "x".repeat(1 << 20));
        let spine = r#"<itemref idref="c"/>"#.repeat(10_000);
        archive(
            &path,
            &[
                (
                    "META-INF/container.xml",
                    r#"<container><rootfiles><rootfile full-path="o.opf"/></rootfiles></container>"#,
                ),
                (
                    "o.opf",
                    &format!(
                        r#"<package><manifest><item id="c" href="c.xhtml"/></manifest><spine>{spine}</spine></package>"#
                    ),
                ),
                ("c.xhtml", &chapter),
            ],
        );
        let mut archive = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();

        let text = book_text(&mut archive).unwrap();

        assert!(text.len() < 3 * MAX_TEXT_BYTES);
    }

    #[test]
    fn slides_are_read_in_their_order() {
        let path = folder("pptx").join("deck.pptx");
        let slide = |text: &str| {
            format!(
                r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:sld>"#
            )
        };
        archive(
            &path,
            &[
                ("ppt/slides/slide10.xml", &slide("Ten")),
                ("ppt/slides/slide2.xml", &slide("Two")),
            ],
        );

        assert_eq!(
            extracted(&path),
            "--- Slide 1 ---\nTwo\n\n--- Slide 2 ---\nTen"
        );
    }

    #[test]
    fn an_open_document_keeps_its_paragraphs() {
        let path = folder("odt").join("letter.odt");
        archive(
            &path,
            &[(
                "content.xml",
                r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:text>
                <text:h>Title</text:h><text:p>One<text:s/>two<text:line-break/>three</text:p>
                </office:text></office:body></office:document-content>"#,
            )],
        );

        assert_eq!(extracted(&path), "Title\nOne two\nthree");
    }

    #[test]
    fn a_book_is_read_chapter_by_chapter() {
        let path = folder("epub").join("book.epub");
        archive(
            &path,
            &[
                (
                    "META-INF/container.xml",
                    r#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#,
                ),
                (
                    "OEBPS/content.opf",
                    r#"<package><manifest><item id="c2" href="text/two.xhtml"/><item id="c1" href="text/one.xhtml"/></manifest><spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#,
                ),
                (
                    "OEBPS/text/one.xhtml",
                    "<html><head><style>p{}</style></head><body><h1>One</h1><p>Fish &amp; chips&nbsp;&#233;</p></body></html>",
                ),
                (
                    "OEBPS/text/two.xhtml",
                    "<html><body><p>Two</p></body></html>",
                ),
            ],
        );

        assert_eq!(extracted(&path), "One\n\nFish & chips é\n\nTwo");
    }

    #[test]
    fn a_notebook_keeps_its_cells_and_outputs() {
        let path = folder("ipynb").join("analysis.ipynb");
        std::fs::write(
            &path,
            r##"{"metadata":{"kernelspec":{"language":"python"}},"cells":[
            {"cell_type":"markdown","source":["# Notes\n","Intro"]},
            {"cell_type":"code","source":"print(1)","outputs":[{"output_type":"stream","text":["1\n"]}]}]}"##,
        )
        .unwrap();

        assert_eq!(
            extracted(&path),
            "# Notes\nIntro\n\n```python\nprint(1)\n```\nOutput:\n1"
        );
    }

    #[test]
    fn rtf_text_leaves_its_tables_and_control_words_out() {
        let rtf = [r"{\rtf1\ansi{\fonttbl{\f0 Arial;}}{\*\generator Word;}\f0 Caf\'e9 \b bold\b0\par Line\tab two \", r"u8364?\par}"].concat();

        assert_eq!(rtf_text(rtf.as_bytes()), "Café bold\nLine\ttwo €\n");
    }

    #[test]
    fn other_files_and_broken_documents_get_no_text() {
        let dir = folder("none");
        let plain = dir.join("notes.txt");
        std::fs::write(&plain, "hi").unwrap();
        let broken = dir.join("broken.docx");
        std::fs::write(&broken, "not a zip").unwrap();

        extract(&plain);
        extract(&broken);

        assert_eq!(sidecar(&plain), None);
        assert_eq!(sidecar(&broken), None);
    }
}
