//! A sent message's mentions as chips, as the composer showed them: `@file`, the `/command` it
//! starts with, and the `#123`, `$name`, `[Image #n]` and `[File #n]` its attachments stand
//! for. Each one becomes an inline tag before the text view parses the markdown, so it is a
//! node of its own that a chip draws whole.

use super::{images, open_local};
use gpui_kit::component::text::{InlineElement, InlineRenderContext, MarkdownNode};
use gpui_kit::component::text::{MarkdownParseContext, MarkdownPlugin, markdown_ast};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_core::MessageAttachment;
use hoshi_ui::assets::file_icon;
use hoshi_ui::prelude::{ActiveTheme as _, PressSelectsNothing as _, code_language_icon};
use markdown::mdast::Node;
use std::sync::Arc;

const TAG: &str = "hoshi-mention";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MentionKind {
    File,
    Command,
    PullRequest,
    Skill,
    Image,
    AttachedFile,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub kind: MentionKind,
    pub range: std::ops::Range<usize>,
    /// The image or file an `[Image #n]` or `[File #n]` stands for.
    pub attachment: Option<MessageAttachment>,
}

/// What a message's attachments let its text mention.
#[derive(Clone, Default)]
pub struct Known {
    pull_requests: Vec<u32>,
    skills: Vec<String>,
    images: Vec<MessageAttachment>,
    files: Vec<MessageAttachment>,
}

impl Known {
    pub fn of(attachments: &[MessageAttachment]) -> Self {
        let mut known = Self::default();
        for attachment in attachments {
            match attachment {
                MessageAttachment::PullRequest { number, .. } => known.pull_requests.push(*number),
                MessageAttachment::Skill { name } => known.skills.push(name.clone()),
                MessageAttachment::Image { .. } => known.images.push(attachment.clone()),
                MessageAttachment::File { .. } => known.files.push(attachment.clone()),
            }
        }
        known
    }
}

fn starts_word(text: &str, at: usize) -> bool {
    text[..at]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace)
}

fn ends_word(text: &str, at: usize, also: &str) -> bool {
    text[at..]
        .chars()
        .next()
        .is_none_or(|next| next.is_whitespace() || also.contains(next))
}

/// The `@path` at `at`: a quoted one, or a run of non-space without its trailing punctuation.
/// `None` unless it reads as a path, so a person, a handle or a decorator stays text.
fn file_mention(text: &str, at: usize) -> Option<usize> {
    let rest = &text[at + 1..];
    if let Some(quoted) = rest.strip_prefix('"') {
        let close = quoted.find(['"', '\n'])?;
        let path = &quoted[..close];
        return (quoted[close..].starts_with('"') && close > 0 && looks_like_path(path))
            .then_some(at + 1 + 1 + close + 1);
    }
    let run = rest
        .find(|char: char| char.is_whitespace() || char == '"')
        .unwrap_or(rest.len());
    let kept = rest[..run].trim_end_matches(['.', ',', ';', ':', '!', '?', ')']);
    (!kept.is_empty() && looks_like_path(kept)).then_some(at + 1 + kept.len())
}

/// Files projects keep at their root without an extension.
const BARE_FILES: [&str; 19] = [
    "Makefile",
    "makefile",
    "Dockerfile",
    "Containerfile",
    "Justfile",
    "justfile",
    "Gemfile",
    "Rakefile",
    "Procfile",
    "Jenkinsfile",
    "Vagrantfile",
    "LICENSE",
    "README",
    "CODEOWNERS",
    "CHANGELOG",
    "AUTHORS",
    "NOTICE",
    "COPYING",
    "VERSION",
];

/// A path the composer could have written: in a folder, with an extension, or a file projects
/// keep without one.
fn looks_like_path(name: &str) -> bool {
    if name.contains(['/', '\\']) || BARE_FILES.contains(&name) {
        return true;
    }
    name.rsplit_once('.').is_some_and(|(_, extension)| {
        !extension.is_empty() && extension.chars().all(char::is_alphanumeric)
    })
}

fn numbered(text: &str, at: usize, prefix: &str) -> Option<(usize, usize)> {
    let rest = text[at..].strip_prefix(prefix)?;
    let digits = rest.find(|char: char| !char.is_ascii_digit())?;
    if digits == 0 || !rest[digits..].starts_with(']') {
        return None;
    }
    let number = rest[..digits].parse().ok()?;
    Some((number, at + prefix.len() + digits + 1))
}

