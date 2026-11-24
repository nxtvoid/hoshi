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
        let message = QueuedMessage {
            to: message
                .to
                .clone()
                .or_else(|| self.shown_choice(thread_id).map(Box::new)),
            ..message
        };
        let moves = self
            .model
            .thread(thread_id)
            .is_some_and(|thread| message.moves(thread));
        if moves && self.holds_off_switch(thread_id) {
            self.queue_message(thread_id, message);
            // It was sent before what queued while the thread compacted.
            if let Some(queued) = self.queued.get_mut(thread_id) {
                queued.rotate_right(1);
            }
            self.save_recovery(thread_id);
            self.show_queue(thread_id);
            return;
        }
        let message_id = message.id.clone();
        if let Err(error) = self.start_queued(thread_id, message) {
            eprintln!("hoshi: failed to send a message after compacting: {error}");
            self.show_queue(thread_id);
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
            || thread.pending_model.is_some()
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
        let Some(next) = queued.iter().position(|message| !message.held) else {
            return;
        };
        let moves = self
            .model
            .thread(thread_id)
            .is_some_and(|thread| queued[next].moves(thread));
        if moves && self.holds_off_switch(thread_id) {
            return;
        }
        let Some(message) = self
            .queued
            .get_mut(thread_id)
            .and_then(|queued| queued.remove(next))
        else {
            return;
        };
        self.save_recovery(thread_id);
        if let Err(error) = self.start_queued(thread_id, message) {
            eprintln!("hoshi: failed to start a queued message: {error}");
        }
        self.show_queue(thread_id);
    }

    /// Starts the messages that waited to move their thread to another provider until what its
    /// agent left running ended.
    pub(crate) fn start_waiting_switches(&mut self) {
        let waiting: Vec<String> = self
            .queued
            .iter()
            .filter(|(thread_id, queued)| {
                let next = queued.iter().find(|message| !message.held);
                self.model
                    .thread(thread_id)
                    .zip(next)
                    .is_some_and(|(thread, next)| next.to.is_some() || next.moves(thread))
            })
            .map(|(thread_id, _)| thread_id.clone())
            .collect();
        for thread_id in waiting {
            self.start_next_queued(&thread_id);
        }
    }

    pub(crate) fn queue_message(&mut self, thread_id: &str, message: QueuedMessage) {
        let held = self.queue_holds(thread_id);
        let sent_on = self
            .model
            .thread(thread_id)
            .filter(|_| message.from_user && message.to.is_none())
            .map(|thread| Box::new(ModelChoice::of(thread)));
        self.queued
            .entry(thread_id.to_owned())
            .or_default()
            .push_back(QueuedMessage {
                held,
                sent_on,
                ..message
            });
        self.save_recovery(thread_id);
        self.show_queue(thread_id);
    }

    /// Queues the user's message to a thread whose turn runs, when they picked another provider
    /// for it: it waits for the turn to end, which is stopped first when its provider's turns
    /// are restarted to move. `false` when the message goes as usual.
    pub(crate) fn queue_to_switch(&mut self, command: &Command) -> bool {
        let Command::MessageSend {
            thread_id,
            message_id,
            text,
            attachments,
        } = command
        else {
            return false;
        };
        let Some(thread) = self.model.thread(thread_id) else {
            return false;
        };
        let picked = match self.routes.get(message_id) {
            Some(route) => route.clone(),
            None => thread.pending_model.clone(),
        }
        .filter(|to| to.provider != thread.provider);
        let blank = text.trim().is_empty() && attachments.is_empty();
        if thread.active_turn_id.is_none()
            || picked.is_none()
            || blank
            || hoshi_core::is_compact_command(text)
            || self.thread_ops.contains_key(thread_id)
        {
            return false;
        }
        let restarts = self
            .adapter_for(&thread.provider)
            .is_some_and(|adapter| adapter.restarts_to_switch());
        let free = self.turns.get(thread_id).is_some_and(|turn| {
            !turn.compacting
                && turn
                    .background
                    .as_ref()
                    .is_none_or(|background| background.tasks.is_empty())
        });
        self.routes.remove(message_id);
        let message = QueuedMessage {
            id: message_id.clone(),
            text: text.trim().to_owned(),
            attachments: attachments.clone(),
            from_user: true,
            held: false,
            to: picked,
            sent_on: None,
        };
        self.queue_message(thread_id, message);
        if restarts && free {
            let interrupt = Command::TurnInterrupt {
                thread_id: thread_id.clone(),
            };
            if let Err(error) = self.run_command(interrupt) {
                eprintln!("hoshi: failed to stop a turn to move its thread: {error}");
            }
        }
        true
    }

    /// Sends a message that runs on `to`, or where its thread is, instead of on what the user
    /// picked for their next one, which stays picked. The route holds though the message waits
    /// on its way, as for git work.
    pub(crate) fn send_routed(
        &mut self,
        message_id: &str,
        to: Option<Box<ModelChoice>>,
        send: Command,
    ) -> Result<(), CommandError> {
        self.routes.insert(message_id.to_owned(), to);
        let sent = self.run_command(send);
        if sent.is_err() {
            self.routes.remove(message_id);
        }
        sent
    }

    /// Whether the message moves its idle thread to another provider.
    pub(crate) fn moves_thread(&self, thread_id: &str, message_id: &str) -> bool {
        let Some(thread) = self.model.thread(thread_id) else {
            return false;
        };
        let to = match self.routes.get(message_id) {
            Some(route) => route.as_deref(),
            None => thread.pending_model.as_deref(),
        };
        thread.active_turn_id.is_none() && to.is_some_and(|to| to.provider != thread.provider)
    }

    /// Has the thread run on `choice` from its next turn, as picking it does.
    pub(crate) fn choose(
        &mut self,
        thread_id: &str,
        choice: ModelChoice,
    ) -> Result<(), CommandError> {
        if self.shown_choice(thread_id).as_ref() == Some(&choice) {
            return Ok(());
        }
        self.decide_and_commit(Command::ThreadModelSet {
            thread_id: thread_id.to_owned(),
            provider: choice.provider,
            model: choice.model,
            effort: choice.effort,
            fast_mode: choice.fast_mode,
            ultracode: choice.ultracode,
        })
    }

    /// What the user's next message runs on, as the composer shows it.
    pub(crate) fn shown_choice(&self, thread_id: &str) -> Option<ModelChoice> {
        self.model.thread(thread_id).map(|thread| {
            thread
                .pending_model
                .as_deref()
                .cloned()
                .unwrap_or_else(|| ModelChoice::of(thread))
        })
    }

    /// Starts a turn of its own for a message taken out of the idle thread's queue; one that
    /// can't start waits there for the user.
    pub(crate) fn start_queued(
        &mut self,
        thread_id: &str,
        message: QueuedMessage,
    ) -> Result<(), CommandError> {
        let started = match message.to.clone() {
            Some(to) => {
                let send = Command::MessageSend {
                    thread_id: thread_id.to_owned(),
                    message_id: message.id.clone(),
                    text: message.text.clone(),
                    attachments: message.attachments.clone(),
                };
                self.send_routed(&message.id, Some(to), send)
            }
            None => self.start_kept(thread_id, message.clone()),
        };
        if started.is_err() {
            self.queued
                .entry(thread_id.to_owned())
                .or_default()
                .push_front(message);
            self.hold_queue(thread_id);
            self.save_recovery(thread_id);
        }
        started
    }

    /// Starts the turn of a message the transcript already has.
    fn start_kept(&mut self, thread_id: &str, message: QueuedMessage) -> Result<(), CommandError> {
        let now = now();
        let kept = self
            .store
            .thread_item(thread_id, &message.id)
            .ok()
            .flatten();
        let moves = self
            .model
            .thread(thread_id)
            .is_some_and(|thread| message.moves(thread));
        let picked = self.shown_choice(thread_id);
        if let (true, Some(sent_on)) = (moves, message.sent_on) {
            self.choose(thread_id, *sent_on)?;
        }
        let events = start_queued_turn(
            &self.model,
            thread_id,
            &message.id,
            kept.as_ref(),
            moves,
            &now,
        );
        let started = self.commit(&events, &now);
        let runs_on = self.model.thread(thread_id).map(RunsOn::of);
        if let (true, Some(picked)) = (moves, picked)
            && let Err(error) = self.choose(thread_id, picked)
        {
            eprintln!("hoshi: failed to keep what was picked for the next message: {error}");
        }
        started?;
        if events.is_empty() {
            return Ok(());
        }
        start_turn(
            self,
            thread_id,
            &message.id,
            message.text,
            message.attachments,
            runs_on,
        );
        Ok(())
    }
}
