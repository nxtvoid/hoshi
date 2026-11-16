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

thread_local! {
    static PREPARED: RefCell<HashMap<Arc<str>, Prepared>> = RefCell::default();
}

/// `text` with its links' icons, read from a cache when it was prepared before.
pub fn prepare(text: &str) -> Prepared {
    if let Some(prepared) = PREPARED.with(|cache| cache.borrow().get(text).cloned()) {
        return prepared;
    }
    let prepared = parse(text);
    PREPARED.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= KEPT {
            cache.clear();
        }
        cache.insert(Arc::from(text), prepared.clone());
    });
    prepared
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
    labeled: HashSet<String>,
    bare_paths: Vec<(usize, usize)>,
}

fn collect(node: &Node, source: &str, found: &mut Found) {
    match node {
        Node::Link(link) => {
            if plain_text(node) != link.url {
                found.labeled.insert(link.url.clone());
            }
            if let (Some(host), Some(position)) = (sites::web_host(&link.url), &link.position) {
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

fn parse(text: &str) -> Prepared {
    let Ok(tree) = markdown::to_mdast(text, &markdown::ParseOptions::gfm()) else {
        return Prepared {
            text: Arc::from(text),
            labeled: Arc::default(),
        };
    };
    let mut found = Found::default();
    collect(&tree, text, &mut found);
    let mut inserts: Vec<(usize, String)> = found
        .icons
        .into_iter()
        .map(|(at, host)| {
            (
                at,
                format!("<img src=\"{SCHEME}{host}\" alt=\"\" width=\"14\" height=\"14\">\u{a0}"),
            )
        })
        .collect();
    for (start, end, url) in found.local_images {
        inserts.push((start, "[".to_owned()));
        inserts.push((end, format!("]({url})")));
    }
    for (start, end) in found.bare_paths {
        inserts.push((start, format!("`{BARE}")));
        inserts.push((end, "`".to_owned()));
    }
    inserts.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    let mut out = text.to_owned();
    for (at, insert) in inserts {
        if out.is_char_boundary(at) {
            out.insert_str(at, &insert);
        }
    }
    Prepared {
        text: Arc::from(out),
        labeled: Arc::new(found.labeled),
    }
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
    use super::{parse, unprepare};

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
