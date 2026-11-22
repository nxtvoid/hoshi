use crate::*;

impl State {
    /// Does the I/O a command needs before the core can decide it: resolves a
    /// project's folder, and creates a new thread's worktree.
    pub(crate) fn prepare(&self, command: Command) -> Result<Command, CommandError> {
        match command {
            Command::ProjectCreate {
                project_id,
                title,
                workspace_root,
            } => {
                let resolved = dunce::canonicalize(&workspace_root)
                    .ok()
                    .filter(|path| path.is_dir())
                    .ok_or_else(|| CommandError::WorkspaceRootNotFound {
                        path: workspace_root.clone(),
                    })?;
                Ok(Command::ProjectCreate {
                    project_id,
                    title,
                    workspace_root: resolved.to_string_lossy().into_owned(),
                })
            }
            Command::ThreadCreate {
                workspace: ThreadWorkspace::Worktree { .. },
                ..
            } => Err(CommandError::Internal {
                detail: "only the engine creates worktrees".into(),
            }),
            Command::ThreadWorktreeRelease { .. } | Command::ThreadCleanUp { .. } => {
                Err(CommandError::Internal {
                    detail: "worktrees and branches are removed through the engine's own calls"
                        .into(),
                })
            }
            Command::MessageSend { ref thread_id, .. } => {
                let switching = self.model.thread(thread_id).is_some_and(|thread| {
                    thread.active_turn_id.is_none() && thread.pending_model.is_some()
                });
                if switching {
                    self.refuse_ending_background_work(thread_id)?;
                }
                self.restore_released_worktree(thread_id)?;
                Ok(command)
            }
            Command::ThreadSnooze { ref thread_id, .. } if self.holds_user_message(thread_id) => {
                Err(CommandError::MessageQueued {
                    thread_id: thread_id.clone(),
                })
            }
            Command::ThreadModelSet {
                ref thread_id,
                ref provider,
                ..
            } => {
                if self
                    .model
                    .thread(thread_id)
                    .is_some_and(|thread| thread.provider != *provider)
                {
                    self.refuse_ending_background_work(thread_id)?;
                }
                Ok(command)
            }
            Command::ThreadCreate {
                thread_id,
                project_id,
                title,
                workspace:
                    workspace @ (ThreadWorkspace::NewWorktree | ThreadWorkspace::BranchWorktree { .. }),
                provider,
                model,
                effort,
                fast_mode,
                ultracode,
                access_mode,
                plan_mode,
            } => {
                let Some(project) = project_id.as_deref() else {
                    return Err(CommandError::ChatNotLocal { thread_id });
                };
                let existing = match workspace {
                    ThreadWorkspace::BranchWorktree { branch } => Some(branch),
                    _ => None,
                };
                let workspace = self.create_worktree(&thread_id, project, existing.as_deref())?;
                Ok(Command::ThreadCreate {
                    thread_id,
                    project_id,
                    title,
                    workspace,
                    provider,
                    model,
                    effort,
                    fast_mode,
                    ultracode,
                    access_mode,
                    plan_mode,
                })
            }
            other => Ok(other),
        }
    }

    /// A message the user sent waits for the thread: for its turn, its compaction, its git
    /// work or its worktree's setup.
    pub(crate) fn holds_user_message(&self, thread_id: &str) -> bool {
        self.queued
            .get(thread_id)
            .is_some_and(|queued| queued.iter().any(|message| message.from_user))
            || self
                .after_compaction
                .get(thread_id)
                .is_some_and(|message| message.from_user)
            || self
                .thread_ops
                .get(thread_id)
                .is_some_and(|held| !held.is_empty())
            || self.setups.holds(thread_id)
    }

    /// The thread's snooze is still ahead, whatever woke it since.
    pub(crate) fn snoozed(&self, thread_id: &str) -> bool {
        self.model
            .thread(thread_id)
            .is_some_and(|thread| snooze_pending(thread, Utc::now()))
    }

    /// [`Self::snoozed`] at `now`, in Unix milliseconds.
    pub(crate) fn snoozed_at_millis(&self, thread_id: &str, now: u64) -> bool {
        let now = i64::try_from(now)
            .ok()
            .and_then(chrono::DateTime::from_timestamp_millis);
        self.model
            .thread(thread_id)
            .zip(now)
            .is_some_and(|(thread, now)| snooze_pending(thread, now))
    }

    /// Refuses what would close the process the thread's completed turn kept while it still
    /// runs commands in the background.
    pub(crate) fn refuse_ending_background_work(
        &self,
        thread_id: &str,
    ) -> Result<(), CommandError> {
        if self
            .lingering
            .get(thread_id)
            .is_some_and(background::Lingering::runs_background_work)
        {
            return Err(CommandError::BackgroundWorkRunning {
                thread_id: thread_id.to_owned(),
            });
        }
        Ok(())
    }

