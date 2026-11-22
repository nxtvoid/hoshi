use crate::*;

impl State {
    /// Keeps the message until the thread is compacted, where the user sees it wait.
    pub(crate) fn hold_until_compacted(&mut self, thread_id: &str, message: QueuedMessage) {
        (self.notify)(Notification::HeldMessage(ThreadHeldMessage {
            thread_id: thread_id.to_owned(),
            message: Some(held_message(thread_id, &message)),
        }));
        self.after_compaction.insert(thread_id.to_owned(), message);
        self.save_recovery(thread_id);
    }

    pub(crate) fn take_held(&mut self, thread_id: &str) -> Option<QueuedMessage> {
        let message = self.after_compaction.remove(thread_id)?;
        self.save_recovery(thread_id);
        (self.notify)(Notification::HeldMessage(ThreadHeldMessage {
            thread_id: thread_id.to_owned(),
            message: None,
        }));
        Some(message)
    }

    /// The message the user sent to go once the context was compacted.
    pub(crate) fn send_after_compaction(&mut self, thread_id: &str) {
        let idle = self
            .model
            .thread(thread_id)
            .is_some_and(|thread| thread.active_turn_id.is_none());
        if !idle {
            return;
        }
        let Some(message) = self.take_held(thread_id) else {
            return;
        };
        let message_id = message.id;
        let send = Command::MessageSend {
            thread_id: thread_id.to_owned(),
            message_id: message_id.clone(),
            text: message.text,
            attachments: message.attachments,
        };
        if let Err(error) = self.run_command(send) {
            eprintln!("hoshi: failed to send a message after compacting: {error}");
            return;
        }
        self.release_queue_after(thread_id, Some(&message_id));
    }

    /// When the idle thread compacts before its prompt cache expires; `None` while it does
    /// not, as when its provider doesn't or something is still due to reach its agent.
    pub(crate) fn idle_compaction_window(&self, thread_id: &str) -> Option<Range<u64>> {
        let thread = self.model.thread(thread_id)?;
        let busy = thread.active_turn_id.is_some()
            || thread.archived
            || self.snoozed(thread_id)
            || thread.parent_thread_id.is_some()
            || self.lingering.contains_key(thread_id)
            || self
                .queued
                .get(thread_id)
                .is_some_and(|queued| !queued.is_empty())
            || self.after_compaction.contains_key(thread_id)
            || self.thread_ops.contains_key(thread_id)
            || self
                .pending_wakes
                .get(thread_id)
                .is_some_and(|wakes| !wakes.is_empty())
            || self
                .delegations
                .values()
                .any(|task| task.parent == thread_id)
            || !self.model.pending_requests(thread_id).is_empty();
        let compacts = self
            .adapter_for(&thread.provider)
            .is_some_and(|adapter| adapter.compacts_idle_threads());
        if busy || !compacts {
            return None;
        }
        compaction_before_expiry(self.idle_compaction, self.context_usages.get(thread_id))
    }

    pub(crate) fn schedule_idle_compaction(&mut self, thread_id: &str) {
        match self.idle_compaction_window(thread_id) {
            Some(window) => {
                self.idle_compactions
                    .insert(thread_id.to_owned(), window.start);
            }
            None => {
                self.idle_compactions.remove(thread_id);
            }
        }
    }

    /// Sends `/compact` to the idle threads whose time came, while their prompt cache holds.
    pub(crate) fn compact_idle_threads(&mut self, now: u64) {
        let due: Vec<String> = self
            .idle_compactions
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(thread_id, _)| thread_id.clone())
            .collect();
        for thread_id in due {
            self.idle_compactions.remove(&thread_id);
            let open = self
                .idle_compaction_window(&thread_id)
                .is_some_and(|window| window.contains(&now));
            if !open {
                continue;
            }
            let compact = Command::MessageSend {
                thread_id,
                message_id: delegation::new_id(),
                text: COMPACT_COMMAND.to_owned(),
                attachments: Vec::new(),
            };
            if let Err(error) = self.run_command(compact) {
                eprintln!("hoshi: failed to compact an idle thread: {error}");
            }
        }
    }

    /// Starts the oldest message waiting for its own turn, such as one sent while the user
    /// stopped the running turn, unless it waits for the user.
    pub(crate) fn start_next_queued(&mut self, thread_id: &str) {
        let idle = self
            .model
            .thread(thread_id)
            .is_some_and(|thread| thread.active_turn_id.is_none());
        let Some(queued) = self.queued.get_mut(thread_id).filter(|_| idle) else {
            return;
        };
        let Some(message) = queued
            .iter()
            .position(|message| !message.held)
            .and_then(|next| queued.remove(next))
        else {
            return;
        };
        self.save_recovery(thread_id);
        self.start_queued(thread_id, message);
        self.show_queue(thread_id);
    }

    /// Starts a turn of its own for a message taken out of the idle thread's queue.
    pub(crate) fn start_queued(&mut self, thread_id: &str, message: QueuedMessage) {
        let now = now();
        let kept = self
            .store
            .thread_item(thread_id, &message.id)
            .ok()
            .flatten();
        let events = start_queued_turn(&self.model, thread_id, &message.id, kept.as_ref(), &now);
        if events.is_empty() {
            return;
        }
        if let Err(error) = self.commit(&events, &now) {
            eprintln!("hoshi: failed to start a queued message: {error}");
            return;
        }
        start_turn(
            self,
            thread_id,
            &message.id,
            message.text,
            message.attachments,
            None,
        );
    }
}
