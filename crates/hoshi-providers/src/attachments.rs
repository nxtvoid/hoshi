//! Lines that tell an agent about a message's attachments.

use crate::TurnAttachment;
use hoshi_core::{MessageAttachment, PullRequestState};

/// Adds a line per image, file and pull request, so the agent knows where each image and
/// file is saved, which one an `[Image #n]` or `[File #n]` in the prompt is, and what the
/// `#123` mentions are. Images and files are each numbered in the order they are attached.
pub(crate) fn with_context(mut text: String, attachments: &[TurnAttachment]) -> String {
    let (mut images, mut files) = (0, 0);
    for attachment in attachments {
        let line = match (&attachment.attachment, &attachment.path) {
            (MessageAttachment::Image { name, .. }, path) => {
                images += 1;
                let Some(path) = path else { continue };
                format!(
                    "[Image #{images} \"{name}\" is saved at: {}]",
                    path.display()
                )
            }
            (MessageAttachment::File { name, .. }, Some(path)) => {
                files += 1;
                let extracted = attachment
                    .extracted_text
                    .as_ref()
                    .map(|text| format!(" (text extracted to: {})", text.display()))
                    .unwrap_or_default();
                format!(
                    "[File #{files} \"{name}\" is saved at: {}{extracted}]",
                    path.display()
                )
            }
            (MessageAttachment::File { name, .. }, None) => {
                files += 1;
                format!("[File #{files} \"{name}\" is unavailable: it could not be found]")
            }
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
            _ => continue,
        };
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&line);
    }
    text
}

fn state_name(state: PullRequestState) -> &'static str {
    match state {
        PullRequestState::Open => "open",
        PullRequestState::Draft => "draft",
        PullRequestState::Merged => "merged",
        PullRequestState::Closed => "closed",
    }
}
