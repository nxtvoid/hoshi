use super::*;

impl ThreadDetails {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn git_row(
        &self,
        thread_id: &str,
        status: &GitStatus,
        pull_request_open: bool,
        working: bool,
        running: Option<Instant>,
        phase: Option<hoshi_core::GitActionPhase>,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let draft = self.prefs.draft_pull_requests;
        let primary = if let Some(started) = running {
            div()
                .id("details-git-progress")
                .flex()
                .flex_1()
                .h_8()
                .min_w_0()
                .items_center()
                .gap(px(10.))
                .px(px(10.))
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors.foreground.opacity(0.8))
                .child(
                    Spinner::new()
                        .icon(IconName::LoaderCircle)
                        .color(colors.muted_foreground)
                        .with_size(px(16.)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(git::phase_text(phase)),
                )
                .child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(crate::format::format_duration(
                            started.elapsed().as_millis() as i64,
                        )),
                )
                .into_any_element()
        } else if let Some(done) = self.done.get(thread_id) {
            let (title, description) = git::done_text(done);
            let url = done
                .pull_request
                .as_ref()
                .map(|pull_request| pull_request.url.clone());
            split_primary("details-git-done", colors)
                .when(url.is_none(), |this| this.cursor_default())
                .child(
                    Icon::new(IconName::CircleCheck)
                        .size_4()
                        .flex_none()
                        .text_color(colors.status_done),
                )
                .child(div().truncate().child(title))
                .when_some(description, |this, description| {
                    this.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(colors.muted_foreground)
                            .child(description),
                    )
                })
                .when_some(url, |this, url| {
                    this.on_click(move |_, _, cx| cx.open_url(&url))
                })
                .into_any_element()
        } else {
            let quick = git::quick_action(status, pull_request_open);
            let blocker = if working {
                Some(m::git_error_busy())
            } else {
                quick.blocker.map(git::blocker_text)
            };
            let button = split_primary("details-git-quick", colors)
                .child(icon(git::label_icon(quick.label), colors))
                .child(div().truncate().child(git::label_text(quick.label, draft)));
            match blocker {
                Some(blocker) => button
                    .opacity(0.6)
                    .cursor_default()
                    .with_tooltip(Tooltip::new(blocker).side(Side::Left).max_width(px(240.)))
                    .into_any_element(),
                None => {
                    let action = quick.action;
                    button
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let request = this.quick_request(action);
                            this.start_git(request, window, cx)
                        }))
                        .into_any_element()
                }
            }
        };
        let menu = running.is_none().then(|| {
            let blockers = git::menu_blockers(status);
            let this = cx.weak_entity();
            let arrow_colors = colors.clone();
            Popover::new("details-git-menu")
                .align(Align::End)
                .trigger(move |open| {
                    split_arrow("details-git-arrow", open, &arrow_colors)
                        .debug_selector(|| "details-git-arrow".into())
                })
                .menu(move |menu, _, _| {
                    let action = |action: GitAction| {
                        let this = this.clone();
                        move |window: &mut Window, cx: &mut App| {
                            this.update(cx, |this, cx| {
                                let request = this.quick_request(action);
                                this.start_git(request, window, cx)
                            })
                            .ok();
                        }
                    };
                    let commit = this.clone();
                    let toggle = this.clone();
                    menu.item(
                        MenuOption::new("details-git-commit", m::git_commit_menu())
                            .icon(IconName::GitCommitHorizontal)
                            .disabled(working || blockers.commit.is_some())
                            .when_some(blockers.commit, |option, blocker| {
                                option.tooltip(blocker_tooltip(blocker))
                            })
                            .on_select(move |window, cx| {
                                commit
                                    .update(cx, |this, cx| this.open_commit_dialog(window, cx))
                                    .ok();
                            }),
                    )
                    .item(
                        MenuOption::new("details-git-push", m::git_push())
                            .icon(IconName::CloudUpload)
                            .disabled(working || blockers.push.is_some())
                            .when_some(blockers.push, |option, blocker| {
                                option.tooltip(blocker_tooltip(blocker))
                            })
                            .on_select(action(GitAction::Push)),
                    )
                    .when(!pull_request_open, |menu| {
                        menu.item(
                            MenuOption::new(
                                "details-git-pr",
                                if draft {
                                    m::git_create_draft_pr()
                                } else {
                                    m::git_create_pr()
                                },
                            )
                            .icon(IconName::GitPullRequest)
                            .disabled(working || blockers.create_pull_request.is_some())
                            .when_some(blockers.create_pull_request, |option, blocker| {
                                option.tooltip(blocker_tooltip(blocker))
                            })
                            .on_select(action(GitAction::CreatePullRequest)),
                        )
                    })
                    .separator()
                    .item(
                        MenuOption::checkbox(
                            "details-git-draft",
                            m::git_draft_pull_requests(),
                            draft,
                        )
                        .on_select(move |_, cx| {
                            toggle
                                .update(cx, |this, cx| this.set_draft(!draft, cx))
                                .ok();
                        }),
                    )
                })
                .into_any_element()
        });
        split_row("details-git", primary, menu, colors)
    }
}
