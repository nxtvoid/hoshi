//! The composer's chips: files, pull requests and skills mentioned in the text, and the images
//! and files attached to the draft, as the web composer shows them.

use super::attachments::{AttachmentKind, Source, format_size};
use crate::drafts::{Chip, Draft};
use gpui_kit::component::Icon;
use gpui_kit::component::input::InlineTokenContext;
use gpui_kit::*;
use hoshi_core::MessageAttachment;
use hoshi_ui::prelude::*;
use std::collections::{HashMap, HashSet};

/// What an attachment chip shows besides its label: its kind, size and, for an image, a preview.
#[derive(Clone)]
pub struct ChipInfo {
    pub kind: AttachmentKind,
    pub size: u64,
    pub source: Source,
}

/// The id of an attachment's chip.
pub fn attachment_chip_id(key: u64) -> String {
    format!("attachment:{key}")
}

/// The key an attachment chip's id names.
pub fn attachment_key(id: &str) -> Option<u64> {
    id.strip_prefix("attachment:")?.parse().ok()
}

/// How the text writes an attachment's chip until sending numbers it.
pub fn attachment_chip_source(kind: AttachmentKind, key: u64) -> String {
    match kind {
        AttachmentKind::Image => format!("[image:{key}]"),
        AttachmentKind::File => format!("[file:{key}]"),
    }
}

