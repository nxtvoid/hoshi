use super::*;

impl ThreadDetails {
    pub(super) fn open_target(&self) -> Option<Editor> {
        open_target(self.editors.as_deref()?, self.prefs.preferred_editor)
    }

    /// Ctrl+O: opens the card's folder where "Open in" would.
    pub(super) fn open_in_target(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(editor) = self.open_target() else {
            return false;
        };
        if self.subject(cx).is_none() {
            return false;
        }
        self.open_in(editor, cx);
        true
    }

    pub(super) fn open_in(&mut self, editor: Editor, cx: &mut Context<Self>) {
        let Some(subject) = self.subject(cx) else {
            return;
        };
        if self.prefs.preferred_editor != Some(editor) {
            self.prefs.preferred_editor = Some(editor);
            self.save_prefs(cx);
        }
        let engine = self.engine(cx);
        self.run(
            move || match subject {
                Subject::Thread(thread_id) => engine.open_in_editor(&thread_id, editor),
                Subject::Draft(project_id) => engine.open_project_in_editor(&project_id, editor),
                Subject::ChatDraft(thread_id) => {
                    engine.open_draft_chat_in_editor(&thread_id, editor)
                }
            },
            cx,
        );
    }

    pub(super) fn compact_context(&mut self, cx: &mut Context<Self>) {
        let Some(thread_id) = self.shown_thread.clone() else {
            return;
        };
        let engine = self.engine(cx);
        self.run(
            move || {
                engine.dispatch(hoshi_core::Command::MessageSend {
                    thread_id,
                    message_id: uuid::Uuid::new_v4().to_string(),
                    text: crate::composer::COMPACT_COMMAND.to_owned(),
                    attachments: Vec::new(),
                })
            },
            cx,
        );
    }

