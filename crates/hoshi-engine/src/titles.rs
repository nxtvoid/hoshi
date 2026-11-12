//! What a thread's generated title is read from.

use hoshi_core::{ItemContent, MessageAttachment, ThreadItem};

/// The whole thread is never sent; its start and its latest messages are.
const MAX_CHARS: usize = 8_000;
const FIRST_MESSAGE_CHARS: usize = 2_000;
const OMITTED: &str = "[…]";

/// The names of the images and files a message carries.
pub(crate) fn attachment_names(attachments: &[MessageAttachment]) -> Vec<String> {
    attachments
        .iter()
        .filter_map(|attachment| match attachment {
            MessageAttachment::Image { name, .. } | MessageAttachment::File { name, .. } => {
                Some(name.clone())
            }
            _ => None,
        })
        .collect()
}

/// The thread's messages as `USER:` and `ASSISTANT:` sections: its first user
/// message, then as many of the latest as fit. Reasoning and tools are left out.
pub(crate) fn conversation(items: &[ThreadItem]) -> (String, Vec<String>) {
    let mut images = Vec::new();
    let sections: Vec<(bool, String)> = items
        .iter()
        .filter_map(|item| match &item.content {
            ItemContent::UserMessage {
                text, attachments, ..
            } => {
                let names = attachment_names(attachments);
                let mut body = text.trim().to_owned();
                if !names.is_empty() {
                    body.push_str(&format!("\n[Attachments: {}]", names.join(", ")));
                }
                images.extend(names);
                Some((true, format!("USER:\n{}", body.trim())))
            }
            ItemContent::AssistantMessage { text } if !text.trim().is_empty() => {
                Some((false, format!("ASSISTANT:\n{}", text.trim())))
            }
            _ => None,
        })
        .collect();

    let first_user = sections.iter().position(|(user, _)| *user);
    let mut budget = MAX_CHARS;
    let mut kept = vec![None; sections.len()];
    if let Some(index) = first_user {
        let text = clip(&sections[index].1, FIRST_MESSAGE_CHARS);
        budget = budget.saturating_sub(text.chars().count());
        kept[index] = Some(text);
    }
    for (index, (_, text)) in sections.iter().enumerate().rev() {
        if kept[index].is_some() {
            continue;
        }
        let length = text.chars().count();
        if length > budget {
            break;
        }
        budget -= length;
        kept[index] = Some(text.clone());
    }

    let mut parts = Vec::new();
    let mut skipped = false;
    for text in kept {
        match text {
            Some(text) => {
                if skipped {
                    parts.push(OMITTED.to_owned());
                    skipped = false;
                }
                parts.push(text);
            }
            None => skipped = !parts.is_empty(),
        }
    }
    (parts.join("\n\n"), images)
}

fn clip(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => format!("{}…", &text[..index]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(content: ItemContent) -> ThreadItem {
        ThreadItem {
            id: "i".into(),
            thread_id: "t".into(),
            created_at: String::new(),
            content,
        }
    }

    fn user(text: &str) -> ThreadItem {
        item(ItemContent::UserMessage {
            text: text.into(),
            attachments: Vec::new(),
            steer: false,
            task_result: None,
            pull_request_update: None,
            rewind: None,
        })
    }

    fn assistant(text: &str) -> ThreadItem {
        item(ItemContent::AssistantMessage { text: text.into() })
    }

    #[test]
    fn the_conversation_keeps_messages_and_names_images() {
        let items = [
            item(ItemContent::UserMessage {
                steer: false,
                task_result: None,
                pull_request_update: None,
                rewind: None,
                text: "why is this red?".into(),
                attachments: vec![MessageAttachment::Image {
                    id: "1".into(),
                    name: "shot.png".into(),
                    mime_type: "image/png".into(),
                    size: 1,
                }],
            }),
            item(ItemContent::Reasoning {
                text: "thinking".into(),
            }),
            assistant("The badge uses the error token."),
        ];

        let (context, images) = conversation(&items);

        assert_eq!(
            context,
            "USER:\nwhy is this red?\n[Attachments: shot.png]\n\nASSISTANT:\nThe badge uses the error token."
        );
        assert_eq!(images, ["shot.png"]);
    }

    #[test]
    fn a_long_thread_keeps_its_start_and_its_latest_messages() {
        let middle = "m".repeat(MAX_CHARS);
        let items = [user("first ask"), assistant(&middle), user("latest ask")];

        let (context, _) = conversation(&items);

        assert_eq!(context, "USER:\nfirst ask\n\n[…]\n\nUSER:\nlatest ask");
    }
}
