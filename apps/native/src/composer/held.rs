//! Above the composer while the thread's conversation is compacted: the message that goes
//! once it is, which the user can send at once, take back to edit, or drop.

use super::Composer;
use super::attachments;
use crate::m;
use gpui_kit::*;
use hoshi_core::{HeldMessage, MessageAttachment};
use hoshi_ui::prelude::*;

/// How many images and files a held message carries.
pub fn attachment_counts(attachments: &[MessageAttachment]) -> (usize, usize) {
    attachments
        .iter()
        .fold((0, 0), |(images, files), attachment| match attachment {
            MessageAttachment::Image { .. } => (images + 1, files),
            MessageAttachment::File { .. } => (images, files + 1),
            _ => (images, files),
        })
}

impl Composer {
    fn held_thread(&self, cx: &App) -> Option<(String, HeldMessage)> {
        let workspace = self.workspace.read(cx);
        let thread_id = workspace.open_thread_id(cx)?;
        let held = workspace.held_messages.get(&thread_id)?.clone();
        Some((thread_id, held))
    }

    fn send_held_now(&mut self, cx: &mut Context<Self>) {
        let Some((thread_id, _)) = self.held_thread(cx) else {
            return;
        };
        let engine = self.workspace.read(cx).engine.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { engine.send_held_message_now(&thread_id) })
                .await;
            if let Err(error) = result {
                this.update(cx, |this, cx| {
                    this.error = Some(super::command_error_message(&error).into());
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Takes the held message back into the composer to edit it, or drops it with `discard`.
    pub(super) fn take_back_held(
        &mut self,
        discard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((thread_id, _)) = self.held_thread(cx) else {
            return;
        };
        let engine = self.workspace.read(cx).engine.clone();
        cx.spawn_in(window, async move |this, cx| {
            let taken = cx
                .background_spawn(async move { engine.take_held_message(&thread_id, discard) })
                .await;
            let Some(taken) = taken.filter(|_| !discard) else {
                return;
            };
            this.update_in(cx, |this, window, cx| this.fill(taken, window, cx))
                .ok();
        })
        .detach();
    }

    /// Puts a message back in the composer: its text, and its images and files as tiles.
    pub(super) fn fill(
        &mut self,
        message: HeldMessage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.read(cx).open_thread_id(cx).as_deref() != Some(&message.thread_id) {
            return;
        }
        let engine = self.workspace.read(cx).engine.clone();
        for attachment in message.attachments {
            let Some(path) = engine.attachment_path(&attachment) else {
                continue;
            };
            self.next_attachment += 1;
            let kept = crate::drafts::Attachment {
                key: self.next_attachment,
                chipped: false,
                attachment,
            };
            self.attachments
                .extend(attachments::restored(&kept, path.into()));
        }
        self.set_text(&message.text, window, cx);
        self.focus(window, cx);
        cx.notify();
    }

    pub(super) fn render_held(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (_, held) = self.held_thread(cx)?;
        let colors = cx.hoshi();
        let (images, files) = attachment_counts(&held.attachments);
        let count = |count: usize, text: fn(usize) -> String| {
            (count > 0).then(|| {
                div()
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .child(text(count))
            })
        };
        let bubble = div()
            .max_w(relative(0.85))
            .rounded_xl()
            .border_1()
            .border_dashed()
            .border_color(colors.muted_foreground.opacity(0.4))
            .px_3()
            .py_2()
            .text_sm()
            .when(!held.text.is_empty(), |bubble| {
                bubble.child(div().line_clamp(3).child(held.text.clone()))
            })
            .children(count(images, m::held_message_images))
            .children(count(files, m::held_message_files));
        let actions = div()
            .flex()
            .items_center()
            .gap_0p5()
            .child(
                Button::new("held-send-now")
                    .icon(IconName::ArrowUp)
                    .label(m::held_message_send_now())
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Ghost)
                    .tooltip(Tooltip::new(m::held_message_send_now_hint()).side(Side::Top))
                    .on_click(cx.listener(|this, _, _, cx| this.send_held_now(cx))),
            )
            .child(
                Button::new("held-edit")
                    .icon(IconName::Pencil)
                    .label(m::held_message_edit())
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Ghost)
                    .text_color(colors.muted_foreground)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.take_back_held(false, window, cx)),
                    ),
            )
            .child(
                Button::new("held-discard")
                    .icon(IconName::X)
                    .label(m::held_message_discard())
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Ghost)
                    .text_color(colors.muted_foreground)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.take_back_held(true, window, cx)),
                    ),
            );
        Some(
            div()
                .id("held-message")
                .mb_2()
                .flex()
                .flex_col()
                .items_end()
                .gap_1()
                .child(bubble)
                .child(actions)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::attachment_counts;
    use hoshi_core::MessageAttachment;

    #[test]
    fn images_and_files_are_counted_apart() {
        let image = MessageAttachment::Image {
            id: "i".into(),
            name: "a.png".into(),
            mime_type: "image/png".into(),
            size: 1,
        };
        let file = MessageAttachment::File {
            id: "f".into(),
            name: "a.txt".into(),
            mime_type: "text/plain".into(),
            size: 1,
        };
        assert_eq!(attachment_counts(&[]), (0, 0));
        assert_eq!(attachment_counts(&[image.clone(), file, image]), (2, 1));
    }
}
