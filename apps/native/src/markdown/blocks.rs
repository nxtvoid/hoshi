//! Markdown cut into blocks that each render as they do inside the whole text, so a reply being
//! written can be drawn a block at a time: only the last one still changes.

use std::ops::Range;

/// `text` cut at blank lines that close every open block. Never next to a heading or a rule,
/// whose gaps depend on the blocks around them, nor before a quote, so every block but the last
/// ends with the paragraph gap below it. `None` when the text uses syntax that ties blocks
/// together: link definitions, footnotes, HTML, directives or display math.
pub fn split(text: &str) -> Option<Vec<Range<usize>>> {
    if text.starts_with('\u{feff}') || text.contains('\r') || ties_blocks(text) {
        return None;
    }
    let mut blocks = Vec::new();
    let mut block_start = 0;
    let mut line_start = 0;
    let mut fence: Option<(&str, usize)> = None;
    let mut after_blank = false;
    let mut after_text = false;
    let mut text_start = 0;
    let mut after_section = false;
    let mut any_text = false;
    for line in text.split('\n') {
        let opener = fence_opener(line);
        if let Some((open, indent)) = fence {
            if !is_blank(line.get(..indent).unwrap_or(line)) {
                break;
            }
            if opener.is_some_and(|(close, at)| {
                close.as_bytes()[0] == open.as_bytes()[0]
                    && close.len() >= open.len()
                    && is_blank(&line[at + close.len()..])
            }) {
                fence = None;
            }
        } else if is_blank(line) {
            after_blank = true;
            after_text = false;
        } else {
            if after_blank
                && any_text
                && !after_section
                && !continues(line)
                && !starts_section(line)
            {
                blocks.push(block_start..line_start);
                block_start = line_start;
            }
            let underline = after_text && is_underline(line);
            if underline
                && text_start == block_start
                && let Some(before) = blocks.pop()
            {
                block_start = before.start;
            }
            if !after_text {
                text_start = line_start;
            }
            after_section = is_heading(line) || is_rule(line) || underline;
            after_text = opener.is_none();
            after_blank = false;
            any_text = true;
            fence = opener;
        }
        line_start += line.len() + 1;
    }
    blocks.push(block_start..text.len());
    Some(blocks)
}

/// Link definitions, footnotes, HTML, directives and display math, any of which can span or
/// tie together blocks. A definition's label ends in `]:` wherever it sits: in a quote, in a
/// list or over several lines.
fn ties_blocks(text: &str) -> bool {
    text.contains("[^")
        || text.contains("]:")
        || text.lines().any(|line| {
            let line = indented_up_to_three(line).unwrap_or("");
            line.starts_with(":::")
                || line.starts_with("$$")
                || line.starts_with('<')
                    && line[1..].starts_with(|c: char| c.is_ascii_alphabetic() || "!?/".contains(c))
        })
}

/// Whether a line holds only spaces and tabs, the only whitespace Markdown counts as blank.
fn is_blank(line: &str) -> bool {
    line.chars().all(|c| c == ' ' || c == '\t')
}

/// The line past up to three spaces of indent, or `None` when it is indented more.
fn indented_up_to_three(line: &str) -> Option<&str> {
    let rest = line.trim_start_matches(' ');
    (line.len() - rest.len() <= 3).then_some(rest)
}

/// The run of backticks or tildes that opens or closes a fence on `line`, and where it starts.
/// A backtick run followed by another backtick on the line is a code span instead.
fn fence_opener(line: &str) -> Option<(&str, usize)> {
    let rest = indented_up_to_three(line)?;
    let mark = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let run = rest.len() - rest.trim_start_matches(mark).len();
    if run < 3 || mark == '`' && rest[run..].contains('`') {
        return None;
    }
    let indent = line.len() - rest.len();
    Some((&rest[..run], indent))
}

/// Whether a line after a blank one still belongs to the block before it: indented content or
/// another item of a list.
fn continues(line: &str) -> bool {
    if line.starts_with([' ', '\t']) {
        return true;
    }
    let ends_marker = |rest: &str| rest.is_empty() || rest.starts_with([' ', '\t']);
    if let Some(rest) = line.strip_prefix(['-', '*', '+']) {
        return ends_marker(rest);
    }
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    (1..=9).contains(&digits)
        && line[digits..]
            .strip_prefix(['.', ')'])
            .is_some_and(ends_marker)
}

/// Whether a line starts a heading, a rule or a quote.
fn starts_section(line: &str) -> bool {
    is_heading(line)
        || is_rule(line)
        || indented_up_to_three(line).is_some_and(|rest| rest.starts_with('>'))
}

fn is_heading(line: &str) -> bool {
    let Some(rest) = indented_up_to_three(line) else {
        return false;
    };
    let hashes = rest.len() - rest.trim_start_matches('#').len();
    (1..=6).contains(&hashes)
        && (rest[hashes..].is_empty() || rest[hashes..].starts_with([' ', '\t']))
}

fn is_rule(line: &str) -> bool {
    let Some(rest) = indented_up_to_three(line) else {
        return false;
    };
    ['-', '*', '_'].into_iter().any(|mark| {
        rest.chars().all(|c| c == mark || c == ' ' || c == '\t')
            && rest.chars().filter(|c| *c == mark).count() >= 3
    })
}

/// Whether a line right under text could turn that text into a heading.
fn is_underline(line: &str) -> bool {
    let rest = line.trim();
    !rest.is_empty() && (rest.chars().all(|c| c == '=') || rest.chars().all(|c| c == '-'))
}

#[cfg(test)]
mod tests;
