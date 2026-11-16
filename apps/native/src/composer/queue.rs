//! Above the composer: the messages queued for the thread, which wait for their turn or for
//! the user to go on with it, and the Resume that goes on with it.

use super::Composer;
use super::held::attachment_counts;
use crate::m;
use gpui_kit::*;
use hoshi_core::{CommandError, MessageSender, QueueEntry, ThreadQueue};
use hoshi_engine::Engine;
use hoshi_ui::prelude::*;

/// What a queued message's row shows: its first line, or what it carries when it has no text.
pub fn entry_label(entry: &QueueEntry) -> String {
    if let Some(line) = entry.text.lines().find(|line| !line.trim().is_empty()) {
        return line.trim().to_owned();
    }
    let (images, files) = attachment_counts(&entry.attachments);
    let mut parts = Vec::new();
    if images > 0 {
        parts.push(m::held_message_images(images));
    }
    if files > 0 {
        parts.push(m::held_message_files(files));
    }
    parts.join(", ")
}

impl Composer {
    /// The open thread's queue, while it holds messages or the thread can be resumed.
    pub(super) fn queue(&self, cx: &App) -> Option<ThreadQueue> {
        let workspace = self.workspace.read(cx);
        let thread_id = workspace.open_thread_id(cx)?;
        workspace.queues.get(&thread_id).cloned()
    }

    fn on_queue(
        &mut self,
        run: impl FnOnce(&Engine, &str) -> Result<(), CommandError> + Send + 'static,
        failed: fn(&CommandError) -> String,
        cx: &mut Context<Self>,
    ) {
        let workspace = self.workspace.read(cx);
        let Some(thread_id) = workspace.open_thread_id(cx) else {
            return;
        };
        let engine = workspace.engine.clone();
        self.error = None;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { run(&engine, &thread_id) })
                .await;
            if let Err(error) = result {
                this.update(cx, |this, cx| {
                    this.error = Some(failed(&error).into());
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub(super) fn resume(&mut self, cx: &mut Context<Self>) {
        self.on_queue(
            |engine, thread_id| engine.resume_thread(thread_id),
            |error| match error {
                CommandError::ThreadBusy { .. } | CommandError::BackgroundWorkRunning { .. } => {
                    super::command_error_message(error)
                }
                _ => m::queue_resume_failed(),
            },
            cx,
        );
    }

    fn send_queued_now(&mut self, message_id: String, cx: &mut Context<Self>) {
        self.on_queue(
            move |engine, thread_id| engine.send_queued_now(thread_id, &message_id),
            super::command_error_message,
            cx,
        );
    }

    fn remove_queued(&mut self, message_id: String, cx: &mut Context<Self>) {
        self.on_queue(
            move |engine, thread_id| engine.remove_queued_message(thread_id, &message_id),
            super::command_error_message,
            cx,
        );
    }

    fn clear_queue(&mut self, cx: &mut Context<Self>) {
        self.on_queue(
            |engine, thread_id| engine.clear_queue(thread_id),
            super::command_error_message,
            cx,
        );
    }

    pub(super) fn render_queue(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let queue = self.queue(cx)?;
        if queue.messages.is_empty() {
            return None;
        }
        let idle = {
            let workspace = self.workspace.read(cx);
            workspace
                .model
                .thread(&queue.thread_id)
                .is_some_and(|thread| thread.active_turn_id.is_none())
        };
        let colors = cx.hoshi().clone();
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .pl_2p5()
            .pr_1()
            .py_0p5()
            .text_xs()
            .text_color(colors.muted_foreground)
            .child(m::queue_count(queue.messages.len()))
            .child(
                Button::new("queue-clear")
                    .label(m::queue_clear())
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Ghost)
                    .text_color(colors.muted_foreground)
                    .on_click(cx.listener(|this, _, _, cx| this.clear_queue(cx))),
            );
        let rows = queue.messages.iter().map(|entry| {
            let id = entry.message_id.clone();
            let send_now = idle && entry.sender == MessageSender::User;
            let (send, remove) = (id.clone(), id.clone());
            div()
                .debug_selector(move || format!("queued-{id}"))
                .flex()
                .items_center()
                .gap_2()
                .pl_2p5()
                .pr_1()
                .py_1()
                .border_t_1()
                .border_color(colors.border)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .child(entry_label(entry)),
                )
                .when(send_now, |row| {
                    let id = format!("queue-send-{send}");
                    row.child(
                        div()
                            .debug_selector({
                                let id = id.clone();
                                move || id.clone()
                            })
                            .child(
                                Button::new(SharedString::from(id))
                                    .label(m::queue_send_now())
                                    .size(ButtonSize::Xs)
                                    .variant(ButtonVariant::Outline)
                                    .tooltip(Tooltip::new(m::queue_send_now_hint()).side(Side::Top))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.send_queued_now(send.clone(), cx)
                                    })),
                            ),
                    )
                })
                .child({
                    let id = format!("queue-remove-{remove}");
                    div()
                        .debug_selector({
                            let id = id.clone();
                            move || id.clone()
                        })
                        .child(
                            Button::new(SharedString::from(id))
                                .icon(IconName::X)
                                .size(ButtonSize::IconXs)
                                .variant(ButtonVariant::Ghost)
                                .text_color(colors.muted_foreground)
                                .tooltip(Tooltip::new(m::queue_remove()).side(Side::Top))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.remove_queued(remove.clone(), cx)
                                })),
                        )
                })
        });
        Some(
            div()
                .debug_selector(|| "queued-messages".into())
                .mx_2p5()
                .rounded_t_xl()
                .border_1()
                .border_b_0()
                .border_color(colors.border)
                .bg(colors.card)
                .child(header)
                .child(
                    div()
                        .id("queued-list")
                        .max_h(px(168.))
                        .overflow_y_scroll()
                        .children(rows),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::entry_label;
    use hoshi_core::{MessageAttachment, MessageSender, QueueEntry};

    fn entry(text: &str, attachments: Vec<MessageAttachment>) -> QueueEntry {
        QueueEntry {
            message_id: "m".into(),
            text: text.into(),
            attachments,
            held: true,
            sender: MessageSender::User,
        }
    }

    #[test]
    fn a_row_shows_the_first_line_or_what_the_message_carries() {
        assert_eq!(
            entry_label(&entry("\n  Also add tests\nfor the parser", Vec::new())),
            "Also add tests"
        );
        let image = MessageAttachment::Image {
            id: "i".into(),
            name: "a.png".into(),
            mime_type: "image/png".into(),
            size: 1,
        };
        assert_eq!(
            entry_label(&entry(" ", vec![image])),
            crate::m::held_message_images(1)
        );
    }
}
