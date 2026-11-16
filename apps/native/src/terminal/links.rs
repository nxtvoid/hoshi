//! Web addresses in a terminal's output, found the way the web terminal finds them.

use std::ops::Range;

const SCHEMES: [&str; 2] = ["https://", "http://"];
const TRAILING_PUNCTUATION: [char; 5] = ['.', ',', ';', '!', '?'];

/// A web address in a line, as character offsets into it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub url: String,
    pub chars: Range<usize>,
}

fn ends_address(character: char) -> bool {
    character.is_whitespace() || matches!(character, '"' | '\'' | '`' | '<' | '>')
}

fn starts_with_scheme(line: &[char], at: usize) -> Option<usize> {
    SCHEMES.iter().find_map(|scheme| {
        let length = scheme.chars().count();
        let candidate = line.get(at..at + length)?;
        candidate
            .iter()
            .zip(scheme.chars())
            .all(|(a, b)| a.eq_ignore_ascii_case(&b))
            .then_some(length)
    })
}

/// Drops sentence punctuation and closing brackets the address didn't open.
fn trim_end(url: &[char]) -> &[char] {
    let mut end = url.len();
    while end > 0 && TRAILING_PUNCTUATION.contains(&url[end - 1]) {
        end -= 1;
    }
    for (open, close) in [('(', ')'), ('[', ']'), ('{', '}')] {
        while end > 0 && url[end - 1] == close {
            let opens = url[..end].iter().filter(|&&c| c == open).count();
            let closes = url[..end].iter().filter(|&&c| c == close).count();
            if opens >= closes {
                break;
            }
            end -= 1;
        }
    }
    &url[..end]
}

/// Whether a link's target is a web address, the only kind a terminal opens.
pub fn is_web_address(target: &str) -> bool {
    let chars: Vec<char> = target.chars().collect();
    starts_with_scheme(&chars, 0).is_some_and(|scheme| chars.len() > scheme)
}

pub fn find_links(line: &[char]) -> Vec<Found> {
    let mut links = Vec::new();
    let mut at = 0;
    while at < line.len() {
        let Some(scheme) = starts_with_scheme(line, at) else {
            at += 1;
            continue;
        };
        let mut end = at + scheme;
        while end < line.len() && !ends_address(line[end]) {
            end += 1;
        }
        if end == at + scheme {
            at = end;
            continue;
        }
        let url = trim_end(&line[at..end]);
        if url.len() > "https://".len() {
            links.push(Found {
                url: url.iter().collect(),
                chars: at..at + url.len(),
            });
        }
        at = end;
    }
    links
}

#[cfg(test)]
mod tests {
    use super::{find_links, is_web_address};

    #[test]
    fn only_web_addresses_are_opened() {
        assert!(is_web_address("https://example.com"));
        assert!(is_web_address("HTTP://x.dev"));
        assert!(!is_web_address("https://"));
        assert!(!is_web_address("file:///C:/run.exe"));
        assert!(!is_web_address("mailto:a@b.dev"));
    }

    fn urls(line: &str) -> Vec<(String, usize, usize)> {
        let chars: Vec<char> = line.chars().collect();
        find_links(&chars)
            .into_iter()
            .map(|link| (link.url, link.chars.start, link.chars.end))
            .collect()
    }

    #[test]
    fn addresses_are_found_with_their_offsets() {
        assert_eq!(
            urls("see https://example.com/a?b=1 and HTTP://x.dev/"),
            vec![
                ("https://example.com/a?b=1".into(), 4, 29),
                ("HTTP://x.dev/".into(), 34, 47),
            ]
        );
        assert!(urls("no links here, just ftp://x and https://").is_empty());
    }

    #[test]
    fn sentence_punctuation_and_unopened_brackets_stay_out() {
        assert_eq!(urls("go to https://a.dev/x.")[0].0, "https://a.dev/x");
        assert_eq!(urls("(https://a.dev/x)")[0].0, "https://a.dev/x");
        assert_eq!(
            urls("https://en.wikipedia.org/wiki/Rust_(language))")[0].0,
            "https://en.wikipedia.org/wiki/Rust_(language)"
        );
        assert_eq!(urls("'https://a.dev/q'")[0].0, "https://a.dev/q");
    }

    #[test]
    fn offsets_count_characters_not_bytes() {
        assert_eq!(
            urls("→ https://a.dev"),
            vec![("https://a.dev".into(), 2, 15)]
        );
    }
}
