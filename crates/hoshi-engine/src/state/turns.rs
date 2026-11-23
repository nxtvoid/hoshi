use crate::*;

impl State {
    /// The adapter of `provider`, when it is enabled.
    pub(crate) fn adapter_for(&self, provider: &str) -> Option<Arc<dyn ProviderAdapter>> {
        self.adapters
            .iter()
            .find(|adapter| adapter.provider() == provider)
            .cloned()
    }

    /// Where the thread's agent runs. A chat's own folder is created when first asked for.
    pub(crate) fn cwd(&self, thread_id: &str) -> Option<String> {
        let thread = self.model.thread(thread_id)?;
        if thread.project_id.is_some() {
            return self.model.thread_cwd(thread_id).map(str::to_owned);
        }
        self.made_chat_folder(thread_id)
    }

    /// The chat's folder, made if it isn't yet, as a thread's or a draft's.
    pub(crate) fn made_chat_folder(&self, thread_id: &str) -> Option<String> {
        let folder = self.chat_folder(thread_id)?;
        std::fs::create_dir_all(&folder).ok()?;
        Some(folder.to_string_lossy().into_owned())
    }

    /// Removes the empty folders of chats that were never started.
    pub(crate) fn forget_unused_chat_folders(&self) {
        let Ok(entries) = std::fs::read_dir(&self.chats_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            if self.model.thread(&name.to_string_lossy()).is_none() {
                std::fs::remove_dir(entry.path()).ok();
            }
        }
    }

    /// `None` for ids that could reach outside the chats folder.
    pub(crate) fn chat_folder(&self, thread_id: &str) -> Option<PathBuf> {
        let safe = !thread_id.is_empty()
            && thread_id
                .chars()
                .all(|char| char.is_ascii_alphanumeric() || char == '-');
        safe.then(|| self.chats_dir.join(thread_id))
    }

    pub(crate) fn receive(&mut self, thread_id: &str, turn_id: &str, output: ProviderOutput) {
        if self.recovery.quitting {
            return;
        }
        let now = now();
        let settled = retry_settled(&self.model, thread_id, turn_id, &output, &now);
        if let Err(error) = self.commit(&settled, &now) {
            eprintln!("hoshi: failed to record a retry's end: {error}");
        }
        let (item_id, kind, delta) = match output {
            ProviderOutput::TextDelta { item_id, delta } => (item_id, StreamKind::Text, delta),
            ProviderOutput::ReasoningDelta { item_id, delta } => {
                (item_id, StreamKind::Reasoning, delta)
            }
            ProviderOutput::ToolOutput { item_id, output } => {
                (item_id, StreamKind::ToolOutput, output)
            }
            other => return self.record(thread_id, turn_id, other),
        };
        let shown = self.streamed.iter().any(|streamed| streamed == thread_id);
        let active = self
            .model
            .thread(thread_id)
            .is_some_and(|thread| thread.active_turn_id.as_deref() == Some(turn_id));
        if shown && active {
            (self.notify)(Notification::Stream(vec![StreamDelta {
                thread_id: thread_id.to_owned(),
                item_id,
                kind,
                delta,
            }]));
        }
    }

