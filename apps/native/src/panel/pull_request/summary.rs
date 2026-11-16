use super::actions::editable;
use super::{
    Editing, PullRequestSurface, card, chip, format_review_threads, label_color, markdown,
    review_look, section_title,
};
use crate::m;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::{
    PullRequest, PullRequestAction, PullRequestDetail, ReviewState, ReviewThread, ThreadStatus,
};
use hoshi_ui::prelude::*;

impl PullRequestSurface {
    /// The small × on a chip that takes it off the pull request.
    fn remove_button(
        &self,
        id: SharedString,
        label: String,
        action: PullRequestAction,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let pending = self.pending;
        let selector = id.to_string();
        div()
            .id(id)
            .debug_selector(move || selector)
            .flex_none()
            .p_0p5()
            .mr(px(-4.))
            .rounded_full()
            .text_color(colors.muted_foreground)
            .when(pending, |button| button.opacity(0.5))
            .when(!pending, |button| {
                button
                    .cursor_pointer()
                    .hover(|button| button.text_color(colors.foreground))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run(action.clone(), window, cx, |_, _, _| {})
                    }))
            })
            .child(Icon::new(IconName::X).size_2p5())
            .with_tooltip(label)
    }

    pub(super) fn summary(
        &self,
        pull_request: &PullRequest,
        detail: &PullRequestDetail,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let working = self
            .workspace
            .read(cx)
            .model
            .thread(&self.thread_id)
            .is_some_and(|thread| thread.status == ThreadStatus::Working);
        let editable = editable(pull_request, detail);
        let unresolved: Vec<&ReviewThread> = detail
            .review_threads
            .iter()
            .filter(|thread| !thread.resolved)
            .collect();
        let field = |icon: IconName, label: String| {
            div()
                .w(px(96.))
                .flex_none()
                .h_6()
                .flex()
                .items_center()
                .gap_1p5()
                .text_color(colors.muted_foreground)
                .child(Icon::new(icon).size_3p5())
                .child(label)
        };
        let values = || div().flex().flex_1().flex_wrap().items_center().gap_1();
        let none = || div().text_color(colors.muted_foreground).child(m::none());
        let reviewers: Vec<AnyElement> = detail
            .reviewers
            .iter()
            .enumerate()
            .map(|(index, reviewer)| {
                let (icon, color, label) = review_look(reviewer.state, colors);
                let removable = editable && reviewer.state == ReviewState::Requested;
                chip(colors)
                    .id(("reviewer-chip", index))
                    .child(Icon::new(icon).size_3().text_color(color))
                    .child(reviewer.name.clone())
                    .when(removable, |chip| {
                        chip.child(self.remove_button(
                            format!("remove-reviewer-{}", reviewer.name).into(),
                            m::remove_reviewer(&reviewer.name),
                            PullRequestAction::RemoveReviewers {
                                names: vec![reviewer.name.clone()],
                            },
                            colors,
                            cx,
                        ))
                    })
                    .with_tooltip(label)
                    .into_any_element()
            })
            .collect();
        let labels: Vec<AnyElement> = detail
            .labels
            .iter()
            .map(|label| {
                chip(colors)
                    .when_some(label_color(&label.color), |chip, color| {
                        chip.child(div().size_2().rounded_full().bg(color))
                    })
                    .child(label.name.clone())
                    .when(editable, |chip| {
                        chip.child(self.remove_button(
                            format!("remove-label-{}", label.name).into(),
                            m::remove_label(&label.name),
                            PullRequestAction::RemoveLabel {
                                name: label.name.clone(),
                            },
                            colors,
                            cx,
                        ))
                    })
                    .into_any_element()
            })
            .collect();
        let prompt = (!unresolved.is_empty()).then(|| {
            m::address_comments_prompt(format_review_threads(&unresolved), pull_request.number)
        });
        let threads: Vec<AnyElement> = unresolved
            .iter()
            .map(|thread| self.review_thread(thread, colors, cx))
            .collect();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .p_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .text_xs()
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap_3()
                            .child(field(IconName::Users, m::pr_reviewers()))
                            .child(
                                values()
                                    .when(detail.reviewers.is_empty(), |values| {
                                        values.child(none())
                                    })
                                    .children(reviewers)
                                    .when(editable, |values| {
                                        values.child(self.reviewer_picker(
                                            pull_request,
                                            detail,
                                            colors,
                                            cx,
                                        ))
                                    }),
                            ),
                    )
                    .when(detail.supports.labels, |fields| {
                        fields.child(
                            div()
                                .flex()
                                .items_start()
                                .gap_3()
                                .child(field(IconName::Tag, m::pr_labels()))
                                .child(
                                    values()
                                        .when(detail.labels.is_empty(), |values| {
                                            values.child(none())
                                        })
                                        .children(labels)
                                        .when(editable, |values| {
                                            values.child(self.label_picker(detail, colors, cx))
                                        }),
                                ),
                        )
                    }),
            )
            .when_some(prompt, |this, prompt| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(section_title(
                                    m::unresolved_comments(unresolved.len()),
                                    colors,
                                ))
                                .child(div().flex_1())
                                .child(
                                    Button::new("address-comments")
                                        .label(m::address_comments())
                                        .size(ButtonSize::Xs)
                                        .disabled(working || self.sending)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.address_comments(prompt.clone(), cx)
                                        })),
                                ),
                        )
                        .children(threads),
                )
            })
            .child(self.description(detail, editable, colors, cx))
            .into_any_element()
    }

    fn description(
        &self,
        detail: &PullRequestDetail,
        editable: bool,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let editing = self.body.is_some();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(section_title(m::pr_description(), colors))
                    .when(editable && !editing, |row| {
                        row.child(div().flex_1()).child(
                            Button::new("edit-description")
                                .label(m::edit())
                                .size(ButtonSize::Xs)
                                .variant(ButtonVariant::Ghost)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.start_body(window, cx)),
                                ),
                        )
                    }),
            )
            .child(if editing {
                self.editor(Editing::Body, m::save(), true, colors, cx)
            } else if detail.body.trim().is_empty() {
                div()
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .child(m::pr_no_description())
                    .into_any_element()
            } else {
                div()
                    .text_sm()
                    .child(markdown(
                        &format!("pr-body-{}", self.thread_id),
                        &detail.body,
                        colors,
                    ))
                    .into_any_element()
            })
    }

    fn review_thread(
        &self,
        thread: &ReviewThread,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mono = cx.theme().mono_font_family.clone();
        let place = match thread.line {
            Some(line) => format!("{}:{line}", thread.path),
            None => thread.path.clone(),
        };
        let id = thread.id.clone();
        let footer = if self.replies.contains_key(&id) {
            self.editor(Editing::Reply(id), m::reply(), true, colors, cx)
        } else {
            let (reply, resolve) = (id.clone(), id.clone());
            div()
                .flex()
                .justify_end()
                .gap_1p5()
                .child(
                    Button::new(SharedString::from(format!("reply-{id}")))
                        .label(m::reply())
                        .size(ButtonSize::Xs)
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.start_reply(reply.clone(), window, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("resolve-{id}")))
                        .label(m::resolve())
                        .size(ButtonSize::Xs)
                        .variant(ButtonVariant::Outline)
                        .disabled(self.pending)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let action = PullRequestAction::SetThreadResolved {
                                thread_id: resolve.clone(),
                                resolved: true,
                            };
                            this.run(action, window, cx, |_, _, _| {})
                        })),
                )
                .into_any_element()
        };
        card(colors)
            .child(
                div()
                    .px_2p5()
                    .py_1p5()
                    .border_b_1()
                    .border_color(colors.border.opacity(0.6))
                    .font_family(mono)
                    .text_color(colors.muted_foreground)
                    .child(place),
            )
            .child(div().flex().flex_col().gap_2().px_2p5().py_2().children(
                thread.comments.iter().map(|comment| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child(comment.author.clone().unwrap_or_default()),
                        )
                        .child(markdown(
                            &format!("pr-comment-{}", comment.url),
                            &comment.body,
                            colors,
                        ))
                }),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .px_2p5()
                    .py_1p5()
                    .border_t_1()
                    .border_color(colors.border.opacity(0.6))
                    .child(footer),
            )
            .into_any_element()
    }
}