/// The prompt as sent: each attachment chip becomes its label in brackets, such as
/// `[Image #1]` or `[File #2]`, the way Claude Code names pasted images. A chip whose
/// attachment is gone goes too. The space a chip was placed with joins any typed after it;
/// the rest of the text keeps its spaces.
pub fn number_attachment_chips(text: &str, labels: &HashMap<u64, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let chip = tail.find(']').and_then(|end| {
            let inner = &tail[1..end];
            let (kind, key) = inner.split_once(':')?;
            let key: u64 = key.parse().ok()?;
            matches!(kind, "image" | "file").then_some((key, end))
        });
        match chip {
            Some((key, end)) => {
                let after = &tail[end + 1..];
                let spaced = after.trim_start_matches([' ', '\t']);
                let label = labels.get(&key);
                if let Some(label) = label {
                    out.push('[');
                    out.push_str(label);
                    out.push(']');
                }
                let spaced_before = out.is_empty() || out.ends_with([' ', '\t', '\n']);
                if spaced.len() < after.len() && (label.is_some() || !spaced_before) {
                    out.push(' ');
                }
                rest = spaced;
            }
            None => {
                out.push('[');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out.trim().to_owned()
}

/// `Image #n` and `File #n` for attachments in the order they are sent.
pub fn attachment_labels(ordered: &[(u64, AttachmentKind)]) -> HashMap<u64, String> {
    let (mut images, mut files) = (0, 0);
    ordered
        .iter()
        .map(|(key, kind)| {
            let label = match kind {
                AttachmentKind::Image => {
                    images += 1;
                    format!("Image #{images}")
                }
                AttachmentKind::File => {
                    files += 1;
                    format!("File #{files}")
                }
            };
            (*key, label)
        })
        .collect()
}

/// A sent message back as a draft: each `[Image #n]` and `[File #n]` its text names becomes
/// the chip of the nth image or file it was sent with, and that attachment goes with its chip.
pub fn chip_attachment_labels(mut draft: Draft) -> Draft {
    let of_kind = |wanted: AttachmentKind| -> Vec<(u64, String)> {
        draft
            .attachments
            .iter()
            .filter_map(|kept| match (&kept.attachment, wanted) {
                (MessageAttachment::Image { name, .. }, AttachmentKind::Image)
                | (MessageAttachment::File { name, .. }, AttachmentKind::File) => {
                    Some((kept.key, name.clone()))
                }
                _ => None,
            })
            .collect()
    };
    let (images, files) = (
        of_kind(AttachmentKind::Image),
        of_kind(AttachmentKind::File),
    );
    let mut chipped = HashSet::new();
    for block in &mut draft.blocks {
        if block.kind.starts_with("code") || !block.chips.is_empty() {
            continue;
        }
        let mut text = String::with_capacity(block.text.len());
        let mut rest = block.text.as_str();
        while let Some(start) = rest.find('[') {
            text.push_str(&rest[..start]);
            let tail = &rest[start..];
            let found = tail.find(']').and_then(|end| {
                let inner = &tail[1..end];
                let (kind, list, number) = match inner.strip_prefix("Image #") {
                    Some(number) => (AttachmentKind::Image, &images, number),
                    None => (AttachmentKind::File, &files, inner.strip_prefix("File #")?),
                };
                let number: usize = number.parse().ok()?;
                let (key, name) = list.get(number.checked_sub(1)?)?;
                Some((kind, *key, name, end))
            });
            let Some((kind, key, name, end)) = found else {
                text.push('[');
                rest = &tail[1..];
                continue;
            };
            let source = attachment_chip_source(kind, key);
            let at = text.len();
            text.push_str(&source);
            block.chips.push(Chip {
                start: at,
                end: text.len(),
                id: attachment_chip_id(key),
                text: source,
                label: name.clone(),
            });
            chipped.insert(key);
            rest = &tail[end + 1..];
        }
        text.push_str(rest);
        block.text = text;
    }
    for kept in &mut draft.attachments {
        kept.chipped |= chipped.contains(&kept.key);
    }
    draft
}

/// An attachment put back after a send failed: the text comes back without its chips, so it
/// shows as a tile and goes with the next message.
pub fn unchipped(
    mut attachment: super::attachments::DraftAttachment,
) -> super::attachments::DraftAttachment {
    attachment.chipped = false;
    attachment
}

/// A chip in the composer's text.
pub fn render_chip(
    context: &InlineTokenContext,
    attachments: &HashMap<u64, ChipInfo>,
    colors: &Colors,
) -> AnyElement {
    let token = context.token();
    let id = token.id().as_ref();
    let tint = if id.starts_with("pr:") {
        colors.pr_open
    } else if id.starts_with("skill:") {
        colors.provider_claude
    } else {
        colors.link
    };
    let attachment = attachment_key(id).and_then(|key| attachments.get(&key));
    let leading: Option<AnyElement> = attachment.map(|info| match info.kind {
        AttachmentKind::Image => img(info.source.preview())
            .size(px(14.))
            .rounded(px(3.))
            .object_fit(ObjectFit::Cover)
            .into_any_element(),
        AttachmentKind::File => Icon::new(IconName::FileText).size_3p5().into_any_element(),
    });
    div()
        .flex()
        .items_center()
        .gap_1()
        .px_0p5()
        .h(context.line_height())
        .max_w(context.available_width())
        .rounded_sm()
        .bg(if context.is_selected() {
            colors.ring.opacity(0.3)
        } else {
            tint.opacity(0.12)
        })
        .text_color(tint)
        .children(leading)
        .child(div().min_w_0().truncate().child(token.label().clone()))
        .when_some(attachment, |chip, info| {
            chip.child(
                div()
                    .flex_none()
                    .text_size(rems(0.75))
                    .text_color(colors.muted_foreground)
                    .child(format_size(info.size)),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{
        AttachmentKind, attachment_labels, chip_attachment_labels, number_attachment_chips,
    };
    use crate::rewind::message_draft;
    use hoshi_core::MessageAttachment;

    #[test]
    fn a_sent_message_gets_its_attachment_chips_back() {
        let image = |name: &str| MessageAttachment::Image {
            id: name.into(),
            name: name.into(),
            mime_type: "image/png".into(),
            size: 1,
        };
        let file = MessageAttachment::File {
            id: "f".into(),
            name: "spec.pdf".into(),
            mime_type: "application/pdf".into(),
            size: 1,
        };
        let draft = chip_attachment_labels(message_draft(
            "see [Image #2] and [File #1], not [Image #3] or [link]\n\nthen [Image #1]",
            &[image("a.png"), file, image("b.png")],
        ));
        let first = &draft.blocks[0];
        assert_eq!(
            first.text,
            "see [image:3] and [file:2], not [Image #3] or [link]"
        );
        let chips: Vec<(&str, &str, &str)> = first
            .chips
            .iter()
            .map(|chip| {
                assert_eq!(&first.text[chip.start..chip.end], chip.text);
                (chip.id.as_str(), chip.text.as_str(), chip.label.as_str())
            })
            .collect();
        assert_eq!(
            chips,
            [
                ("attachment:3", "[image:3]", "b.png"),
                ("attachment:2", "[file:2]", "spec.pdf")
            ]
        );
        assert_eq!(draft.blocks.last().unwrap().text, "then [image:1]");
        assert!(draft.attachments.iter().all(|kept| kept.chipped));
        let unnamed = chip_attachment_labels(message_draft("just text", &[image("a.png")]));
        assert!(!unnamed.attachments[0].chipped);
    }

    #[test]
    fn chips_become_numbered_labels_and_lost_ones_go() {
        let labels = attachment_labels(&[
            (7, AttachmentKind::Image),
            (3, AttachmentKind::File),
            (9, AttachmentKind::Image),
        ]);
        assert_eq!(
            number_attachment_chips("see [image:9] and [file:3]  then [image:7]", &labels),
            "see [Image #2] and [File #1] then [Image #1]"
        );
        assert_eq!(
            number_attachment_chips("gone [image:4] here [link] a[b", &labels),
            "gone here [link] a[b"
        );
        let indented = "look at\n  a.rs  [image:7]  \n```\n    fn main() {}\n\tdone\n```";
        assert_eq!(
            number_attachment_chips(indented, &labels),
            "look at\n  a.rs  [Image #1] \n```\n    fn main() {}\n\tdone\n```"
        );
    }
}
