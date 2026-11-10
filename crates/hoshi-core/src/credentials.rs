//! Finds what looks like a credential in free text, so a note can warn about it and keep
//! it from agents. A guess that errs toward the well-known shapes: tokens with a vendor's
//! prefix, private keys, passwords in URLs, what follows a label such as `password:`, and
//! long random-looking strings.

use std::ops::Range;

/// What an agent reads in place of a credential.
pub const REDACTED: &str = "[hidden credential]";

const PRIVATE_KEY_START: &str = "-----BEGIN ";
const PRIVATE_KEY_LABEL: &str = "PRIVATE KEY-----";
const PRIVATE_KEY_END: &str = "-----END ";

/// Prefixes vendors give their tokens, with the least that must follow.
const TOKEN_PREFIXES: [(&str, usize); 30] = [
    ("sk-ant-", 20),
    ("sk-proj-", 20),
    ("sk-", 20),
    ("sk_live_", 12),
    ("sk_test_", 12),
    ("rk_live_", 12),
    ("whsec_", 16),
    ("ghp_", 20),
    ("gho_", 20),
    ("ghu_", 20),
    ("ghs_", 20),
    ("ghr_", 20),
    ("github_pat_", 20),
    ("glpat-", 16),
    ("xoxb-", 16),
    ("xoxp-", 16),
    ("xoxa-", 16),
    ("xoxs-", 16),
    ("xapp-", 16),
    ("AIza", 30),
    ("ATBB", 16),
    ("ATATT", 20),
    ("npm_", 20),
    ("hf_", 20),
    ("dop_v1_", 20),
    ("shpat_", 20),
    ("figd_", 20),
    ("ya29.", 20),
    ("AKIA", 16),
    ("ASIA", 16),
];

/// Labels that name a secret wherever they end a word, as in `DB_PASSWORD`.
const LABEL_ENDINGS: [&str; 8] = [
    "password",
    "passwd",
    "passphrase",
    "secret",
    "token",
    "apikey",
    "contraseña",
    "credential",
];
/// Labels too short to trust inside a longer word: `pass`, but not `bypass`.
const LABEL_WORDS: [&str; 4] = ["pass", "pwd", "clave", "pin"];
/// What makes a label ending in `key` name a secret.
const KEY_KINDS: [&str; 8] = [
    "api",
    "secret",
    "access",
    "private",
    "auth",
    "license",
    "encryption",
    "signing",
];
/// Say how a credential is sent; the credential follows.
const SCHEMES: [&str; 2] = ["bearer", "basic"];
const PLACEHOLDERS: [&str; 12] = [
    "true",
    "false",
    "null",
    "none",
    "nil",
    "undefined",
    "yes",
    "no",
    "todo",
    "tbd",
    "n/a",
    "changeme",
];

/// A word of the text, without the punctuation around it.
struct Token<'a> {
    text: &'a str,
    start: usize,
}

impl Token<'_> {
    fn range(&self) -> Range<usize> {
        self.start..self.start + self.text.len()
    }
}

/// The byte ranges of `text` that look like credentials, in order and apart.
pub fn find_credentials(text: &str) -> Vec<Range<usize>> {
    let mut found = private_keys(text);
    let tokens = tokens(text);
    let mut labelled: Option<Labelled> = None;
    for (index, token) in tokens.iter().enumerate() {
        let in_key = found
            .iter()
            .any(|key| key.start <= token.start && token.start < key.end);
        let expected = labelled.take();
        if in_key {
            continue;
        }
        if let Some(password) = url_password(token) {
            found.push(password);
            continue;
        }
        if has_token_prefix(token.text) || is_jwt(token.text) {
            found.push(token.range());
            continue;
        }
        if matches!(token.text, "=" | ":" | "=>") {
            labelled = expected.map(|_| Labelled::Separated);
            continue;
        }
        let next = tokens.get(index + 1);
        let announced = match expected {
            Some(Labelled::Separated) => is_secret_value(text, next, token),
            Some(Labelled::Bare) => is_strong(token.text) && token.text.chars().count() >= 8,
            None => false,
        };
        if announced {
            found.push(token.range());
            continue;
        }
        if let Some((label, value)) = assignment(token)
            && is_label(label)
        {
            if value.text.is_empty() {
                labelled = Some(Labelled::Separated);
            } else if is_secret_value(text, next, &value) {
                found.push(value.range());
            }
            continue;
        }
        if is_label(token.text) || SCHEMES.contains(&token.text.to_lowercase().as_str()) {
            labelled = Some(Labelled::Bare);
            continue;
        }
        if is_opaque(token.text) {
            found.push(token.range());
        }
    }
    found.sort_by_key(|range| range.start);
    found.dedup_by(|next, kept| next.start < kept.end);
    found
}

