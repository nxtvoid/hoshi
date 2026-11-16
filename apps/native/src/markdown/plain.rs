//! Markdown read as plain text, for one-line previews: marks go, the words they wrap stay,
//! and underscores inside words are kept as CommonMark reads them.

use markdown::mdast::Node;

/// The text a reader sees in `markdown`, its blocks joined by spaces.
pub fn plain_text(markdown: &str) -> String {
    let Ok(tree) = markdown::to_mdast(markdown, &markdown::ParseOptions::gfm()) else {
        return markdown.split_whitespace().collect::<Vec<_>>().join(" ");
    };
    let mut out = String::with_capacity(markdown.len());
    collect(&tree, &mut out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collect(node: &Node, out: &mut String) {
    match node {
        Node::Text(text) => out.push_str(&text.value),
        Node::InlineCode(code) => out.push_str(&code.value),
        Node::Code(code) => {
            out.push(' ');
            out.push_str(&code.value);
            out.push(' ');
        }
        Node::Image(image) => out.push_str(&image.alt),
        Node::Break(_) => out.push(' '),
        Node::Html(_) | Node::ThematicBreak(_) | Node::Definition(_) => {}
        _ => {
            let block = !matches!(
                node,
                Node::Emphasis(_)
                    | Node::Strong(_)
                    | Node::Delete(_)
                    | Node::Link(_)
                    | Node::LinkReference(_)
            );
            for child in node.children().into_iter().flatten() {
                collect(child, out);
            }
            if block {
                out.push(' ');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::plain_text;

    #[test]
    fn underscores_inside_words_stay() {
        assert_eq!(
            plain_text("Use html_preview, not html_render"),
            "Use html_preview, not html_render"
        );
        assert_eq!(
            plain_text("snake_case and file_name.rs"),
            "snake_case and file_name.rs"
        );
    }

    #[test]
    fn marks_go_and_their_words_stay() {
        assert_eq!(
            plain_text("## **Plan**: _read_ `a_b.rs`, see [docs](https://x.io)"),
            "Plan: read a_b.rs, see docs"
        );
        assert_eq!(plain_text("__init__.py"), "init.py");
        assert_eq!(plain_text("- one\n- two\n\n> quoted"), "one two quoted");
        assert_eq!(plain_text("~~gone~~ kept"), "gone kept");
    }
}
