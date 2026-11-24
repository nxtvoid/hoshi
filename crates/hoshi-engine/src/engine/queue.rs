use crate::*;

impl Engine {
    /// Has the threads whose turn the last close or crash cut short go on where they left off,
    /// told of the background work that ended with it. Once, after the adapters and settings
    /// are set.
    pub fn continue_cut_threads(&self) {
        lock(&self.state).continue_cut_threads();
    }

    /// The messages queued for the thread that wait for the user to go on with it, as after a
    /// restart or a failed turn.
    pub fn held_queue(&self, thread_id: &str) -> Vec<HeldMessage> {
        lock(&self.state).held_queue(thread_id)
    }

    /// Sends what waits in the thread's held queue, one turn each.
    pub fn resume_queue(&self, thread_id: &str) {
        lock(&self.state).release_queue(thread_id);
    }

    /// The messages waiting in the thread's queue, in the order they go, and whether the
    /// user can resume it. [`Notification::Queue`] tells of each change.
    pub fn thread_queue(&self, thread_id: &str) -> hoshi_core::ThreadQueue {
        lock(&self.state).thread_queue(thread_id)
    }

    /// Goes on with the stopped thread: when its last turn was stopped, cut short or failed on
    /// its usage limit and nothing waits for an answer, its agent is told to continue first;
    /// then what waited in its queue follows.
    pub fn resume_thread(&self, thread_id: &str) -> Result<(), CommandError> {
        let resumable = {
            let state = lock(&self.state);
            if state.model.thread(thread_id).is_none() {
                return Err(CommandError::ThreadNotFound {
                    thread_id: thread_id.to_owned(),
                });
            }
            state.resumable(thread_id) && state.model.pending_requests(thread_id).is_empty()
        };
        if resumable {
            self.dispatch(Command::MessageSend {
                thread_id: thread_id.to_owned(),
                message_id: delegation::new_id(),
                text: recovery::LIMIT_RESET_PROMPT.to_owned(),
                attachments: Vec::new(),
            })?;
        }
        lock(&self.state).release_queue(thread_id);
        Ok(())
    }

    /// Starts a turn with one of the user's queued messages, while the thread runs none; the
    /// rest of its queue follows.
    pub fn send_queued_now(&self, thread_id: &str, message_id: &str) -> Result<(), CommandError> {
        lock(&self.state).send_queued_now(thread_id, message_id)
    }

    /// Takes a message out of the thread's queue; the transcript shows it never read.
    pub fn remove_queued_message(
        &self,
        thread_id: &str,
        message_id: &str,
    ) -> Result<(), CommandError> {
        lock(&self.state).remove_queued(thread_id, Some(message_id))
    }

    /// Takes every message out of the thread's queue.
    pub fn clear_queue(&self, thread_id: &str) -> Result<(), CommandError> {
        lock(&self.state).remove_queued(thread_id, None)
    }

    /// Whether the project's threads a restart cut short go on by themselves, as it sets for
    /// itself or inherits from `general.continue_after_restart`; `None` for chats.
    pub fn continue_after_restart_in(
        &self,
        project_id: Option<&str>,
    ) -> hoshi_core::EffectiveSetting<bool> {
        lock(&self.state).continue_after_restart_in(project_id)
    }

    /// When the thread goes on by itself, as RFC 3339: once the usage limit that failed its
    /// turn resets.
    pub fn usage_limit_resume(&self, thread_id: &str) -> Option<String> {
        lock(&self.state).limit_resume(thread_id)
    }

    /// The thread no longer goes on by itself once its usage limit resets, whatever the
    /// settings say.
    pub fn cancel_usage_limit_resume(&self, thread_id: &str) {
        lock(&self.state).choose_limit_resume(thread_id, false);
    }

    /// The thread goes on by itself once the usage limit that failed its turn resets, whatever
    /// the settings say. Whether it will: not when no reset is ahead.
    pub fn resume_at_usage_limit_reset(&self, thread_id: &str) -> bool {
        lock(&self.state).choose_limit_resume(thread_id, true)
    }

    /// Threads whose turn fails on their usage limit are snoozed until it resets.
    pub fn set_snooze_until_limit_resets(&self, on: bool) {
        lock(&self.state).recovery.snooze_until_limit_resets = on;
    }

    /// Snoozes the thread until the usage limit that failed its turn resets. Whether it did:
    /// not when no reset is ahead, or the thread is archived or waits for the user.
    pub fn snooze_until_usage_limit_resets(&self, thread_id: &str) -> bool {
        lock(&self.state).snooze_until_limit_resets(thread_id)
    }

    /// The thread's snooze is the one its usage limit put it in, until the reset; waking it
    /// with [`Command::ThreadWake`] drops only that snooze, not the resumption.
    pub fn usage_limit_snoozed(&self, thread_id: &str) -> bool {
        lock(&self.state).limit_snoozed(thread_id)
    }

    /// Compacts the thread's conversation with `/compact`, then sends the message, as when
    /// the user comes back to a long thread whose prompt cache ran out. A thread whose turn
    /// runs takes the message at once instead.
    pub fn compact_then_send(
        &self,
        thread_id: &str,
        message_id: &str,
        text: String,
        attachments: Vec<MessageAttachment>,
    ) -> Result<(), CommandError> {
        let mut state = lock(&self.state);
        let busy = state
            .model
            .thread(thread_id)
            .ok_or_else(|| CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            })?
            .active_turn_id
            .is_some();
        let send = Command::MessageSend {
            thread_id: thread_id.to_owned(),
            message_id: message_id.to_owned(),
            text: text.clone(),
            attachments: attachments.clone(),
        };
        if busy {
            if !state.queue_to_switch(&send) {
                state.run_command(send)?;
            }
            state.release_queue_after(thread_id, Some(message_id));
            return Ok(());
        }
        state.hold_until_compacted(
            thread_id,
            QueuedMessage {
                id: message_id.to_owned(),
                text,
                attachments,
                from_user: true,
                held: false,
                to: None,
            },
        );
        let compacted = state.run_command(Command::MessageSend {
            thread_id: thread_id.to_owned(),
            message_id: delegation::new_id(),
            text: COMPACT_COMMAND.to_owned(),
            attachments: Vec::new(),
        });
        if compacted.is_err()
            && state
                .after_compaction
                .get(thread_id)
                .is_some_and(|message| message.id == message_id)
        {
            state.take_held(thread_id);
        }
        compacted
    }

    /// Stops compacting the thread, so the message it holds goes at once.
    pub fn send_held_message_now(&self, thread_id: &str) -> Result<(), CommandError> {
        let mut state = lock(&self.state);
        if !state.after_compaction.contains_key(thread_id) {
            return Ok(());
        }
        // A restart ended the compaction it waited for.
        if !state.turns.contains_key(thread_id) {
            state.send_after_compaction(thread_id);
            return Ok(());
        }
        state.run_command(Command::TurnInterrupt {
            thread_id: thread_id.to_owned(),
        })
    }

    /// Takes back the message the thread holds until it is compacted, for the user to edit
    /// it. `discard` drops it, with the images it carried.
    pub fn take_held_message(&self, thread_id: &str, discard: bool) -> Option<HeldMessage> {
        let mut state = lock(&self.state);
        let message = state.take_held(thread_id)?;
        if discard {
            for attachment in &message.attachments {
                state.attachments.remove(attachment);
            }
        }
        Some(held_message(thread_id, &message))
    }
}