/// What follows `label:`: something strong, or the only word left on its line.
fn is_secret_value(text: &str, next: Option<&Token<'_>>, value: &Token<'_>) -> bool {
    stands_for_value(value.text) && (is_strong(value.text) || ends_line(text, next, value))
}

/// `text` with each credential replaced by [`REDACTED`], and how many there were. The
/// credentials the user said are none, in `ignored`, stay.
pub fn redact_credentials(text: &str, ignored: &[String]) -> (String, u32) {
    let ranges = unignored(text, ignored);
    let mut redacted = String::with_capacity(text.len());
    let mut cursor = 0;
    for range in &ranges {
        redacted.push_str(&text[cursor..range.start]);
        redacted.push_str(REDACTED);
        cursor = range.end;
    }
    redacted.push_str(&text[cursor..]);
    (redacted, ranges.len() as u32)
}

/// The credentials of `text` the user did not say are none.
pub fn unignored(text: &str, ignored: &[String]) -> Vec<Range<usize>> {
    find_credentials(text)
        .into_iter()
        .filter(|range| !ignored.iter().any(|value| value == &text[range.clone()]))
        .collect()
}

/// A label was read, and what follows may be its secret.
#[derive(Clone, Copy)]
enum Labelled {
    /// `password` alone: only something strong after it counts.
    Bare,
    /// `password:` or `password =`.
    Separated,
}

fn private_keys(text: &str) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(offset) = text[from..].find(PRIVATE_KEY_START) {
        let start = from + offset;
        let header_end = text[start..]
            .find('\n')
            .map_or(text.len(), |line| start + line);
        if !text[start..header_end].contains(PRIVATE_KEY_LABEL) {
            from = start + PRIVATE_KEY_START.len();
            continue;
        }
        let end = text[header_end..]
            .find(PRIVATE_KEY_END)
            .map(|offset| header_end + offset)
            .map_or(text.len(), |footer| {
                text[footer..]
                    .find('\n')
                    .map_or(text.len(), |line| footer + line)
            });
        found.push(start..end);
        from = end;
    }
    found
}

fn tokens(text: &str) -> Vec<Token<'_>> {
    text.split_whitespace()
        .filter_map(|word| {
            let offset = word.as_ptr() as usize - text.as_ptr() as usize;
            let trimmed = word.trim_start_matches(['"', '\'', '`', '(', '[', '{', '*']);
            let start = offset + (word.len() - trimmed.len());
            let trimmed = trimmed
                .trim_end_matches(['"', '\'', '`', ')', ']', '}', '*', ',', ';'])
                .trim_end_matches('.');
            (!trimmed.is_empty()).then_some(Token {
                text: trimmed,
                start,
            })
        })
        .collect()
}

/// `label=value` or `label:value` in one word; `value` is empty when the word ends there.
fn assignment<'a>(token: &Token<'a>) -> Option<(&'a str, Token<'a>)> {
    let at = token.text.find(['=', ':'])?;
    let label = &token.text[..at];
    if label.is_empty() {
        return None;
    }
    let rest = token.text[at + 1..].trim_start_matches(['"', '\'', '`']);
    let start = token.start + (token.text.len() - rest.len());
    Some((label, Token { text: rest, start }))
}

