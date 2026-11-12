//! The user message Claude Code receives: the prompt's text, what its mentions
//! stand for, its images, and its files.

use crate::TurnAttachment;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hoshi_core::{MessageAttachment, PullRequestState};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// What a conversation may hold of images and documents, below the API's 32 MB request
/// limit with room for text, instructions and framing.
const MAX_MEDIA_BYTES: u64 = 28 * 1024 * 1024;
/// The API reads at most this many PDF pages in one request.
const MAX_PDF_PAGES: u32 = 100;
/// Larger PDFs are left for the agent to read itself.
const MAX_PDF_BYTES: u64 = 20 * 1024 * 1024;

/// The images and PDF pages a conversation holds, as sent inline to Claude.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MediaBudget {
    bytes: u64,
    pages: u32,
}

impl MediaBudget {
    /// What earlier messages sent inline, deciding as each was decided when it was sent.
    pub(crate) fn after(earlier: &[TurnAttachment]) -> Self {
        let mut budget = Self::default();
        for attachment in earlier {
            match &attachment.attachment {
                MessageAttachment::Image { size, .. } => {
                    budget.take(inline_bytes(*size), 0);
                }
                MessageAttachment::File { .. } => {
                    if let Some((size, pages)) = inline_pdf(attachment) {
                        budget.take(inline_bytes(size), pages);
                    }
                }
                _ => {}
            }
        }
        budget
    }

    /// Takes room for `bytes` and `pages`, when there is.
    fn take(&mut self, bytes: u64, pages: u32) -> bool {
        let fits = self.bytes + bytes <= MAX_MEDIA_BYTES && self.pages + pages <= MAX_PDF_PAGES;
        if fits {
            self.bytes += bytes;
            self.pages += pages;
        }
        fits
    }
}

/// Its size once base64 encoded in a request.
fn inline_bytes(size: u64) -> u64 {
    4 * size.div_ceil(3) + 128
}

/// The message's `content`: plain text unless it carries images, documents or a skill.
/// What it sends inline is taken from `media`.
pub(crate) fn message_content(
    prompt: &str,
    attachments: &[TurnAttachment],
    media: &mut MediaBudget,
) -> Value {
    let skills: HashSet<&str> = attachments
        .iter()
        .filter_map(|attachment| match &attachment.attachment {
            MessageAttachment::Skill { name } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    let (text, last_skill) = with_slash_skills(prompt, &skills);
    let text = with_context(text, attachments);

    let mut blocks = Vec::new();
    match last_skill {
        Some(start) => {
            if !text[..start].trim().is_empty() {
                blocks.push(json!({ "type": "text", "text": text }));
            }
            blocks.push(json!({ "type": "text", "text": &text[start..] }));
        }
        None if !text.is_empty() => blocks.push(json!({ "type": "text", "text": text })),
        None => {}
    }
    media.take(text.len() as u64, 0);
    // Images first, so a document never takes the room of an image sent with it.
    let images: Vec<Option<Value>> = attachments
        .iter()
        .map(|attachment| image_block(attachment, media))
        .collect();
    for (attachment, image) in attachments.iter().zip(images) {
        match image {
            Some(block) => blocks.push(block),
            None => blocks.extend(document_block(attachment, media)),
        }
    }
    match blocks.as_slice() {
        [block] if block["type"] == "text" && last_skill.is_none() => block["text"].clone(),
        _ => Value::Array(blocks),
    }
}

/// Writes each `$name` of a known skill as `/name`, and says where the last one starts.
/// Claude expands a skill only when `/name` opens a text block, and only the last one.
fn with_slash_skills(prompt: &str, skills: &HashSet<&str>) -> (String, Option<usize>) {
    let mut text = String::with_capacity(prompt.len());
    let mut last = None;
    let mut rest = prompt;
    while let Some(index) = rest.find('$') {
        let (before, mention) = rest.split_at(index);
        text.push_str(before);
        let at_word_start = text.is_empty() || text.ends_with(char::is_whitespace);
        let name_end = mention[1..]
            .find(char::is_whitespace)
            .map_or(mention.len(), |end| end + 1);
        let name = &mention[1..name_end];
        if at_word_start && skills.contains(name) {
            last = Some(text.len());
            text.push('/');
            text.push_str(name);
            rest = &mention[name_end..];
        } else {
            text.push('$');
            rest = &mention[1..];
        }
    }
    text.push_str(rest);
    (text, last)
}

/// Adds a line per image, file and pull request, so the agent knows where each image and
/// file is saved, which one an `[Image #n]` or `[File #n]` in the prompt is, and what the
/// `#123` mentions are. Images and files are each numbered in the order they are attached.
fn with_context(mut text: String, attachments: &[TurnAttachment]) -> String {
    let (mut images, mut files) = (0, 0);
    for attachment in attachments {
        let line = match (&attachment.attachment, &attachment.path) {
            (MessageAttachment::Image { name, .. }, path) => {
                images += 1;
                let Some(path) = path else { continue };
                format!(
                    "[Image #{images} \"{name}\" is saved at: {}]",
                    path.display()
                )
            }
            (MessageAttachment::File { name, .. }, Some(path)) => {
                files += 1;
                let extracted = attachment
                    .extracted_text
                    .as_ref()
                    .map(|text| format!(" (text extracted to: {})", text.display()))
                    .unwrap_or_default();
                format!(
                    "[File #{files} \"{name}\" is saved at: {}{extracted}]",
                    path.display()
                )
            }
            (MessageAttachment::File { name, .. }, None) => {
                files += 1;
                format!("[File #{files} \"{name}\" is unavailable: it could not be found]")
            }
            (
                MessageAttachment::PullRequest {
                    number,
                    title,
                    url,
                    head_branch,
                    base_branch,
                    state,
                },
                _,
            ) => format!(
                "[Pull request #{number} \"{title}\" ({}, {head_branch} into {base_branch}): {url}]",
                state_name(*state)
            ),
            _ => continue,
        };
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&line);
    }
    text
}

fn state_name(state: PullRequestState) -> &'static str {
    match state {
        PullRequestState::Open => "open",
        PullRequestState::Draft => "draft",
        PullRequestState::Merged => "merged",
        PullRequestState::Closed => "closed",
    }
}

