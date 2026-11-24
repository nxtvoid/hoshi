use super::*;
use hoshi_core::ForgeKind;
use rows::{forge_mark, icon_button};

impl ThreadDetails {
    /// The row of the git action under way, of the last one done, or of the one to run next;
    /// `None` when there is nothing to do.
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
    ) -> Option<AnyElement> {
        let draft = self.prefs.draft_pull_requests;
        if let Some(started) = running {
            return Some(
                row_static("details-git-progress", colors)
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
                            .font_weight(FontWeight::NORMAL)
                            .text_color(colors.muted_foreground)
                            .child(crate::format::format_duration(
                                started.elapsed().as_millis() as i64,
                            )),
                    )
                    .into_any_element(),
            );
        }
        if let Some(pending) = self.pending_git.get(thread_id) {
            return Some(self.pending_row(thread_id, pending, colors, cx));
        }
        if let Some(done) = self.done.get(thread_id) {
            let (title, description) = git::done_text(done);
            let url = done
                .pull_request
                .as_ref()
                .map(|pull_request| pull_request.url.clone());
            let base = match url {
                Some(url) => {
                    row("details-git-done", colors).on_click(move |_, _, cx| cx.open_url(&url))
                }
                None => row_static("details-git-done", colors),
            };
            return Some(
                base.child(
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
                .into_any_element(),
            );
        }
        let quick = git::quick_action(status, pull_request_open);
        if !git::offers(&quick) {
            return None;
        }
        let blocker = if working {
            Some(m::git_error_busy())
        } else {
            quick.blocker.map(git::blocker_text)
        };
        let content = |base: Stateful<Div>| {
            base.child(action_icon(quick.label, status.forge, colors))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(git::label_text(quick.label, draft)),
                )
        };
        Some(match blocker {
            Some(blocker) => content(row_static("details-git-quick", colors))
                .opacity(0.6)
                .with_tooltip(
                    Tooltip::new(blocker)
                        .side(Side::Left)
                        .side_offset(px(rows::BESIDE_ROW))
                        .max_width(px(240.)),
                )
                .into_any_element(),
            None => {
                let (action, label) = (quick.action, quick.label);
                content(row("details-git-quick", colors))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let request = this.quick_request(action);
                        this.start_git(request, Some(label), window, cx)
                    }))
                    .into_any_element()
            }
        })
    }

    /// The main action, pressed: a ring that closes as it waits to run, and the button that
    /// calls it off.
    fn pending_row(
        &self,
        thread_id: &str,
        pending: &PendingGit,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let elapsed = pending.started.elapsed().as_secs_f32();
        let total = UNDO_FOR.as_secs_f32();
        let left = (total - elapsed).ceil().max(1.) as u32;
        let hover = colors.foreground.opacity(0.07);
        let thread_id = thread_id.to_owned();
        row_static("details-git-pending", colors)
            .debug_selector(|| "details-git-pending".into())
            .pr_1()
            .child(
                div().relative().flex_none().size_4().child(
                    div()
                        .absolute()
                        .top(px(OVERHANG))
                        .left(px(OVERHANG))
                        .size(px(RING_SIZE))
                        .child(countdown_ring(elapsed / total, colors.muted_foreground))
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(10.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors.foreground)
                                .child(left.to_string()),
                        ),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(git::label_text(pending.label, pending.request.draft)),
            )
            .child(
                div()
                    .id("details-git-undo")
                    .debug_selector(|| "details-git-undo".into())
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(5.))
                    .h(px(22.))
                    .px_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_xs()
                    .text_color(colors.foreground)
                    .bg(colors.foreground.opacity(0.04))
                    .hover(move |style| style.bg(hover))
                    .child(
                        Icon::new(IconName::Undo2)
                            .size(px(13.))
                            .flex_none()
                            .text_color(colors.foreground),
                    )
                    .child(m::undo())
                    .on_click(cx.listener(move |this, _, _, cx| this.undo_git(&thread_id, cx))),
            )
            .into_any_element()
    }

    /// The `⋯` at the end of the branch row: every git action, each with why it can't run.
    pub(super) fn git_menu(
        &self,
        status: &GitStatus,
        pull_request_open: bool,
        working: bool,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let draft = self.prefs.draft_pull_requests;
        let blockers = git::menu_blockers(status);
        let forge = status.forge;
        let this = cx.weak_entity();
        let trigger_colors = colors.clone();
        let menu_colors = colors.clone();
        Popover::new("details-git-menu")
            .anchor(self.branch_anchor.clone())
            .align(Align::End)
            .match_trigger_width()
            .trigger(move |open| {
                icon_button(
                    "details-git-arrow",
                    icon(IconName::Ellipsis, &trigger_colors),
                    open,
                    &trigger_colors,
                )
                .debug_selector(|| "details-git-arrow".into())
            })
            .menu(move |menu, _, _| {
                let action = |action: GitAction| {
                    let this = this.clone();
                    move |window: &mut Window, cx: &mut App| {
                        this.update(cx, |this, cx| {
                            let request = this.quick_request(action);
                            this.start_git(request, None, window, cx)
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
                        .when_some(forge, |option, forge| {
                            option.leading(div().mr_1().child(forge_mark(forge, &menu_colors)))
                        })
                        .when(forge.is_none(), |option| option.icon(IconName::CloudUpload))
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
                        .when_some(forge, |option, forge| {
                            option.leading(div().mr_1().child(forge_mark(forge, &menu_colors)))
                        })
                        .when(forge.is_none(), |option| {
                            option.icon(IconName::GitPullRequest)
                        })
                        .disabled(working || blockers.create_pull_request.is_some())
                        .when_some(blockers.create_pull_request, |option, blocker| {
                            option.tooltip(blocker_tooltip(blocker))
                        })
                        .on_select(action(GitAction::CreatePullRequest)),
                    )
                })
                .separator()
                .item(
                    MenuOption::checkbox("details-git-draft", m::git_draft_pull_requests(), draft)
                        .on_select(move |_, cx| {
                            toggle
                                .update(cx, |this, cx| this.set_draft(!draft, cx))
                                .ok();
                        }),
                )
            })
            .into_any_element()
    }
}

const RING_SIZE: f32 = 18.;
/// How far the ring reaches past the 16px of an icon on each side, so the text stays put.
const OVERHANG: f32 = (16. - RING_SIZE) / 2.;

/// A circle that closes clockwise from the top as `fraction` goes to 1, over a faint track.
fn countdown_ring(fraction: f32, color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let stroke = px(1.5);
            let radius = bounds.size.width.min(bounds.size.height) / 2. - stroke / 2.;
            let center = bounds.center();
            if let Some(track) = crate::composer::arc(center, radius, stroke, 1.) {
                window.paint_path(track, color.opacity(0.25));
            }
            if fraction > 0.
                && let Some(filled) = crate::composer::arc(center, radius, stroke, fraction.min(1.))
            {
                window.paint_path(filled, color);
            }
        },
    )
    .size_full()
}

/// What an action's row leads with: the forge it sends to, else the action's icon.
fn action_icon(label: git::Label, forge: Option<ForgeKind>, colors: &Colors) -> AnyElement {
    match forge.filter(|_| git::reaches_forge(label)) {
        Some(forge) => forge_mark(forge, colors),
        None => icon(git::label_icon(label), colors).into_any_element(),
    }
}
