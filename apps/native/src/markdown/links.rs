//! What the text view needs to know about a message's links before it draws them: where each
//! web link's site icon goes, and which links say something other than their address, the
//! ones whose address shows on hover.

use super::sites::{self, SCHEME};
use markdown::mdast::Node;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Prepared messages kept, so a row drawn again skips parsing its markdown.
const KEPT: usize = 512;

#[derive(Clone, Default)]
pub struct Prepared {
    /// The markdown with an icon before each web link.
    pub text: Arc<str>,
    /// Addresses of links whose text is not the address.
    pub labeled: Arc<HashSet<String>>,
}

/// A prepared message and what its parse found, so text that extends it, as a reply does while
/// it streams, parses again only from `reparse_from`.
struct Parsed {
    prepared: Prepared,
    found: Found,
    /// Where the line of the second to last top-level block starts: the blocks before it
    /// stay as they are whatever text comes after.
    reparse_from: usize,
}

thread_local! {
    static PREPARED: RefCell<HashMap<Arc<str>, Parsed>> = RefCell::default();
}

/// `text` with its links' icons, read from a cache when it was prepared before.
pub fn prepare(text: &str) -> Prepared {
    if let Some(prepared) = PREPARED.with(|cache| {
        cache
            .borrow()
            .get(text)
            .map(|parsed| parsed.prepared.clone())
    }) {
        return prepared;
    }
    let extended = PREPARED.with(|cache| {
        let mut cache = cache.borrow_mut();
        let key = cache
            .keys()
            .filter(|key| key.len() < text.len() && text.starts_with(&***key))
            .max_by_key(|key| key.len())
            .cloned()?;
        cache.remove(&key)
    });
    let parsed = match extended {
        Some(before) if !has_definitions(text) => parse_after(text, before),
        _ => parse_from(text, Found::default(), 0),
    };
    let prepared = parsed.prepared.clone();
    PREPARED.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= KEPT {
            cache.clear();
        }
        cache.insert(Arc::from(text), parsed);
    });
    prepared
}

/// Whether a line may define a link reference or footnote, which links before it can use.
fn has_definitions(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim_start_matches([' ', '\t', '>']);
        line.starts_with('[') && line.contains("]:")
    })
}

/// `text`, which extends the text `before` was parsed from.
fn parse_after(text: &str, before: Parsed) -> Parsed {
    let start = before.reparse_from;
    let mut found = before.found;
    found.icons.retain(|(at, _)| *at < start);
    found.local_images.retain(|(at, _, _)| *at < start);
    found.labeled.retain(|(at, _)| *at < start);
    found.bare_paths.retain(|(at, _)| *at < start);
    parse_from(text, found, start)
}

fn plain_text(node: &Node) -> String {
    match node {
        Node::Text(text) => text.value.clone(),
        Node::InlineCode(code) => code.value.clone(),
        _ => node
            .children()
            .map(|children| children.iter().map(plain_text).collect())
            .unwrap_or_default(),
    }
}

/// What a message's markdown gains before it is drawn: an icon before each web link, and a
/// link around each image of a file on this machine, so clicking the image opens it.
#[derive(Default)]
struct Found {
    icons: Vec<(usize, String)>,
    local_images: Vec<(usize, usize, String)>,
    labeled: Vec<(usize, String)>,
    bare_paths: Vec<(usize, usize)>,
}

fn collect(node: &Node, source: &str, found: &mut Found) {
    match node {
        Node::Link(link) => {
            let Some(position) = &link.position else {
                return;
            };
            if plain_text(node) != link.url {
                found
                    .labeled
                    .push((position.start.offset, link.url.clone()));
            }
            if let Some(host) = sites::web_host(&link.url) {
                found.icons.push((position.start.offset, host));
            }
        }
        Node::Text(text) => {
            if let Some(position) = &text.position {
                found.bare_paths.extend(
                    bare_paths(
                        source
                            .get(position.start.offset..position.end.offset)
                            .unwrap_or(""),
                    )
                    .map(|(start, end)| {
                        (position.start.offset + start, position.start.offset + end)
                    }),
                );
            }
        }
        Node::Image(image) => {
            if let (Some(_), Some(position)) = (super::local_path(&image.url), &image.position) {
                found.local_images.push((
                    position.start.offset,
                    position.end.offset,
                    image.url.clone(),
                ));
            }
        }
        _ => {
            for child in node.children().into_iter().flatten() {
                collect(child, source, found);
            }
        }
    }
}

/// Marks the code [`prepare`] puts around an absolute path written as plain text, so the path
/// draws as a file chip and copies without the backticks.
pub const BARE: char = '\u{2060}';

