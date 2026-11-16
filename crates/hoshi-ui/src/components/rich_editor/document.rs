//! The rich editor's document: a list of blocks, each with its kind and its text, and how it
//! reads and writes markdown. Text is written as typed, never escaped: what the user sees is
//! what is sent.

/// What a block is. Quotes and list items hold one line of text each; neighbors of the same
/// kind read as one quote or one list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Paragraph,
    /// Level 1 to 3.
    Heading(u8),
    Quote,
    Bullet,
    Ordered,
    /// A fenced block; its language as written after the fence, empty for none.
    Code(String),
    Rule,
}

impl BlockKind {
    /// Whether a block of this kind holds text the user types.
    pub fn has_text(&self) -> bool {
        *self != Self::Rule
    }

    fn joins(&self, next: &Self) -> bool {
        matches!(
            (self, next),
            (Self::Paragraph, Self::Paragraph)
                | (Self::Quote, Self::Quote)
                | (Self::Bullet, Self::Bullet)
                | (Self::Ordered, Self::Ordered)
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockData {
    pub kind: BlockKind,
    pub text: String,
}

/// A chip as it sits in a block: where, and what it stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChipData {
    /// Byte range in the block's text.
    pub range: std::ops::Range<usize>,
    pub id: String,
    /// What the text holds for it.
    pub text: String,
    /// What the chip shows.
    pub label: String,
}

/// A block with its chips, to show it again as it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockSnapshot {
    pub kind: BlockKind,
    pub text: String,
    pub chips: Vec<ChipData>,
}

impl BlockData {
    pub fn new(kind: BlockKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
        }
    }

    pub fn paragraph(text: impl Into<String>) -> Self {
        Self::new(BlockKind::Paragraph, text)
    }
}

/// The blocks as the markdown the agent reads.
pub fn to_markdown(blocks: &[BlockData]) -> String {
    let mut out = String::new();
    let mut number = 0;
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            let previous = &blocks[index - 1];
            out.push_str(if previous.kind.joins(&block.kind) {
                "\n"
            } else {
                "\n\n"
            });
            if previous.kind != BlockKind::Ordered {
                number = 0;
            }
        }
        match &block.kind {
            BlockKind::Paragraph => out.push_str(&block.text),
            BlockKind::Heading(level) => {
                out.push_str(&"#".repeat(usize::from(*level)));
                out.push(' ');
                out.push_str(&block.text);
            }
            BlockKind::Quote => {
                if block.text.is_empty() {
                    out.push('>');
                } else {
                    out.push_str("> ");
                    out.push_str(&block.text);
                }
            }
            BlockKind::Bullet => {
                out.push_str("- ");
                out.push_str(&block.text);
            }
            BlockKind::Ordered => {
                number += 1;
                out.push_str(&format!("{number}. "));
                out.push_str(&block.text);
            }
            BlockKind::Code(language) => {
                out.push_str("```");
                out.push_str(language);
                out.push('\n');
                out.push_str(&block.text);
                out.push_str("\n```");
            }
            BlockKind::Rule => out.push_str("---"),
        }
    }
    out.trim().to_owned()
}

/// The language of an opening fence typed on its own line, like ```tsx; `None` otherwise.
pub fn fence_language(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("```")?;
    rest.chars()
        .all(|char| char.is_alphanumeric() || "_+#.-".contains(char))
        .then(|| rest.to_owned())
}

fn ordered_marker(line: &str) -> Option<&str> {
    let digits = line.find(|char: char| !char.is_ascii_digit())?;
    (digits > 0)
        .then(|| line[digits..].strip_prefix(". "))
        .flatten()
}

/// What typing a markdown marker at the start of a paragraph turns it into: the new kind and
/// the text left after the marker. `None` when the text starts with no marker.
pub fn shortcut(text: &str) -> Option<(BlockKind, &str)> {
    if let Some(rest) = text.strip_prefix("> ") {
        return Some((BlockKind::Quote, rest));
    }
    if let Some(rest) = text.strip_prefix("- ").or_else(|| text.strip_prefix("* ")) {
        return Some((BlockKind::Bullet, rest));
    }
    if let Some(rest) = ordered_marker(text) {
        return Some((BlockKind::Ordered, rest));
    }
    for level in (1..=3u8).rev() {
        let marker = format!("{} ", "#".repeat(usize::from(level)));
        if let Some(rest) = text.strip_prefix(marker.as_str()) {
            return Some((BlockKind::Heading(level), rest));
        }
    }
    (text == "---").then_some((BlockKind::Rule, ""))
}

