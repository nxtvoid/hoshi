//! Inline marks as the web composer's shortcuts write them: Ctrl/Cmd+B, I, E and Shift+S wrap
//! the selection in the markdown marker, or take it away when the selection already has it.

use std::ops::Range;

/// An inline mark and the markdown that writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Bold,
    Italic,
    Code,
    Strike,
}

impl Mark {
    pub fn marker(self) -> &'static str {
        match self {
            Self::Bold => "**",
            Self::Italic => "*",
            Self::Code => "`",
            Self::Strike => "~~",
        }
    }

    fn char(self) -> char {
        self.marker().chars().next().unwrap_or('*')
    }

    /// Whether runs of marker characters this long before and after the text hold this mark.
    fn held_by(self, before: usize, after: usize) -> bool {
        match self {
            Self::Italic => before % 2 == 1 && after % 2 == 1,
            _ => {
                let length = self.marker().len();
                before >= length && after >= length
            }
        }
    }
}

/// The mark a keystroke toggles, with the platform's primary modifier.
pub fn mark_shortcut(
    key: &str,
    control: bool,
    platform: bool,
    alt: bool,
    shift: bool,
) -> Option<Mark> {
    let primary = if cfg!(target_os = "macos") {
        platform && !control
    } else {
        control && !platform
    };
    if !primary || alt {
        return None;
    }
    match (key, shift) {
        ("b", _) => Some(Mark::Bold),
        ("i", false) => Some(Mark::Italic),
        ("e", false) => Some(Mark::Code),
        ("s", true) => Some(Mark::Strike),
        _ => None,
    }
}

/// The replacements, last first, and the selection after them.
pub type MarkEdits = (Vec<(Range<usize>, &'static str)>, Range<usize>);

fn run_before(text: &str, at: usize, char: char) -> usize {
    text[..at].chars().rev().take_while(|c| *c == char).count()
}

fn run_after(text: &str, at: usize, char: char) -> usize {
    text[at..].chars().take_while(|c| *c == char).count()
}

/// Toggles `mark` on `selection` of `text`: unwraps it when the markers sit just outside or
/// just inside the selection, wraps it otherwise. An empty selection gets an empty pair with
/// the caret between, as typing into a toggled mark.
pub fn toggle(text: &str, selection: Range<usize>, mark: Mark) -> MarkEdits {
    let (start, end) = (
        selection.start.min(text.len()),
        selection.end.min(text.len()),
    );
    let char = mark.char();
    let width = mark.marker().len();
    let marker = mark.marker();
    if mark.held_by(run_before(text, start, char), run_after(text, end, char)) {
        return (
            vec![(end..end + width, ""), (start - width..start, "")],
            start - width..end - width,
        );
    }
    let inside = &text[start..end];
    let leading = run_after(inside, 0, char).min(inside.len());
    let trailing = run_before(inside, inside.len(), char);
    if leading < inside.len() && mark.held_by(leading, trailing) {
        return (
            vec![(end - width..end, ""), (start..start + width, "")],
            start..end - 2 * width,
        );
    }
    (
        vec![(end..end, marker), (start..start, marker)],
        start + width..end + width,
    )
}

/// A marked stretch of text: what it holds and where its markers sit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkSpan {
    pub mark: Mark,
    pub content: Range<usize>,
    pub open: Range<usize>,
    pub close: Range<usize>,
}

fn taken(spans: &[MarkSpan], at: usize) -> bool {
    spans
        .iter()
        .any(|span| span.open.start <= at && at < span.close.end)
}

/// Where `marker` closes the stretch opened before `from`: the next one on the same line
/// with text between that doesn't touch a space on either side.
fn closing(text: &str, from: usize, marker: &str, single: bool) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut at = from;
    while let Some(found) = text[at..].find(marker) {
        let close = at + found;
        if text[from..close].contains('\n') {
            return None;
        }
        let lone = !single
            || (bytes.get(close + 1) != Some(&b'*') && (close == 0 || bytes[close - 1] != b'*'));
        let content = &text[from..close];
        if lone
            && !content.is_empty()
            && !content.starts_with(char::is_whitespace)
            && !content.ends_with(char::is_whitespace)
        {
            return Some(close);
        }
        at = close + marker.len();
    }
    None
}

