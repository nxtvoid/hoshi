use super::split;
use markdown::mdast::Node;

fn parse(source: &str) -> Vec<Node> {
    let mut options = markdown::ParseOptions::gfm();
    options.constructs.math_text = true;
    options.constructs.math_flow = true;
    let Ok(mut root) = markdown::to_mdast(source, &options) else {
        return vec![Node::Text(markdown::mdast::Text {
            value: source.to_owned(),
            position: None,
        })];
    };
    without_positions(&mut root);
    root.children().cloned().unwrap_or_default()
}

fn without_positions(node: &mut Node) {
    node.position_set(None);
    if let Some(children) = node.children_mut() {
        children.iter_mut().for_each(without_positions);
    }
}

fn pieces(text: &str) -> Vec<&str> {
    split(text)
        .expect("split")
        .into_iter()
        .map(|range| &text[range])
        .collect()
}

const SAMPLES: &[&str] = &[
    "# Title\n\nFirst paragraph with `code` and **bold**.\n\nSecond [link](https://example.com).",
    "## Step 1: Install\n\n## Step 1: Install\n\nafter",
    "> Setup\n> ===\n\n> Setup\n> ===",
    "# onetwo\n\none\ntwo\n===",
    "# Setup\n\npara\n\n- a\n\n  ```\n  b\nx\n\nSetup\n===\n",
    "Setup\n===\n\n> ## **Setup**\n\n- # [Setup](https://example.com)\n\n```sh\n# Setup\n```",
    "- one\n- two\n\n- loose three\n\n  continued\n\nAfter the list.",
    "1. first\n\n2. second\n\nText\n\n3) restarted",
    "```ts\nconst a = 1;\n\n\nconst b = 2;\n```\n\nafter\n\n~~~\nx\n\n~~~~\n\ny\n~~~\n\nend",
    "> quote\n\n> another quote\n\n> [!NOTE]\n> alert",
    "| a | b |\n| - | - |\n| 1 | 2 |\n\nAfter the table.",
    "para\n\n    indented code\n\nnext",
    "Setext\n===\n\nnext\n---\n\n- [ ] task\n- [x] done",
    "- a\n\n  ```\n  x\n\n```\n\nb",
    "- a\n\n  ```\n  x\n\ny\n\n  ```\n\nb",
    "```a``` is inline.\n\nprose\n\n```\none\n\ntwo\n```\n\nafter",
    "a\n\n---\n\nb\n\n***\n\n## c\n\nd",
    "costs $5 and $10\n\nthen `x`\n\n* * *\n\nend",
    "a\n\nb\nc\n---\n\nd\n\ne\n===\n\nf",
    "- item\n\nafter\n\n    code\n\n1) one\n\nlast",
    "```\na\n```\u{00a0}\n\nb\n\nc",
    "a\n\u{00a0}\nb\n\nc",
    "\n\nHello\n\nthere",
];

#[test]
fn every_block_parses_as_it_does_in_the_whole_text_at_every_streamed_prefix() {
    for text in SAMPLES {
        for end in (0..=text.len()).filter(|end| text.is_char_boundary(*end)) {
            let source = &text[..end];
            let blocks = pieces(source);
            assert_eq!(blocks.concat(), source);
            let apart: Vec<Node> = blocks.iter().flat_map(|block| parse(block)).collect();
            assert_eq!(apart, parse(source), "{source:?} cut as {blocks:?}");
        }
    }
}

#[test]
fn blank_lines_before_the_first_text_stay_with_it() {
    assert_eq!(pieces("\n\nHello"), ["\n\nHello"]);
    assert_eq!(pieces("\n\na\n\nb"), ["\n\na\n\n", "b"]);
}

#[test]
fn only_spaces_and_tabs_make_a_line_blank() {
    assert_eq!(pieces("a\n\u{00a0}\nb"), ["a\n\u{00a0}\nb"]);
    assert_eq!(
        pieces("```\na\n```\u{00a0}\n\nb"),
        ["```\na\n```\u{00a0}\n\nb"]
    );
    assert_eq!(pieces("```\na\n``` \t\n\nb"), ["```\na\n``` \t\n\n", "b"]);
}

#[test]
fn it_cuts_only_between_closed_blocks() {
    assert_eq!(
        pieces("a\n\n```\nb\n\nc\n```\n\n- d\n\n- e\n\nf"),
        ["a\n\n", "```\nb\n\nc\n```\n\n- d\n\n- e\n\n", "f"]
    );
}

#[test]
fn headings_and_rules_stay_with_the_blocks_around_them_and_quotes_with_the_one_before() {
    assert_eq!(pieces("a\n\n---\n\nb\n\nc"), ["a\n\n---\n\nb\n\n", "c"]);
    assert_eq!(pieces("a\n\n## b\n\nc\n\nd"), ["a\n\n## b\n\nc\n\n", "d"]);
    assert_eq!(
        pieces("a\n\nb\n---\n\nc\n\nd"),
        ["a\n\nb\n---\n\nc\n\n", "d"]
    );
    assert_eq!(pieces("a\n\n> b\n\nc"), ["a\n\n> b\n\n", "c"]);
}

/// Headings and rules take gaps that depend on their neighbors; any other block leaves the
/// paragraph gap below it, which the rows of a reply's blocks rebuild between them.
#[test]
fn every_block_but_the_last_ends_and_every_block_but_the_first_starts_with_a_plain_block() {
    let section = |node: &Node| matches!(node, Node::Heading(_) | Node::ThematicBreak(_));
    for text in SAMPLES {
        for end in (0..=text.len()).filter(|end| text.is_char_boundary(*end)) {
            let blocks = pieces(&text[..end]);
            for (index, block) in blocks.iter().enumerate() {
                let nodes = parse(block);
                if index + 1 < blocks.len() {
                    assert!(!nodes.last().is_some_and(section), "{blocks:?}");
                }
                if index > 0 {
                    let first = nodes.first();
                    assert!(
                        !first.is_some_and(
                            |node| section(node) || matches!(node, Node::Blockquote(_))
                        ),
                        "{blocks:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn text_with_syntax_that_ties_blocks_together_stays_whole() {
    for text in [
        "[ref]\n\n[ref]: /later",
        "> [ref]: /url\n\n[ref]",
        "- [ref]:\n  /url\n\n[ref]",
        "a[^x]\n\n[^x]: note",
        "<details>\n\nbody\n\n</details>",
        ":::note\n\ncontent\n\n:::",
        "$$\na\n\nb\n$$",
        "a\r\n\r\nb",
    ] {
        assert_eq!(split(text), None, "{text:?}");
    }
}
