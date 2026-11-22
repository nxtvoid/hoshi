use super::*;

impl Sidebar {
    pub(super) fn context_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let open = self.menu.as_ref()?;
        let serial = open.serial;
        let this = cx.weak_entity();
        let closing = this.clone();
        let on_close = move |_: &mut Window, cx: &mut App| {
            closing
                .update(cx, |this, cx| {
                    if this.menu.as_ref().map(|menu| menu.serial) == Some(serial) {
                        this.menu = None;
                        cx.notify();
                    }
                })
                .ok();
        };
        let id = SharedString::from(format!("context-menu-{serial}"));
        match &open.target {
            MenuTarget::Project(project_id) => {
                let project_id = project_id.clone();
                Some(
                    context_menu(id, open.position, on_close, move |menu, _, _| {
                        project_menu(menu, this, project_id)
                    })
                    .into_any_element(),
                )
            }
            MenuTarget::Snooze(thread_id) => {
                let thread_id = thread_id.clone();
                let presets = self
                    .workspace
                    .read(cx)
                    .settings
                    .general
                    .snooze_presets
                    .clone();
                Some(
                    context_menu(id, open.position, on_close, move |menu, _, _| {
                        snooze_menu(menu, this, thread_id, &presets)
                    })
                    .into_any_element(),
                )
            }
            MenuTarget::Draft(key) => {
                let (workspace, key) = (self.workspace.downgrade(), key.clone());
                Some(
                    context_menu(id, open.position, on_close, move |menu, _, _| {
                        menu.item(
                            MenuOption::new("draft-discard", m::discard_draft())
                                .destructive()
                                .on_select(move |_, cx| {
                                    workspace
                                        .update(cx, |workspace, cx| {
                                            workspace.forget_drafts(std::slice::from_ref(&key), cx)
                                        })
                                        .ok();
                                }),
                        )
                    })
                    .into_any_element(),
                )
            }
            MenuTarget::Thread(thread_id) => {
                let workspace = self.workspace.read(cx);
                let thread = workspace.model.thread(thread_id)?.clone();
                let has_draft = workspace.drafts.contains_key(thread_id)
                    && self.shown_thread.as_deref() != Some(thread_id.as_str());
                let pull_request = workspace.pull_requests.get(thread_id).cloned();
                let can_clean = merged_cleanup(&thread, pull_request.as_ref()).is_some();
                let can_release = thread.worktree_path.is_some()
                    && !thread.worktree_released
                    && !thread.worktree_adopted;
                let snoozed = is_snoozed(&workspace.model, &thread, Utc::now());
                let presets = workspace.settings.general.snooze_presets.clone();
                let others =
                    crate::provider_switch::other_models(&workspace.models, &thread.provider);
                let switch = (!others.is_empty()).then(|| self.workspace.clone());
                Some(
                    context_menu(id, open.position, on_close, move |menu, _, _| {
                        let item = |key: &'static str, label: String, action: ThreadAction| {
                            let (this, id) = (this.clone(), thread.id.clone());
                            MenuOption::new(key, label).on_select(move |window, cx| {
                                this.update(cx, |this, cx| {
                                    this.thread_action(id.clone(), action, window, cx)
                                })
                                .ok();
                            })
                        };
                        let menu = menu.item(item(
                            "thread-pin",
                            if thread.pinned {
                                m::unpin_thread()
                            } else {
                                m::pin_thread()
                            },
                            ThreadAction::Pin(!thread.pinned),
                        ));
                        let menu = if thread.archived {
                            menu
                        } else if snoozed {
                            menu.item(item("thread-wake", m::wake_thread(), ThreadAction::Wake))
                        } else {
                            let (this, id) = (this.clone(), thread.id.clone());
                            menu.submenu(
                                "thread-snooze",
                                m::snooze_menu(),
                                None,
                                move |menu, _, _| snooze_menu(menu, this, id, &presets),
                            )
                        };
                        let menu = menu
                            .item(item("thread-fork", m::fork_thread(), ThreadAction::Fork))
                            .item(item(
                                "thread-archive",
                                if thread.archived {
                                    m::unarchive_thread()
                                } else {
                                    m::archive_thread()
                                },
                                ThreadAction::Archive(!thread.archived),
                            ))
                            .item(item(
                                "thread-title",
                                m::regenerate_title(),
                                ThreadAction::RegenerateTitle,
                            ));
                        let menu = match switch {
                            Some(workspace) => {
                                let id = thread.id.clone();
                                menu.submenu(
                                    "thread-continue-with",
                                    m::continue_with(),
                                    Some(IconName::ArrowRightLeft),
                                    move |menu, _, cx| {
                                        crate::provider_switch::continue_with_menu(
                                            menu, workspace, &id, cx,
                                        )
                                    },
                                )
                            }
                            None => menu,
                        };
                        let menu = if has_draft {
                            menu.item(item(
                                "thread-discard-draft",
                                m::discard_draft(),
                                ThreadAction::DiscardDraft,
                            ))
                        } else {
                            menu
                        };
                        let menu = if can_clean || can_release {
                            let menu = menu.separator();
                            let menu = if can_clean {
                                menu.item(item(
                                    "thread-clean-up",
                                    m::clean_up_thread(),
                                    ThreadAction::CleanUp,
                                ))
                            } else {
                                menu
                            };
                            if can_release {
                                menu.item(item(
                                    "thread-release",
                                    m::release_worktree(),
                                    ThreadAction::Release,
                                ))
                            } else {
                                menu
                            }
                        } else {
                            menu
                        };
                        menu.separator().item(
                            item("thread-delete", m::delete_thread(), ThreadAction::Delete)
                                .destructive(),
                        )
                    })
                    .into_any_element(),
                )
            }
        }
    }

    pub(super) fn setup_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (form, _) = self.setup.as_ref()?;
        let state = form.read(cx);
        let importing = state.importing();
        let project = state.project_title().to_owned();
        let weak = form.downgrade();
        let act =
            |run: fn(&mut WorktreeSetupForm, &mut Window, &mut Context<WorktreeSetupForm>)| {
                let weak = weak.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    weak.update(cx, |form, cx| run(form, window, cx)).ok();
                }
            };
        let dialog = Dialog::new("worktree-setup")
            .open(true)
            .size(DialogSize::Custom(px(512.)))
            .close_button(false)
            .on_open_change(cx.listener(|this, change: &OpenChange, _, cx| {
                if !change.open {
                    this.setup = None;
                    cx.notify();
                }
            }))
            .title(if importing {
                m::worktree_setup_import_title()
            } else {
                m::worktree_setup_title()
            })
            .child(form.clone());
        let dialog = if importing {
            dialog
                .footer(
                    DialogButton::new("worktree-setup-back")
                        .label(m::back())
                        .variant(DialogButtonVariant::Outline)
                        .on_click(act(|form, _, cx| form.back(cx))),
                )
                .footer(
                    DialogButton::new("worktree-setup-import")
                        .label(m::worktree_setup_import())
                        .disabled(!state.can_import(cx))
                        .on_click(act(|form, window, cx| form.import(window, cx))),
                )
        } else {
            let share = form.update(cx, |form, cx| form.share_menu(cx));
            dialog
                .description(m::worktree_setup_description(project))
                .footer(share)
                .footer(
                    DialogButton::new("worktree-setup-cancel")
                        .label(m::cancel())
                        .variant(DialogButtonVariant::Outline)
                        .close(),
                )
                .footer(
                    DialogButton::new("worktree-setup-save")
                        .label(m::save())
                        .disabled(!form.read(cx).can_save(cx))
                        .on_click(act(|form, _, cx| form.save(cx))),
                )
        };
        Some(dialog.into_any_element())
    }
}
