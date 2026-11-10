//! The user message Claude Code receives: the prompt's text, what its mentions
//! stand for, and its images.

use crate::TurnAttachment;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hoshi_core::{MessageAttachment, PullRequestState};
use serde_json::{Value, json};
use std::collections::HashSet;

/// The message's `content`: plain text unless it carries images or a skill.
pub(crate) fn message_content(prompt: &str, attachments: &[TurnAttachment]) -> Value {
    let skills: HashSet<&str> = attachments
        .iter()
        .filter_map(|attachment| match &attachment.attachment {
            MessageAttachment::Skill { name } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    let (text, last_skill) = with_slash_skills(prompt, &skills);
    let text = with_context(text, attachments);

    let mut blocks = Vec::new();
    match last_skill {
        Some(start) => {
            if !text[..start].trim().is_empty() {
                blocks.push(json!({ "type": "text", "text": text }));
            }
            blocks.push(json!({ "type": "text", "text": &text[start..] }));
        }
        None if !text.is_empty() => blocks.push(json!({ "type": "text", "text": text })),
        None => {}
    }
    blocks.extend(attachments.iter().filter_map(image_block));
    match blocks.as_slice() {
        [block] if block["type"] == "text" && last_skill.is_none() => block["text"].clone(),
        _ => Value::Array(blocks),
    }
}

/// Writes each `$name` of a known skill as `/name`, and says where the last one starts.
/// Claude expands a skill only when `/name` opens a text block, and only the last one.
fn with_slash_skills(prompt: &str, skills: &HashSet<&str>) -> (String, Option<usize>) {
    let mut text = String::with_capacity(prompt.len());
    let mut last = None;
    let mut rest = prompt;
    while let Some(index) = rest.find('$') {
        let (before, mention) = rest.split_at(index);
        text.push_str(before);
        let at_word_start = text.is_empty() || text.ends_with(char::is_whitespace);
        let name_end = mention[1..]
            .find(char::is_whitespace)
            .map_or(mention.len(), |end| end + 1);
        let name = &mention[1..name_end];
        if at_word_start && skills.contains(name) {
            last = Some(text.len());
            text.push('/');
            text.push_str(name);
            rest = &mention[name_end..];
        } else {
            text.push('$');
            rest = &mention[1..];
        }
    }
    text.push_str(rest);
    (text, last)
}

/// Adds a line per image and pull request, so the agent knows where each image is
/// saved, which one an `[Image #n]` in the prompt is, and what the `#123` mentions are.
/// Images are numbered in the order their blocks follow the text. A note the user
/// attached follows as its text, marked as theirs.
fn with_context(mut text: String, attachments: &[TurnAttachment]) -> String {
    let mut images = 0;
    for attachment in attachments {
        if matches!(attachment.attachment, MessageAttachment::Image { .. }) {
            images += 1;
        }
        let line = match (&attachment.attachment, &attachment.path) {
            (MessageAttachment::Image { name, .. }, Some(path)) => format!(
                "[Image #{images} \"{name}\" is saved at: {}]",
                path.display()
            ),
            (
                MessageAttachment::PullRequest {
                    number,
                    title,
                    url,
                    head_branch,
                    base_branch,
                    state,
                },
                _,
            ) => format!(
                "[Pull request #{number} \"{title}\" ({}, {head_branch} into {base_branch}): {url}]",
                state_name(*state)
            ),
            (
                MessageAttachment::Note {
                    title,
                    text,
                    hidden,
                    ..
                },
                _,
            ) => note_block(title, text.as_deref().unwrap_or_default(), *hidden),
            _ => continue,
        };
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&line);
    }
    text
}

/// A note of the user's own, attached to the message. It is their reference material, not
/// instructions from them, and says so.
fn note_block(title: &str, text: &str, hidden: u32) -> String {
    let title = title.replace('"', "'");
    let withheld = match hidden {
        0 => String::new(),
        1 => format!(
            " One credential in it was withheld and reads {}.",
            hoshi_core::REDACTED
        ),
        count => format!(
            " {count} credentials in it were withheld and read {}.",
            hoshi_core::REDACTED
        ),
    };
    format!(
        "<user_note title=\"{title}\">\n{text}\n</user_note>\n[The note above is one of the user's own notes, attached to this message for reference.{withheld}]"
    )
}

fn state_name(state: PullRequestState) -> &'static str {
    match state {
        PullRequestState::Open => "open",
        PullRequestState::Draft => "draft",
        PullRequestState::Merged => "merged",
        PullRequestState::Closed => "closed",
    }
}

