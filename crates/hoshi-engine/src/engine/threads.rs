use crate::*;

impl Engine {
    pub fn regenerate_title(&self, thread_id: &str) -> Result<(), CommandError> {
        let mut state = lock(&self.state);
        if state.model.thread(thread_id).is_none() {
            return Err(CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            });
        }
        state.rename_from_conversation(thread_id, true);
        Ok(())
    }

    /// The thread's folder, or the project's root for a thread not created yet. `None` for
    /// a chat not created yet, which has no folder.
    pub(crate) fn composer_cwd(
        &self,
        project_id: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<Option<String>, CommandError> {
        let state = lock(&self.state);
        match (thread_id, project_id) {
            (Some(thread_id), _) => {
                state
                    .cwd(thread_id)
                    .map(Some)
                    .ok_or_else(|| CommandError::ThreadNotFound {
                        thread_id: thread_id.to_owned(),
                    })
            }
            (None, Some(project_id)) => state
                .model
                .project(project_id)
                .map(|project| Some(project.workspace_root.clone()))
                .ok_or_else(|| CommandError::ProjectNotFound {
                    project_id: project_id.to_owned(),
                }),
            (None, None) => Ok(None),
        }
    }

    /// The files and folders that best match `query`, for `@` mentions. Reads
    /// the folder the first time.
    pub fn search_paths(
        &self,
        project_id: Option<&str>,
        thread_id: Option<&str>,
        query: &str,
    ) -> Result<Vec<WorkspaceEntry>, CommandError> {
        let Some(cwd) = self.composer_cwd(project_id, thread_id)? else {
            return Ok(Vec::new());
        };
        let root = PathBuf::from(&cwd);
        let entries = self
            .paths
            .get(&cwd, move || Some(paths::read(&root)))
            .unwrap_or_default();
        Ok(paths::search(&entries, query, PATH_SEARCH_LIMIT))
    }

    /// Hands a delegated task's answer back to its thread now, as it stands, with its plan
    /// when it still plans.
    pub fn send_task_answer(&self, thread_id: &str) {
        lock(&self.state).hand_back(thread_id, &TurnOutcome::Completed);
    }

    /// The user cancels a delegated task: it stops if it runs, its answer never goes back,
    /// and the thread that still waited for it hears it was stopped.
    pub fn cancel_task(&self, thread_id: &str) {
        lock(&self.state).stop_task(thread_id);
    }

    pub fn snapshot(&self) -> Snapshot {
        let state = lock(&self.state);
        Snapshot {
            sequence: state.sequence,
            projects: state.model.projects().cloned().collect(),
            threads: state.model.threads().cloned().collect(),
            pending_requests: state.model.all_pending_requests(),
            pending_tasks: state
                .delegations
                .keys()
                .filter(|task| state.awaited(task))
                .cloned()
                .collect(),
            open_tool_calls: state.model.open_tool_calls().cloned().collect(),
            pull_requests: state
                .pull_requests
                .known()
                .map(|(thread_id, pull_request)| ThreadPullRequest {
                    thread_id: thread_id.clone(),
                    pull_request: Some(pull_request.clone()),
                })
                .collect(),
            context_usages: state
                .context_usages
                .iter()
                .map(|(thread_id, usage)| ThreadContextUsage {
                    thread_id: thread_id.clone(),
                    usage: *usage,
                })
                .collect(),
            background_tasks: state
                .turns
                .values()
                .filter_map(|turn| turn.background.clone())
                .chain(
                    state
                        .lingering
                        .values()
                        .filter_map(|lingering| lingering.background.clone()),
                )
                .filter(|background| !background.tasks.is_empty())
                .collect(),
            task_previews: state.model.task_previews(),
            held_messages: state
                .after_compaction
                .iter()
                .map(|(thread_id, message)| held_message(thread_id, message))
                .collect(),
            project_branches: state
                .root_branches
                .iter()
                .map(|(project_id, branch)| hoshi_core::ProjectBranch {
                    project_id: project_id.clone(),
                    branch: branch.clone(),
                })
                .collect(),
        }
    }

    /// One transcript item, with what the whole transcript leaves out.
    pub fn thread_item(
        &self,
        thread_id: &str,
        item_id: &str,
    ) -> Result<Option<ThreadItem>, CommandError> {
        lock(&self.state)
            .store
            .thread_item(thread_id, item_id)
            .map_err(internal)
    }

    /// The latest turns before item `before`, the end when `None`, light enough to send
    /// as a whole: `thread_item` returns what they leave out.
    pub fn transcript(
        &self,
        thread_id: &str,
        before: Option<&str>,
    ) -> Result<TranscriptPage, CommandError> {
        let (items, has_older) = lock(&self.state)
            .store
            .transcript_page(thread_id, before, PAGE_TURNS, PAGE_MAX_ITEMS)
            .map_err(internal)?;
        let items = items
            .into_iter()
            .map(|item| ThreadItem {
                content: item.content.summary(),
                ..item
            })
            .collect();
        Ok(TranscriptPage { items, has_older })
    }

    /// Carries out a command from the UI. Git work it needs, such as creating a worktree,
    /// runs outside the lock.
    pub fn dispatch(&self, command: Command) -> Result<(), CommandError> {
        let command = match command {
            Command::ThreadCreate {
                workspace: ThreadWorkspace::NewWorktree | ThreadWorkspace::BranchWorktree { .. },
                ..
            } => return self.create_worktree_thread(command),
            Command::MessageSend { .. } => match self.restore_before_sending(command)? {
                Some(command) => command,
                None => return Ok(()),
            },
            other => other,
        };
        let mut state = lock(&self.state);
        let deleted: Vec<String> = state
            .threads_deleted_by(&command)
            .into_iter()
            .map(|thread| thread.id.clone())
            .collect();
        let stopped = match &command {
            Command::BackgroundTaskStop { thread_id, task_id } => state
                .sole_background_command(thread_id, task_id)
                .map(|since| (thread_id.clone(), task_id.clone(), since)),
            _ => None,
        };
        let sent_to = match &command {
            Command::MessageSend {
                thread_id,
                message_id,
                ..
            } => Some((thread_id.clone(), message_id.clone())),
            _ => None,
        };
        if let Command::TurnInterrupt { thread_id } = &command
            && state.hold_for_stop(thread_id)
            && state
                .model
                .thread(thread_id)
                .is_some_and(|thread| thread.active_turn_id.is_none())
        {
            return Ok(());
        }
        state.run_command(command)?;
        if let Some((thread_id, message_id)) = sent_to {
            state.release_queue_after(&thread_id, Some(&message_id));
        }
        let model = &state.model;
        self.terminals
            .retain_threads(|thread_id| model.thread(thread_id).is_some());
        drop(state);
        if let Some((thread_id, task_id, since)) = stopped {
            self.end_servers_of(thread_id, task_id, since);
        }
        if !deleted.is_empty() {
            let browsing = self.browsing.clone();
            thread::spawn(move || {
                for thread_id in &deleted {
                    browsing.forget(thread_id);
                }
            });
        }
        Ok(())
    }

    /// Creates again outside the lock the worktree a message's released thread had. `None`
    /// when the message went, or waits for git work on its thread.
    pub(crate) fn restore_before_sending(
        &self,
        command: Command,
    ) -> Result<Option<Command>, CommandError> {
        let Command::MessageSend { thread_id, .. } = &command else {
            return Ok(Some(command));
        };
        let thread_id = thread_id.clone();
        let (op, root, path, branch) = {
            let mut state = lock(&self.state);
            let restore = state
                .model
                .thread(&thread_id)
                .filter(|_| !state.thread_ops.contains_key(&thread_id))
                .filter(|thread| thread.worktree_released && !thread.worktree_adopted)
                .and_then(|thread| {
                    let path = thread.worktree_path.clone()?;
                    let missing = !Path::new(&path).is_dir()
                        && !state.folder_ops.contains(&git::normalize(&path));
                    missing.then(|| (thread.clone(), path))
                });
            let Some((thread, path)) = restore else {
                return Ok(Some(command));
            };
            let branch = thread
                .branch
                .clone()
                .ok_or_else(|| CommandError::WorktreeFailed {
                    detail: "the thread has no branch to check out".into(),
                })?;
            let root = state.project_root(&thread)?;
            let op = state.begin_op(&thread_id, &[&path])?;
            (op, root, path, branch)
        };
        let restored = git::restore_worktree(&root, &path, &branch);
        let mut state = lock(&self.state);
        match restored {
            Ok(()) => {
                state.start_worktree_setup(&thread_id);
                state.end_op(op, Some(command))?;
            }
            Err(detail) => {
                state.end_op(op, None)?;
                return Err(CommandError::WorktreeFailed { detail });
            }
        }
        Ok(None)
    }

    /// The UI shows this thread's transcript, `None` for none: only its text streams to it.
    pub fn stream_thread(&self, thread_id: Option<String>) {
        self.stream_threads(thread_id.into_iter().collect());
    }

    /// The UI holds these threads' transcripts, shown or kept for a quick return: only their
    /// text streams to it.
    pub fn stream_threads(&self, thread_ids: Vec<String>) {
        lock(&self.state).streamed = thread_ids;
    }

    /// Closes every page and the headless browser, as when the app quits.
    /// Ends the agent processes kept between turns, and the commands running in them, as
    /// when the app quits.
    /// Ends the agent processes kept between turns, as hoshi quits.
    pub fn close_lingering_agents(&self) {
        let mut state = lock(&self.state);
        state.recovery.quitting = true;
        state.close_all_lingering();
    }

    /// Ends every agent process, those running turns and those kept between turns, and
    /// everything they started, as when the app quits. Waits a moment for them to end.
    pub fn end_agents(&self) {
        {
            let mut state = lock(&self.state);
            state.recovery.quitting = true;
            state.close_all_lingering();
            for turn in state.turns.values() {
                turn.running.close();
            }
        }
        hoshi_providers::agent_processes::end_all();
    }
}