    /// Decides `command` and commits its events, for work the engine did itself first.
    pub(crate) fn decide_and_commit(&mut self, command: Command) -> Result<(), CommandError> {
        let now = now();
        let events = decide(&self.model, command, &now)?;
        self.commit(&events, &now)
    }

    /// Worktrees (with their repository) that a deletion leaves without a thread.
    /// The images and files in the transcripts of the threads `command` deletes.
    pub(crate) fn images_freed_by(&self, command: &Command) -> Vec<MessageAttachment> {
        self.threads_deleted_by(command)
            .into_iter()
            .flat_map(|thread| self.store.thread_items(&thread.id).unwrap_or_default())
            .flat_map(|item| match item.content {
                ItemContent::UserMessage { attachments, .. } => attachments,
                _ => Vec::new(),
            })
            .filter(|attachment| {
                matches!(
                    attachment,
                    MessageAttachment::Image { .. } | MessageAttachment::File { .. }
                )
            })
            .collect()
    }

    pub(crate) fn threads_deleted_by(&self, command: &Command) -> Vec<&hoshi_core::Thread> {
        match command {
            Command::ThreadDelete { thread_id } => {
                self.model.thread(thread_id).into_iter().collect()
            }
            Command::ProjectRemove { project_id } => self
                .model
                .threads()
                .filter(|thread| thread.project_id.as_ref() == Some(project_id))
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn worktrees_freed_by(&self, command: &Command) -> Vec<(String, String)> {
        let deleted: Vec<String> = self
            .threads_deleted_by(command)
            .into_iter()
            .map(|thread| thread.id.clone())
            .collect();
        self.threads_deleted_by(command)
            .into_iter()
            .filter(|thread| !thread.worktree_adopted)
            .filter(|thread| self.sharing_worktree(thread, &deleted).is_none())
            .filter_map(|thread| {
                let path = thread.worktree_path.clone()?;
                let project = self.model.project(thread.project_id.as_deref()?)?;
                Some((project.workspace_root.clone(), path))
            })
            .fold(Vec::new(), |mut freed, (root, path)| {
                if !freed.iter().any(|(_, kept): &(String, String)| {
                    git::normalize(kept) == git::normalize(&path)
                }) {
                    freed.push((root, path));
                }
                freed
            })
    }

    pub(crate) fn commit(&mut self, events: &[Event], now: &str) -> Result<(), CommandError> {
        if events.is_empty() {
            return Ok(());
        }
        let resolved = self.with_models_named(events);
        let events = resolved.as_deref().unwrap_or(events);
        let sequences = self.store.append(events, now).map_err(internal)?;
        for (sequence, event) in sequences.into_iter().zip(events) {
            self.model.apply(event);
            self.sequence = sequence;
            (self.notify)(Notification::Event(EventEnvelope {
                sequence,
                event: event.clone(),
            }));
        }
        self.close_lingering_after(events);
        self.recovery_after(events);
        Ok(())
    }

    /// Carries out a command: decides its events, records them, and starts what they call for.
    /// A message to a thread git works on waits for that work to end.
    pub(crate) fn run_command(&mut self, command: Command) -> Result<(), CommandError> {
        // A thread goes with the tasks it delegated, which nothing else would list.
        if let Command::ThreadDelete { thread_id } = &command {
            let tasks: Vec<String> = self
                .model
                .threads()
                .filter(|thread| thread.parent_thread_id.as_deref() == Some(thread_id))
                .map(|thread| thread.id.clone())
                .collect();
            for task in tasks {
                self.run_command(Command::ThreadDelete { thread_id: task })?;
            }
        }
        if let Command::MessageSend { thread_id, .. } = &command
            && let Some(held) = self.thread_ops.get_mut(thread_id)
        {
            held.push(command);
            return Ok(());
        }
        let restoring = match &command {
            Command::MessageSend { thread_id, .. } if self.worktree_missing(thread_id) => {
                Some(thread_id.clone())
            }
            _ => None,
        };
        let creating = match &command {
            Command::ThreadCreate {
                thread_id,
                workspace: ThreadWorkspace::NewWorktree | ThreadWorkspace::BranchWorktree { .. },
                ..
            } => Some(thread_id.clone()),
            _ => None,
        };
        let command = self.prepare(command)?;
        if let Some(thread_id) = restoring {
            self.start_worktree_setup(&thread_id);
        }
        let Some(command) = self.setups.hold(command) else {
            return Ok(());
        };
        self.run_prepared(command)?;
        if let Some(thread_id) = creating {
            self.start_worktree_setup(&thread_id);
        }
        Ok(())
    }

    /// Carries out a command `prepare` already went through, or one the engine itself made.
    pub(crate) fn run_prepared(&mut self, command: Command) -> Result<(), CommandError> {
        let interrupt = match &command {
            Command::TurnInterrupt { thread_id } => Some(thread_id.clone()),
            _ => None,
        };
        let stop = match &command {
            Command::SubagentStop {
                thread_id,
                tool_use_id,
            } => Some((thread_id.clone(), tool_use_id.clone())),
            _ => None,
        };
        let stop_background = match &command {
            Command::BackgroundTaskStop { thread_id, task_id } => {
                Some((thread_id.clone(), task_id.clone()))
            }
            _ => None,
        };
        let answer = match &command {
            Command::RequestRespond {
                thread_id,
                request_id,
                response,
            } => Some((thread_id.clone(), request_id.clone(), response.clone())),
            _ => None,
        };
        // A question whose turn is gone, as after a restart, is answered in a message.
        let by_message = answer
            .as_ref()
            .and_then(|(thread_id, request_id, response)| {
                if self.turns.contains_key(thread_id) {
                    return None;
                }
                let asked = self.model.pending_request(request_id)?;
                Some((
                    thread_id.clone(),
                    answers_message(&asked.content, response)?,
                ))
            });
        let freed_worktrees = self.worktrees_freed_by(&command);
        let freed_images = self.images_freed_by(&command);
        let deleted_threads: Vec<String> = self
            .threads_deleted_by(&command)
            .into_iter()
            .map(|thread| thread.id.clone())
            .collect();
        let forgotten_checkpoints: Vec<(String, String)> = self
            .threads_deleted_by(&command)
            .into_iter()
            .filter_map(|thread| Some((self.project_root(thread).ok()?, thread.id.clone())))
            .collect();
        let removed_with_project: Vec<String> = match &command {
            Command::ProjectRemove { .. } => self
                .threads_deleted_by(&command)
                .into_iter()
                .map(|thread| thread.id.clone())
                .collect(),
            _ => Vec::new(),
        };
        let now = now();
        let events = decide(&self.model, command, &now)?;
        self.commit(&events, &now)?;
        self.react(&events);
        for thread_id in &removed_with_project {
            self.forget_thread(thread_id);
        }
        if let Some(thread_id) = interrupt {
            match self.turns.get(&thread_id) {
                Some(turn) => turn.running.interrupt(),
                None => self.interrupt_parked(&thread_id),
            }
        }
        if let Some((thread_id, request_id, response)) = answer
            && let Some(turn) = self.turns.get(&thread_id)
        {
            turn.running.respond(&request_id, &response);
        }
        if let Some((thread_id, text)) = by_message {
            self.run_command(Command::MessageSend {
                thread_id,
                message_id: delegation::new_id(),
                text,
                attachments: Vec::new(),
            })?;
        }
        if let Some((thread_id, tool_use_id)) = stop
            && let Some(turn) = self.turns.get(&thread_id)
        {
            turn.running.stop_subagent(&tool_use_id);
        }
        if let Some((thread_id, task_id)) = stop_background {
            self.stop_background_command(&thread_id, &task_id);
        }
        // A deleted thread (or a removed project's threads) must not keep an agent running.
        let orphaned: Vec<_> = self
            .turns
            .keys()
            .filter(|thread_id| self.model.thread(thread_id).is_none())
            .cloned()
            .collect();
        for thread_id in orphaned {
            if let Some(turn) = self.turns.remove(&thread_id) {
                turn.running.interrupt();
            }
        }
        let orphaned: Vec<_> = self
            .lingering
            .keys()
            .filter(|thread_id| self.model.thread(thread_id).is_none())
            .cloned()
            .collect();
        for thread_id in orphaned {
            self.close_lingering(&thread_id);
        }
        let State {
            model,
            context_usages,
            ..
        } = self;
        context_usages.retain(|thread_id, _| model.thread(thread_id).is_some());
        self.setups
            .retain_threads(|thread_id| self.model.thread(thread_id).is_some());
        if !freed_worktrees.is_empty() {
            let checkpoints = self.checkpoints.clone();
            thread::spawn(move || {
                for (repository, path) in freed_worktrees {
                    checkpoints.settle(&path);
                    if let Err(error) = git::remove_worktree(&repository, &path) {
                        eprintln!("hoshi: kept worktree {path}: {error}");
                    }
                }
            });
        }
        for image in &freed_images {
            self.attachments.remove(image);
        }
        for thread_id in &deleted_threads {
            self.pages.remove_thread(thread_id);
        }
        for (root, thread_id) in forgotten_checkpoints {
            self.checkpoints.queue(&root.clone(), move || {
                if let Err(error) = checkpoint::forget(&root, &thread_id) {
                    eprintln!("hoshi: kept the checkpoints of thread {thread_id}: {error}");
                }
            });
        }
        Ok(())
    }

    /// Starts what recorded events call for: turns, titles, watches, cleanups.
    pub(crate) fn react(&mut self, events: &[Event]) {
        let mut message: Option<QueuedMessage> = None;
        let mut joining: Option<String> = None;
        let titled: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                Event::ThreadTitleSet { thread_id, title } => {
                    Some((thread_id.clone(), title.clone()))
                }
                _ => None,
            })
            .collect();
        for event in events {
            match event {
                Event::ItemAdded {
                    item:
                        ThreadItem {
                            id,
                            thread_id,
                            content:
                                ItemContent::UserMessage {
                                    text,
                                    attachments,
                                    task_result,
                                    pull_request_update,
                                    ..
                                },
                            ..
                        },
                } => {
                    message = Some(QueuedMessage {
                        id: id.clone(),
                        text: text.clone(),
                        attachments: attachments.clone(),
                        from_user: task_result.is_none() && pull_request_update.is_none(),
                        held: false,
                    });
                    joining = Some(thread_id.clone());
                    if let Some((thread_id, provisional)) = titled.first() {
                        let request = TitleRequest {
                            context: text.clone(),
                            previous_title: None,
                            images: titles::attachment_names(attachments),
                            model: None,
                            instructions: String::new(),
                        };
                        self.name_thread(thread_id, provisional, request);
                    }
                }
                Event::TurnStarted {
                    thread_id, turn_id, ..
                } => {
                    let started = message.take();
                    let (prompt, attachments) = started
                        .map(|message| (message.text, message.attachments))
                        .unwrap_or_default();
                    start_turn(self, thread_id, turn_id, prompt, attachments, None);
                }
                Event::ThreadCreated { thread } if thread.worktree_path.is_none() => {
                    refresh_branches(self.this.clone(), vec![thread.id.clone()]);
                }
                Event::ProjectCreated { project } => {
                    let watched = (project.id.clone(), project.workspace_root.clone());
                    watch_projects(self.this.clone(), vec![watched]);
                }
                Event::ProjectRemoved { project_id } => {
                    if let Some(watcher) = &mut self.watcher {
                        watcher.unwatch(project_id);
                    }
                    self.repositories.remove(project_id);
                    self.root_branches.remove(project_id);
                }
                Event::ThreadDeleted { thread_id } => self.forget_thread(thread_id),
                _ => {}
            }
        }
        if let (Some(message), Some(thread_id)) = (message, joining) {
            self.join_turn(&thread_id, message);
        }
    }

