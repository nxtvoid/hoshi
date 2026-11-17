use std::collections::HashMap;
use std::ops::Range;
use std::sync::LazyLock;

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

use super::diff_view::{DiffLine, DiffLineKind};

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);

/// Longer lines, such as minified code, stay plain: syntect slows down a lot on them.
const MAX_LINE_BYTES: usize = 2_000;

/// Theme highlight names for each line's text.
pub type LineSyntax = Vec<(Range<usize>, &'static str)>;

/// Fills in the syntax of the lines of `path`'s diff, from its extension or file name.
/// Each side is read in order, so a string or comment spanning lines keeps its color;
/// both sides start over at every hunk. Blocking: run it off the UI thread for long diffs.
pub fn highlight_diff(path: &str, lines: &mut [DiffLine]) {
    let Some(syntax) = syntax_for(path) else {
        return;
    };
    let mut names = HashMap::new();
    let mut old = Side::new(syntax);
    let mut new = Side::new(syntax);
    for line in lines {
        match line.kind {
            DiffLineKind::Hunk => {
                old = Side::new(syntax);
                new = Side::new(syntax);
            }
            DiffLineKind::Removed => line.syntax = old.line(&line.text, &mut names),
            DiffLineKind::Added => line.syntax = new.line(&line.text, &mut names),
            DiffLineKind::Context => {
                old.line(&line.text, &mut names);
                line.syntax = new.line(&line.text, &mut names);
            }
            DiffLineKind::Note => {}
        }
    }
}

fn syntax_for(path: &str) -> Option<&'static SyntaxReference> {
    let syntaxes = &*SYNTAXES;
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let extension = name.rsplit_once('.').map(|(_, extension)| extension);
    extension
        .into_iter()
        .chain([name])
        .find_map(|token| syntaxes.find_syntax_by_extension(token))
        .filter(|syntax| syntax.name != "Plain Text")
}

struct Side {
    parser: ParseState,
    scopes: ScopeStack,
    /// Past a line too long or failing to read, the parser's state is unknown: the side stays plain.
    lost: bool,
}

impl Side {
    fn new(syntax: &SyntaxReference) -> Self {
        Self {
            parser: ParseState::new(syntax),
            scopes: ScopeStack::new(),
            lost: false,
        }
    }

    fn line(&mut self, text: &str, names: &mut HashMap<Scope, Option<&'static str>>) -> LineSyntax {
        let mut spans = LineSyntax::new();
        if text.len() > MAX_LINE_BYTES {
            self.lost = true;
        }
        if self.lost {
            return spans;
        }
        let line = format!("{text}\n");
        let Ok(operations) = self.parser.parse_line(&line, &SYNTAXES) else {
            self.lost = true;
            return spans;
        };
        let mut cursor = 0;
        for (index, operation) in operations {
            push(
                &mut spans,
                text,
                cursor..index.min(text.len()),
                &self.scopes,
                names,
            );
            let _ = self.scopes.apply(&operation);
            cursor = index;
        }
        push(&mut spans, text, cursor..text.len(), &self.scopes, names);
        spans
    }
}

/// What a bare `storage.type` scope names: the grammar uses it for both `int` and `let`.
const STORAGE_TYPE: &str = "storage.type";

const DECLARATIONS: [&str; 13] = [
    "class",
    "enum",
    "function",
    "impl",
    "interface",
    "module",
    "namespace",
    "struct",
    "trait",
    "type",
    "let",
    "var",
    "const",
];

fn declares(word: &str) -> bool {
    DECLARATIONS.contains(&word)
}

fn push(
    spans: &mut LineSyntax,
    text: &str,
    range: Range<usize>,
    scopes: &ScopeStack,
    names: &mut HashMap<Scope, Option<&'static str>>,
) {
    if range.start >= range.end {
        return;
    }
    let name = scopes
        .scopes
        .iter()
        .rev()
        .find_map(|scope| *names.entry(*scope).or_insert_with(|| semantic_name(*scope)));
    let name = match name {
        None => return,
        Some(STORAGE_TYPE) if declares(text.get(range.clone()).unwrap_or_default()) => "keyword",
        Some(STORAGE_TYPE) => "type",
        Some(name) => name,
    };
    match spans.last_mut() {
        Some((last, last_name)) if last.end == range.start && *last_name == name => {
            last.end = range.end;
        }
        _ => spans.push((range, name)),
    }
}

