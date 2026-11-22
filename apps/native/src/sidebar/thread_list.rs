use super::*;

impl Sidebar {
    pub(super) fn list(&mut self, colors: &Colors, cx: &mut Context<Self>) -> AnyElement {
        let search = self.search.read(cx).value().to_string();
        let view = self.prefs.view;
        let workspace = self.workspace.read(cx);
        let has_projects = workspace.model.projects().next().is_some();
        if !has_projects && view != View::Chats {
            return div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .gap_3()
                .p_4()
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(m::no_projects()),
                )
                .child(
                    Button::new("add-first-project")
                        .label(m::add_project())
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(SidebarEvent::OpenPalette(palette::Mode::AddProject))
                        })),
                )
                .into_any_element();
        }
        let project = self
            .project_id
            .as_deref()
            .and_then(|id| workspace.model.project(id))
            .cloned();
        if project.is_none() && view == View::Project {
            return empty_message(m::no_project_selected(), colors);
        }

        let project_titles: HashMap<String, String> = workspace
            .model
            .projects()
            .map(|project| (project.id.clone(), project.title.clone()))
            .collect();
        let now = Utc::now();
        let snoozed: HashMap<String, String> = workspace
            .model
            .threads()
            .filter(|thread| is_snoozed(&workspace.model, thread, now))
            .filter_map(|thread| Some((thread.id.clone(), thread.snoozed_until.clone()?)))
            .collect();
        let woke: HashSet<String> = workspace
            .model
            .threads()
            .filter(|thread| shows_woke(&workspace.model, thread, now))
            .map(|thread| thread.id.clone())
            .collect();
        let rows = RowContext {
            open: self.shown_thread.clone(),
            snoozed,
            woke,
            show_project: view == View::Inbox,
            project_titles,
            pull_requests: workspace.pull_requests.clone(),
            unseen: workspace.unseen.clone(),
            root_branches: workspace.project_branches.clone(),
            drafts: workspace.drafts.keys().cloned().collect(),
        };
        let lone_project = (rows.project_titles.len() == 1)
            .then(|| {
                rows.project_titles
                    .iter()
                    .next()
                    .map(|(id, title)| (id.clone(), title.clone()))
            })
            .flatten();
        let sections = self.current_sections(cx);

        let heading = match view {
            View::Inbox => self.view_heading(IconName::Inbox, m::inbox(), lone_project, colors, cx),
            View::Chats => self.view_heading(
                IconName::MessageCircle,
                m::chats(),
                lone_project,
                colors,
                cx,
            ),
            View::Project => self.project_heading(project.as_ref(), colors, cx),
        };

        let inbox = view == View::Inbox;
        let narrowed = !search.is_empty() || self.prefs.filter != Filter::All;
        let only_active = !inbox && sections.len() == 1 && sections[0].id == SectionId::Active;
        let body = if sections.is_empty() {
            empty_message(
                if narrowed {
                    m::no_matching_threads()
                } else if inbox {
                    m::inbox_empty()
                } else if view == View::Chats {
                    m::no_chats()
                } else {
                    m::no_threads()
                },
                colors,
            )
        } else {
            let force_open = !search.is_empty();
            div()
                .id("thread-sections")
                .track_scroll(&self.list_scroll)
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .gap_1()
                .px(px(6.))
                .pb_2()
                .overflow_y_scroll()
                .children(sections.into_iter().map(|section| {
                    self.section(section, !only_active, force_open, &rows, colors, cx)
                }))
                .into_any_element()
        };

        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .pt(px(6.))
            .child(heading)
            .children(self.new_thread_drafts(colors, cx))
            .child(body)
            .into_any_element()
    }

    /// New threads with unsent text: this project's or the chats' one, or every one in the inbox.
    /// The draft on screen is left out.
    pub(super) fn new_thread_drafts(
        &mut self,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let workspace = self.workspace.read(cx);
        let shown = workspace
            .open
            .is_none()
            .then(|| workspace.draft_key(cx))
            .flatten();
        let mut keys: Vec<String> = match self.prefs.view {
            View::Inbox => workspace
                .drafts
                .keys()
                .filter(|key| drafts::new_thread_project(key).is_some())
                .cloned()
                .collect(),
            View::Chats => vec![drafts::key(None, None)],
            View::Project => vec![drafts::key(None, self.project_id.as_deref())],
        };
        keys.sort();
        let rows: Vec<(String, Option<Project>)> = keys
            .into_iter()
            .filter(|key| workspace.drafts.contains_key(key) && Some(key) != shown.as_ref())
            .filter_map(|key| match drafts::new_thread_project(&key)? {
                None => Some((key, None)),
                Some(project_id) => {
                    let project = workspace.model.project(project_id)?.clone();
                    Some((key, Some(project)))
                }
            })
            .collect();
        if rows.is_empty() {
            return None;
        }
        let inbox = self.prefs.view == View::Inbox;
        let rows: Vec<AnyElement> = rows
            .into_iter()
            .map(|(key, project)| {
                let label = match &project {
                    None => m::new_chat(),
                    Some(project) if inbox => project.title.clone(),
                    Some(_) => m::new_thread(),
                };
                let group = SharedString::from(format!("draft-{key}"));
                let (opening, menu_for, discarding) = (project.clone(), key.clone(), key.clone());
                div()
                    .id(SharedString::from(format!("draft-row-{key}")))
                    .debug_selector({
                        let key = key.clone();
                        move || format!("draft-row-{key}")
                    })
                    .group(group.clone())
                    .relative()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py(px(6.))
                    .rounded_md()
                    .cursor_pointer()
                    .bg(colors.draft_surface)
                    .hover(|this| this.bg(colors.draft.opacity(0.1)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_new_draft(opening.as_ref().map(|project| project.id.clone()), cx)
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.open_menu(MenuTarget::Draft(menu_for.clone()), event.position, cx)
                        }),
                    )
                    .child(
                        Icon::new(IconName::SquarePen)
                            .size(px(14.))
                            .flex_none()
                            .text_color(colors.draft),
                    )
                    .child(div().flex_1().min_w_0().truncate().text_sm().child(label))
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .italic()
                            .text_color(colors.draft)
                            .group_hover(group.clone(), |style| style.opacity(0.))
                            .child(m::draft()),
                    )
                    .child(
                        div()
                            .absolute()
                            .right_1()
                            .opacity(0.)
                            .group_hover(group, |style| style.opacity(1.))
                            .child(
                                Button::new(SharedString::from(format!("discard-{key}")))
                                    .icon(IconName::X)
                                    .size(ButtonSize::IconXs)
                                    .variant(ButtonVariant::Ghost)
                                    .tooltip(
                                        Tooltip::new(m::discard_draft())
                                            .side(Side::Right)
                                            .side_offset(px(15.)),
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let key = discarding.clone();
                                        this.workspace.update(cx, |workspace, cx| {
                                            workspace.forget_drafts(&[key], cx)
                                        });
                                    })),
                            ),
                    )
                    .into_any_element()
            })
            .collect();
        Some(
            div()
                .flex()
                .flex_col()
                .gap_px()
                .px(px(6.))
                .children(rows)
                .child(div().mx_2().my(px(6.)).h_px().bg(colors.sidebar_border))
                .into_any_element(),
        )
    }

    pub(super) fn section(
        &mut self,
        section: Section,
        show_header: bool,
        force_open: bool,
        rows: &RowContext,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = section.id;
        let open = force_open || !self.prefs.collapsed.contains(&id);
        let count = section.threads.len();
        let merged: Vec<String> = if id == SectionId::Done {
            section
                .threads
                .iter()
                .filter(|thread| {
                    thread.status != ThreadStatus::Working
                        && merged_cleanup(thread, rows.pull_requests.get(&thread.id)).is_some()
                })
                .map(|thread| thread.id.clone())
                .collect()
        } else {
            Vec::new()
        };
        let shown: Vec<Thread> = section
            .threads
            .into_iter()
            .filter(|thread| open || rows.open.as_deref() == Some(thread.id.as_str()))
            .collect();
        div()
            .flex()
            .flex_col()
            .when(show_header, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .id(SharedString::from(format!("section-{id:?}")))
                                .debug_selector(move || format!("section-{id:?}"))
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap_1()
                                .px_2()
                                .pt_2()
                                .pb_1()
                                .rounded_md()
                                .text_size(rems(0.6875))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors.muted_foreground)
                                .when(!force_open, |this| {
                                    this.cursor_pointer()
                                        .hover(|this| this.text_color(colors.sidebar_foreground))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.toggle_section(id, cx)
                                        }))
                                })
                                .child(
                                    Icon::new(if open {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronRight
                                    })
                                    .size_3(),
                                )
                                .child(section_label(id).to_uppercase())
                                .child(
                                    div()
                                        .ml_auto()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(count.to_string()),
                                ),
                        )
                        .when(!merged.is_empty(), |this| {
                            let label = m::cleanup_merged(merged.len());
                            let running = self.cleaning_merged;
                            this.child(
                                div()
                                    .id("clean-up-merged")
                                    .mt_1()
                                    .mr_1()
                                    .size_5()
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.))
                                    .text_color(colors.muted_foreground)
                                    .when(running, |this| this.opacity(0.5))
                                    .when(!running, |this| {
                                        this.cursor_pointer()
                                            .hover(|this| {
                                                this.bg(colors.sidebar_accent)
                                                    .text_color(colors.sidebar_foreground)
                                            })
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.clean_up_merged(merged.clone(), window, cx)
                                            }))
                                    })
                                    .child(Icon::new(IconName::Brush).size(px(14.)))
                                    .with_tooltip(Tooltip::new(label).side(Side::Right)),
                            )
                        }),
                )
            })
            .child(
                div().flex().flex_col().gap_px().children(
                    shown
                        .into_iter()
                        .map(|thread| self.row(thread, rows, colors, cx)),
                ),
            )
            .into_any_element()
    }
}