    /// Marks `folders` as ones git changes outside the lock for `thread_id`. Refused while
    /// other git work runs on the thread or in one of the folders.
    pub(crate) fn begin_op(
        &mut self,
        thread_id: &str,
        folders: &[&str],
    ) -> Result<WorkspaceOp, CommandError> {
        let folders: Vec<String> = folders
            .iter()
            .map(|folder| git::normalize(folder))
            .collect();
        let taken = self.thread_ops.contains_key(thread_id)
            || folders
                .iter()
                .any(|folder| self.folder_ops.contains(folder));
        if taken {
            return Err(CommandError::ThreadBusy {
                thread_id: thread_id.to_owned(),
            });
        }
        self.thread_ops.insert(thread_id.to_owned(), Vec::new());
        self.folder_ops.extend(folders.iter().cloned());
        Ok(WorkspaceOp {
            thread_id: thread_id.to_owned(),
            folders,
        })
    }

    /// Ends git work [`State::begin_op`] started: runs `first`, then the messages sent to the
    /// thread meanwhile, then the turns that waited for its folders. Returns how `first` went.
    pub(crate) fn end_op(
        &mut self,
        op: WorkspaceOp,
        first: Option<Command>,
    ) -> Result<(), CommandError> {
        let held = self.thread_ops.remove(&op.thread_id).unwrap_or_default();
        for folder in &op.folders {
            self.folder_ops.remove(folder);
        }
        let result = first.map_or(Ok(()), |command| self.run_command(command));
        for command in held {
            if let Err(error) = self.run_command(command) {
                eprintln!("hoshi: failed to send a message held for git work: {error}");
            }
        }
        for folder in &op.folders {
            self.start_parked(folder);
        }
        result
    }
}
