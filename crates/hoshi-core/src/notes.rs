use crate::MessageAttachment;
use crate::credentials::{redact_credentials, unignored};
use serde::{Deserialize, Serialize};
use std::ops::Range;
use ts_rs::TS;

const TITLE_CHARS: usize = 80;
const PREVIEW_CHARS: usize = 160;
/// Stands for a credential in a note's title and preview.
const MASK: &str = "••••••";
const HINT_CHARS: usize = 4;

/// A note as it is kept: the user's own text, private to them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredNote {
    pub id: String,
    /// `None` for a general note.
    pub project_id: Option<String>,
    /// The note as markdown: what is searched, checked for credentials and read by agents.
    pub text: String,
    /// The same note as its editor shows it, in the editor's own JSON, which hoshi never
    /// reads. `None` for a note only ever written as text.
    pub document: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// What looked like credentials and the user said are none.
    pub ignored: Vec<String>,
}

/// A note open in its editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Note {
    pub id: String,
    pub project_id: Option<String>,
    /// Its first line, with any credential in it masked.
    pub title: String,
    /// What follows the title, on one line, masked the same way.
    pub preview: String,
    pub text: String,
    /// The note as its editor left it, in the editor's own JSON; `None` to show `text`.
    pub document: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// What looks like a credential in the text, each by its first characters, in order.
    pub credentials: Vec<String>,
}

/// A note in a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NoteSummary {
    pub id: String,
    pub project_id: Option<String>,
    /// Its first line; empty for an empty note.
    pub title: String,
    /// What follows the title, on one line.
    pub preview: String,
    pub updated_at: String,
    /// How many credentials the text seems to hold. The title and preview mask them.
    pub credentials: u32,
}

/// What saving a note answers with: how a list shows it now, and what its editor flags.
/// The caller has the rest, having just sent it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavedNote {
    pub summary: NoteSummary,
    /// What looks like a credential in the text, each by its first characters, in order.
    pub credentials: Vec<String>,
}

/// What a list of notes is searched for: text, whatever its case.
pub struct NoteSearch(String);

impl NoteSearch {
    /// `None` for a query that asks for nothing, which every note answers.
    pub fn new(query: &str) -> Option<Self> {
        let query = query.trim().to_lowercase();
        (!query.is_empty()).then_some(Self(query))
    }

    pub fn finds(&self, text: &str) -> bool {
        text.to_lowercase().contains(&self.0)
    }
}

impl StoredNote {
    pub fn credentials(&self) -> Vec<Range<usize>> {
        unignored(&self.text, &self.ignored)
    }

    /// The note read once for all a save answers with.
    pub fn saved(&self) -> SavedNote {
        let credentials = self.credentials();
        let (title, preview) = title_and_preview(&masked(&self.text, &credentials));
        SavedNote {
            summary: NoteSummary {
                id: self.id.clone(),
                project_id: self.project_id.clone(),
                title,
                preview,
                updated_at: self.updated_at.clone(),
                credentials: credentials.len() as u32,
            },
            credentials: credentials
                .into_iter()
                .map(|range| hint(&self.text[range]))
                .collect(),
        }
    }

    pub fn open(&self) -> Note {
        let SavedNote {
            summary,
            credentials,
        } = self.saved();
        Note {
            id: summary.id,
            project_id: summary.project_id,
            title: summary.title,
            preview: summary.preview,
            text: self.text.clone(),
            document: self.document.clone(),
            created_at: self.created_at.clone(),
            updated_at: summary.updated_at,
            credentials,
        }
    }

    pub fn summary(&self) -> NoteSummary {
        self.saved().summary
    }

    /// The note as a message carries it to an agent, with its credentials left out.
    pub fn attachment(&self) -> MessageAttachment {
        let (text, hidden) = redact_credentials(&self.text, &self.ignored);
        let (title, _) = title_and_preview(&masked(&self.text, &self.credentials()));
        MessageAttachment::Note {
            id: self.id.clone(),
            title,
            text: Some(text.trim().to_owned()),
            hidden,
        }
    }
}

fn masked(text: &str, credentials: &[Range<usize>]) -> String {
    let mut masked = String::with_capacity(text.len());
    let mut cursor = 0;
    for range in credentials {
        masked.push_str(&text[cursor..range.start]);
        masked.push_str(MASK);
        cursor = range.end;
    }
    masked.push_str(&text[cursor..]);
    masked
}

fn title_and_preview(text: &str) -> (String, String) {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let title = lines
        .next()
        .map(|line| line.trim_start_matches('#').trim_start())
        .unwrap_or_default();
    let mut preview = String::new();
    for line in lines {
        for word in line.split_whitespace() {
            if !preview.is_empty() {
                preview.push(' ');
            }
            preview.push_str(word);
        }
        if preview.chars().count() >= PREVIEW_CHARS {
            break;
        }
    }
    (cut(title, TITLE_CHARS), cut(&preview, PREVIEW_CHARS))
}

fn cut(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

/// The start of a credential, enough for the user to tell which one it is.
fn hint(credential: &str) -> String {
    let shown = HINT_CHARS.min(credential.chars().count() / 2).max(1);
    let mut hint: String = credential.chars().take(shown).collect();
    hint.push('…');
    hint
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(text: &str) -> StoredNote {
        StoredNote {
            id: "n".into(),
            project_id: None,
            text: text.into(),
            document: None,
            created_at: "t".into(),
            updated_at: "t".into(),
            ignored: Vec::new(),
        }
    }

    #[test]
    fn the_first_line_names_a_note_and_the_rest_previews_it() {
        let summary =
            note("\n## Cache ideas\n\ntry a 5 min TTL\n  measure   with 20 threads\n").summary();

        assert_eq!(summary.title, "Cache ideas");
        assert_eq!(summary.preview, "try a 5 min TTL measure with 20 threads");
        assert_eq!(summary.credentials, 0);
        assert_eq!(note("  \n").summary().title, "");
    }

    #[test]
    fn a_list_never_shows_a_credential() {
        let summary =
            note("staging password: hunter2\ntoken ghp_a1B2c3D4e5F6g7H8i9J0k1L2m3").summary();

        assert_eq!(summary.title, "staging password: ••••••");
        assert_eq!(summary.preview, "token ••••••");
        assert_eq!(summary.credentials, 2);
    }

    #[test]
    fn an_open_note_names_its_credentials_by_their_start() {
        let mut stored = note("password: hunter2\nkey ghp_a1B2c3D4e5F6g7H8i9J0k1L2m3");

        assert_eq!(stored.open().credentials, ["hun…", "ghp_…"]);

        stored.ignored = vec!["hunter2".into()];
        assert_eq!(stored.open().credentials, ["ghp_…"]);
        assert_eq!(stored.summary().title, "password: hunter2");
    }

    #[test]
    fn an_attached_note_leaves_its_credentials_out() {
        let attachment = note("Deploy\npassword: hunter2\n").attachment();

        assert_eq!(
            attachment,
            MessageAttachment::Note {
                id: "n".into(),
                title: "Deploy".into(),
                text: Some("Deploy\npassword: [hidden credential]".into()),
                hidden: 1,
            }
        );
    }

    #[test]
    fn a_search_ignores_case_and_an_empty_one_asks_for_nothing() {
        let text = "Revisar el CACHÉ de PRs";

        assert!(NoteSearch::new(" caché ").unwrap().finds(text));
        assert!(!NoteSearch::new("deploy").unwrap().finds(text));
        assert!(NoteSearch::new("  ").is_none());
    }
}