/// An image too large for what the conversation has left, or unreadable, is left out;
/// its line still tells the agent where it is.
fn image_block(attachment: &TurnAttachment, media: &mut MediaBudget) -> Option<Value> {
    let MessageAttachment::Image { mime_type, .. } = &attachment.attachment else {
        return None;
    };
    let bytes = std::fs::read(attachment.path.as_ref()?).ok()?;
    if !media.take(inline_bytes(bytes.len() as u64), 0) {
        return None;
    }
    Some(json!({
        "type": "image",
        "source": {
            "type": "base64",
            "media_type": mime_type,
            "data": STANDARD.encode(bytes),
        },
    }))
}

/// A PDF Claude reads whole, when its pages can be counted and it fits what the
/// conversation has left; any other file it reads from where its line says.
fn document_block(attachment: &TurnAttachment, media: &mut MediaBudget) -> Option<Value> {
    let (size, pages) = inline_pdf(attachment)?;
    if !media.take(inline_bytes(size), pages) {
        return None;
    }
    let bytes = std::fs::read(attachment.path.as_ref()?).ok()?;
    Some(json!({
        "type": "document",
        "source": {
            "type": "base64",
            "media_type": "application/pdf",
            "data": STANDARD.encode(bytes),
        },
    }))
}

/// The size and pages of a PDF small enough to send inline; `None` for other files and
/// for PDFs whose pages can't be counted.
fn inline_pdf(attachment: &TurnAttachment) -> Option<(u64, u32)> {
    let MessageAttachment::File { mime_type, .. } = &attachment.attachment else {
        return None;
    };
    let essence = mime_type.split(';').next().unwrap_or_default().trim();
    if !essence.eq_ignore_ascii_case("application/pdf") {
        return None;
    }
    let path = attachment.path.as_ref()?;
    let size = std::fs::metadata(path).ok()?.len();
    if size == 0 || size > MAX_PDF_BYTES {
        return None;
    }
    Some((size, pdf_pages(path)?))
}

/// The pages of the PDF at `path`, counted once per file.
fn pdf_pages(path: &Path) -> Option<u32> {
    static COUNTED: OnceLock<Mutex<HashMap<PathBuf, Option<u32>>>> = OnceLock::new();
    let counted = COUNTED.get_or_init(Default::default);
    if let Some(pages) = counted.lock().ok()?.get(path) {
        return *pages;
    }
    let pages = std::fs::read(path)
        .ok()
        .and_then(|bytes| count_pdf_pages(&bytes));
    counted.lock().ok()?.insert(path.to_owned(), pages);
    pages
}

const PDF_DELIMITERS: &[u8] = b"()<>[]{}/%";

fn pdf_delimiter(byte: u8) -> bool {
    byte <= 32 || PDF_DELIMITERS.contains(&byte)
}

fn find(bytes: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| at + from)
}

