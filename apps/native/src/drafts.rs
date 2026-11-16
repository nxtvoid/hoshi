//! Unsent composer drafts by where they belong, kept across restarts in `drafts.json` beside the
//! settings: the text block by block with its chips, the images and files the engine already
//! saved, and what the pull request and skill chips stand for.

use hoshi_core::MessageAttachment;
use hoshi_ui::prelude::{BlockKind, BlockSnapshot, ChipData};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const NEW_CHAT: &str = "new:chat";

/// Where a draft belongs: a thread, or a thread not started yet in a project or among the chats.
pub fn key(thread_id: Option<&str>, project_id: Option<&str>) -> String {
    match (thread_id, project_id) {
        (Some(thread_id), _) => thread_id.to_owned(),
        (None, Some(project_id)) => format!("new:{project_id}"),
        (None, None) => NEW_CHAT.to_owned(),
    }
}

/// The project a new thread's draft is for, `Some(None)` for a new chat, `None` for a thread's.
pub fn new_thread_project(key: &str) -> Option<Option<&str>> {
    match key.strip_prefix("new:")? {
        "chat" => Some(None),
        project_id => Some(Some(project_id)),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub blocks: Vec<Block>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// The pull requests and skills chips stand for, by chip id.
    #[serde(default)]
    pub mentions: Vec<(String, MessageAttachment)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub chips: Vec<Chip>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chip {
    pub start: usize,
    pub end: usize,
    pub id: String,
    pub text: String,
    pub label: String,
}

/// An image or file the engine saved for the draft.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    /// The key its chip in the text names.
    pub key: u64,
    /// Shown as a chip in the text, which it goes with.
    pub chipped: bool,
    pub attachment: MessageAttachment,
}

impl Draft {
    /// A draft of plain text, as older drafts were kept.
    pub fn from_text(text: &str) -> Self {
        Self {
            blocks: hoshi_ui::prelude::from_markdown(text)
                .into_iter()
                .map(|block| Block {
                    kind: kind_name(&block.kind),
                    text: block.text,
                    chips: Vec::new(),
                })
                .collect(),
            ..Self::default()
        }
    }

    pub fn from_snapshot(blocks: Vec<BlockSnapshot>) -> Self {
        Self {
            blocks: blocks
                .into_iter()
                .map(|block| Block {
                    kind: kind_name(&block.kind),
                    text: block.text,
                    chips: block
                        .chips
                        .into_iter()
                        .map(|chip| Chip {
                            start: chip.range.start,
                            end: chip.range.end,
                            id: chip.id,
                            text: chip.text,
                            label: chip.label,
                        })
                        .collect(),
                })
                .collect(),
            ..Self::default()
        }
    }

    pub fn snapshot(&self) -> Vec<BlockSnapshot> {
        self.blocks
            .iter()
            .map(|block| BlockSnapshot {
                kind: kind_of(&block.kind),
                text: block.text.clone(),
                chips: block
                    .chips
                    .iter()
                    .map(|chip| ChipData {
                        range: chip.start..chip.end,
                        id: chip.id.clone(),
                        text: chip.text.clone(),
                        label: chip.label.clone(),
                    })
                    .collect(),
            })
            .collect()
    }

    /// Holds nothing to send: no text and no attachments.
    pub fn is_empty(&self) -> bool {
        self.attachments.is_empty() && self.blocks.iter().all(|block| block.text.trim().is_empty())
    }
}

fn kind_name(kind: &BlockKind) -> String {
    match kind {
        BlockKind::Paragraph => "paragraph".into(),
        BlockKind::Heading(level) => format!("heading:{level}"),
        BlockKind::Quote => "quote".into(),
        BlockKind::Bullet => "bullet".into(),
        BlockKind::Ordered => "ordered".into(),
        BlockKind::Code(language) => format!("code:{language}"),
        BlockKind::Rule => "rule".into(),
    }
}

fn kind_of(name: &str) -> BlockKind {
    match name.split_once(':') {
        Some(("heading", level)) => BlockKind::Heading(level.parse().unwrap_or(1).clamp(1, 3)),
        Some(("code", language)) => BlockKind::Code(language.to_owned()),
        _ => match name {
            "quote" => BlockKind::Quote,
            "bullet" => BlockKind::Bullet,
            "ordered" => BlockKind::Ordered,
            "rule" => BlockKind::Rule,
            _ => BlockKind::Paragraph,
        },
    }
}

/// A draft as the file keeps it: older files hold plain text.
#[derive(Deserialize)]
#[serde(untagged)]
enum Stored {
    Full(Draft),
    Text(String),
}

pub fn file(home: &Path) -> PathBuf {
    home.join("drafts.json")
}

pub fn parse(text: &str) -> HashMap<String, Draft> {
    serde_json::from_str::<HashMap<String, Stored>>(text)
        .unwrap_or_default()
        .into_iter()
        .map(|(key, stored)| {
            let draft = match stored {
                Stored::Full(draft) => draft,
                Stored::Text(text) => Draft::from_text(&text),
            };
            (key, draft)
        })
        .filter(|(_, draft)| !draft.is_empty())
        .collect()
}

pub fn load(home: &Path) -> HashMap<String, Draft> {
    std::fs::read_to_string(file(home))
        .map(|text| parse(&text))
        .unwrap_or_default()
}

pub fn save(home: &Path, drafts: &HashMap<String, Draft>) -> std::io::Result<()> {
    std::fs::write(file(home), serde_json::to_string(drafts)?)
}

#[cfg(test)]
mod tests {
    use super::{Attachment, Draft, key, new_thread_project, parse};
    use hoshi_core::MessageAttachment;
    use hoshi_ui::prelude::{BlockKind, BlockSnapshot, ChipData};

    #[test]
    fn keys_name_a_thread_or_a_new_one() {
        assert_eq!(key(Some("t"), Some("p")), "t");
        assert_eq!(key(None, Some("p")), "new:p");
        assert_eq!(key(None, None), "new:chat");
        assert_eq!(new_thread_project("new:p"), Some(Some("p")));
        assert_eq!(new_thread_project("new:chat"), Some(None));
        assert_eq!(new_thread_project("t"), None);
    }

    #[test]
    fn a_draft_keeps_its_chips_and_attachments_through_the_file() {
        let mut draft = Draft::from_snapshot(vec![
            BlockSnapshot {
                kind: BlockKind::Code("rust".into()),
                text: "fn main() {}".into(),
                chips: Vec::new(),
            },
            BlockSnapshot {
                kind: BlockKind::Paragraph,
                text: "see [image:2]".into(),
                chips: vec![ChipData {
                    range: 4..13,
                    id: "attachment:2".into(),
                    text: "[image:2]".into(),
                    label: "shot.png".into(),
                }],
            },
        ]);
        draft.attachments.push(Attachment {
            key: 2,
            chipped: true,
            attachment: MessageAttachment::Image {
                id: "abc".into(),
                name: "shot.png".into(),
                mime_type: "image/png".into(),
                size: 3,
            },
        });
        let text = serde_json::to_string(
            &[("t", draft.clone())]
                .into_iter()
                .collect::<std::collections::HashMap<_, _>>(),
        )
        .unwrap();
        let back = parse(&text);
        assert_eq!(back["t"], draft);
        assert_eq!(back["t"].snapshot()[0].kind, BlockKind::Code("rust".into()));
    }

    #[test]
    fn older_plain_text_drafts_still_load_and_blank_ones_go() {
        let back = parse(r#"{"t": "half a thought", "u": "   "}"#);
        assert_eq!(back["t"].blocks[0].text, "half a thought");
        assert!(!back.contains_key("u"));
    }
}