/// The marks the text writes as markdown, as the web composer shows them: inline code first,
/// whose insides hold no other mark, then strikethrough, bold and italic.
pub fn spans(text: &str) -> Vec<MarkSpan> {
    let mut found: Vec<MarkSpan> = Vec::new();
    let bytes = text.as_bytes();
    for mark in [Mark::Code, Mark::Strike, Mark::Bold, Mark::Italic] {
        let marker = mark.marker();
        let single = mark == Mark::Italic;
        let mut at = 0;
        while let Some(offset) = text[at..].find(marker) {
            let open = at + offset;
            at = open + marker.len();
            if taken(&found, open) {
                continue;
            }
            if single
                && (bytes.get(open + 1) == Some(&b'*') || (open > 0 && bytes[open - 1] == b'*'))
            {
                continue;
            }
            let start = open + marker.len();
            let Some(close) = closing(text, start, marker, single) else {
                continue;
            };
            let end = close + marker.len();
            if found.iter().any(|span| {
                span.mark == Mark::Code && span.open.start < end && start < span.close.end
            }) {
                continue;
            }
            found.push(MarkSpan {
                mark,
                content: start..close,
                open: open..start,
                close: close..end,
            });
            at = end;
        }
    }
    found.sort_by_key(|span| span.open.start);
    found
}

/// How a folded mark splits so it can wrap like text: one piece per word, the opening marker
/// joined to the first and the closing one to the last, and the whitespace runs between them.
pub fn fold_pieces(text: &str, span: &MarkSpan) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let content = &text[span.content.clone()];
    let mut words: Vec<Range<usize>> = Vec::new();
    let mut gaps = Vec::new();
    let mut word_start = None;
    for (offset, character) in content.char_indices() {
        let at = span.content.start + offset;
        match (character.is_whitespace(), word_start) {
            (true, Some(start)) => {
                words.push(start..at);
                word_start = None;
                gaps.push(at..at + character.len_utf8());
            }
            (true, None) => match gaps.last_mut() {
                Some(gap) if gap.end == at => gap.end = at + character.len_utf8(),
                _ => gaps.push(at..at + character.len_utf8()),
            },
            (false, None) => word_start = Some(at),
            (false, Some(_)) => {}
        }
    }
    if let Some(start) = word_start {
        words.push(start..span.content.end);
    }
    if words.is_empty() {
        words.push(span.open.clone());
    }
    let last = words.len() - 1;
    words[0].start = span.open.start;
    words[last].end = span.close.end;
    (words, gaps)
}

#[cfg(test)]
mod tests {
    use super::{Mark, fold_pieces, mark_shortcut, spans, toggle};
    use std::ops::Range;

    #[test]
    fn a_folded_mark_splits_into_words_that_keep_its_markers_at_the_ends() {
        let text = "a **one  two three** b";
        let span = &spans(text)[0];
        let (words, gaps) = fold_pieces(text, span);
        let words: Vec<&str> = words.iter().map(|range| &text[range.clone()]).collect();
        let gaps: Vec<&str> = gaps.iter().map(|range| &text[range.clone()]).collect();
        assert_eq!(words, ["**one", "two", "three**"]);
        assert_eq!(gaps, ["  ", " "]);

        let text = "*solo*";
        let (words, gaps) = fold_pieces(text, &spans(text)[0]);
        assert_eq!(
            words
                .iter()
                .map(|range| &text[range.clone()])
                .collect::<Vec<_>>(),
            ["*solo*"]
        );
        assert!(gaps.is_empty());
    }

    fn apply(text: &str, edits: &[(Range<usize>, &str)]) -> String {
        let mut out = text.to_owned();
        for (range, replacement) in edits {
            out.replace_range(range.clone(), replacement);
        }
        out
    }

