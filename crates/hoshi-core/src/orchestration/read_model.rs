use super::*;

#[derive(Debug, Clone, Default)]
pub struct ReadModel {
    pub(super) projects: BTreeMap<String, Project>,
    pub(super) threads: BTreeMap<String, Thread>,
    /// Requests waiting for an answer, by id.
    pub(super) requests: BTreeMap<String, ThreadItem>,
    /// Tool calls without output yet, of threads and their subagents.
    pub(super) open_calls: BTreeSet<OpenToolCall>,
    /// The latest message of each delegated task, shortened.
    pub(super) task_previews: BTreeMap<String, String>,
    /// How each running turn's provider retries, until it gets through.
    pub(super) retries: BTreeMap<String, TurnRetry>,
    /// The MCP servers each thread already told of failing to start, since the app started.
    pub(super) mcp_failures: BTreeMap<String, BTreeSet<String>>,
    /// The compaction each thread's running turn shows as running, by thread.
    pub(super) compacting: BTreeMap<String, String>,
}

impl ReadModel {
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        Self {
            projects: snapshot
                .projects
                .iter()
                .map(|project| (project.id.clone(), project.clone()))
                .collect(),
            threads: snapshot
                .threads
                .iter()
                .map(|thread| (thread.id.clone(), thread.clone()))
                .collect(),
            requests: snapshot
                .pending_requests
                .iter()
                .map(|item| (item.id.clone(), item.clone()))
                .collect(),
            open_calls: snapshot.open_tool_calls.iter().cloned().collect(),
            task_previews: snapshot
                .task_previews
                .iter()
                .map(|preview| (preview.thread_id.clone(), preview.text.clone()))
                .collect(),
            retries: BTreeMap::new(),
            mcp_failures: BTreeMap::new(),
            compacting: BTreeMap::new(),
        }
    }

    /// How the thread's running turn retries to reach the model, while it does.
    pub fn retry(&self, thread_id: &str) -> Option<&TurnRetry> {
        self.retries.get(thread_id)
    }

    /// The latest message of each delegated task, shortened.
    pub fn task_previews(&self) -> Vec<TaskPreview> {
        self.task_previews
            .iter()
            .map(|(thread_id, text)| TaskPreview {
                thread_id: thread_id.clone(),
                text: text.clone(),
            })
            .collect()
    }

    pub fn open_tool_calls(&self) -> impl Iterator<Item = &OpenToolCall> {
        self.open_calls.iter()
    }

    /// Every request waiting for an answer, oldest first.
    pub fn all_pending_requests(&self) -> Vec<ThreadItem> {
        let mut pending: Vec<_> = self.requests.values().cloned().collect();
        pending.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        pending
    }

    /// The request `request_id`, while it waits for an answer.
    pub fn pending_request(&self, request_id: &str) -> Option<&ThreadItem> {
        self.requests.get(request_id)
    }

    /// The requests waiting for an answer in the thread's transcript and its subagents', in
    /// no particular order.
    pub fn pending_requests_within<'a>(
        &'a self,
        thread_id: &'a str,
    ) -> impl Iterator<Item = &'a ThreadItem> + 'a {
        self.requests
            .values()
            .filter(move |item| root_thread(&item.thread_id) == thread_id)
    }

    /// The thread's requests waiting for an answer, oldest first.
    pub fn pending_requests(&self, thread_id: &str) -> Vec<&ThreadItem> {
        let mut pending: Vec<_> = self
            .requests
            .values()
            .filter(|item| item.thread_id == thread_id)
            .collect();
        pending.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        pending
    }

    pub fn projects(&self) -> impl Iterator<Item = &Project> {
        self.projects.values()
    }

    pub fn threads(&self) -> impl Iterator<Item = &Thread> {
        self.threads.values()
    }

    pub fn thread(&self, thread_id: &str) -> Option<&Thread> {
        self.threads.get(thread_id)
    }

    pub fn project(&self, project_id: &str) -> Option<&Project> {
        self.projects.get(project_id)
    }

    /// The threads forked from `thread_id`, oldest first.
    pub fn forks_of(&self, thread_id: &str) -> Vec<&Thread> {
        let mut forks: Vec<&Thread> = self
            .threads
            .values()
            .filter(|thread| {
                thread
                    .forked_from
                    .as_ref()
                    .is_some_and(|origin| origin.thread_id == thread_id)
            })
            .collect();
        forks.sort_by(|a, b| (&a.created_at, &a.id).cmp(&(&b.created_at, &b.id)));
        forks
    }

    /// Where the thread's agent runs: its worktree when it has one, never the
    /// project root in that case. `None` for a chat, whose folder only the engine knows.
    pub fn thread_cwd(&self, thread_id: &str) -> Option<&str> {
        let thread = self.threads.get(thread_id)?;
        match &thread.worktree_path {
            Some(path) => Some(path),
            None => self
                .projects
                .get(thread.project_id.as_deref()?)
                .map(|project| project.workspace_root.as_str()),
        }
    }

    pub fn apply(&mut self, event: &Event) {
        match event {
            Event::ProjectCreated { project } => {
                self.projects.insert(project.id.clone(), project.clone());
            }
            Event::ProjectWorktreeSetupSet { project_id, setup } => {
                if let Some(project) = self.projects.get_mut(project_id) {
                    project.worktree_setup.clone_from(setup);
                }
            }
            Event::ProjectSettingsSet {
                project_id,
                settings,
            } => {
                if let Some(project) = self.projects.get_mut(project_id) {
                    project.settings.clone_from(settings);
                }
            }
            Event::ProjectRemoved { project_id } => {
                self.projects.remove(project_id);
                self.threads
                    .retain(|_, thread| thread.project_id.as_ref() != Some(project_id));
                let threads = &self.threads;
                self.requests
                    .retain(|_, item| threads.contains_key(&item.thread_id));
                self.open_calls
                    .retain(|call| threads.contains_key(root_thread(&call.thread_id)));
                self.task_previews
                    .retain(|thread_id, _| threads.contains_key(thread_id));
                self.retries
                    .retain(|thread_id, _| threads.contains_key(thread_id));
                self.mcp_failures
                    .retain(|thread_id, _| threads.contains_key(thread_id));
                self.compacting
                    .retain(|thread_id, _| threads.contains_key(thread_id));
            }
            Event::ThreadCreated { thread } => {
                self.threads
                    .insert(thread.id.clone(), Thread::clone(thread));
            }
            Event::ThreadDeleted { thread_id } => {
                self.threads.remove(thread_id);
                self.task_previews.remove(thread_id);
                self.retries.remove(thread_id);
                self.mcp_failures.remove(thread_id);
                self.compacting.remove(thread_id);
                self.requests.retain(|_, item| &item.thread_id != thread_id);
                self.open_calls
                    .retain(|call| root_thread(&call.thread_id) != thread_id);
            }
            Event::ThreadTitleSet { thread_id, title } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.title.clone_from(title);
                }
            }
            Event::ThreadBranchSet { thread_id, branch } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.branch.clone_from(branch);
                }
            }
            Event::ThreadWorktreeAttached {
                thread_id,
                worktree_path,
                branch,
                adopted,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.worktree_path = Some(worktree_path.clone());
                    thread.branch.clone_from(branch);
                    thread.worktree_released = false;
                    thread.worktree_adopted = *adopted;
                }
            }
            Event::ThreadWorktreeDetached { thread_id, branch } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.worktree_path = None;
                    thread.branch.clone_from(branch);
                    thread.worktree_released = false;
                    thread.worktree_adopted = false;
                }
            }
            Event::ThreadWorktreeReleased { thread_id } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.worktree_released = true;
                }
            }
            Event::ThreadWorktreeRestored { thread_id } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.worktree_released = false;
                }
            }
            Event::ThreadCleanedUp { thread_id } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.worktree_path = None;
                    thread.branch = None;
                    thread.worktree_released = false;
                    thread.worktree_adopted = false;
                    thread.session_id = None;
                    thread.provider_sessions.clear();
                }
            }
            Event::ThreadArchivedSet {
                thread_id,
                archived,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.archived = *archived;
                }
            }
            Event::ThreadPinnedSet { thread_id, pinned } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.pinned = *pinned;
                }
            }
            Event::ThreadSnoozed {
                thread_id,
                until,
                at,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.snoozed_until = Some(until.clone());
                    thread.snoozed_at = Some(at.clone());
                }
            }
            Event::ThreadUnsnoozed { thread_id, woke_at } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.snoozed_until = None;
                    thread.snoozed_at = None;
                    if woke_at.is_some() {
                        thread.last_snooze_wake_at.clone_from(woke_at);
                    }
                }
            }
            Event::ThreadVisited { thread_id, at } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.last_visited_at = Some(at.clone());
                }
            }
            Event::ThreadModelSet {
                thread_id,
                provider,
                model,
                effort,
                fast_mode,
                ultracode,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.provider.clone_from(provider);
                    thread.model.clone_from(model);
                    thread.effort = *effort;
                    thread.fast_mode = *fast_mode;
                    thread.ultracode = *ultracode;
                }
            }
            Event::ThreadAccessSet {
                thread_id,
                access_mode,
                plan_mode,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.access_mode = *access_mode;
                    thread.plan_mode = *plan_mode;
                }
            }
            Event::TurnStarted {
                thread_id,
                turn_id,
                at,
                model,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.status = ThreadStatus::Working;
                    thread.active_turn_id = Some(turn_id.clone());
                    thread.updated_at.clone_from(at);
                    if let Some(model) = model {
                        thread.last_turn_model = Some(Box::new(model.clone()));
                    }
                }
                self.retries.remove(thread_id);
            }
            Event::SessionAttached {
                thread_id,
                session_id,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.session_id = Some(session_id.clone());
                }
            }
            Event::ItemAdded { item } => {
                if let Some(thread) = self.threads.get_mut(&item.thread_id) {
                    thread.updated_at.clone_from(&item.created_at);
                    if let (Some(_), ItemContent::AssistantMessage { text }) =
                        (&thread.parent_thread_id, &item.content)
                    {
                        self.task_previews
                            .insert(thread.id.clone(), task_preview(text));
                    }
                }
                let call = OpenToolCall {
                    thread_id: item.thread_id.clone(),
                    item_id: item.id.clone(),
                };
                match &item.content {
                    ItemContent::ToolCall {
                        output: None,
                        interrupted: false,
                        ..
                    } => {
                        self.open_calls.insert(call);
                    }
                    ItemContent::ToolCall { .. } => {
                        self.open_calls.remove(&call);
                    }
                    ItemContent::McpServerFailed { name, .. } => {
                        self.mcp_failures
                            .entry(item.thread_id.clone())
                            .or_default()
                            .insert(name.clone());
                    }
                    ItemContent::Compaction {
                        status: CompactionStatus::Running,
                        ..
                    } => {
                        self.compacting
                            .insert(item.thread_id.clone(), item.id.clone());
                    }
                    ItemContent::Compaction { .. } => {
                        self.compacting.remove(&item.thread_id);
                    }
                    _ => {}
                }
                match item.content.request_status() {
                    Some(RequestStatus::Pending) => {
                        self.requests.insert(item.id.clone(), item.clone());
                    }
                    Some(_) => {
                        self.requests.remove(&item.id);
                    }
                    None => {}
                }
            }
            Event::ToolCallCompleted {
                thread_id, item_id, ..
            }
            | Event::ToolCallInterrupted { thread_id, item_id } => {
                self.open_calls.remove(&OpenToolCall {
                    thread_id: thread_id.clone(),
                    item_id: item_id.clone(),
                });
            }
            Event::ToolCallDetailSet { .. } | Event::MessagesDropped { .. } => {}
            Event::TurnRetrying { thread_id, retry } => match retry {
                Some(retry) => {
                    self.retries.insert(thread_id.clone(), retry.clone());
                }
                None => {
                    self.retries.remove(thread_id);
                }
            },
            Event::TurnCompleted {
                thread_id,
                outcome,
                at,
                ..
            } => {
                self.retries.remove(thread_id);
                self.compacting.remove(thread_id);
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.active_turn_id = None;
                    thread.status = match outcome {
                        TurnOutcome::Failed { .. } => ThreadStatus::Error,
                        TurnOutcome::Completed | TurnOutcome::Interrupted => ThreadStatus::Idle,
                    };
                    thread.updated_at.clone_from(at);
                    thread.last_turn_ended_at = Some(at.clone());
                }
            }
            Event::ThreadStatusSet { thread_id, status } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.status = *status;
                }
            }
            Event::ThreadDelegated {
                thread_id,
                parent_thread_id,
                shares_worktree,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.parent_thread_id = Some(parent_thread_id.clone());
                    thread.worktree_adopted |= *shares_worktree;
                }
            }
            Event::TaskCancelled { thread_id } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.task_cancelled = true;
                }
            }
            Event::TaskAnswered { thread_id } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.task_answered = true;
                }
            }
            Event::TaskReopened { thread_id } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.task_answered = false;
                }
            }
            Event::PullRequestWatchSet {
                thread_id,
                watching,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.watching_pull_request = *watching;
                }
            }
            Event::ConversationCursorSet { thread_id, cursor } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.conversation_cursor = Some(cursor.clone());
                    thread.resume_at = None;
                }
            }
            Event::ThreadRewound {
                thread_id,
                resume_at,
                last_turn_model,
                ..
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    if resume_at.is_none() {
                        thread.session_id = None;
                        thread.provider_sessions.clear();
                    }
                    thread.conversation_cursor.clone_from(resume_at);
                    thread.resume_at.clone_from(resume_at);
                    thread.last_turn_model = last_turn_model.clone().map(Box::new);
                }
                self.requests
                    .retain(|_, item| root_thread(&item.thread_id) != thread_id);
                self.open_calls
                    .retain(|call| root_thread(&call.thread_id) != thread_id);
            }
            Event::ThreadModelPending { thread_id, choice } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.pending_model = choice.clone().map(Box::new);
                }
            }
            Event::ProviderSwitched {
                thread_id,
                to,
                session_id,
                conversation_cursor,
                resume_at,
                provider_sessions,
                pending_handoffs,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.provider.clone_from(&to.provider);
                    thread.model.clone_from(&to.model);
                    thread.effort = to.effort;
                    thread.fast_mode = to.fast_mode;
                    thread.ultracode = to.ultracode;
                    thread.session_id.clone_from(session_id);
                    thread.conversation_cursor.clone_from(conversation_cursor);
                    thread.resume_at.clone_from(resume_at);
                    thread.provider_sessions.clone_from(provider_sessions);
                    thread.pending_handoffs.clone_from(pending_handoffs);
                    thread.pending_model = None;
                }
            }
            Event::ContextHandoffsSet {
                thread_id,
                handoffs,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.pending_handoffs.clone_from(handoffs);
                }
            }
        }
    }
}
