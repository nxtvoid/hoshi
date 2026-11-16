//! "Edit from here": takes the thread back to before one of the user's messages, which returns
//! to the composer. The files go back too when the user asks.

use crate::drafts::{Attachment, Draft};
use crate::m;
use crate::transcript::TranscriptView;
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::{CommandError, MessageAttachment};
use hoshi_engine::Engine;
use hoshi_ui::assets::IconName;
use hoshi_ui::prelude::*;
use std::sync::Arc;

/// The thread went back to before a message; its text and attachments return to the composer.
pub struct Rewound(pub Draft);

impl EventEmitter<Rewound> for TranscriptView {}

/// The message as a draft to write again, its attachments beside the text.
pub fn message_draft(text: &str, attachments: &[MessageAttachment]) -> Draft {
    Draft {
        attachments: attachments
            .iter()
            .enumerate()
            .map(|(index, attachment)| Attachment {
                key: index as u64 + 1,
                chipped: false,
                attachment: attachment.clone(),
            })
            .collect(),
        ..Draft::from_text(text)
    }
}

pub fn rewind_failure(error: &CommandError, thread_id: &str) -> String {
    match error {
        CommandError::NoCheckpoint { .. } => m::rewind_no_checkpoint(),
        CommandError::ThreadBusy { thread_id: busy } if busy == thread_id => m::rewind_busy(),
        CommandError::ThreadBusy { .. } => m::rewind_folder_busy(),
        CommandError::GitFailed { detail } => detail.clone(),
        _ => m::rewind_failed_detail(),
    }
}

pub struct RewindTarget {
    pub thread_id: String,
    pub message_id: String,
    pub text: String,
    pub attachments: Vec<MessageAttachment>,
}

fn rewind(
    target: &RewindTarget,
    engine: Arc<Engine>,
    view: WeakEntity<TranscriptView>,
    window: &mut Window,
    cx: &mut App,
) {
    let request = ConfirmRequest::new(m::edit_from_here_title(), m::revert_keep_changes())
        .description(m::edit_from_here_description())
        .cancel_label(m::cancel())
        .alternative(m::revert_files_too(), true)
        .tone(ConfirmTone::Warning)
        .icon(IconName::SquarePen);
    let choice = DialogLayer::choose(request, window, cx);
    let (thread_id, message_id) = (target.thread_id.clone(), target.message_id.clone());
    let draft = message_draft(&target.text, &target.attachments);
    window
        .spawn(cx, async move |cx| {
            let Some(choice) = choice.await else {
                return;
            };
            let revert_files = choice == ConfirmChoice::Alternative;
            let thread = thread_id.clone();
            let rewound = cx
                .background_spawn(async move {
                    engine.rewind_thread(&thread, &message_id, revert_files)
                })
                .await;
            match rewound {
                Ok(()) => {
                    view.update(cx, |_, cx| cx.emit(Rewound(draft))).ok();
                }
                Err(error) => {
                    cx.update(|window, cx| {
                        let notice = ConfirmRequest::new(m::rewind_failed(), m::dialog_ok())
                            .description(rewind_failure(&error, &thread_id))
                            .tone(ConfirmTone::Warning);
                        DialogLayer::confirm(notice, window, cx).detach();
                    })
                    .ok();
                }
            }
        })
        .detach();
}

/// The button beside a message's time; off while the agent works.
pub fn rewind_button(
    target: RewindTarget,
    busy: bool,
    engine: Option<Arc<Engine>>,
    view: WeakEntity<TranscriptView>,
    colors: &Colors,
) -> AnyElement {
    let id = SharedString::from(format!("rewind-{}", target.message_id));
    let enabled = !busy && engine.is_some();
    div()
        .id(id.clone())
        .debug_selector({
            let selector = id.to_string();
            move || selector.clone()
        })
        .size_6()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .text_color(colors.muted_foreground)
        .when(!enabled, |button| button.opacity(0.5))
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .hover(|button| button.bg(colors.muted).text_color(colors.foreground))
        })
        .child(Icon::new(IconName::SquarePen).size_3p5())
        .on_click(move |_, window, cx| {
            if let Some(engine) = engine.clone().filter(|_| enabled) {
                rewind(&target, engine, view.clone(), window, cx);
            }
        })
        .with_tooltip(m::edit_from_here())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{message_draft, rewind_failure};
    use hoshi_core::{CommandError, MessageAttachment};

    #[test]
    fn a_refused_rewind_says_why() {
        let busy = |thread_id: &str| CommandError::ThreadBusy {
            thread_id: thread_id.into(),
        };
        assert_eq!(rewind_failure(&busy("t"), "t"), crate::m::rewind_busy());
        assert_eq!(
            rewind_failure(&busy("other"), "t"),
            crate::m::rewind_folder_busy()
        );
        let missing = CommandError::NoCheckpoint {
            thread_id: "t".into(),
            message_id: "m".into(),
        };
        assert_eq!(
            rewind_failure(&missing, "t"),
            crate::m::rewind_no_checkpoint()
        );
        let git = CommandError::GitFailed {
            detail: "index.lock exists".into(),
        };
        assert_eq!(rewind_failure(&git, "t"), "index.lock exists");
    }

    #[test]
    fn a_rewound_message_comes_back_with_its_attachments() {
        let file = MessageAttachment::File {
            id: "f".into(),
            name: "a.pdf".into(),
            mime_type: "application/pdf".into(),
            size: 1,
        };
        let draft = message_draft("fix this", std::slice::from_ref(&file));
        assert_eq!(draft.blocks[0].text, "fix this");
        assert_eq!(draft.attachments.len(), 1);
        assert_eq!(draft.attachments[0].attachment, file);
        assert!(!draft.attachments[0].chipped);
    }
}