/// The password of `scheme://user:password@host`.
fn url_password(token: &Token<'_>) -> Option<Range<usize>> {
    let after_scheme = token.text.find("://")? + 3;
    let authority = token.text[after_scheme..].split(['/', '?', '#']).next()?;
    let user_info = &authority[..authority.rfind('@')?];
    let colon = user_info.find(':')?;
    let password = &user_info[colon + 1..];
    if password.is_empty() || !stands_for_value(password) {
        return None;
    }
    let start = token.start + after_scheme + colon + 1;
    Some(start..start + password.len())
}

fn has_token_prefix(word: &str) -> bool {
    TOKEN_PREFIXES.iter().any(|(prefix, least)| {
        word.strip_prefix(prefix).is_some_and(|rest| {
            rest.len() >= *least
                && rest.chars().any(|char| char.is_ascii_digit())
                && rest.chars().all(is_token_char)
        })
    })
}

fn is_jwt(word: &str) -> bool {
    word.len() >= 40
        && word.starts_with("eyJ")
        && word.matches('.').count() == 2
        && word.chars().all(is_token_char)
}

fn is_token_char(char: char) -> bool {
    char.is_ascii_alphanumeric() || matches!(char, '_' | '-' | '.' | '=' | '/' | '+')
}

fn is_label(word: &str) -> bool {
    let word = word
        .trim_matches(['"', '\'', '`', '*', '_'])
        .trim_end_matches([':', '='])
        .to_lowercase();
    if word.is_empty() || word.chars().count() > 40 {
        return false;
    }
    LABEL_ENDINGS.iter().any(|ending| word.ends_with(ending))
        || LABEL_WORDS.iter().any(|ending| {
            word.strip_suffix(ending)
                .is_some_and(|before| before.is_empty() || before.ends_with(['_', '-', '.']))
        })
        || (word.ends_with("key") && KEY_KINDS.iter().any(|kind| word.contains(kind)))
}

/// Not a placeholder, a reference to a variable, or a row of one character.
fn stands_for_value(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    value.chars().count() >= 3
        && !value.starts_with(['$', '<', '%', '{'])
        && !chars.all(|char| char == first)
        && !PLACEHOLDERS.contains(&value.to_lowercase().as_str())
}

/// Letters with digits: not a word of prose.
fn is_strong(value: &str) -> bool {
    value.chars().count() >= 6
        && value.chars().any(|char| char.is_ascii_digit())
        && value.chars().any(char::is_alphabetic)
}

/// Whether `token` is the last word of its line.
fn ends_line(text: &str, next: Option<&Token<'_>>, token: &Token<'_>) -> bool {
    let end = token.start + token.text.len();
    let until = next.map_or(text.len(), |next| next.start);
    text[end..until].contains('\n') || next.is_none()
}

