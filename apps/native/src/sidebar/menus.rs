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
                let thread_id = thread_id.clone();
                let sidebar = cx.entity();
                self.workspace.read(cx).model.thread(&thread_id)?;
                Some(
                    context_menu(id, open.position, on_close, move |menu, _, cx| {
                        thread_menu(menu, &sidebar, &thread_id, cx)
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

/// What can be done with a thread.
pub(crate) fn thread_menu(
    menu: PopoverMenu,
    sidebar: &Entity<Sidebar>,
    thread_id: &str,
    cx: &App,
) -> PopoverMenu {
    let state = sidebar.read(cx);
    let this = sidebar.downgrade();
    let workspace = state.workspace.read(cx);
    let Some(thread) = workspace.model.thread(thread_id).cloned() else {
        return menu;
    };
    let has_draft = workspace.drafts.contains_key(thread_id)
        && state.shown_thread.as_deref() != Some(thread_id);
    let can_clean = merged_cleanup(&thread, workspace.pull_requests.get(thread_id)).is_some();
    let can_release =
        thread.worktree_path.is_some() && !thread.worktree_released && !thread.worktree_adopted;
    let snoozed = is_snoozed(&workspace.model, &thread, Utc::now());
    let presets = workspace.settings.general.snooze_presets.clone();
    let item = |key: &'static str, label: String, icon: IconName, action: ThreadAction| {
        let (this, id) = (this.clone(), thread.id.clone());
        MenuOption::new(key, label)
            .icon(icon)
            .on_select(move |window, cx| {
                this.update(cx, |this, cx| {
                    this.thread_action(id.clone(), action, window, cx)
                })
                .ok();
            })
    };
    let (pin, pin_icon) = if thread.pinned {
        (m::unpin_thread(), IconName::PinOff)
    } else {
        (m::pin_thread(), IconName::Pin)
    };
    let (archive, archive_icon) = if thread.archived {
        (m::unarchive_thread(), IconName::ArchiveRestore)
    } else {
        (m::archive_thread(), IconName::Archive)
    };
    let menu = menu.item(item(
        "thread-pin",
        pin,
        pin_icon,
        ThreadAction::Pin(!thread.pinned),
    ));
    let menu = if thread.archived {
        menu
    } else if snoozed {
        menu.item(item(
            "thread-wake",
            m::wake_thread(),
            IconName::AlarmClockOff,
            ThreadAction::Wake,
        ))
    } else {
        let (this, id) = (this.clone(), thread.id.clone());
        menu.submenu(
            "thread-snooze",
            m::snooze_menu(),
            Some(IconName::AlarmClock),
            move |menu, _, _| snooze_menu(menu, this, id, &presets),
        )
    };
    menu.item(item(
        "thread-fork",
        m::fork_thread(),
        IconName::GitFork,
        ThreadAction::Fork,
    ))
    .item(item(
        "thread-archive",
        archive,
        archive_icon,
        ThreadAction::Archive(!thread.archived),
    ))
    .item(item(
        "thread-title",
        m::regenerate_title(),
        IconName::Sparkles,
        ThreadAction::RegenerateTitle,
    ))
    .when(has_draft, |menu| {
        menu.item(item(
            "thread-discard-draft",
            m::discard_draft(),
            IconName::X,
            ThreadAction::DiscardDraft,
        ))
    })
    .when(can_clean || can_release, |menu| menu.separator())
    .when(can_clean, |menu| {
        menu.item(item(
            "thread-clean-up",
            m::clean_up_thread(),
            IconName::BrushCleaning,
            ThreadAction::CleanUp,
        ))
    })
    .when(can_release, |menu| {
        menu.item(item(
            "thread-release",
            m::release_worktree(),
            IconName::FolderX,
            ThreadAction::Release,
        ))
    })
    .separator()
    .item(
        item(
            "thread-delete",
            m::delete_thread(),
            IconName::Trash,
            ThreadAction::Delete,
        )
        .destructive(),
    )
}
