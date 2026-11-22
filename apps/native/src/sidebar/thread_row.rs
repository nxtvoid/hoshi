use super::*;

impl Sidebar {
    pub(super) fn row(
        &mut self,
        thread: Thread,
        rows: &RowContext,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = rows.open.as_deref() == Some(thread.id.as_str());
        let status = match thread.status {
            ThreadStatus::Working => Some((m::status_working(), colors.status_working)),
            ThreadStatus::WaitingForInput => {
                Some((m::status_waiting_for_input(), colors.status_waiting))
            }
            ThreadStatus::Error => Some((m::status_error(), colors.status_error)),
            ThreadStatus::Idle if rows.unseen.contains(&thread.id) => {
                Some((m::status_completed(), colors.status_done))
            }
            ThreadStatus::Idle => None,
        };
        let project = rows.show_project.then(|| match &thread.project_id {
            None => m::chat(),
            Some(id) => rows.project_titles.get(id).cloned().unwrap_or_default(),
        });
        let pull_request = rows.pull_requests.get(&thread.id).cloned();
        let root_branch = thread
            .project_id
            .as_ref()
            .and_then(|id| rows.root_branches.get(id).cloned().flatten());
        let mismatch = branch_mismatch(&thread, root_branch.as_deref());
        let title = if thread.title.is_empty() {
            m::untitled_thread()
        } else {
            thread.title.clone()
        };
        let second_line = thread.branch.is_some() || project.is_some() || pull_request.is_some();
        let mono = cx.theme().mono_font_family.clone();
        let (clicked, menu_for, hovered) =
            (thread.id.clone(), thread.id.clone(), thread.id.clone());
        let draft = !selected && rows.drafts.contains(&thread.id);
        let snoozed = rows
            .snoozed
            .get(&thread.id)
            .map(|until| crate::snooze::left_text(snooze_left(until, Utc::now())));
        let woke = snoozed.is_none() && status.is_none() && rows.woke.contains(&thread.id);
        let group = SharedString::from(format!("thread-row-{}", thread.id));
        let hover_action = (!thread.archived && !woke)
            .then(|| self.row_snooze_button(&thread.id, snoozed.is_some(), colors, cx));
        let trailing = match (snoozed, status) {
            (Some(left), _) => div()
                .flex_none()
                .flex()
                .items_center()
                .gap_1()
                .text_xs()
                .text_color(colors.status_working)
                .child(Icon::new(IconName::AlarmClock).size_3())
                .child(left)
                .into_any_element(),
            (None, Some((label, color))) => div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_xs()
                .text_color(color)
                .child(div().size(px(6.)).rounded_full().bg(color))
                .child(label)
                .into_any_element(),
            (None, None) if woke => self.woke_chip(&thread.id, colors, cx),
            (None, None) if draft => div()
                .flex_none()
                .text_xs()
                .italic()
                .text_color(colors.draft)
                .child(m::draft())
                .into_any_element(),
            (None, None) => div()
                .flex_none()
                .text_xs()
                .text_color(colors.muted_foreground)
                .child(relative_time(&thread.updated_at, Utc::now()))
                .into_any_element(),
        };
        let trailing = div()
            .flex_none()
            .when(hover_action.is_some(), |this| {
                this.group_hover(group.clone(), |style| style.opacity(0.))
            })
            .child(trailing);

        let selector = format!("thread-{}", thread.id);
        let row = div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .group(group.clone())
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px_2()
            .py(px(6.))
            .rounded_md()
            .cursor_pointer()
            .text_color(colors.sidebar_foreground)
            .hover(|this| this.bg(colors.sidebar_accent.opacity(0.6)))
            .when(selected, |this| {
                this.bg(colors.sidebar_accent)
                    .text_color(colors.sidebar_accent_foreground)
            })
            .on_hover(
                cx.listener(move |this, over: &bool, _, cx| this.hover_row(&hovered, *over, cx)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.quiet_card(&clicked, cx);
                this.open_thread(&clicked, cx)
            }))
            .child(anchor(&self.cards, CardTarget::Thread(thread.id.clone())))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.open_menu(MenuTarget::Thread(menu_for.clone()), event.position, cx)
                }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .child(
                        svg()
                            .path(crate::composer::provider_mark(&thread.provider))
                            .size(px(14.))
                            .flex_none()
                            .text_color(crate::composer::provider_tint(&thread.provider, colors)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .when(thread.archived, |this| {
                                this.text_color(colors.muted_foreground)
                            })
                            .child(title),
                    )
                    .when(thread.forked_from.is_some(), |this| {
                        let selector = format!("fork-mark-{}", thread.id);
                        this.child(
                            div()
                                .debug_selector(move || selector.clone())
                                .flex_none()
                                .text_color(colors.muted_foreground)
                                .child(Icon::new(IconName::GitFork).size_3()),
                        )
                    })
                    .child(trailing),
            )
            .when_some(hover_action, |this, action| {
                this.child(
                    div()
                        .absolute()
                        .top(px(4.))
                        .right_1()
                        .opacity(0.)
                        .group_hover(group, |style| style.opacity(1.))
                        .child(action),
                )
            })
            .when(second_line, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .min_w_0()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .when_some(project, |this, project| {
                            this.child(
                                div()
                                    .flex_none()
                                    .max_w(relative(0.45))
                                    .truncate()
                                    .text_color(colors.sidebar_foreground.opacity(0.8))
                                    .child(project),
                            )
                        })
                        .when_some(thread.branch.clone(), |this, branch| {
                            this.child(
                                Icon::new(if thread.worktree_path.is_some() {
                                    IconName::GitFork
                                } else {
                                    IconName::GitBranch
                                })
                                .size_3()
                                .flex_none()
                                .when(thread.worktree_released, |icon| icon.opacity(0.4)),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(rems(0.6875))
                                    .font_family(mono.clone())
                                    .child(branch),
                            )
                        })
                        .when_some(pull_request, |this, pull_request| {
                            this.child(self.pull_request_pill(
                                &thread.id,
                                &pull_request,
                                colors,
                                cx,
                            ))
                        }),
                )
            })
            .when(mismatch, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .min_w_0()
                        .text_xs()
                        .text_color(colors.warning)
                        .child(Icon::new(IconName::CircleAlert).size_3().flex_none())
                        .child(div().truncate().child(m::root_on_other_branch())),
                )
            });

        row.into_any_element()
    }

    /// On a hovered row: wake it when snoozed, or open its snooze choices.
    pub(super) fn row_snooze_button(
        &self,
        thread_id: &str,
        snoozed: bool,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = thread_id.to_owned();
        let button = Button::new(SharedString::from(format!("snooze-row-{thread_id}")))
            .size(ButtonSize::IconXs)
            .variant(ButtonVariant::Ghost);
        let button =
            if snoozed {
                button
                    .icon(IconName::AlarmClockOff)
                    .text_color(colors.status_working)
                    .tooltip(
                        Tooltip::new(m::wake_thread_now())
                            .side(Side::Right)
                            .side_offset(px(15.)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.wake_thread(id.clone(), window, cx)
                    }))
            } else {
                button
                    .icon(IconName::Clock)
                    .tooltip(
                        Tooltip::new(m::snooze_thread())
                            .side(Side::Right)
                            .side_offset(px(15.)),
                    )
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        this.open_menu(MenuTarget::Snooze(id.clone()), event.position(), cx)
                    }))
            };
        let selector = format!("snooze-row-{thread_id}");
        div()
            .debug_selector(move || selector.clone())
            .child(button)
            .into_any_element()
    }

    /// "Woke": the thread woke from its snooze since the user saw it; a click puts it away.
    pub(super) fn woke_chip(
        &self,
        thread_id: &str,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = thread_id.to_owned();
        let selector = format!("woke-{thread_id}");
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .flex_none()
            .px_1p5()
            .rounded(px(4.))
            .bg(colors.warning.opacity(0.12))
            .text_xs()
            .text_color(colors.warning)
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.dismiss_woke(id.clone(), cx)
            }))
            .child(m::woke_chip())
            .with_tooltip(Tooltip::new(m::dismiss_woke()).side(Side::Right))
            .into_any_element()
    }

    pub(super) fn pull_request_pill(
        &self,
        thread_id: &str,
        pull_request: &PullRequest,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let (icon, color) = pull_request_mark(pull_request, colors);
        let failing = pull_request.checks == Some(ChecksState::Failing);
        let comments = pull_request.unresolved_comments;
        let (id, hovered) = (thread_id.to_owned(), thread_id.to_owned());
        div()
            .id(SharedString::from(format!("pull-request-{thread_id}")))
            .relative()
            .ml_auto()
            .mr(px(-4.))
            .my(px(-2.))
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .px_1()
            .py(px(2.))
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|this| this.bg(colors.sidebar_accent).text_color(colors.foreground))
            .on_hover(
                cx.listener(move |this, over: &bool, _, cx| this.hover_pill(&hovered, *over, cx)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.quiet_card(&id, cx);
                this.open_thread(&id, cx);
                cx.emit(SidebarEvent::OpenPullRequest(id.clone()));
            }))
            .child(anchor(
                &self.cards,
                CardTarget::PullRequest(thread_id.to_owned()),
            ))
            .child(Icon::new(icon).size_3().text_color(color))
            .child(format!("#{}", pull_request.number))
            .when(failing, |this| {
                this.child(div().size(px(6.)).rounded_full().bg(colors.status_error))
            })
            .when(comments > 0, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .child(Icon::new(IconName::MessageSquare).size_3())
                        .child(comments.to_string()),
                )
            })
    }
}