/// The pages of a PDF, counted only from an uncompressed page tree whose `/Count`
/// agrees with the page objects it has; `None` for anything else, rather than decoding
/// streams to count them.
fn count_pdf_pages(bytes: &[u8]) -> Option<u32> {
    if bytes.len() as u64 > MAX_PDF_BYTES || !bytes.starts_with(b"%PDF-") {
        return None;
    }
    let tail = &bytes[bytes.len().saturating_sub(1024)..];
    find(tail, b"%%EOF", 0)?;
    let (mut pages, mut count) = (0u32, 0u32);
    let (mut catalog, mut page_tree) = (false, false);
    let mut previous: &[u8] = b"";
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'%' {
            while index < bytes.len() && bytes[index] != b'\n' && bytes[index] != b'\r' {
                index += 1;
            }
            continue;
        }
        if byte <= 32 {
            index += 1;
            continue;
        }
        if previous == b"/Type" && byte != b'/' {
            return None;
        }
        if previous == b"/Kids" {
            if byte != b'[' {
                return None;
            }
            previous = b"";
        }
        if byte == b'(' {
            let mut depth = 1;
            index += 1;
            while index < bytes.len() && depth > 0 {
                match bytes[index] {
                    b'\\' => index += 1,
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                index += 1;
            }
            if depth != 0 {
                return None;
            }
            previous = b"";
            continue;
        }
        if byte == b'<' {
            if bytes.get(index + 1) == Some(&b'<') {
                index += 2;
            } else {
                index = find(bytes, b">", index + 1)? + 1;
            }
            previous = b"";
            continue;
        }
        if pdf_delimiter(byte) && byte != b'/' {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        while index < bytes.len() && !pdf_delimiter(bytes[index]) {
            index += 1;
        }
        if index - start > 64 {
            return None;
        }
        let token = &bytes[start..index];
        if token.contains(&b'#')
            || [&b"/ObjStm"[..], b"/XRef", b"/Encrypt", b"/Prev"].contains(&token)
        {
            return None;
        }
        if token == b"stream" {
            index = find(bytes, b"endstream", index)? + 9;
            previous = b"";
            continue;
        }
        if previous == b"/Type" {
            match token {
                b"/Page" => pages += 1,
                b"/Catalog" => catalog = true,
                b"/Pages" => page_tree = true,
                _ => {}
            }
        }
        if previous == b"/Count" {
            let number = std::str::from_utf8(token).ok()?;
            let valid = number.len() <= 9
                && number.bytes().all(|digit| digit.is_ascii_digit())
                && !number.starts_with('0');
            if !valid {
                return None;
            }
            let mut next = index;
            while next < bytes.len() {
                if bytes[next] <= 32 {
                    next += 1;
                } else if bytes[next] == b'%' {
                    while next < bytes.len() && bytes[next] != b'\n' && bytes[next] != b'\r' {
                        next += 1;
                    }
                } else {
                    break;
                }
            }
            // A second number makes it an indirect reference, not the page count.
            if next < bytes.len() && !pdf_delimiter(bytes[next]) {
                return None;
            }
            count = count.max(number.parse().ok()?);
        }
        previous = token;
    }
    (catalog && page_tree && pages > 0 && count == pages).then_some(pages)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(name: &str) -> TurnAttachment {
        TurnAttachment {
            attachment: MessageAttachment::Skill { name: name.into() },
            path: None,
            extracted_text: None,
        }
    }

    fn content(prompt: &str, attachments: &[TurnAttachment]) -> Value {
        message_content(prompt, attachments, &mut MediaBudget::default())
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hoshi-prompt-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A PDF of `pages` empty pages, with an uncompressed page tree.
    fn pdf(pages: u32) -> Vec<u8> {
        let kids: Vec<String> = (0..pages).map(|page| format!("{} 0 R", page + 3)).collect();
        let mut pdf = format!(
            "%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [{}] /Count {pages} >> endobj\n",
            kids.join(" ")
        );
        for page in 0..pages {
            pdf.push_str(&format!(
                "{} 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >> endobj\n",
                page + 3
            ));
        }
        pdf.push_str("trailer << /Root 1 0 R >>\n%%EOF\n");
        pdf.into_bytes()
    }

    fn file(name: &str, mime_type: &str, path: Option<PathBuf>) -> TurnAttachment {
        TurnAttachment {
            attachment: MessageAttachment::File {
                id: name.into(),
                name: name.into(),
                mime_type: mime_type.into(),
                size: 1,
            },
            path,
            extracted_text: None,
        }
    }

    #[test]
    fn a_plain_prompt_stays_plain_text() {
        assert_eq!(content("fix it, costs $5", &[]), json!("fix it, costs $5"));
    }

    #[test]
    fn a_skill_mentioned_mid_prompt_opens_its_own_block() {
        assert_eq!(
            content("check this with $pdf please", &[skill("pdf")]),
            json!([
                { "type": "text", "text": "check this with /pdf please" },
                { "type": "text", "text": "/pdf please" },
            ])
        );
    }

    #[test]
    fn a_skill_at_the_start_is_one_block_and_unknown_dollars_stay() {
        assert_eq!(
            content("$pdf read a$pdf and $other", &[skill("pdf")]),
            json!([{ "type": "text", "text": "/pdf read a$pdf and $other" }])
        );
    }

    #[test]
    fn pull_requests_and_images_add_context_and_image_blocks() {
        let dir = folder("images");
        let path = dir.join("shot.png");
        std::fs::write(&path, b"png").unwrap();
        let image = |id: &str, name: &str, path: PathBuf| TurnAttachment {
            attachment: MessageAttachment::Image {
                id: id.into(),
                name: name.into(),
                mime_type: "image/png".into(),
                size: 3,
            },
            path: Some(path),
            extracted_text: None,
        };
        let attachments = [
            TurnAttachment {
                attachment: MessageAttachment::PullRequest {
                    number: 12,
                    title: "Add login".into(),
                    url: "https://github.com/o/r/pull/12".into(),
                    head_branch: "login".into(),
                    base_branch: "main".into(),
                    state: PullRequestState::Open,
                },
                path: None,
                extracted_text: None,
            },
            image("i", "shot.png", path.clone()),
            image("gone", "gone.png", PathBuf::from("/nowhere/gone.png")),
        ];

        let text = format!(
            "review #12\n\n[Pull request #12 \"Add login\" (open, login into main): https://github.com/o/r/pull/12]\n\n[Image #1 \"shot.png\" is saved at: {}]\n\n[Image #2 \"gone.png\" is saved at: {}]",
            path.display(),
            PathBuf::from("/nowhere/gone.png").display()
        );
        assert_eq!(
            content("review #12", &attachments),
            json!([
                { "type": "text", "text": text },
                {
                    "type": "image",
                    "source": { "type": "base64", "media_type": "image/png", "data": "cG5n" },
                },
            ])
        );
    }

    #[test]
    fn files_are_named_where_they_are_saved_and_a_pdf_is_sent_whole() {
        let dir = folder("files");
        let report = dir.join("report.pdf");
        std::fs::write(&report, pdf(2)).unwrap();
        let notes = dir.join("notes.docx");
        std::fs::write(&notes, b"docx").unwrap();
        let mut document = file(
            "notes.docx",
            "application/octet-stream",
            Some(notes.clone()),
        );
        document.extracted_text = Some(dir.join("notes.docx.txt"));
        let attachments = [
            file("report.pdf", "application/pdf", Some(report.clone())),
            document,
            file("gone.txt", "text/plain", None),
        ];

        let content = content("[File #1] and [File #2]", &attachments);

        let text = format!(
            "[File #1] and [File #2]\n\n[File #1 \"report.pdf\" is saved at: {}]\n\n[File #2 \"notes.docx\" is saved at: {} (text extracted to: {})]\n\n[File #3 \"gone.txt\" is unavailable: it could not be found]",
            report.display(),
            notes.display(),
            dir.join("notes.docx.txt").display()
        );
        assert_eq!(content[0], json!({ "type": "text", "text": text }));
        assert_eq!(content[1]["type"], "document");
        assert_eq!(content[1]["source"]["media_type"], "application/pdf");
        assert_eq!(
            content[1]["source"]["data"],
            STANDARD.encode(pdf(2)).as_str()
        );
        assert_eq!(content.as_array().unwrap().len(), 2);
    }

    #[test]
    fn pdfs_past_the_conversation_s_pages_are_left_to_read_from_disk() {
        let dir = folder("budget");
        let big = dir.join("big.pdf");
        std::fs::write(&big, pdf(60)).unwrap();
        let earlier = [file("big.pdf", "application/pdf", Some(big.clone()))];
        let mut media = MediaBudget::after(&earlier);

        let content = message_content(
            "again",
            &[file("big.pdf", "application/pdf", Some(big))],
            &mut media,
        );

        assert!(content.is_string(), "{content}");
        assert_eq!(media.pages, 60);
    }

    #[test]
    fn pages_are_counted_only_from_a_page_tree_that_agrees() {
        assert_eq!(count_pdf_pages(&pdf(3)), Some(3));
        let mut lying = String::from_utf8(pdf(3)).unwrap();
        lying = lying.replace("/Count 3", "/Count 4");
        assert_eq!(count_pdf_pages(lying.as_bytes()), None);
        let compressed = String::from_utf8(pdf(1))
            .unwrap()
            .replace("trailer", "4 0 obj << /Type /ObjStm >> endobj\ntrailer");
        assert_eq!(count_pdf_pages(compressed.as_bytes()), None);
        assert_eq!(count_pdf_pages(b"not a pdf"), None);
    }
}