/// A long string of mixed letters and digits no word looks like, as a random key does.
fn is_opaque(word: &str) -> bool {
    if word.len() < 24 || !word.is_ascii() {
        return false;
    }
    let padded = word.ends_with('=') || word.contains('+');
    let allowed = |char: char| {
        char.is_ascii_alphanumeric()
            || matches!(char, '_' | '-' | '=' | '+')
            || (padded && char == '/')
    };
    if !word.chars().all(allowed) {
        return false;
    }
    let count = |test: fn(&char) -> bool| word.chars().filter(test).count();
    let digits = count(char::is_ascii_digit);
    let upper = count(char::is_ascii_uppercase);
    let lower = count(char::is_ascii_lowercase);
    let mut longest_lower_run = 0;
    let mut run = 0;
    for char in word.chars() {
        run = if char.is_ascii_lowercase() {
            run + 1
        } else {
            0
        };
        longest_lower_run = longest_lower_run.max(run);
    }
    digits >= 3 && upper >= 1 && lower >= 1 && longest_lower_run < 7
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> Vec<&str> {
        find_credentials(text)
            .into_iter()
            .map(|range| &text[range])
            .collect()
    }

    #[test]
    fn vendor_tokens_are_found_by_their_prefix() {
        let text =
            "github ghp_a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8 and\nAWS AKIAIOSFODNN7EXAMPLE1 here";

        assert_eq!(
            found(text),
            [
                "ghp_a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8",
                "AKIAIOSFODNN7EXAMPLE1"
            ]
        );
        assert!(found("install sk-learn and sk-something-descriptive-here").is_empty());
    }

    #[test]
    fn a_value_after_a_label_is_found_and_prose_is_not() {
        assert_eq!(found("password: admin"), ["admin"]);
        assert_eq!(found("DB_PASSWORD=hunter2 # staging"), ["hunter2"]);
        assert_eq!(found("api_key = \"abc123def\""), ["abc123def"]);
        assert_eq!(
            found("notes\n\nAPI_KEY:7741_hTbQweRTYUI_oPLKJ"),
            ["7741_hTbQweRTYUI_oPLKJ"]
        );
        assert_eq!(
            found("API_KEY:7741_hTbQweRTYUI_oPLKJ and more words after it\non two lines"),
            ["7741_hTbQweRTYUI_oPLKJ"]
        );
        assert_eq!(found("user nxtvoid, pass ATBB3x9kQ"), ["ATBB3x9kQ"]);
        assert_eq!(
            found("Authorization: Bearer abc123def456"),
            ["abc123def456"]
        );
        assert!(found("the password should be rotated every month").is_empty());
        assert!(found("password: ask the team for it").is_empty());
        assert!(found("bypass: enabled1 for now").is_empty());
        assert!(found("token: $GITHUB_TOKEN").is_empty());
        assert!(found("secret: <paste here>").is_empty());
    }

    #[test]
    fn a_private_key_is_one_credential_to_its_footer() {
        let text = "deploy key\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk\nAAAAB3NzaC1\n-----END OPENSSH PRIVATE KEY-----\nkeep it safe";

        assert_eq!(
            found(text),
            [
                "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk\nAAAAB3NzaC1\n-----END OPENSSH PRIVATE KEY-----"
            ]
        );
        assert!(found("-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----").is_empty());
    }

    #[test]
    fn a_password_in_a_url_is_found_alone() {
        assert_eq!(
            found("postgres://app:s3cr3t-pw@localhost:5432/dev"),
            ["s3cr3t-pw"]
        );
        assert!(found("https://example.com/a:b@c and git@github.com:o/r.git").is_empty());
    }

    #[test]
    fn long_random_strings_are_found_and_code_is_not() {
        assert_eq!(
            found("key 9fK2mQ8xL0pZ4vB7nT1cW6yH3jR5dG"),
            ["9fK2mQ8xL0pZ4vB7nT1cW6yH3jR5dG"]
        );
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        assert_eq!(found(jwt), [jwt]);
        assert!(found("commit 3f2a9c1d8e7b6a5f4c3d2e1f0a9b8c7d6e5f4a3b").is_empty());
        assert!(
            found("see useComposerSuggestions2AndMore in src/components/Thread2View.tsx")
                .is_empty()
        );
        assert!(found("id 550e8400-e29b-41d4-a716-446655440000").is_empty());
    }

    #[test]
    fn redacting_hides_each_credential_but_the_ignored_ones() {
        let text = "user: nxtvoid\npassword: hunter2\ntoken: abc123def";

        assert_eq!(
            redact_credentials(text, &[]),
            (
                "user: nxtvoid\npassword: [hidden credential]\ntoken: [hidden credential]".into(),
                2
            )
        );
        assert_eq!(
            redact_credentials(text, &["abc123def".into()]),
            (
                "user: nxtvoid\npassword: [hidden credential]\ntoken: abc123def".into(),
                1
            )
        );
    }

    #[test]
    fn text_with_accents_and_emoji_is_read_whole() {
        assert_eq!(found("contraseña: añadir2día 🔑"), ["añadir2día"]);
        assert!(found("—— ¿clave? ——").is_empty());
    }
}