/// Absolute paths of files in a run of plain text, as byte ranges: a whole word that starts
/// like `C:\` or, with at least two segments, like `/`, without the punctuation after it.
fn bare_paths(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    text.split(char::is_whitespace).filter_map(move |word| {
        let start = word.as_ptr() as usize - text.as_ptr() as usize;
        let word = word.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}', '"', '\'']);
        let posix = word.starts_with('/') && word[1..].contains('/');
        ((posix || super::is_windows_path(word)) && super::file_links::code_path(word).is_some())
            .then_some((start, start + word.len()))
    })
}

/// `text` whose blocks before `start` were parsed into `found`: parses the rest.
fn parse_from(text: &str, mut found: Found, start: usize) -> Parsed {
    let rest = &text[start..];
    let Ok(tree) = markdown::to_mdast(rest, &markdown::ParseOptions::gfm()) else {
        return Parsed {
            prepared: Prepared {
                text: Arc::from(text),
                labeled: Arc::default(),
            },
            found: Found::default(),
            reparse_from: 0,
        };
    };
    let mut more = Found::default();
    collect(&tree, rest, &mut more);
    let shift = |at: usize| start + at;
    found
        .icons
        .extend(more.icons.into_iter().map(|(at, host)| (shift(at), host)));
    found.local_images.extend(
        more.local_images
            .into_iter()
            .map(|(from, to, url)| (shift(from), shift(to), url)),
    );
    found
        .labeled
        .extend(more.labeled.into_iter().map(|(at, url)| (shift(at), url)));
    found.bare_paths.extend(
        more.bare_paths
            .into_iter()
            .map(|(from, to)| (shift(from), shift(to))),
    );
    let blocks = tree.children().map_or(&[][..], Vec::as_slice);
    let reparse_from = blocks
        .len()
        .checked_sub(2)
        .and_then(|second_to_last| blocks[second_to_last].position())
        .map_or(start, |position| {
            let at = shift(position.start.offset);
            text[..at].rfind('\n').map_or(0, |line| line + 1)
        });
    Parsed {
        prepared: with_inserts(text, &found),
        found,
        reparse_from,
    }
}

fn with_inserts(text: &str, found: &Found) -> Prepared {
    let mut inserts: Vec<(usize, String)> = found
        .icons
        .iter()
        .map(|(at, host)| {
            (
                *at,
                format!("<img src=\"{SCHEME}{host}\" alt=\"\" width=\"14\" height=\"14\">\u{a0}"),
            )
        })
        .collect();
    for (start, end, url) in &found.local_images {
        inserts.push((*start, "[".to_owned()));
        inserts.push((*end, format!("]({url})")));
    }
    for (start, end) in &found.bare_paths {
        inserts.push((*start, format!("`{BARE}")));
        inserts.push((*end, "`".to_owned()));
    }
    let mut inserts: Vec<(usize, usize, String)> = inserts
        .into_iter()
        .enumerate()
        .filter(|(_, (at, _))| text.is_char_boundary(*at))
        .map(|(order, (at, insert))| (at, order, insert))
        .collect();
    inserts.sort_by_key(|(at, order, _)| (*at, std::cmp::Reverse(*order)));
    let mut out =
        String::with_capacity(text.len() + inserts.iter().map(|i| i.2.len()).sum::<usize>());
    let mut copied = 0;
    for (at, _, insert) in inserts {
        out.push_str(&text[copied..at]);
        out.push_str(&insert);
        copied = at;
    }
    out.push_str(&text[copied..]);
    Prepared {
        text: Arc::from(out),
        labeled: Arc::new(found.labeled.iter().map(|(_, url)| url.clone()).collect()),
    }
}

#[cfg(test)]
fn parse(text: &str) -> Prepared {
    parse_from(text, Found::default(), 0).prepared
}

/// Copied markdown without what [`prepare`] added: the site icons before links, as the
/// source or as the image a partial copy rebuilds, and the links around local images.
pub fn unprepare(text: &str) -> String {
    let icons = [format!("<img src=\"{SCHEME}"), format!("![]({SCHEME}")];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'scan: while !rest.is_empty() {
        for (index, start) in icons.iter().enumerate() {
            if let Some(after) = rest.strip_prefix(start.as_str()) {
                let close = if index == 0 { '>' } else { ')' };
                if let Some(end) = after.find(close) {
                    let tail = &after[end + 1..];
                    rest = tail.strip_prefix('\u{a0}').unwrap_or(tail);
                    continue 'scan;
                }
            }
        }
        if let Some(after) = rest
            .strip_prefix('`')
            .and_then(|after| after.strip_prefix(BARE))
            && let Some((path, tail)) = after.split_once('`')
        {
            out.push_str(path);
            rest = tail;
            continue;
        }
        if let Some(after) = rest.strip_prefix("[![")
            && let Some((image, tail)) = local_image_link(after)
        {
            out.push_str(&image);
            rest = tail;
            continue;
        }
        let mut chars = rest.chars();
        if let Some(char) = chars.next() {
            out.push(char);
        }
        rest = chars.as_str();
    }
    out
}