    pub(crate) fn record(&mut self, thread_id: &str, turn_id: &str, output: ProviderOutput) {
        match &output {
            ProviderOutput::ContextHandoffDelivered { injected } => {
                self.handoff_delivered(thread_id, turn_id, *injected);
                return;
            }
            ProviderOutput::SessionStarted { .. } => self.native_fork_started(thread_id, turn_id),
            ProviderOutput::TurnCompleted { outcome } => {
                self.handoff_turn_ended(thread_id, turn_id, outcome);
            }
            _ => {}
        }
        match output {
            ProviderOutput::ContextUsage(usage) => {
                self.record_context_usage(thread_id, turn_id, usage);
                return;
            }
            ProviderOutput::UsageLimits(update) => {
                let provider = self.model.thread(thread_id).map(|t| t.provider.clone());
                if let Some(provider) = provider {
                    let mut limits = self
                        .usage_limits
                        .get(&provider)
                        .cloned()
                        .unwrap_or_default();
                    limits.merge(update);
                    self.keep_usage_limits(&provider, limits);
                }
                return;
            }
            ProviderOutput::WorkingDirectory { path } => {
                self.follow_session(thread_id, turn_id, &path);
                return;
            }
            ProviderOutput::ConversationCursor { cursor } => {
                if let Some(turn) = self.turns.get_mut(thread_id)
                    && turn.id == turn_id
                {
                    turn.cursor = Some(cursor);
                }
                return;
            }
            ProviderOutput::BackgroundTasks { tasks, waiting } => {
                if let Some(turn) = self.turns.get_mut(thread_id)
                    && turn.id == turn_id
                {
                    let background = ThreadBackgroundTasks {
                        thread_id: thread_id.to_owned(),
                        tasks,
                        waiting,
                    };
                    turn.background = Some(background.clone());
                    (self.notify)(Notification::BackgroundTasks(background));
                    self.save_recovery(thread_id);
                }
                return;
            }
            _ => {}
        }
        if let ProviderOutput::ToolCallStarted { input, .. } = &output
            && let Some(turn) = self.turns.get_mut(thread_id)
            && turn.id == turn_id
            && let Some(workspace) = &mut turn.workspace
        {
            workspace.record_tool_input(input);
        }
        let ended = match &output {
            ProviderOutput::TurnCompleted { outcome } => Some(outcome.clone()),
            _ => None,
        };
        let completed = ended.is_some();
        let now = now();
        let ending = self
            .turns
            .get(thread_id)
            .filter(|turn| completed && turn.id == turn_id);
        let checkpointed =
            ending.and_then(|turn| Some((turn.checkpointed.clone()?, turn.arrivals)));
        if let Some(cursor) = ending.and_then(|turn| turn.cursor.clone()) {
            let events = record_conversation_cursor(&self.model, thread_id, cursor);
            if let Err(error) = self.commit(&events, &now) {
                eprintln!("hoshi: failed to record where the conversation is: {error}");
            }
        }
        if checkpointed.is_some() {
            // Holds the turn's place in the transcript while its changes are read.
            let pending = record_turn_changes(&self.model, thread_id, turn_id, None, &now);
            if let Err(error) = self.commit(&pending, &now) {
                eprintln!("hoshi: failed to record a turn's changes: {error}");
            }
        }
        let events = record_output(&self.model, thread_id, turn_id, output, &now);
        if let Err(error) = self.commit(&events, &now) {
            eprintln!("hoshi: failed to record provider output: {error}");
        }
        let current = !events.is_empty();
        if completed {
            let ended_turn = match self.turns.get(thread_id) {
                Some(turn) if turn.id == turn_id => self.turns.remove(thread_id),
                _ => None,
            };
            let compacts_when_idle = ended == Some(TurnOutcome::Completed)
                && ended_turn.as_ref().is_some_and(|turn| !turn.compacting);
            let workspace = ended_turn.and_then(|mut turn| {
                let workspace = turn.workspace.take();
                self.keep_between_turns(thread_id, turn);
                workspace
            });
            // The agent may have created a worktree or switched branches.
            let check = workspace
                .filter(|workspace| !workspace.moved)
                .map(|workspace| self.worktree_check(thread_id, turn_id, workspace));
            match checkpointed {
                Some((dir, arrivals)) => {
                    self.read_turn_changes(thread_id, turn_id, dir, arrivals, check);
                }
                None => refresh_after_turn(self.this.clone(), thread_id.to_owned(), check),
            }
            if self.titles_to_refine.remove(thread_id) {
                self.rename_from_conversation(thread_id, false);
            }
            if let Some(outcome) = &ended {
                if current {
                    self.turn_ended(thread_id, turn_id, outcome);
                }
                self.deliver_task_result(thread_id, outcome);
            }
            self.start_next_queued(thread_id);
            self.send_after_compaction(thread_id);
            self.deliver_pending_wakes(thread_id);
            if compacts_when_idle {
                self.schedule_idle_compaction(thread_id);
            }
        }
    }

    /// Checkpoints the tree a finished turn left, then records what changed since it began
    /// in the folder it ended in. A worktree the turn created and worked in becomes the
    /// thread's folder first, its changes counted from how it was checked out.
    pub(crate) fn read_turn_changes(
        &self,
        thread_id: &str,
        turn_id: &str,
        dir: String,
        arrivals: usize,
        check: Option<WorktreeCheck>,
    ) {
        let weak = self.this.clone();
        let (thread_id, turn_id) = (thread_id.to_owned(), turn_id.to_owned());
        self.checkpoints.queue(&dir.clone(), move || {
            let created = check
                .as_ref()
                .and_then(created_worktree)
                .filter(|(path, _)| *path != git::normalize(&dir));
            let (folder, arrivals) = match &created {
                Some((path, _)) => {
                    let reference =
                        checkpoint::reference(&thread_id, &turn_id, Moment::Arrived(arrivals + 1));
                    if let Err(error) = checkpoint::capture_created(path, &reference) {
                        eprintln!("hoshi: no checkpoint of the worktree {path}: {error}");
                    }
                    (path.clone(), arrivals + 1)
                }
                None => (dir, arrivals),
            };
            let files = changes_since_arrival(&folder, &thread_id, &turn_id, arrivals)
                .unwrap_or_else(|error| {
                    eprintln!("hoshi: no checkpoint after the turn in {folder}: {error}");
                    Vec::new()
                });
            thread::spawn(move || {
                let Some(shared) = weak.upgrade() else { return };
                let mut state = lock(&shared);
                let now = now();
                if let Some((path, branch)) = created {
                    let events = record_worktree(&state.model, &thread_id, path, branch, true);
                    if let Err(error) = state.commit(&events, &now) {
                        eprintln!("hoshi: failed to record a worktree: {error}");
                    }
                }
                let events =
                    record_turn_changes(&state.model, &thread_id, &turn_id, Some(files), &now);
                if let Err(error) = state.commit(&events, &now) {
                    eprintln!("hoshi: failed to record a turn's changes: {error}");
                }
                drop(state);
                refresh_after_turn(weak, thread_id, None);
            });
        });
    }