    /// Runs an engine call in the background, telling its failure under the card, opened for it.
    pub(super) fn run(
        &mut self,
        call: impl FnOnce() -> Result<(), CommandError> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        self.error = None;
        cx.notify();
        self.runs += 1;
        let run = (self.subject_changes, self.runs);
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { call() }).await;
            if let Err(error) = result {
                this.update(cx, |this, cx| {
                    if (this.subject_changes, this.runs) != run {
                        return;
                    }
                    this.error = Some(error_message(&error));
                    if !this.shown(cx) {
                        if this.inline {
                            this.set_open(true, cx);
                        } else {
                            this.popover_open = true;
                        }
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub(super) fn stop_server(&mut self, server: &LocalServer, cx: &mut Context<Self>) {
        let Some(thread_id) = self.shown_thread.clone() else {
            return;
        };
        if self.stopping.is_some() {
            return;
        }
        let (pid, port) = (server.pid, server.port);
        self.stopping = Some((pid, port));
        self.server_error = None;
        cx.notify();
        let engine = self.engine(cx);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { engine.stop_server(&thread_id, pid, port) })
                .await;
            this.update(cx, |this, cx| {
                this.stopping = None;
                if let Err(error) = result {
                    this.server_error = Some(stop_error(&error));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn quick_request(&self, action: GitAction) -> GitActionRequest {
        GitActionRequest {
            action,
            message: None,
            paths: None,
            new_branch: false,
            draft: self.prefs.draft_pull_requests,
        }
    }

    /// Runs a git action, asking first when it would push straight to the default branch.
    /// With `undoable`, the action waits [`UNDO_FOR`] first, showing as that label.
    pub(super) fn start_git(
        &mut self,
        request: GitActionRequest,
        undoable: Option<git::Label>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(thread_id) = self.shown_thread.clone() else {
            return;
        };
        self.start_git_in(thread_id, request, undoable, window, cx);
    }

    fn start_git_in(
        &mut self,
        thread_id: String,
        request: GitActionRequest,
        undoable: Option<git::Label>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(status) = self.statuses.get(&thread_id) else {
            return;
        };
        if !git::pushes_to_default_branch(status, request.action) {
            match undoable {
                Some(label) => self.delay_git(thread_id, request, label, window, cx),
                None => self.run_git(thread_id, request, window, cx),
            }
            return;
        }
        let branch = status.branch.clone().unwrap_or_default();
        let this = cx.weak_entity();
        DialogLayer::open(window, cx, move |dialog, _, _| {
            let go = |new_branch: bool| {
                let this = this.clone();
                let thread_id = thread_id.clone();
                let request = GitActionRequest {
                    new_branch,
                    ..request.clone()
                };
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    let (thread_id, request) = (thread_id.clone(), request.clone());
                    this.update(cx, |this, cx| this.run_git(thread_id, request, window, cx))
                        .ok();
                }
            };
            dialog
                .alert()
                .title(m::git_default_branch_title(&branch))
                .description(m::git_default_branch_description(&branch))
                .footer(
                    DialogButton::new("git-default-cancel")
                        .label(m::cancel())
                        .variant(DialogButtonVariant::Outline)
                        .close(),
                )
                .footer(
                    DialogButton::new("git-default-new-branch")
                        .label(m::git_default_branch_new())
                        .variant(DialogButtonVariant::Outline)
                        .on_click(go(true))
                        .close(),
                )
                .footer(
                    DialogButton::new("git-default-confirm")
                        .label(m::git_default_branch_confirm(&branch))
                        .on_click(go(false))
                        .close(),
                )
        });
    }

    fn delay_git(
        &mut self,
        thread_id: String,
        request: GitActionRequest,
        label: git::Label,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.running.contains_key(&thread_id) {
            return;
        }
        let started = Instant::now();
        self.pending_git.insert(
            thread_id.clone(),
            PendingGit {
                request,
                label,
                started,
            },
        );
        self.done.remove(&thread_id);
        cx.notify();
        let engine = self.engine(cx);
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(UNDO_FOR).await;
            let id = thread_id.clone();
            let status = cx
                .background_spawn(async move { engine.git_status(&id) })
                .await;
            this.update_in(cx, |this, window, cx| {
                let due = this
                    .pending_git
                    .get(&thread_id)
                    .is_some_and(|pending| pending.started == started);
                let Some(pending) = due.then(|| this.pending_git.remove(&thread_id)).flatten()
                else {
                    return;
                };
                let shown = this.shown_thread.as_deref() == Some(thread_id.as_str());
                let refused = match status {
                    Ok(status) => {
                        this.statuses.insert(thread_id.clone(), status);
                        this.thread_working(&thread_id, cx).then(m::git_error_busy)
                    }
                    Err(error) => Some(error_message(&error)),
                };
                if let Some(refused) = refused {
                    if shown {
                        this.error = Some(refused);
                    }
                    cx.notify();
                    return;
                }
                this.start_git_in(thread_id, pending.request, None, window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn thread_working(&self, thread_id: &str, cx: &App) -> bool {
        self.workspace
            .read(cx)
            .model
            .thread(thread_id)
            .is_some_and(|thread| thread.active_turn_id.is_some())
    }

    pub(super) fn undo_git(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        if self.pending_git.remove(thread_id).is_some() {
            cx.notify();
        }
    }

    pub(super) fn run_git(
        &mut self,
        thread_id: String,
        request: GitActionRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_git.remove(&thread_id);
        if self.running.contains_key(&thread_id) {
            return;
        }
        self.running.insert(thread_id.clone(), Instant::now());
        self.done.remove(&thread_id);
        self.workspace.update(cx, |workspace, _| {
            workspace.git_phases.remove(&thread_id);
        });
        cx.notify();
        let engine = self.engine(cx);
        cx.spawn_in(window, async move |this, cx| {
            let id = thread_id.clone();
            let result = cx
                .background_spawn(async move { engine.run_git_action(&id, &request) })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.running.remove(&thread_id);
                this.load_checkout(thread_id.clone(), true, cx);
                match result {
                    Ok(result) => this.finished(thread_id, result, cx),
                    Err(error) => DialogLayer::confirm(
                        ConfirmRequest::new(m::git_action_failed(), m::dialog_ok())
                            .description(git::error_text(&error))
                            .tone(ConfirmTone::Warning),
                        window,
                        cx,
                    )
                    .detach(),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn finished(
        &mut self,
        thread_id: String,
        result: GitActionResult,
        cx: &mut Context<Self>,
    ) {
        self.done.insert(thread_id.clone(), result.clone());
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DONE_FOR).await;
            this.update(cx, |this, cx| {
                if this.done.get(&thread_id) == Some(&result) {
                    this.done.remove(&thread_id);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn open_commit_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread_id) = self.shown_thread.clone() else {
            return;
        };
        let Some(status) = self.statuses.get(&thread_id).cloned() else {
            return;
        };
        commit::open(
            cx.weak_entity(),
            thread_id,
            status,
            self.engine(cx),
            window,
            cx,
        );
    }
}