/// The chips of `text`, in order; `leading` when `text` starts the message, where a `/command`
/// may stand.
pub fn segments(text: &str, known: &Known, leading: bool) -> Vec<Segment> {
    let mut chips: Vec<Segment> = Vec::new();
    let chip = |kind, range, attachment| Segment {
        kind,
        range,
        attachment,
    };
    if leading {
        let start = text.len() - text.trim_start().len();
        if text[start..].starts_with('/') {
            let end = text[start + 1..]
                .find(|char: char| !(char.is_alphanumeric() || "_:-".contains(char)))
                .map_or(text.len(), |length| start + 1 + length);
            if end > start + 1 && ends_word(text, end, "") {
                chips.push(chip(MentionKind::Command, start..end, None));
            }
        }
    }
    for (at, char) in text.char_indices() {
        match char {
            '@' if starts_word(text, at) => {
                if let Some(end) = file_mention(text, at) {
                    chips.push(chip(MentionKind::File, at..end, None));
                }
            }
            '#' if starts_word(text, at) => {
                let digits = text[at + 1..]
                    .find(|char: char| !char.is_ascii_digit())
                    .unwrap_or(text.len() - at - 1);
                let end = at + 1 + digits;
                let known_pr = text[at + 1..end]
                    .parse()
                    .is_ok_and(|number: u32| known.pull_requests.contains(&number));
                if digits > 0 && known_pr && ends_word(text, end, ".,;:!?)") {
                    chips.push(chip(MentionKind::PullRequest, at..end, None));
                }
            }
            '$' if starts_word(text, at) => {
                let name_end = text[at + 1..]
                    .find(|char: char| !(char.is_alphanumeric() || "_:-".contains(char)))
                    .map_or(text.len(), |length| at + 1 + length);
                let name = &text[at + 1..name_end];
                let valid = name
                    .chars()
                    .next()
                    .is_some_and(|first| first.is_ascii_alphabetic());
                if valid
                    && known.skills.iter().any(|skill| skill == name)
                    && ends_word(text, name_end, "")
                {
                    chips.push(chip(MentionKind::Skill, at..name_end, None));
                }
            }
            '[' => {
                if let Some((number, end)) = numbered(text, at, "[Image #") {
                    if let Some(image) = known.images.get(number.wrapping_sub(1)) {
                        chips.push(chip(MentionKind::Image, at..end, Some(image.clone())));
                    }
                } else if let Some((number, end)) = numbered(text, at, "[File #")
                    && let Some(file) = known.files.get(number.wrapping_sub(1))
                {
                    chips.push(chip(MentionKind::AttachedFile, at..end, Some(file.clone())));
                }
            }
            _ => {}
        }
    }
    chips.sort_by_key(|chip| chip.range.start);
    let mut kept: Vec<Segment> = Vec::new();
    for chip in chips {
        if kept
            .last()
            .is_none_or(|last| chip.range.start >= last.range.end)
        {
            kept.push(chip);
        }
    }
    kept
}

/// What a message's text nodes hold that the view draws differently: mentions, and line
/// breaks, which a message keeps as written.
#[derive(Default)]
struct Found {
    mentions: Vec<(usize, Segment)>,
    /// Each line break, through the indent that follows it, and what draws it as written.
    breaks: Vec<(std::ops::Range<usize>, String)>,
}

/// A block that each line of what it holds starts with a prefix of.
#[derive(Clone, Copy)]
enum Container {
    Quote,
    /// A list item, whose lines start with this many columns of indent.
    ListItem(usize),
}

fn collect(
    node: &Node,
    source: &str,
    first: usize,
    containers: &mut Vec<Container>,
    found: &mut Found,
    known: &Known,
) {
    let container = match node {
        Node::Blockquote(_) => Some(Container::Quote),
        Node::ListItem(item) => item
            .position
            .as_ref()
            .map(|position| Container::ListItem(list_indent(source, position.start.offset))),
        _ => None,
    };
    if let Some(container) = container {
        containers.push(container);
        for child in node.children().into_iter().flatten() {
            collect(child, source, first, containers, found, known);
        }
        containers.pop();
        return;
    }
    match node {
        Node::Code(_) | Node::InlineCode(_) | Node::Link(_) | Node::Html(_) => {}
        Node::Text(text) => {
            let Some(position) = &text.position else {
                return;
            };
            let start = position.start.offset;
            let Some(written) = source.get(start..position.end.offset) else {
                return;
            };
            if let Some(kept) = kept_bytes(written, &text.value) {
                for segment in segments(&text.value, known, start == first) {
                    let range = kept[segment.range.start]..kept[segment.range.end - 1] + 1;
                    found.mentions.push((start, Segment { range, ..segment }));
                }
            }
            for (at, _) in written.match_indices('\n') {
                found
                    .breaks
                    .push(line_break(source, start + at, containers));
            }
        }
        _ => {
            for child in node.children().into_iter().flatten() {
                collect(child, source, first, containers, found, known);
            }
        }
    }
}