    /// Reads a running turn's changes in the folder its thread moved to from now on,
    /// counted from how that folder is now, or from how a worktree the turn created was
    /// checked out.
    pub(crate) fn follow_turn(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        folder: &str,
        created: bool,
    ) {
        let Some(turn) = self
            .turns
            .get_mut(thread_id)
            .filter(|turn| turn.id == turn_id)
        else {
            return;
        };
        let same = |dir: &String| git::normalize(dir) == git::normalize(folder);
        if turn.checkpointed.as_ref().is_none_or(same) {
            return;
        }
        turn.arrivals += 1;
        turn.checkpointed = Some(folder.to_owned());
        let reference = checkpoint::reference(thread_id, turn_id, Moment::Arrived(turn.arrivals));
        let dir = folder.to_owned();
        self.checkpoints.queue(&dir.clone(), move || {
            let captured = if created {
                checkpoint::capture_created(&dir, &reference)
            } else {
                checkpoint::capture(&dir, &reference)
            };
            if let Err(error) = captured {
                eprintln!("hoshi: no checkpoint where the turn went in {dir}: {error}");
            }
        });
    }

    /// Starts the turns that waited for git to finish with `folder`.
    pub(crate) fn start_parked(&mut self, folder: &str) {
        for turn in self.parked.remove(folder).unwrap_or_default() {
            let current = self.model.thread(&turn.thread_id).is_some_and(|thread| {
                thread.active_turn_id.as_deref() == Some(turn.turn_id.as_str())
            });
            if !current {
                continue;
            }
            start_turn(
                self,
                &turn.thread_id,
                &turn.turn_id,
                turn.prompt,
                turn.attachments,
                Some(turn.runs_on),
            );
        }
    }

    /// Ends a parked turn the user stopped before git was done with its folder.
    pub(crate) fn interrupt_parked(&mut self, thread_id: &str) {
        let mut stopped = Vec::new();
        for turns in self.parked.values_mut() {
            turns.retain(|turn| {
                let parked = turn.thread_id == thread_id;
                if parked {
                    stopped.push(turn.turn_id.clone());
                }
                !parked
            });
        }
        for turn_id in stopped {
            let outcome = TurnOutcome::Interrupted;
            self.receive(
                thread_id,
                &turn_id,
                ProviderOutput::TurnCompleted { outcome },
            );
        }
    }

    /// Drops what the engine keeps for a thread that is gone.
    pub(crate) fn forget_thread(&mut self, thread_id: &str) {
        if let Some(held) = self.thread_ops.get_mut(thread_id) {
            held.clear();
        }
        for turns in self.parked.values_mut() {
            turns.retain(|turn| turn.thread_id != thread_id);
        }
        if let Some(mcp) = &self.mcp {
            mcp.forget(thread_id);
        }
        self.delegations.remove(thread_id);
        self.pull_requests.forget(thread_id);
        self.pull_request_cache.forget(thread_id);
        self.queued.remove(thread_id);
        self.after_compaction.remove(thread_id);
        self.idle_compactions.remove(thread_id);
        self.watches.remove(thread_id);
        self.watch_reads.remove(thread_id);
        self.pending_wakes.remove(thread_id);
        self.titles_to_refine.remove(thread_id);
        self.session_moves.remove(thread_id);
        self.recovery.forget(thread_id);
        if let Some(folder) = self.chat_folder(thread_id) {
            let _ = std::fs::remove_dir_all(folder);
        }
    }

    /// Passes a message sent to a busy thread on to its running turn, or keeps it for a turn
    /// of its own when the turn is ending.
    pub(crate) fn join_turn(&mut self, thread_id: &str, message: QueuedMessage) {
        let attachments = self.turn_attachments(message.attachments.clone());
        let joined = self.turns.get(thread_id).is_some_and(|turn| {
            turn.running
                .steer(&message.id, &message.text, &attachments, message.from_user)
        });
        if joined {
            return;
        }
        let held = self.queue_holds(thread_id);
        self.queued
            .entry(thread_id.to_owned())
            .or_default()
            .push_back(QueuedMessage { held, ..message });
        self.save_recovery(thread_id);
        self.show_queue(thread_id);
        // The turn may have ended between the message's events and this call.
        if !self.turns.contains_key(thread_id) {
            self.start_next_queued(thread_id);
        }
    }