    fn run(text: &str, selection: Range<usize>, mark: Mark) -> (String, String) {
        let (edits, selection) = toggle(text, selection, mark);
        let out = apply(text, &edits);
        let selected = out[selection].to_owned();
        (out, selected)
    }

    #[test]
    fn wraps_the_selection_and_keeps_it_on_the_text() {
        assert_eq!(
            run("say hi", 4..6, Mark::Bold),
            ("say **hi**".into(), "hi".into())
        );
        assert_eq!(
            run("say hi", 4..6, Mark::Italic),
            ("say *hi*".into(), "hi".into())
        );
        assert_eq!(
            run("say hi", 4..6, Mark::Code),
            ("say `hi`".into(), "hi".into())
        );
        assert_eq!(
            run("say hi", 4..6, Mark::Strike),
            ("say ~~hi~~".into(), "hi".into())
        );
    }

    #[test]
    fn unwraps_markers_outside_or_inside_the_selection() {
        assert_eq!(
            run("say **hi**", 6..8, Mark::Bold),
            ("say hi".into(), "hi".into())
        );
        assert_eq!(
            run("say **hi**", 4..10, Mark::Bold),
            ("say hi".into(), "hi".into())
        );
        assert_eq!(run("`x`", 1..2, Mark::Code), ("x".into(), "x".into()));
    }

    #[test]
    fn italic_and_bold_tell_their_asterisks_apart() {
        assert_eq!(
            run("**hi**", 2..4, Mark::Italic),
            ("***hi***".into(), "hi".into())
        );
        assert_eq!(
            run("***hi***", 3..5, Mark::Italic),
            ("**hi**".into(), "hi".into())
        );
        assert_eq!(
            run("***hi***", 3..5, Mark::Bold),
            ("*hi*".into(), "hi".into())
        );
        assert_eq!(
            run("*hi*", 1..3, Mark::Bold),
            ("***hi***".into(), "hi".into())
        );
    }

    #[test]
    fn an_empty_selection_opens_a_pair_and_closes_an_empty_one() {
        let (edits, caret) = toggle("a ", 2..2, Mark::Bold);
        assert_eq!(apply("a ", &edits), "a ****");
        assert_eq!(caret, 4..4);
        let (edits, caret) = toggle("a ****", 4..4, Mark::Bold);
        assert_eq!(apply("a ****", &edits), "a ");
        assert_eq!(caret, 2..2);
    }

    #[test]
    fn marks_are_found_with_their_markers() {
        let text = "a **bold** and *it* with `co*de*` ~~gone~~";
        let found: Vec<_> = super::spans(text)
            .into_iter()
            .map(|span| (span.mark, &text[span.content]))
            .collect();
        assert_eq!(
            found,
            [
                (Mark::Bold, "bold"),
                (Mark::Italic, "it"),
                (Mark::Code, "co*de*"),
                (Mark::Strike, "gone"),
            ]
        );
    }

    #[test]
    fn loose_markers_mark_nothing() {
        assert!(super::spans("2 * 3 * 4").is_empty());
        assert!(super::spans("** nope **").is_empty());
        assert!(super::spans("*open\nclose*").is_empty());
        assert!(super::spans("**").is_empty());
    }

    #[test]
    fn shortcuts_follow_the_web_composer() {
        let primary = |key, shift| {
            if cfg!(target_os = "macos") {
                mark_shortcut(key, false, true, false, shift)
            } else {
                mark_shortcut(key, true, false, false, shift)
            }
        };
        assert_eq!(primary("b", false), Some(Mark::Bold));
        assert_eq!(primary("i", false), Some(Mark::Italic));
        assert_eq!(primary("e", false), Some(Mark::Code));
        assert_eq!(primary("s", true), Some(Mark::Strike));
        assert_eq!(primary("s", false), None);
        assert_eq!(mark_shortcut("b", false, false, false, false), None);
    }
}