/// `alt](url)](url)` after a `[![`: the image alone, and what follows the link.
fn local_image_link(after: &str) -> Option<(String, &str)> {
    let (alt, after) = after.split_once("](")?;
    let (url, after) = after.split_once(')')?;
    let tail = after
        .strip_prefix("](")?
        .strip_prefix(url)?
        .strip_prefix(')')?;
    (!alt.contains(']')).then(|| (format!("![{alt}]({url})"), tail))
}

#[cfg(test)]
mod tests {
    use super::{parse, prepare, unprepare};

    #[test]
    fn a_streamed_text_prepares_as_a_whole_parse_of_it_does() {
        let texts = [
            "Intro with https://github.com/x/y and /home/dev/a.rs.\n\n- [one](https://a.io/1)\n- two C:\\dev\\b.rs\n\n```rust\nlet x = \"https://no.io\";\n```\n\nSee [docs](https://docs.rs/z) and ![shot](C:/tmp/shot.png) after.\n",
            "para about https://a.io\n``` x `y` https://b.io\nmore https://c.io\n\nTitle https://d.io\n---\n\n| a | b |\n| - | - |\n| https://e.io | /home/dev/c.rs |\n",
            "> quote https://a.io\nlazy https://b.io\n\n1. first\n\n   still first https://c.io\n2. second\n\n    indented https://f.io\n\nend",
            "Use [a][ref] here.\n\n[ref]: https://g.io\n",
        ];
        for text in texts {
            for step in [1, 3, 17] {
                let chars: Vec<char> = text.chars().collect();
                for end in (step..chars.len()).step_by(step).chain([chars.len()]) {
                    let prefix: String = chars[..end].iter().collect();
                    let streamed = prepare(&prefix);
                    let whole = parse(&prefix);
                    assert_eq!(streamed.text, whole.text, "{prefix:?}");
                    assert_eq!(streamed.labeled, whole.labeled, "{prefix:?}");
                }
            }
        }
    }

    #[test]
    fn web_links_get_an_icon_and_labeled_ones_their_address() {
        let prepared = parse(
            "See [the repo](https://bitbucket.org/a/b), https://github.com/x/y and `https://no.code`.",
        );
        assert_eq!(
            &*prepared.text,
            "See <img src=\"hoshi-site:bitbucket.org\" alt=\"\" width=\"14\" height=\"14\">\u{a0}[the repo](https://bitbucket.org/a/b), <img src=\"hoshi-site:github.com\" alt=\"\" width=\"14\" height=\"14\">\u{a0}https://github.com/x/y and `https://no.code`."
        );
        assert!(prepared.labeled.contains("https://bitbucket.org/a/b"));
        assert!(!prepared.labeled.contains("https://github.com/x/y"));
    }

    #[test]
    fn a_path_written_as_text_becomes_code_that_copies_as_written() {
        let text = "Archivo: C:\\Users\\dev\\.hoshi\\chats\\a\\chatbot.html, and /home/dev/x.rs:3.\n\nNot /help, `C:\\a\\b.rs` or C:/a/…/b.rs.";
        let prepared = parse(text);
        assert_eq!(
            &*prepared.text,
            "Archivo: `\u{2060}C:\\Users\\dev\\.hoshi\\chats\\a\\chatbot.html`, and `\u{2060}/home/dev/x.rs:3`.\n\nNot /help, `C:\\a\\b.rs` or C:/a/…/b.rs."
        );
        assert_eq!(unprepare(&prepared.text), text);
    }

    #[test]
    fn local_links_get_no_icon() {
        let prepared = parse("[main](C:/a/main.rs)");
        assert_eq!(&*prepared.text, "[main](C:/a/main.rs)");
        assert!(prepared.labeled.contains("C:/a/main.rs"));
    }

    #[test]
    fn a_local_image_becomes_a_link_to_its_file() {
        let prepared = parse("Here: ![shot](C:/tmp/shot.png) and ![web](https://x.io/a.png)");
        assert_eq!(
            &*prepared.text,
            "Here: [![shot](C:/tmp/shot.png)](C:/tmp/shot.png) and ![web](https://x.io/a.png)"
        );
    }

    #[test]
    fn a_copy_loses_what_preparing_added() {
        let source = "See [the repo](https://bitbucket.org/a/b), https://github.com/x/y and ![shot](C:/tmp/shot.png).";
        assert_eq!(unprepare(&parse(source).text), source);
        assert_eq!(
            unprepare("![](hoshi-site:github.com)\u{a0}[x](https://github.com/x) and [![a]](b)"),
            "[x](https://github.com/x) and [![a]](b)"
        );
    }
}