/// An unreadable image is left out; its line still tells the agent where it was.
fn image_block(attachment: &TurnAttachment) -> Option<Value> {
    let MessageAttachment::Image { mime_type, .. } = &attachment.attachment else {
        return None;
    };
    let bytes = std::fs::read(attachment.path.as_ref()?).ok()?;
    Some(json!({
        "type": "image",
        "source": {
            "type": "base64",
            "media_type": mime_type,
            "data": STANDARD.encode(bytes),
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn skill(name: &str) -> TurnAttachment {
        TurnAttachment {
            attachment: MessageAttachment::Skill { name: name.into() },
            path: None,
        }
    }

    #[test]
    fn a_plain_prompt_stays_plain_text() {
        assert_eq!(
            message_content("fix it, costs $5", &[]),
            json!("fix it, costs $5")
        );
    }

    #[test]
    fn a_skill_mentioned_mid_prompt_opens_its_own_block() {
        let content = message_content("check this with $pdf please", &[skill("pdf")]);

        assert_eq!(
            content,
            json!([
                { "type": "text", "text": "check this with /pdf please" },
                { "type": "text", "text": "/pdf please" },
            ])
        );
    }

    #[test]
    fn a_skill_at_the_start_is_one_block_and_unknown_dollars_stay() {
        let content = message_content("$pdf read a$pdf and $other", &[skill("pdf")]);

        assert_eq!(
            content,
            json!([{ "type": "text", "text": "/pdf read a$pdf and $other" }])
        );
    }

    #[test]
    fn pull_requests_and_images_add_context_and_image_blocks() {
        let dir = std::env::temp_dir().join(format!("hoshi-prompt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shot.png");
        std::fs::write(&path, b"png").unwrap();
        let attachments = [
            TurnAttachment {
                attachment: MessageAttachment::PullRequest {
                    number: 12,
                    title: "Add login".into(),
                    url: "https://github.com/o/r/pull/12".into(),
                    head_branch: "login".into(),
                    base_branch: "main".into(),
                    state: PullRequestState::Open,
                },
                path: None,
            },
            TurnAttachment {
                attachment: MessageAttachment::Image {
                    id: "i".into(),
                    name: "shot.png".into(),
                    mime_type: "image/png".into(),
                    size: 3,
                },
                path: Some(path.clone()),
            },
            TurnAttachment {
                attachment: MessageAttachment::Image {
                    id: "gone".into(),
                    name: "gone.png".into(),
                    mime_type: "image/png".into(),
                    size: 3,
                },
                path: Some(PathBuf::from("/nowhere/gone.png")),
            },
        ];

        let content = message_content("review #12", &attachments);

        let text = format!(
            "review #12\n\n[Pull request #12 \"Add login\" (open, login into main): https://github.com/o/r/pull/12]\n\n[Image #1 \"shot.png\" is saved at: {}]\n\n[Image #2 \"gone.png\" is saved at: {}]",
            path.display(),
            PathBuf::from("/nowhere/gone.png").display()
        );
        assert_eq!(
            content,
            json!([
                { "type": "text", "text": text },
                {
                    "type": "image",
                    "source": { "type": "base64", "media_type": "image/png", "data": "cG5n" },
                },
            ])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_attached_note_follows_the_prompt_as_the_users_own() {
        let note = |hidden| TurnAttachment {
            attachment: MessageAttachment::Note {
                id: "n".into(),
                title: "Deploy \"staging\"".into(),
                text: Some("run make deploy\npassword: [hidden credential]".into()),
                hidden,
            },
            path: None,
        };

        assert_eq!(
            message_content("use my note", &[note(1)]),
            json!(
                "use my note\n\n<user_note title=\"Deploy 'staging'\">\nrun make deploy\npassword: [hidden credential]\n</user_note>\n[The note above is one of the user's own notes, attached to this message for reference. One credential in it was withheld and reads [hidden credential].]"
            )
        );
        let plain = message_content("", &[note(0)]);
        assert!(
            plain
                .as_str()
                .unwrap()
                .ends_with("attached to this message for reference.]")
        );
    }
}