    pub(crate) fn turn_attachments(
        &self,
        attachments: Vec<MessageAttachment>,
    ) -> Vec<TurnAttachment> {
        attachments
            .into_iter()
            .map(|attachment| {
                let path = self.attachments.path_of(&attachment);
                let extracted_text = match &attachment {
                    MessageAttachment::File { .. } => {
                        path.as_deref().and_then(document_text::sidecar)
                    }
                    _ => None,
                };
                TurnAttachment {
                    path,
                    extracted_text,
                    attachment,
                }
            })
            .collect()
    }

    /// The images and files the thread's earlier messages carried since its conversation
    /// last compacted, leaving out those of the message about to go, `current`.
    pub(crate) fn earlier_media(
        &self,
        thread_id: &str,
        current: &[TurnAttachment],
    ) -> Vec<TurnAttachment> {
        let earlier = match self.store.media_since_compaction(thread_id) {
            Ok(earlier) => earlier,
            Err(error) => {
                eprintln!("hoshi: failed to read the media of thread {thread_id}: {error}");
                return Vec::new();
            }
        };
        let earlier = earlier
            .into_iter()
            .filter(|attachment| !current.iter().any(|sent| sent.attachment == *attachment))
            .collect();
        self.turn_attachments(earlier)
    }

    pub(crate) fn record_context_usage(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        usage: ContextUsage,
    ) {
        let active = self
            .model
            .thread(thread_id)
            .is_some_and(|thread| thread.active_turn_id.as_deref() == Some(turn_id));
        if !active {
            return;
        }
        let mut usage = usage.merged_into(self.context_usages.get(thread_id).copied());
        if let Some(ttl) = usage.cache_ttl_secs {
            usage.cache_expires_at = Some(unix_millis() + ttl * 1000);
        }
        self.context_usages.insert(thread_id.to_owned(), usage);
        let update = ThreadContextUsage {
            thread_id: thread_id.to_owned(),
            usage,
        };
        if let Err(error) = self.store.save_context_usage(&update) {
            eprintln!("hoshi: failed to cache a context usage: {error}");
        }
        (self.notify)(Notification::ContextUsage(update));
    }

    /// The agent's session works elsewhere than the thread's folder: in a worktree it
    /// entered, or back in the project root. git confirms which in the background; when
    /// moves overlap, the latest one wins.
    pub(crate) fn follow_session(&mut self, thread_id: &str, turn_id: &str, path: &str) {
        let Some(cwd) = self.model.thread_cwd(thread_id).map(git::normalize) else {
            return;
        };
        let path = git::normalize(path);
        if path == cwd && !self.session_moves.contains_key(thread_id) {
            return;
        }
        let Some(workspace) = self
            .turns
            .get_mut(thread_id)
            .filter(|turn| turn.id == turn_id)
            .and_then(|turn| turn.workspace.as_mut())
        else {
            return;
        };
        workspace.moved = true;
        let workspace = workspace.clone();
        let check = self.worktree_check(thread_id, turn_id, workspace);
        self.session_moves
            .insert(thread_id.to_owned(), path.clone());
        follow_session_move(self.this.clone(), thread_id.to_owned(), path, check);
    }
}

impl State {
    /// What a turn starting now in `cwd` reads its changes against: the project's worktrees
    /// before it, and a checkpoint of the folder, queued now.
    pub(crate) fn turn_workspace(
        &self,
        thread_id: &str,
        turn_id: &str,
        cwd: &str,
    ) -> (Option<TurnWorkspace>, Option<String>) {
        let project = self
            .model
            .thread(thread_id)
            .and_then(|thread| thread.project_id.as_deref())
            .and_then(|project_id| self.model.project(project_id));
        let workspace = project.and_then(|project| {
            let repository = project.workspace_root.clone();
            let worktrees_before = git::worktrees(&repository)?;
            Some(TurnWorkspace {
                repository,
                worktrees_before,
                tool_inputs: Vec::new(),
                moved: false,
            })
        });
        let checkpointed = workspace.as_ref().map(|_| cwd.to_owned());
        if let Some(dir) = &checkpointed {
            let (dir, reference) = (
                dir.clone(),
                checkpoint::reference(thread_id, turn_id, Moment::Before),
            );
            self.checkpoints.queue(&dir.clone(), move || {
                if let Err(error) = checkpoint::capture(&dir, &reference) {
                    eprintln!("hoshi: no checkpoint before the turn in {dir}: {error}");
                }
            });
        }
        (workspace, checkpointed)
    }
}
