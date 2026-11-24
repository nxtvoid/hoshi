use super::*;

impl ThreadDetails {
    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        let open = self.is_open(cx);
        self.set_open(!open, cx);
    }

    /// Whether the card shows for the thread on screen: unless the user closed it there.
    pub(super) fn is_open(&self, cx: &App) -> bool {
        self.workspace
            .read(cx)
            .shown_thread_id(cx)
            .is_none_or(|thread_id| !self.prefs.closed_threads.contains(&thread_id))
    }

    /// Opens or closes the card for the thread on screen, and remembers it across restarts.
    pub fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        let workspace = self.workspace.read(cx);
        let Some(thread_id) = workspace.shown_thread_id(cx) else {
            return;
        };
        let closed = &mut self.prefs.closed_threads;
        let changed = if open {
            closed.remove(&thread_id)
        } else {
            closed.insert(thread_id)
        };
        if !changed {
            return;
        }
        closed.retain(|thread_id| workspace.thread_or_draft(thread_id));
        self.save_prefs(cx);
        cx.notify();
    }

    /// Whether the chat leaves room for the card; without it the details open as a popover.
    #[cfg(test)]
    pub fn is_inline(&self) -> bool {
        self.inline
    }

    pub(super) fn set_inline(&mut self, inline: bool, cx: &mut Context<Self>) {
        if self.inline != inline {
            self.inline = inline;
            self.popover_open = false;
            cx.notify();
        }
    }

    /// What the floating card takes from the chat's lanes while it shows.
    pub fn lane_room(&self, cx: &App) -> LaneRoom {
        let card = self.inline && self.is_open(cx) && self.subject(cx).is_some();
        if card {
            LaneRoom {
                right: px(CARD_WIDTH + CARD_GAP + CARD_CLEARANCE),
                gutter: px(CARD_CLEARANCE),
            }
        } else {
            LaneRoom::default()
        }
    }

    /// Whether the details show now, as the card or the popover.
    pub(super) fn shown(&self, cx: &App) -> bool {
        if self.inline {
            self.is_open(cx)
        } else {
            self.popover_open
        }
    }

    pub(super) fn subject(&self, cx: &App) -> Option<Subject> {
        let workspace = self.workspace.read(cx);
        if let Some(thread_id) = workspace.open_thread_id(cx) {
            return Some(Subject::Thread(thread_id));
        }
        let draft = workspace.draft.as_ref()?;
        match &draft.project_id {
            Some(project_id) => workspace
                .model
                .project(project_id)
                .is_some()
                .then(|| Subject::Draft(project_id.clone())),
            None => Some(Subject::ChatDraft(draft.thread_id.clone())),
        }
    }

    pub(super) fn load_chat_folder(&mut self, thread_id: String, cx: &mut Context<Self>) {
        if self.chat_folders.contains_key(&thread_id) {
            return;
        }
        let engine = self.engine(cx);
        cx.spawn(async move |this, cx| {
            let id = thread_id.clone();
            let folder = cx
                .background_spawn(async move { engine.draft_chat_folder(&id) })
                .await;
            this.update(cx, |this, cx| {
                if let Some(folder) = folder {
                    this.chat_folders.insert(thread_id, folder);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn engine(&self, cx: &App) -> std::sync::Arc<hoshi_engine::Engine> {
        self.workspace.read(cx).engine.clone()
    }

    pub(super) fn show_thread(&mut self, thread: Option<String>, cx: &mut Context<Self>) {
        let engine = self.engine(cx);
        if let Some(old) = self.shown_thread.take() {
            let engine = engine.clone();
            cx.background_spawn(async move { engine.unwatch_servers(&old) })
                .detach();
        }
        self.shown_thread = thread.clone();
        self.popover_open = false;
        self.error = None;
        self.server_error = None;
        let Some(thread_id) = thread else {
            return;
        };
        let workspace = self.workspace.downgrade();
        cx.spawn(async move |_, cx| {
            let id = thread_id.clone();
            let servers = cx
                .background_spawn(async move { engine.watch_servers(&id) })
                .await;
            workspace
                .update(cx, |workspace, cx| {
                    workspace.servers.insert(thread_id, servers);
                    cx.notify();
                })
                .ok();
        })
        .detach();
    }

    /// Reads the thread's checkout again when it may have changed: at once when its branch,
    /// worktree or status changes, and at most once a second while its agent works.
    pub(super) fn ensure_fresh(&mut self, thread: &Thread, cx: &mut Context<Self>) {
        let checkout = format!(
            "{:?}|{:?}|{:?}",
            thread.branch, thread.worktree_path, thread.status
        );
        if let Some(fresh) = &self.fresh
            && fresh.thread_id == thread.id
            && fresh.checkout == checkout
        {
            if fresh.activity == thread.updated_at {
                return;
            }
            let wait = ACTIVITY_EVERY.saturating_sub(fresh.at.elapsed());
            if !wait.is_zero() {
                if !self.refresh_pending {
                    self.refresh_pending = true;
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(wait).await;
                        this.update(cx, |this, cx| {
                            this.refresh_pending = false;
                            cx.notify();
                        })
                        .ok();
                    })
                    .detach();
                }
                return;
            }
        }
        self.fresh = Some(Freshness {
            thread_id: thread.id.clone(),
            checkout,
            activity: thread.updated_at.clone(),
            at: Instant::now(),
        });
        self.load_checkout(thread.id.clone(), thread.project_id.is_some(), cx);
    }

    pub(super) fn load_checkout(
        &mut self,
        thread_id: String,
        project: bool,
        cx: &mut Context<Self>,
    ) {
        let engine = self.engine(cx);
        let read_folder = !project && !self.chat_folders.contains_key(&thread_id);
        cx.spawn(async move |this, cx| {
            let id = thread_id.clone();
            let (changes, status, folder) = cx
                .background_spawn(async move {
                    let changes = engine.thread_changes(&id).ok();
                    let status = project.then(|| engine.git_status(&id).ok()).flatten();
                    let folder = read_folder.then(|| engine.thread_folder(&id)).flatten();
                    (changes, status, folder)
                })
                .await;
            this.update(cx, |this, cx| {
                match changes {
                    Some(changes) => this.changes.insert(thread_id.clone(), changes),
                    None => this.changes.remove(&thread_id),
                };
                match status {
                    Some(status) => this.statuses.insert(thread_id.clone(), status),
                    None => this.statuses.remove(&thread_id),
                };
                if let Some(folder) = folder {
                    this.chat_folders.insert(thread_id, folder);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Draws again in `wait`, unless an earlier draw is already due.
    pub(super) fn tick_in(&mut self, wait: Duration, cx: &mut Context<Self>) {
        let at = Instant::now() + wait;
        if self.next_tick.is_some_and(|due| due <= at) {
            return;
        }
        self.next_tick = Some(at);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                if this.next_tick == Some(at) {
                    this.next_tick = None;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Saves the prefs after the previous save, so the file ends with the latest.
    pub(super) fn save_prefs(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        let prefs = self.prefs.clone();
        let previous = self.prefs_saving.take();
        self.prefs_saving = Some(cx.background_spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            if let Err(error) = prefs::save(&home, &prefs) {
                eprintln!("hoshi: could not save the details prefs: {error}");
            }
        }));
    }

    pub(super) fn show(&mut self, surface: Surface, cx: &mut Context<Self>) {
        let Some(thread_id) = self.shown_thread.clone() else {
            return;
        };
        self.popover_open = false;
        cx.emit(DetailsEvent::ShowSurface { thread_id, surface });
        cx.notify();
    }

    /// The thread's agents, its subagents read from where their calls were last found.
    pub(super) fn agents(&mut self, thread_id: &str, cx: &App) -> Vec<ThreadAgent> {
        let workspace = self.workspace.read(cx);
        let items: &[ThreadItem] = workspace
            .open
            .as_ref()
            .map(|open| open.read(cx))
            .filter(|open| open.thread_id == thread_id)
            .map_or(&[], |open| &open.items);
        let key = (
            thread_id.to_owned(),
            items.len(),
            items.first().map(|item| item.id.clone()),
            items.last().map(|item| item.id.clone()),
        );
        if self
            .subagent_calls
            .as_ref()
            .is_none_or(|cached| cached.key != key)
        {
            self.subagent_calls = Some(SubagentCalls {
                calls: subagent_calls(thread_id, items),
                key,
            });
        }
        let calls = self
            .subagent_calls
            .as_ref()
            .map_or(&[][..], |cached| &cached.calls);
        let threads: Vec<&Thread> = workspace.model.threads().collect();
        let pending = &workspace.pending_tasks;
        let busy = busy_threads(&threads, pending, workspace.background_tasks.keys());
        thread_agents(
            thread_id,
            threads,
            calls.iter().filter_map(|index| items.get(*index)),
            pending,
            &busy,
        )
    }

    pub(super) fn show_item(&mut self, item_id: String, cx: &mut Context<Self>) {
        if let Some(thread_id) = self.shown_thread.clone() {
            cx.emit(DetailsEvent::ShowItem { thread_id, item_id });
        }
    }

    pub(super) fn open_thread(&mut self, thread_id: String, cx: &mut Context<Self>) {
        self.popover_open = false;
        self.workspace
            .update(cx, |workspace, cx| workspace.open_thread(&thread_id, cx));
    }

    pub(super) fn set_draft(&mut self, draft: bool, cx: &mut Context<Self>) {
        self.prefs.draft_pull_requests = draft;
        self.save_prefs(cx);
        cx.notify();
    }
}