/// The markdown with each mention as a tag and each line break kept, and the mentions in the
/// order the tags count them.
pub fn tag(text: &str, known: &Known) -> (String, Vec<Mention>) {
    let Ok(tree) = markdown::to_mdast(text, &markdown::ParseOptions::gfm()) else {
        return (text.to_owned(), Vec::new());
    };
    let first = text.len() - text.trim_start().len();
    let mut found = Found::default();
    collect(&tree, text, first, &mut Vec::new(), &mut found, known);
    let mentions: Vec<Mention> = found
        .mentions
        .iter()
        .map(|(offset, segment)| Mention {
            kind: segment.kind,
            text: text[offset + segment.range.start..offset + segment.range.end].into(),
            attachment: segment.attachment.clone(),
        })
        .collect();
    let mut edits: Vec<(std::ops::Range<usize>, String)> = found
        .mentions
        .iter()
        .enumerate()
        .map(|(index, (offset, segment))| {
            let range = offset + segment.range.start..offset + segment.range.end;
            let lead = if alone_on_its_line(text, &range) {
                NOT_A_BLOCK
            } else {
                ""
            };
            (range, format!(r#"{lead}<{TAG} i="{index}"/>"#))
        })
        .chain(found.breaks)
        .collect();
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut out = text.to_owned();
    for (range, replacement) in edits {
        out.replace_range(range, &replacement);
    }
    (out, mentions)
}

/// For each byte of `value`, where it stands in `written`, when `value` is `written` without
/// the spaces markdown drops around its line breaks.
fn kept_bytes(written: &str, value: &str) -> Option<Vec<usize>> {
    let bytes = written.as_bytes();
    let blank = |byte: u8| byte == b' ' || byte == b'\t';
    let mut kept: Vec<usize> = Vec::with_capacity(value.len());
    let mut line_start = false;
    for (at, &byte) in bytes.iter().enumerate() {
        if line_start && (blank(byte) || byte == b'>') {
            continue;
        }
        if byte == b'\n' {
            while kept.last().is_some_and(|&last| blank(bytes[last])) {
                kept.pop();
            }
        }
        line_start = byte == b'\n';
        kept.push(at);
    }
    (kept.len() == value.len()
        && kept
            .iter()
            .zip(value.bytes())
            .all(|(&at, b)| bytes[at] == b))
    .then_some(kept)
}

/// The line break at `at` as a hard break. The next line keeps the prefix of the quotes and
/// list items its paragraph stands in, and the indent past it in spaces markdown keeps. A
/// line that leaves out part of that prefix keeps only the break.
fn line_break(
    source: &str,
    at: usize,
    containers: &[Container],
) -> (std::ops::Range<usize>, String) {
    let mut line = Blanks::new(&source[at + 1..]);
    let mut prefix = String::new();
    for container in containers {
        match *container {
            Container::Quote => {
                line.take(3);
                if !line.next_is('>') {
                    return (at..at + 1, "\\\n".to_owned());
                }
                line.at += 1;
                line.column += 1;
                line.take(1);
                prefix.push_str("> ");
            }
            Container::ListItem(indent) => {
                if line.take(indent) < indent {
                    return (at..at + 1, "\\\n".to_owned());
                }
                prefix.push_str(&" ".repeat(indent));
            }
        }
    }
    let indent = line.take(usize::MAX);
    (
        at..at + 1 + line.at,
        format!("\\\n{prefix}{}", "\u{a0}".repeat(indent)),
    )
}

/// Reads the spaces and tabs a line starts with by columns, tab stops every 4, so a tab can
/// be taken in part.
struct Blanks<'a> {
    line: &'a str,
    at: usize,
    column: usize,
    /// Columns left of a tab already read.
    spare: usize,
}

impl<'a> Blanks<'a> {
    fn new(line: &'a str) -> Self {
        Self {
            line,
            at: 0,
            column: 0,
            spare: 0,
        }
    }

    /// Takes up to `most` columns of blanks; how many it took.
    fn take(&mut self, most: usize) -> usize {
        let mut taken = 0;
        while taken < most {
            if self.spare > 0 {
                let part = self.spare.min(most - taken);
                self.spare -= part;
                taken += part;
                continue;
            }
            match self.line.as_bytes().get(self.at) {
                Some(b' ') => {
                    self.at += 1;
                    self.column += 1;
                    taken += 1;
                }
                Some(b'\t') => {
                    let tab = 4 - self.column % 4;
                    self.at += 1;
                    self.column += tab;
                    self.spare = tab;
                }
                _ => break,
            }
        }
        taken
    }

    fn next_is(&self, char: char) -> bool {
        self.spare == 0 && self.line[self.at..].starts_with(char)
    }
}

/// The columns of indent the lines of the list item at `start` start with: the item's own
/// indent, its marker and the spaces after it, but not a task's checkbox.
fn list_indent(source: &str, start: usize) -> usize {
    let line_start = source[..start].rfind('\n').map_or(0, |at| at + 1);
    let column = |offset: usize| width(&source[line_start..offset]);
    let before = &source[line_start..start];
    let mut from = line_start + before.trim_end_matches([' ', '\t']).len();
    if from > line_start && source[..from].ends_with('>') && source[from..].starts_with(' ') {
        from += 1;
    }
    let rest = &source[start..];
    let marker = start + rest.len() - rest.trim_start_matches([' ', '\t']).len();
    let digits = source[marker..]
        .find(|char: char| !char.is_ascii_digit())
        .unwrap_or(0);
    let marker_end = marker + digits + 1;
    let after = &source[marker_end..];
    let content = marker_end + after.len() - after.trim_start_matches([' ', '\t']).len();
    let gap = column(content) - column(marker_end);
    let blank = source[content..].starts_with('\n') || content == source.len();
    let content_column = if gap == 0 || gap > 4 || blank {
        column(marker_end) + 1
    } else {
        column(content)
    };
    content_column - column(from)
}

/// How many columns `text` spans from the start of a line, with tab stops every 4.
fn width(text: &str) -> usize {
    text.chars().fold(0, |columns, char| {
        if char == '\t' {
            columns + 4 - columns % 4
        } else {
            columns + 1
        }
    })
}

/// Leads a tag that would otherwise stand alone on its line, which markdown reads as a block
/// of HTML rather than a mention within the text. It draws nothing.
const NOT_A_BLOCK: &str = "\u{200B}";

/// Whether only whitespace and list or quote markers share `range`'s line.
fn alone_on_its_line(text: &str, range: &std::ops::Range<usize>) -> bool {
    let start = text[..range.start].rfind('\n').map_or(0, |at| at + 1);
    let end = text[range.end..]
        .find('\n')
        .map_or(text.len(), |at| range.end + at);
    let marker =
        |char: char| char.is_whitespace() || char.is_ascii_digit() || "->*+.)".contains(char);
    text[range.end..end].trim().is_empty() && text[start..range.start].chars().all(marker)
}

#[derive(Clone, Debug)]
pub struct Mention {
    pub kind: MentionKind,
    /// As written in the message.
    pub text: SharedString,
    pub attachment: Option<MessageAttachment>,
}

/// Turns a message's mention tags into chips.
pub struct Mentions(pub Arc<Vec<Mention>>);

impl MarkdownPlugin for Mentions {
    fn name(&self) -> &str {
        TAG
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        _: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let markdown_ast::Node::Html(html) = node else {
            return None;
        };
        let index: usize = html
            .value
            .strip_prefix(&format!(r#"<{TAG} i=""#))?
            .split('"')
            .next()?
            .parse()
            .ok()?;
        let mention = self.0.get(index)?.clone();
        Some(MarkdownNode::new(TAG, mention.clone()).text(mention.text))
    }

    fn render_inline(
        &self,
        node: &MarkdownNode,
        context: &InlineRenderContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<InlineElement> {
        let mention = node.data::<Mention>()?;
        let line_height = context.line_height();
        let font_size = context.font_size();
        let chip = chip(
            mention,
            node.source_range().map_or(0, |range| range.start),
            font_size,
            cx,
        );
        Some(
            InlineElement::new(chip.h(line_height))
                .with_baseline(line_height / 2. + font_size * 0.35),
        )
    }
}

fn size_label(attachment: &MessageAttachment) -> Option<SharedString> {
    let size = match attachment {
        MessageAttachment::Image { size, .. } | MessageAttachment::File { size, .. } => *size,
        _ => return None,
    };
    (size > 0).then(|| crate::composer::format_size(size).into())
}

fn attachment_name(attachment: &MessageAttachment) -> SharedString {
    match attachment {
        MessageAttachment::Image { name, .. } | MessageAttachment::File { name, .. } => {
            name.clone().into()
        }
        _ => SharedString::default(),
    }
}

/// A mention drawn as the composer draws it: tinted, and an attachment with its preview or
/// icon, name and size.
pub fn chip(mention: &Mention, at: usize, font_size: Pixels, cx: &App) -> Stateful<Div> {
    let colors = cx.hoshi().clone();
    let tint = match mention.kind {
        MentionKind::PullRequest => colors.pr_open,
        MentionKind::Skill => colors.provider_claude,
        _ => colors.link,
    };
    let base = div()
        .id(("mention", at))
        .flex()
        .items_center()
        .px(px(2.))
        .rounded_sm()
        .bg(tint.opacity(0.12))
        .text_color(tint)
        .text_size(font_size)
        .whitespace_nowrap();
    let Some(attachment) = mention.attachment.clone() else {
        return base.child(mention.text.clone());
    };
    let name = attachment_name(&attachment);
    let size = size_label(&attachment);
    let icon_size = font_size;
    let lead = match &attachment {
        MessageAttachment::Image { id, mime_type, .. } => {
            images::image(id, mime_type, cx).map(|image| {
                img(ImageSource::Image(image))
                    .size(icon_size)
                    .rounded(px(3.))
                    .object_fit(ObjectFit::Cover)
                    .into_any_element()
            })
        }
        _ => {
            let (icon, color) =
                code_language_icon(name.rsplit_once('.').map_or("", |(_, extension)| extension));
            Some(
                svg()
                    .path(file_icon(icon))
                    .size(icon_size)
                    .flex_none()
                    .text_color(color(&colors))
                    .into_any_element(),
            )
        }
    };
    let muted = colors.muted_foreground;
    base.gap_1()
        .cursor_pointer()
        .children(lead)
        .child(name.clone())
        .when_some(size, |chip, size| {
            chip.child(
                div()
                    .text_size(font_size * 0.85)
                    .text_color(muted)
                    .child(size),
            )
        })
        .press_selects_nothing()
        .on_click(move |_, window, cx| open_attachment(&attachment, window, cx))
}

/// An image opens large over the window; a file where the system or the editor opens it.
pub fn open_attachment(attachment: &MessageAttachment, window: &mut Window, cx: &mut App) {
    match attachment {
        MessageAttachment::Image {
            id,
            mime_type,
            name,
            ..
        } => images::show(id.clone(), mime_type.clone(), name.clone(), window, cx),
        MessageAttachment::File { .. } => {
            let Some(links) = cx.try_global::<super::Links>() else {
                return;
            };
            if let Some(path) = links.engine.attachment_path(attachment) {
                open_local(path, cx);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{Known, MentionKind, segments, tag};
    use hoshi_core::{MessageAttachment, PullRequestState};

    fn known() -> Known {
        Known::of(&[
            MessageAttachment::PullRequest {
                number: 12,
                title: String::new(),
                url: String::new(),
                head_branch: String::new(),
                base_branch: String::new(),
                state: PullRequestState::Open,
            },
            MessageAttachment::Skill {
                name: "review".into(),
            },
            MessageAttachment::Image {
                id: "a1".into(),
                name: "shot.png".into(),
                mime_type: "image/png".into(),
                size: 2048,
            },
        ])
    }

    #[test]
    fn mentions_the_attachments_stand_for_become_chips() {
        fn kinds(text: &str, leading: bool) -> Vec<(MentionKind, &str)> {
            segments(text, &known(), leading)
                .into_iter()
                .map(|segment| (segment.kind, &text[segment.range]))
                .collect()
        }
        assert_eq!(
            kinds(
                "/review read @src/main.rs, see #12 and #13 with $review [Image #1] [Image #2]",
                true
            ),
            [
                (MentionKind::Command, "/review"),
                (MentionKind::File, "@src/main.rs"),
                (MentionKind::PullRequest, "#12"),
                (MentionKind::Skill, "$review"),
                (MentionKind::Image, "[Image #1]"),
            ]
        );
        assert_eq!(kinds("a/b and email@x.com", true), []);
        assert_eq!(
            kinds(
                "@Test User, @nxtvoid, @Override and @Injectable() with @.env @Makefile @CHANGELOG @docs/",
                false
            ),
            [
                (MentionKind::File, "@.env"),
                (MentionKind::File, "@Makefile"),
                (MentionKind::File, "@CHANGELOG"),
                (MentionKind::File, "@docs/"),
            ]
        );
        assert_eq!(kinds(r#"cc @"Test User" now"#, false), []);
        assert_eq!(
            kinds(r#"open @"My Docs/a.md" now"#, false),
            [(MentionKind::File, r#"@"My Docs/a.md""#)]
        );
    }

    #[test]
    fn line_breaks_stay_as_written() {
        let (text, _) = tag("one\ntwo\n\n```\na\nb\n```", &known());
        assert_eq!(text, "one\\\ntwo\n\n```\na\nb\n```");
    }

    #[test]
    fn indented_lines_keep_their_break_and_indent() {
        let (text, mentions) = tag("look at\n[core]\n  a.rs @src/b.rs \n\tc.rs", &known());
        assert_eq!(
            text,
            "look at\\\n[core]\\\n\u{a0}\u{a0}a.rs <hoshi-mention i=\"0\"/> \\\n\u{a0}\u{a0}\u{a0}\u{a0}c.rs"
        );
        assert_eq!(mentions[0].text.as_ref(), "@src/b.rs");
        for (message, expected) in [
            ("look at\n  **a.rs**", "look at\\\n\u{a0}\u{a0}**a.rs**"),
            ("> one\n  > two", "> one\\\n> two"),
            ("> one\n   > two", "> one\\\n> two"),
            ("  > one\n>   two", "  > one\\\n> \u{a0}\u{a0}two"),
            ("- [ ] one\n    two", "- [ ] one\\\n  \u{a0}\u{a0}two"),
            ("1. one\n   two", "1. one\\\n   two"),
            ("- one\n two", "- one\\\n two"),
            ("one\n  > two", "one\n  > two"),
            ("- one\n\t  two", "- one\\\n  \u{a0}\u{a0}\u{a0}\u{a0}two"),
            ("> - one\n>     two", "> - one\\\n>   \u{a0}\u{a0}two"),
            ("- one\n  two", "- one\\\n  two"),
            ("- one\n    two", "- one\\\n  \u{a0}\u{a0}two"),
            (
                "> one\n>   @src/b.rs here",
                "> one\\\n> \u{a0}\u{a0}<hoshi-mention i=\"0\"/> here",
            ),
        ] {
            assert_eq!(tag(message, &known()).0, expected, "{message:?}");
        }
    }

    fn has_block_html(node: &markdown::mdast::Node, in_paragraph: bool) -> bool {
        use markdown::mdast::Node;
        match node {
            Node::Html(_) => !in_paragraph,
            Node::Paragraph(_) | Node::Heading(_) => node
                .children()
                .into_iter()
                .flatten()
                .any(|child| has_block_html(child, true)),
            _ => node
                .children()
                .into_iter()
                .flatten()
                .any(|child| has_block_html(child, in_paragraph)),
        }
    }

    #[test]
    fn a_mention_alone_on_its_line_stays_within_the_text() {
        for message in [
            "one\n\n[Image #1]\n\ntwo",
            "[Image #1]",
            "  [Image #1]  \nthen",
            "- [Image #1]\n- @src/a.rs",
            "> [Image #1]",
        ] {
            let (text, mentions) = tag(message, &known());
            assert!(!mentions.is_empty(), "{message:?}");
            let tree = markdown::to_mdast(&text, &markdown::ParseOptions::gfm()).unwrap();
            assert!(!has_block_html(&tree, false), "{text:?}");
        }
        let (text, _) = tag("see [Image #1]", &known());
        assert_eq!(text, "see <hoshi-mention i=\"0\"/>");
    }

    #[test]
    fn mentions_in_code_stay_text() {
        let (text, mentions) = tag(
            "see `@src/a.rs` and @src/b.rs\n\n```\n@src/c.rs\n```",
            &known(),
        );
        assert_eq!(
            text,
            "see `@src/a.rs` and <hoshi-mention i=\"0\"/>\n\n```\n@src/c.rs\n```"
        );
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].text.as_ref(), "@src/b.rs");
    }
}
