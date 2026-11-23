use super::*;

impl Sidebar {
    /// Wakes the list when the soonest snooze ends, so the thread leaves Snoozed on time.
    pub(super) fn schedule_refresh(&mut self, cx: &mut Context<Self>) {
        let now = Utc::now();
        let Some(at) = sections::next_refresh(self.workspace.read(cx).model.threads(), now) else {
            return;
        };
        if self
            .refresh
            .as_ref()
            .is_some_and(|(scheduled, _)| *scheduled <= at)
        {
            return;
        }
        let wait = (at - now).to_std().unwrap_or_default();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                this.refresh = None;
                this.workspace.update(cx, |_, cx| cx.notify());
            })
            .ok();
        });
        self.refresh = Some((at, task));
    }

    /// Writes the prefs a moment after the last change, so a drag writes once.
    pub(super) fn save_prefs(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        let prefs = self.prefs.clone();
        self.saving = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            cx.background_spawn(async move { prefs::save(&home, &prefs) })
                .await
                .ok();
        }));
    }

    pub(super) fn set_view(&mut self, view: View, cx: &mut Context<Self>) {
        self.prefs.view = view;
        self.save_prefs(cx);
        self.follow_scope(cx);
        cx.notify();
    }

    /// Picks a project on the rail: its threads, and an empty composer in it, as the web's
    /// `selectProject` leaves the open thread.
    pub(super) fn select_project(
        &mut self,
        project_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.project_id = Some(project_id.clone());
        self.search
            .update(cx, |search, cx| search.set_value("", window, cx));
        self.set_view(View::Project, cx);
        self.workspace.update(cx, |workspace, cx| {
            workspace.start_draft(Some(project_id), cx)
        });
    }

    /// With no thread open, the main view is a new thread in what the rail picked.
    pub(super) fn follow_scope(&self, cx: &mut Context<Self>) {
        let project = match self.prefs.view {
            View::Chats => None,
            _ => match &self.project_id {
                Some(id) => Some(id.clone()),
                None => return,
            },
        };
        self.workspace.update(cx, |workspace, cx| {
            let current = workspace.draft.as_ref().map(|draft| &draft.project_id);
            if workspace.open.is_none() && current != Some(&project) {
                workspace.start_draft(project, cx);
            }
        });
    }

    pub(super) fn toggle_section(&mut self, section: SectionId, cx: &mut Context<Self>) {
        if !self.prefs.collapsed.remove(&section) {
            self.prefs.collapsed.insert(section);
        }
        self.save_prefs(cx);
        cx.notify();
    }

    pub(super) fn set_width(&mut self, width: f32, cx: &mut Context<Self>) {
        self.prefs.width = prefs::clamp_width(width);
        self.save_prefs(cx);
        cx.notify();
    }

    pub(super) fn move_project(&mut self, moved: &str, over: &str, cx: &mut Context<Self>) {
        let ids: Vec<String> = self
            .ordered_projects(cx)
            .into_iter()
            .map(|project| project.id)
            .collect();
        self.prefs.project_order = prefs::move_id(&ids, moved, over);
        self.save_prefs(cx);
        cx.notify();
    }

    pub(super) fn ordered_projects(&self, cx: &App) -> Vec<Project> {
        let mut projects: Vec<Project> =
            self.workspace.read(cx).model.projects().cloned().collect();
        projects.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        prefs::order_by(projects, &self.prefs.project_order, |project| &project.id)
    }

    pub(super) fn new_thread(&mut self, cx: &mut Context<Self>) {
        let no_projects = self.workspace.read(cx).model.projects().next().is_none();
        match (self.prefs.view, &self.project_id) {
            (View::Project, Some(project_id)) => {
                let project_id = Some(project_id.clone());
                self.workspace
                    .update(cx, |workspace, cx| workspace.start_draft(project_id, cx));
            }
            (View::Chats, _) => self.open_new_draft(None, cx),
            _ if no_projects => self.open_new_draft(None, cx),
            _ => cx.emit(SidebarEvent::OpenPalette(palette::Mode::NewThread)),
        }
    }

    /// What the sidebar shows, for the palette to start threads where the web would.
    pub fn scope(&self, cx: &App) -> palette::SidebarScope {
        palette::SidebarScope {
            view: self.prefs.view,
            project_id: self.project_id.clone(),
            project_order: self
                .ordered_projects(cx)
                .into_iter()
                .map(|project| project.id)
                .collect(),
        }
    }

    pub fn view(&self) -> View {
        self.prefs.view
    }

    /// The project's icon, once the rail has looked it up.
    pub fn project_icon(&self, project_id: &str) -> Option<PathBuf> {
        self.icons.get(project_id).cloned().flatten()
    }

    /// Shows `view`, on `project_id` when one is given.
    pub fn show(&mut self, view: View, project_id: Option<String>, cx: &mut Context<Self>) {
        if project_id.is_some() {
            self.project_id = project_id;
        }
        self.set_view(view, cx);
    }

    pub(super) fn open_thread(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |workspace, cx| workspace.open_thread(thread_id, cx));
    }

    /// The sections the list shows now.
    pub(super) fn current_sections(&self, cx: &App) -> Vec<Section> {
        let search = self.search.read(cx).value().to_string();
        let workspace = self.workspace.read(cx);
        let threads: Vec<Thread> = workspace.listed_threads();
        sidebar_sections(
            &threads,
            &workspace.pull_requests,
            &Query {
                view: self.prefs.view,
                project_id: self.project_id.as_deref(),
                search: &search,
                filter: self.prefs.filter,
                sort: self.prefs.sort,
                inactive_after: sections::inactive_after(
                    workspace.settings.general.inactive_after_days,
                ),
                logins: &workspace.logins,
                model: &workspace.model,
            },
            Utc::now(),
            &workspace.unseen,
        )
    }

    pub(super) fn step(&mut self, step: isize, attention: bool, cx: &mut Context<Self>) {
        let order: Vec<String> = if attention {
            let workspace = self.workspace.read(cx);
            let threads: Vec<Thread> = workspace.listed_threads();
            inbox_sections(
                sections::with_delegated_work(&threads),
                &workspace.pull_requests,
                &workspace.model,
                Utc::now(),
                &workspace.unseen,
                &workspace.logins,
            )
            .into_iter()
            .find(|section| section.id == SectionId::Attention)
            .map(|section| {
                section
                    .threads
                    .into_iter()
                    .map(|thread| thread.id)
                    .collect()
            })
            .unwrap_or_default()
        } else {
            let searching = !self.search.read(cx).value().is_empty();
            prefs::visible_order(
                &self.current_sections(cx),
                |section| searching || !self.prefs.collapsed.contains(&section),
                self.shown_thread.as_deref(),
            )
        };
        if let Some(next) = prefs::step_thread(&order, self.shown_thread.as_deref(), step) {
            self.open_thread(&next, cx);
        }
    }

    pub(super) fn dispatch(&self, command: Command, cx: &mut Context<Self>) {
        let engine = self.workspace.read(cx).engine.clone();
        cx.background_spawn(async move {
            if let Err(error) = engine.dispatch(command) {
                eprintln!("hoshi: sidebar command failed: {error:?}");
            }
        })
        .detach();
    }

    pub(super) fn open_menu(
        &mut self,
        target: MenuTarget,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.menu_serial += 1;
        self.cards.close();
        self.menu = Some(OpenMenu {
            serial: self.menu_serial,
            target,
            position,
        });
        cx.notify();
    }

    pub(super) fn thread_action(
        &mut self,
        thread_id: String,
        action: ThreadAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            ThreadAction::Pin(pinned) => {
                self.dispatch(Command::ThreadPin { thread_id, pinned }, cx)
            }
            ThreadAction::Wake => self.wake_thread(thread_id, window, cx),
            ThreadAction::Archive(archived) => self.dispatch(
                Command::ThreadArchive {
                    thread_id,
                    archived,
                },
                cx,
            ),
            ThreadAction::RegenerateTitle => {
                let engine = self.workspace.read(cx).engine.clone();
                cx.background_spawn(async move { engine.regenerate_title(&thread_id) })
                    .detach();
            }
            ThreadAction::Fork => crate::fork::fork_thread(
                &self.workspace,
                thread_id,
                hoshi_core::ForkPoint::LatestStable,
                window,
                cx,
            ),
            ThreadAction::CleanUp | ThreadAction::Release => {
                let release = matches!(action, ThreadAction::Release);
                let engine = self.workspace.read(cx).engine.clone();
                cx.spawn_in(window, async move |_, cx| {
                    let result = cx
                        .background_spawn(async move {
                            if release {
                                engine.release_worktree(&thread_id)
                            } else {
                                engine.clean_up_thread(&thread_id)
                            }
                        })
                        .await;
                    if let Err(error) = result {
                        let title = if release {
                            m::release_worktree()
                        } else {
                            m::clean_up_thread()
                        };
                        cx.update(|window, cx| {
                            DialogLayer::confirm(
                                ConfirmRequest::new(title, m::dialog_ok())
                                    .description(cleanup_error_message(&error))
                                    .tone(ConfirmTone::Warning),
                                window,
                                cx,
                            )
                        })
                        .map(|answer| answer.detach())
                        .ok();
                    }
                })
                .detach();
            }
            ThreadAction::DiscardDraft => self.workspace.update(cx, |workspace, cx| {
                workspace.forget_drafts(&[thread_id], cx)
            }),
            ThreadAction::Delete => {
                let workspace = self.workspace.read(cx);
                let Some(thread) = workspace.model.thread(&thread_id) else {
                    return;
                };
                let title = if thread.title.is_empty() {
                    m::untitled_thread()
                } else {
                    thread.title.clone()
                };
                let threads: Vec<Thread> = workspace.listed_threads();
                let tasks = delegated_tasks(&thread_id, &threads);
                let description = if tasks > 0 {
                    m::delete_thread_with_tasks_confirm(tasks, &title)
                } else {
                    m::delete_thread_confirm(&title)
                };
                let answer = DialogLayer::confirm(
                    ConfirmRequest::new(m::delete_thread(), m::delete_thread())
                        .description(description)
                        .cancel_label(m::cancel())
                        .destructive(),
                    window,
                    cx,
                );
                cx.spawn(async move |this, cx| {
                    if answer.await {
                        this.update(cx, |this, cx| {
                            this.dispatch(Command::ThreadDelete { thread_id }, cx)
                        })
                        .ok();
                    }
                })
                .detach();
            }
        }
    }

    pub(super) fn remove_project(
        &mut self,
        project_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.workspace.read(cx).model.project(&project_id) else {
            return;
        };
        let answer = DialogLayer::confirm(
            ConfirmRequest::new(m::remove_project(), m::remove_project())
                .description(m::remove_project_confirm(&project.title))
                .cancel_label(m::cancel())
                .destructive(),
            window,
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await {
                this.update(cx, |this, cx| {
                    this.dispatch(Command::ProjectRemove { project_id }, cx)
                })
                .ok();
            }
        })
        .detach();
    }

    pub(super) fn edit_setup(
        &mut self,
        project_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = self.workspace.read(cx);
        let Some(project) = workspace.model.project(project_id).cloned() else {
            return;
        };
        let engine = workspace.engine.clone();
        let form = cx.new(|cx| WorktreeSetupForm::new(project, engine, window, cx));
        let subscription = cx.subscribe(
            &form,
            |this, _, event: &WorktreeSetupEvent, cx| match event {
                WorktreeSetupEvent::Close => {
                    this.setup = None;
                    cx.notify();
                }
            },
        );
        self.setup = Some((form, subscription));
        cx.notify();
    }

    /// Cleans up every merged thread of a section that still has a worktree, one after another.
    /// Each one is checked on its own; those that would lose work are kept and listed after.
    pub(super) fn clean_up_merged(
        &mut self,
        threads: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = threads.len();
        let label = m::cleanup_merged(count);
        let answer = DialogLayer::confirm(
            ConfirmRequest::new(label.clone(), m::clean_up_thread())
                .description(m::cleanup_merged_confirm(count))
                .cancel_label(m::cancel())
                .destructive(),
            window,
            cx,
        );
        let engine = self.workspace.read(cx).engine.clone();
        cx.spawn_in(window, async move |this, cx| {
            if !answer.await {
                return;
            }
            this.update(cx, |this, cx| {
                this.cleaning_merged = true;
                cx.notify();
            })
            .ok();
            let failures = cx
                .background_spawn(async move {
                    threads
                        .iter()
                        .filter_map(|id| engine.clean_up_thread(id).err())
                        .map(|error| cleanup_error_message(&error))
                        .collect::<Vec<_>>()
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.cleaning_merged = false;
                cx.notify();
                if !failures.is_empty() {
                    let reasons: BTreeSet<String> = failures.iter().cloned().collect();
                    DialogLayer::confirm(
                        ConfirmRequest::new(label, m::dialog_ok())
                            .description(m::cleanup_merged_failed(
                                failures.len(),
                                reasons.into_iter().collect::<Vec<_>>().join(" "),
                            ))
                            .tone(ConfirmTone::Warning),
                        window,
                        cx,
                    )
                    .detach();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Shows the draft of a new thread in `project_id`, or of a new chat.
    pub(super) fn open_new_draft(&mut self, project_id: Option<String>, cx: &mut Context<Self>) {
        match &project_id {
            Some(id) => {
                self.project_id = Some(id.clone());
                self.prefs.view = View::Project;
            }
            None => self.prefs.view = View::Chats,
        }
        self.save_prefs(cx);
        self.workspace
            .update(cx, |workspace, cx| workspace.start_draft(project_id, cx));
        cx.notify();
    }
}
