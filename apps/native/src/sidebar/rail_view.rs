use super::*;

impl Sidebar {
    /// The ring with this id, kept from the rail's last draw in `before` when it had one.
    pub(super) fn ring(
        &mut self,
        before: &mut HashMap<SharedString, Entity<SignalRing>>,
        id: impl Into<SharedString>,
        signal: Signal,
        cx: &mut App,
    ) -> AnyElement {
        let id: SharedString = id.into();
        let ring = before
            .remove(&id)
            .unwrap_or_else(|| cx.new(|_| SignalRing { signal }));
        ring.update(cx, |ring, _| ring.signal = signal);
        self.rings.insert(id, ring.clone());
        ring.into_any_element()
    }

    pub(super) fn icon(&mut self, project_id: &str, cx: &mut Context<Self>) -> Option<PathBuf> {
        if let Some(found) = self.icons.get(project_id) {
            return found.clone();
        }
        self.icons.insert(project_id.to_owned(), None);
        let engine = self.workspace.read(cx).engine.clone();
        let id = project_id.to_owned();
        cx.spawn(async move |this, cx| {
            let lookup = id.clone();
            let found = cx
                .background_spawn(async move { engine.project_icon_path(&lookup) })
                .await;
            if found.is_some() {
                this.update(cx, |this, cx| {
                    this.icons.insert(id, found);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
        None
    }

    pub(super) fn shows_rail(&self, cx: &App) -> bool {
        let model = &self.workspace.read(cx).model;
        model.projects().next().is_some()
            || model.threads().any(|thread| thread.project_id.is_none())
    }

    pub(super) fn rail(&mut self, colors: &Colors, cx: &mut Context<Self>) -> AnyElement {
        let mut rings = std::mem::take(&mut self.rings);
        let projects = self.ordered_projects(cx);
        let workspace = self.workspace.read(cx);
        let threads: Vec<Thread> = workspace.listed_threads();
        let has_chats = threads.iter().any(|thread| thread.project_id.is_none());
        if projects.is_empty() && !has_chats {
            return div().into_any_element();
        }
        let unseen = workspace.unseen.clone();
        let inbox = inbox_sections(
            sections::with_delegated_work(&threads),
            &workspace.pull_requests,
            &workspace.model,
            Utc::now(),
            &unseen,
            &workspace.logins,
        );
        let inbox_count = sections::inbox_count(&inbox);
        let inbox_waiting = inbox
            .iter()
            .any(|section| section.id == SectionId::Attention);
        let open_project = self
            .shown_thread
            .as_deref()
            .and_then(|id| workspace.model.thread(id)?.project_id.clone());
        let view = self.prefs.view;
        let now = Utc::now();
        let awake: Vec<Thread> = threads
            .iter()
            .filter(|thread| !is_snoozed(&workspace.model, thread, now))
            .cloned()
            .collect();

        let inbox_label = if inbox_count > 0 {
            m::inbox_waiting(inbox_count)
        } else {
            m::inbox()
        };
        let inbox_signal = (inbox_count > 0).then_some(if inbox_waiting {
            Signal::Waiting
        } else {
            Signal::Done
        });
        let inbox_ring =
            inbox_signal.map(|signal| self.ring(&mut rings, "signal-inbox", signal, cx));
        let inbox_button = rail_button("rail-inbox", colors)
            .when(view == View::Inbox, |this| selected(this, colors))
            .when_some(inbox_signal, |this, signal| {
                this.border_color(signal_color(signal, colors))
            })
            .on_click(cx.listener(|this, _, _, cx| {
                let view = if this.prefs.view == View::Inbox {
                    View::Project
                } else {
                    View::Inbox
                };
                this.set_view(view, cx)
            }))
            .children(inbox_ring)
            .child(Icon::new(IconName::Inbox).size_4())
            .when(inbox_count > 0, |this| {
                this.child(count_badge(inbox_count, colors))
            })
            .with_tooltip(rail_tooltip(Tooltip::new(inbox_label)));

        let chats_signal = project_signal(&awake, None, &unseen);
        let chats_ring =
            chats_signal.map(|signal| self.ring(&mut rings, "signal-chats", signal, cx));
        let chats_button = rail_button("rail-chats", colors)
            .when(view == View::Chats, |this| selected(this, colors))
            .when_some(chats_signal, |this, signal| {
                this.border_color(signal_color(signal, colors))
            })
            .on_click(cx.listener(|this, _, _, cx| {
                if this.prefs.view == View::Chats {
                    this.set_view(View::Project, cx)
                } else {
                    this.open_new_draft(None, cx)
                }
            }))
            .children(chats_ring)
            .child(Icon::new(IconName::MessageCircle).size_4())
            .with_tooltip(rail_tooltip(Tooltip::new(m::chats())));

        let in_project = view == View::Project;
        let buttons: Vec<AnyElement> = projects
            .into_iter()
            .map(|project| {
                let signal = project_signal(&awake, Some(&project.id), &unseen);
                let ring = signal.map(|signal| {
                    self.ring(&mut rings, format!("signal-{}", project.id), signal, cx)
                });
                let icon = self.icon(&project.id, cx);
                let chosen = in_project && self.project_id.as_deref() == Some(project.id.as_str());
                let holds_open =
                    !in_project && open_project.as_deref() == Some(project.id.as_str());
                let initials = initials(&project.title);
                let (title, root) = (project.title.clone(), project.workspace_root.clone());
                let status = signal.map(signal_label);
                let id = project.id.clone();
                let drag = ProjectDrag {
                    id: project.id.clone(),
                    initials: initials.clone(),
                    icon: icon.clone(),
                };
                let (dropped_on, menu_for) = (project.id.clone(), project.id.clone());
                let selector = format!("rail-{}", project.id);
                rail_button(SharedString::from(selector.clone()), colors)
                    .debug_selector(move || selector.clone())
                    .text_size(rems(0.6875))
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(chosen, |this| selected(this, colors))
                    .when(holds_open, |this| {
                        this.bg(colors.sidebar_accent.opacity(0.6))
                    })
                    .when_some(signal, |this, signal| {
                        this.border_color(signal_color(signal, colors))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_project(id.clone(), window, cx)
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.open_menu(
                                MenuTarget::Project(menu_for.clone()),
                                event.position,
                                cx,
                            )
                        }),
                    )
                    .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
                    .drag_over::<ProjectDrag>({
                        let ring = colors.sidebar_ring;
                        move |style, _, _, _| style.border_color(ring)
                    })
                    .on_drop(cx.listener(move |this, drag: &ProjectDrag, _, cx| {
                        this.move_project(&drag.id, &dropped_on, cx)
                    }))
                    .children(ring)
                    .child(project_mark(icon, initials, px(20.), px(5.)))
                    .with_tooltip(rail_tooltip(Tooltip::element(move |_, _| {
                        div()
                            .flex()
                            .flex_col()
                            .child(div().font_weight(FontWeight::MEDIUM).child(title.clone()))
                            .child(div().opacity(0.7).child(root.clone()))
                            .children(status.clone())
                    })))
                    .into_any_element()
            })
            .collect();
        let has_projects = !buttons.is_empty();

        div()
            .id("project-rail")
            .track_scroll(&self.rail_scroll)
            .w_12()
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.))
            .py_2()
            .border_r_1()
            .border_color(colors.sidebar_border)
            .overflow_y_scroll()
            .child(inbox_button)
            .child(chats_button)
            .when(has_projects, |this| {
                this.child(
                    div()
                        .flex_none()
                        .my(px(2.))
                        .h_px()
                        .w_6()
                        .bg(colors.sidebar_border),
                )
            })
            .children(buttons)
            .into_any_element()
    }
}