/// The theme's highlight name for a TextMate scope, as the code blocks of the transcript
/// name it, or `None` to let an enclosing scope decide.
fn semantic_name(scope: Scope) -> Option<&'static str> {
    let scope = scope.build_string();
    let is = |prefix: &str| {
        scope == prefix
            || scope
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('.'))
    };
    let name = if is("punctuation.definition.string")
        || is("punctuation.definition.comment")
        || is("invalid")
        || is("meta")
        || is("source")
        || is("text")
    {
        return None;
    } else if is("comment.block.documentation") || is("comment.line.documentation") {
        "comment.doc"
    } else if is("comment") {
        "comment"
    } else if is("constant.character.escape") {
        "string.escape"
    } else if is("string.regexp") {
        "string.regex"
    } else if is("string") {
        "string"
    } else if is("constant.numeric") {
        "number"
    } else if is("constant.language.boolean") {
        "boolean"
    } else if is("constant") || is("support.constant") || is("entity.name.constant") {
        "constant"
    } else if is("keyword.operator") {
        "operator"
    } else if is("keyword.control.import") {
        "preproc"
    } else if is("keyword") || is("storage.modifier") {
        "keyword"
    } else if is(STORAGE_TYPE) {
        if scope[STORAGE_TYPE.len()..].split('.').any(declares) {
            "keyword"
        } else {
            STORAGE_TYPE
        }
    } else if is("entity.name.function")
        || is("support.function")
        || is("variable.function")
        || is("meta.function-call")
    {
        "function"
    } else if is("entity.name.type")
        || is("entity.name.class")
        || is("entity.name.struct")
        || is("entity.name.enum")
        || is("entity.name.trait")
        || is("entity.name.interface")
        || is("entity.other.inherited-class")
        || is("support.type")
        || is("support.class")
    {
        "type"
    } else if is("entity.name.tag") {
        "tag"
    } else if is("entity.other.attribute-name") {
        "attribute"
    } else if is("entity.name.label") {
        "label"
    } else if is("entity.name.section") || is("markup.heading") {
        "title"
    } else if is("variable.language") {
        "variable.special"
    } else if is("variable.other.member")
        || is("variable.other.property")
        || is("support.variable.property")
        || is("meta.mapping.key")
    {
        "property"
    } else if is("variable") {
        "variable"
    } else if is("markup.bold") {
        "emphasis.strong"
    } else if is("markup.italic") {
        "emphasis"
    } else if is("markup.underline.link") {
        "link_uri"
    } else if is("markup.raw") {
        "text.literal"
    } else if is("markup.list") {
        "punctuation.list_marker"
    } else if is("punctuation.section") || is("punctuation.definition.block") {
        "punctuation.bracket"
    } else if is("punctuation.separator") || is("punctuation.terminator") {
        "punctuation.delimiter"
    } else if is("punctuation") {
        "punctuation"
    } else {
        return None;
    };
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::highlight_diff;
    use crate::parse_unified_diff;

    fn named<'a>(
        text: &'a str,
        syntax: &[(std::ops::Range<usize>, &'static str)],
    ) -> Vec<(&'a str, &'static str)> {
        syntax
            .iter()
            .map(|(range, name)| (&text[range.clone()], *name))
            .collect()
    }

    #[test]
    fn names_the_tokens_of_each_line_by_the_file_extension() {
        let mut lines = parse_unified_diff("@@ -1 +1 @@\n-const a = 1\n+const a = \"b\"");
        highlight_diff("web/next.config.ts", &mut lines);

        let removed = named(&lines[1].text, &lines[1].syntax);
        let added = named(&lines[2].text, &lines[2].syntax);
        assert!(removed.contains(&("const", "keyword")), "{removed:?}");
        assert!(removed.contains(&("1", "number")), "{removed:?}");
        assert!(added.contains(&("\"b\"", "string")), "{added:?}");
    }

    #[test]
    fn a_comment_opened_on_one_side_colors_the_lines_after_it_on_that_side_only() {
        let mut lines =
            parse_unified_diff("@@ -1,3 +1,2 @@\n-let x = 1;\n+/* new\n end */\n-let y = 2;");
        highlight_diff("src/main.rs", &mut lines);

        assert_eq!(
            named(&lines[3].text, &lines[3].syntax),
            [("end */", "comment")]
        );
        let removed = named(&lines[4].text, &lines[4].syntax);
        assert!(removed.contains(&("let", "keyword")), "{removed:?}");
    }

    #[test]
    fn a_side_stays_plain_after_a_line_too_long_to_read() {
        let long = format!("x */ {}", "a".repeat(super::MAX_LINE_BYTES));
        let mut lines =
            parse_unified_diff(&format!("@@ -1,3 +1,3 @@\n /* open\n {long}\n let y = 2;"));
        highlight_diff("src/main.rs", &mut lines);

        assert!(lines[2].syntax.is_empty());
        assert!(lines[3].syntax.is_empty());
    }

    #[test]
    fn a_storage_type_names_a_type_unless_it_declares() {
        let mut lines = parse_unified_diff("@@ -1 +1 @@\n-int count = 1;\n+let count = 1;");
        highlight_diff("src/count.c", &mut lines);
        let removed = named(&lines[1].text, &lines[1].syntax);
        assert!(removed.contains(&("int", "type")), "{removed:?}");

        let scope = syntect::parsing::Scope::new("storage.type.function.rust").unwrap();
        assert_eq!(super::semantic_name(scope), Some("keyword"));
    }

    #[test]
    fn an_unknown_file_stays_plain() {
        let mut lines = parse_unified_diff("@@ -1 +1 @@\n-a\n+b");
        highlight_diff("notes.unknownext", &mut lines);

        assert!(lines.iter().all(|line| line.syntax.is_empty()));
    }
}