/// Markdown as blocks: the way a sent prompt comes back to be edited.
pub fn from_markdown(markdown: &str) -> Vec<BlockData> {
    let mut blocks = Vec::new();
    let mut lines = markdown.lines().peekable();
    while let Some(line) = lines.next() {
        if let Some(language) = fence_language(line) {
            let mut code = Vec::new();
            for inner in lines.by_ref() {
                if inner.trim() == "```" {
                    break;
                }
                code.push(inner);
            }
            blocks.push(BlockData::new(BlockKind::Code(language), code.join("\n")));
            continue;
        }
        if line.trim().is_empty() {
            let between_paragraphs = blocks
                .last()
                .is_some_and(|block: &BlockData| block.kind == BlockKind::Paragraph)
                && lines.peek().is_some_and(|next| {
                    !next.trim().is_empty()
                        && shortcut(next).is_none()
                        && fence_language(next).is_none()
                });
            if between_paragraphs {
                blocks.push(BlockData::paragraph(""));
            }
            continue;
        }
        if line == ">" {
            blocks.push(BlockData::new(BlockKind::Quote, ""));
            continue;
        }
        match shortcut(line) {
            Some((kind, rest)) => blocks.push(BlockData::new(kind, rest)),
            None => blocks.push(BlockData::paragraph(line)),
        }
    }
    if blocks.is_empty() {
        blocks.push(BlockData::paragraph(""));
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::{BlockData, BlockKind, fence_language, from_markdown, shortcut, to_markdown};

    fn doc() -> Vec<BlockData> {
        vec![
            BlockData::paragraph("aqui [image:1]"),
            BlockData::paragraph("second line"),
            BlockData::new(BlockKind::Quote, "sdas"),
            BlockData::new(BlockKind::Quote, "more"),
            BlockData::new(BlockKind::Code("tsx".into()), "const a = 1\nconst b = 2"),
            BlockData::new(BlockKind::Ordered, "one"),
            BlockData::new(BlockKind::Ordered, "two"),
            BlockData::new(BlockKind::Bullet, "dot"),
            BlockData::new(BlockKind::Heading(2), "Title"),
            BlockData::new(BlockKind::Rule, ""),
            BlockData::paragraph("end"),
        ]
    }

    const MARKDOWN: &str = "aqui [image:1]\nsecond line\n\n> sdas\n> more\n\n```tsx\nconst a = 1\nconst b = 2\n```\n\n1. one\n2. two\n\n- dot\n\n## Title\n\n---\n\nend";

    #[test]
    fn blocks_write_the_markdown_the_web_composer_writes() {
        assert_eq!(to_markdown(&doc()), MARKDOWN);
    }

    #[test]
    fn markdown_reads_back_as_the_same_blocks() {
        assert_eq!(from_markdown(MARKDOWN), doc());
        assert_eq!(
            from_markdown("a\n\nb"),
            vec![
                BlockData::paragraph("a"),
                BlockData::paragraph(""),
                BlockData::paragraph("b")
            ]
        );
        assert_eq!(from_markdown(""), vec![BlockData::paragraph("")]);
    }

    #[test]
    fn a_marker_at_the_start_turns_the_paragraph() {
        assert_eq!(shortcut("> hi"), Some((BlockKind::Quote, "hi")));
        assert_eq!(shortcut("- "), Some((BlockKind::Bullet, "")));
        assert_eq!(shortcut("* x"), Some((BlockKind::Bullet, "x")));
        assert_eq!(shortcut("12. x"), Some((BlockKind::Ordered, "x")));
        assert_eq!(shortcut("### x"), Some((BlockKind::Heading(3), "x")));
        assert_eq!(shortcut("---"), Some((BlockKind::Rule, "")));
        assert_eq!(shortcut("#hashtag"), None);
        assert_eq!(shortcut("3.5 apples"), None);
        assert_eq!(fence_language("```tsx"), Some("tsx".into()));
        assert_eq!(fence_language("```"), Some(String::new()));
        assert_eq!(fence_language("```a b"), None);
    }
}
