use super::actions::{
    MergeBlocker, changes_requested_warning, checkout_command, merge_blocker, merge_method_label,
    merge_question, shows_merge, state_actions,
};
use super::{PullRequestSurface, state_look};
use crate::m;
use crate::sections::relative_time;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::{
    ChecksState, MergeMethod, PullRequest, PullRequestAction, PullRequestDetail, PullRequestState,
};
use hoshi_ui::prelude::*;

/// The state icon's color, or the color of what holds an open pull request back.
fn icon_color(pull_request: &PullRequest, colors: &Colors) -> Hsla {
    let (_, color, _) = state_look(pull_request.state, colors);
    if pull_request.state != PullRequestState::Open {
        return color;
    }
    if pull_request.conflicts || pull_request.checks == Some(ChecksState::Failing) {
        colors.status_error
    } else if pull_request.changes_requested {
        colors.warning
    } else {
        color
    }
}

impl PullRequestSurface {
    /// Asks first, then makes the change; the question opens after the menu it came from closes.
    fn confirm_then(
        &mut self,
        request: ConfirmRequest,
        action: PullRequestAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let this = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let confirmed = DialogLayer::confirm(request, window, cx);
            window
                .spawn(cx, async move |cx| {
                    if confirmed.await {
                        this.update_in(cx, |this, window, cx| {
                            this.run(action, window, cx, |_, _, _| {})
                        })
                        .ok();
                    }
                })
                .detach();
        });
    }

    fn merge(&mut self, method: MergeMethod, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(pull_request), Some(detail)) = (self.pull_request(cx), self.detail(cx)) else {
            return;
        };
        let warning = changes_requested_warning(&detail);
        let request = ConfirmRequest::new(merge_method_label(method), m::merge())
            .description(merge_question(&pull_request, method, warning.as_deref()))
            .cancel_label(m::cancel());
        self.confirm_then(request, PullRequestAction::Merge { method }, window, cx);
    }

    fn set_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if open {
            self.run(
                PullRequestAction::SetOpen { open },
                window,
                cx,
                |_, _, _| {},
            );
            return;
        }
        let Some(pull_request) = self.pull_request(cx) else {
            return;
        };
        let request = ConfirmRequest::new(m::close_pull_request(), m::close_pull_request())
            .description(m::close_pull_request_confirm(pull_request.number))
            .cancel_label(m::cancel())
            .destructive()
            .icon(IconName::GitPullRequestClosed);
        self.confirm_then(request, PullRequestAction::SetOpen { open }, window, cx);
    }

    pub(super) fn header(
        &self,
        pull_request: &PullRequest,
        detail: Option<&PullRequestDetail>,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let (icon, _, _) = state_look(pull_request.state, colors);
        let color = icon_color(pull_request, colors);
        let url = pull_request.url.clone();
        let mono = cx.theme().mono_font_family.clone();
        let updated = m::pr_updated(relative_time(&pull_request.updated_at, chrono::Utc::now()));
        let checkout = checkout_command(pull_request);
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_1p5()
            .px_3()
            .pt_3()
            .pb_2p5()
            .border_b_1()
            .border_color(colors.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(
                        div()
                            .id("pr-link")
                            .flex()
                            .items_center()
                            .gap_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .cursor_pointer()
                            .hover(|link| link.text_color(colors.foreground))
                            .when_some(detail, |link, detail| {
                                link.child(div().truncate().child(detail.repository.clone()))
                            })
                            .child(
                                div()
                                    .flex_none()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(colors.foreground)
                                    .child(format!("#{}", pull_request.number)),
                            )
                            .child(Icon::new(IconName::ExternalLink).size_3().flex_none())
                            .on_click(move |_, _, cx| cx.open_url(&url))
                            .with_tooltip(m::open_in_browser()),
                    )
                    .child(div().flex_1())
                    .when_some(detail, |row, detail| {
                        row.children(self.merge_menu(pull_request, detail, colors, cx))
                    })
                    .child(self.more_menu(pull_request, detail, cx)),
            )
            .child(self.title(pull_request, detail, colors, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .child(Icon::new(icon).size_3p5().flex_none().text_color(color))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .when_some(pull_request.author.clone(), |line, author| {
                                line.child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(colors.foreground)
                                        .child(author),
                                )
                                .child(div().whitespace_nowrap().child(" · "))
                            })
                            .child(div().whitespace_nowrap().child(updated)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap_1()
                            .font_family(mono.clone())
                            .child(
                                div()
                                    .flex_none()
                                    .child(format!("{} ←", pull_request.base_branch)),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(colors.foreground.opacity(0.85))
                                    .child(pull_request.head_branch.clone()),
                            )
                            .child(self.copy_checkout_button(checkout, mono, colors, cx)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(Icon::new(IconName::File).size_3())
                            .when_some(detail, |line, detail| {
                                line.child(m::diff_file_count(detail.changed_files))
                            })
                            .child(DiffStat::new(
                                pull_request.additions,
                                pull_request.deletions,
                            )),
                    ),
            )
    }

    fn title(
        &self,
        pull_request: &PullRequest,
        detail: Option<&PullRequestDetail>,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let (Some((title, _)), Some(_)) = (&self.title, detail) {
            let pending = self.pending;
            title.update(cx, |title, cx| title.set_disabled(pending, cx));
            return div()
                .debug_selector(|| "pr-title-input".into())
                .text_base()
                .font_weight(FontWeight::SEMIBOLD)
                .child(text_input(title))
                .into_any_element();
        }
        let editable = detail.is_some_and(|detail| detail.can_edit);
        div()
            .group("pr-title")
            .flex()
            .items_start()
            .gap_1()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .line_height(relative(1.375))
                    .child(pull_request.title.clone()),
            )
            .when(editable, |row| {
                row.child(
                    div()
                        .id("edit-title")
                        .debug_selector(|| "edit-title".into())
                        .mt_0p5()
                        .p_0p5()
                        .rounded_sm()
                        .cursor_pointer()
                        .text_color(colors.muted_foreground)
                        .opacity(0.)
                        .group_hover("pr-title", |button| button.opacity(1.))
                        .hover(|button| button.text_color(colors.foreground))
                        .child(Icon::new(IconName::Pencil).size_3p5())
                        .on_click(cx.listener(|this, _, window, cx| this.start_title(window, cx)))
                        .with_tooltip(m::edit_title()),
                )
            })
            .into_any_element()
    }

    fn merge_menu(
        &self,
        pull_request: &PullRequest,
        detail: &PullRequestDetail,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !shows_merge(pull_request, detail) {
            return None;
        }
        let blocker = merge_blocker(pull_request, detail);
        let warning = changes_requested_warning(detail);
        let conflicts = blocker == Some(MergeBlocker::Conflicts);
        let disabled = blocker.is_some() || self.pending;
        let hint = blocker.map(MergeBlocker::label).or(warning.clone());
        let (bg, fg, hover) = if conflicts {
            (
                colors.destructive.opacity(0.1),
                colors.destructive,
                colors.destructive.opacity(0.2),
            )
        } else if warning.is_some() {
            (
                colors.warning.opacity(0.14),
                colors.warning,
                colors.warning.opacity(0.2),
            )
        } else {
            (
                colors.pr_open,
                colors.primary_foreground,
                colors.pr_open.opacity(0.9),
            )
        };
        let icon = match blocker {
            Some(MergeBlocker::Permission) => IconName::Lock,
            Some(MergeBlocker::Draft) => IconName::GitPullRequestDraft,
            Some(MergeBlocker::Conflicts) => IconName::GitMergeConflict,
            None if warning.is_some() => IconName::CircleX,
            None => IconName::GitMerge,
        };
        let pending = self.pending;
        let trigger = move |open: bool| {
            div()
                .id("merge-menu-trigger")
                .debug_selector(|| "merge-menu".into())
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .h_6()
                .px_2()
                .rounded(px(8.))
                .bg(if open { hover } else { bg })
                .text_color(fg)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .when(disabled && !conflicts, |trigger| trigger.opacity(0.5))
                .when(!disabled, |trigger| {
                    trigger.cursor_pointer().hover(move |style| style.bg(hover))
                })
                .map(|trigger| {
                    if pending {
                        trigger.child(spinner(px(12.), fg))
                    } else {
                        trigger.child(Icon::new(icon).size_3())
                    }
                })
                .child(m::merge())
                .child(Icon::new(IconName::ChevronDown).size_3())
        };
        let menu = if disabled {
            trigger(false).into_any_element()
        } else {
            let this = cx.weak_entity();
            let methods = detail.merge_methods.clone();
            Popover::new("merge-menu")
                .align(Align::End)
                .w(px(192.))
                .trigger(trigger)
                .menu(move |menu, _, _| {
                    menu.items(methods.into_iter().map(|method| {
                        let this = this.clone();
                        MenuOption::new(
                            ("merge-method", method as usize),
                            merge_method_label(method),
                        )
                        .on_select(move |window, cx| {
                            this.update(cx, |this, cx| this.merge(method, window, cx))
                                .ok();
                        })
                    }))
                })
                .into_any_element()
        };
        Some(match hint {
            Some(hint) => {
                TooltipTrigger::new("merge-hint", menu, Tooltip::new(hint).side(Side::Bottom))
                    .into_any_element()
            }
            None => menu,
        })
    }

    fn more_menu(
        &self,
        pull_request: &PullRequest,
        detail: Option<&PullRequestDetail>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let this = cx.weak_entity();
        let actions = state_actions(pull_request, detail);
        let (url, checkout) = (pull_request.url.clone(), checkout_command(pull_request));
        let draft = pull_request.state == PullRequestState::Draft;
        let pending = self.pending;
        Popover::new("pull-request-menu")
            .align(Align::End)
            .w(px(208.))
            .trigger(|open| {
                use gpui_kit::component::Selectable as _;
                Button::new("pull-request-actions")
                    .icon(IconName::Ellipsis)
                    .size(ButtonSize::IconXs)
                    .variant(ButtonVariant::Ghost)
                    .selected(open)
                    .tooltip(Tooltip::new(m::pull_request_actions()).side(Side::Bottom))
            })
            .menu(move |menu, _, _| {
                let copy = |id: &'static str, label: String, text: String| {
                    let this = this.clone();
                    MenuOption::new(id, label).on_select(move |_, cx| {
                        let text = text.clone();
                        this.update(cx, |this, cx| this.copy(text, cx)).ok();
                    })
                };
                let opened = url.clone();
                let mut menu = menu
                    .item(
                        MenuOption::new("pr-open-browser", m::open_in_browser())
                            .on_select(move |_, cx| cx.open_url(&opened)),
                    )
                    .item(copy("pr-copy-link", m::copy_link(), url))
                    .item(copy(
                        "pr-copy-checkout",
                        m::copy_checkout_command(),
                        checkout,
                    ));
                if actions.toggle_draft || actions.close_or_reopen {
                    menu = menu.separator();
                }
                if actions.toggle_draft {
                    let this = this.clone();
                    menu = menu.item(
                        MenuOption::new(
                            "pr-toggle-draft",
                            if draft {
                                m::mark_ready_for_review()
                            } else {
                                m::convert_to_draft()
                            },
                        )
                        .disabled(pending)
                        .on_select(move |window, cx| {
                            this.update(cx, |this, cx| {
                                this.run(
                                    PullRequestAction::SetDraft { draft: !draft },
                                    window,
                                    cx,
                                    |_, _, _| {},
                                )
                            })
                            .ok();
                        }),
                    );
                }
                if actions.close_or_reopen {
                    let this = this.clone();
                    let open = actions.open;
                    let option = MenuOption::new(
                        "pr-close",
                        if open {
                            m::close_pull_request()
                        } else {
                            m::reopen_pull_request()
                        },
                    )
                    .disabled(pending)
                    .on_select(move |window, cx| {
                        this.update(cx, |this, cx| this.set_open(!open, window, cx))
                            .ok();
                    });
                    menu = menu.item(if open { option.destructive() } else { option });
                }
                menu
            })
    }

    fn copy_checkout_button(
        &self,
        command: String,
        mono: SharedString,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let copied = self.copied.is_some();
        let tooltip = command.clone();
        div()
            .id("copy-checkout")
            .debug_selector(|| "copy-checkout".into())
            .flex()
            .flex_none()
            .size_5()
            .items_center()
            .justify_center()
            .rounded_sm()
            .cursor_pointer()
            .hover(|button| button.bg(colors.accent).text_color(colors.foreground))
            .child(
                Icon::new(if copied {
                    IconName::Check
                } else {
                    IconName::Copy
                })
                .size_3()
                .when(copied, |icon| icon.text_color(colors.pr_open)),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.copy_checkout(command.clone(), cx)))
            .with_tooltip(
                Tooltip::element(move |_, _| {
                    div().font_family(mono.clone()).child(tooltip.clone())
                })
                .side(Side::Bottom)
                .side_offset(px(8.)),
            )
    }
}
