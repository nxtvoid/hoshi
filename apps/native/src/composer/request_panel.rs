//! What the agent waits on, above the composer: an approval, a plan, questions or a secret.

use super::Composer;
use crate::m;
use crate::requests::{
    QuestionProgress, answer_question, answer_text, approval_choices, previous_question,
    toggle_option,
};
use crate::tool_calls::tool_target;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::*;
use hoshi_core::{
    ApprovalDecision, Command, ItemContent, Question, RequestResponse, Thread, ThreadItem,
};
use hoshi_ui::prelude::*;

/// The secret being typed, for the request it answers.
pub(super) struct SecretField {
    request_id: String,
    input: Entity<InputState>,
    _subscription: Subscription,
}

impl SecretField {
    pub(super) fn request_id(&self) -> &str {
        &self.request_id
    }
}

impl Composer {
    /// The oldest request of the open thread still waiting for the user.
    pub(super) fn pending_request(&self, cx: &App) -> Option<ThreadItem> {
        let workspace = self.workspace.read(cx);
        let thread_id = workspace.open_thread_id(cx)?;
        workspace
            .model
            .pending_requests(&thread_id)
            .first()
            .map(|item| (*item).clone())
    }

    fn respond(&mut self, request: &ThreadItem, response: RequestResponse, cx: &mut Context<Self>) {
        let engine = self.workspace.read(cx).engine.clone();
        let command = Command::RequestRespond {
            thread_id: request.thread_id.clone(),
            request_id: request.id.clone(),
            response,
        };
        self.error = None;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { engine.dispatch(command) })
                .await;
            if let Err(error) = result {
                this.update(cx, |this, cx| {
                    this.error = Some(super::command_error_message(&error).into());
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }

    pub(super) fn decide(
        &mut self,
        request: &ThreadItem,
        decision: ApprovalDecision,
        feedback: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.respond(
            request,
            RequestResponse::Decision { decision, feedback },
            cx,
        );
    }

    fn progress(&self, request_id: &str) -> QuestionProgress {
        self.progress.get(request_id).cloned().unwrap_or_default()
    }

    /// Answers the current question and moves on; `false` when there was nothing to answer.
    pub(super) fn advance_from(
        &mut self,
        request: &ThreadItem,
        from: &QuestionProgress,
        custom: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let ItemContent::Questions { questions, .. } = &request.content else {
            return false;
        };
        let answer = answer_text(&from.selected, custom);
        let (next, answers) = answer_question(from, questions.len(), &answer);
        if let Some(answers) = answers {
            self.respond(request, RequestResponse::Answers { answers }, cx);
            return true;
        }
        let moved = next != *from;
        self.progress.insert(request.id.clone(), next);
        cx.notify();
        moved
    }

    pub(super) fn pick_option(
        &mut self,
        request: &ThreadItem,
        label: &str,
        multi_select: bool,
        cx: &mut Context<Self>,
    ) {
        let next = toggle_option(&self.progress(&request.id), label, multi_select);
        if multi_select {
            self.progress.insert(request.id.clone(), next);
            cx.notify();
        } else {
            self.advance_from(request, &next, "", cx);
        }
    }

    fn back(&mut self, request: &ThreadItem, cx: &mut Context<Self>) {
        let ItemContent::Questions { questions, .. } = &request.content else {
            return;
        };
        let back = previous_question(&self.progress(&request.id), questions);
        self.progress.insert(request.id.clone(), back);
        cx.notify();
    }

    /// The option the digit picks, on the question on screen.
    pub(super) fn option_for_digit(
        &self,
        request: &ThreadItem,
        key: &str,
    ) -> Option<(String, bool)> {
        let ItemContent::Questions { questions, .. } = &request.content else {
            return None;
        };
        let digit: usize = key.parse().ok().filter(|digit| (1..=9).contains(digit))?;
        let question = questions.get(self.progress(&request.id).index)?;
        let option = question.options.get(digit - 1)?;
        Some((option.label.clone(), question.multi_select))
    }

    fn answer_secret(
        &mut self,
        request: &ThreadItem,
        secret: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let engine = self.workspace.read(cx).engine.clone();
        let (thread_id, request_id) = (request.thread_id.clone(), request.id.clone());
        self.sending = true;
        self.error = None;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    engine.answer_secret_request(&thread_id, &request_id, secret.as_deref())
                })
                .await;
            this.update(cx, |this, cx| {
                this.sending = false;
                if let Err(error) = result {
                    this.error = Some(super::command_error_message(&error).into());
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn secret_field(
        &mut self,
        request: &ThreadItem,
        placeholder: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(field) = &self.secret
            && field.request_id == request.id
        {
            return field.input.clone();
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(placeholder)
        });
        let answered = request.clone();
        let subscription = cx.subscribe_in(&input, window, move |this, input, event, _, cx| {
            if let InputEvent::PressEnter { .. } = event {
                let value = input.read(cx).value().to_string();
                if !value.trim().is_empty() && !this.sending {
                    this.answer_secret(&answered, Some(value), cx);
                }
            }
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        self.secret = Some(SecretField {
            request_id: request.id.clone(),
            input: input.clone(),
            _subscription: subscription,
        });
        input
    }

    /// `request`'s panel; `task` is the delegated task it comes from, when not the open thread.
    pub(super) fn render_request(
        &mut self,
        request: &ThreadItem,
        task: Option<&Thread>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match &request.content {
            ItemContent::Approval {
                tool_call_id,
                tool_name,
                input,
                reason,
                can_accept_for_session,
                ..
            } => {
                let cwd = task
                    .and_then(|task| task.worktree_path.clone())
                    .or_else(|| self.cwd(cx));
                let mut target = tool_target(tool_name, input, cwd.as_deref());
                if target.is_empty() {
                    target = input.to_owned();
                }
                let choices = approval_choices(tool_call_id.is_some(), *can_accept_for_session);
                self.approval_panel(request, tool_name, target, reason.as_deref(), &choices, cx)
            }
            ItemContent::ProposedPlan { plan, .. } => {
                let task = task.map(|task| task.id.clone());
                self.plan_panel(request, plan, task, cx)
            }
            ItemContent::Questions { questions, .. } => {
                self.questions_panel(request, questions, cx)
            }
            ItemContent::SecretRequest {
                label,
                reason,
                placeholder,
                ..
            } => {
                let placeholder = placeholder
                    .clone()
                    .unwrap_or_else(m::secret_request_placeholder);
                let field = self.secret_field(request, placeholder, window, cx);
                self.secret_panel(request, label, reason, field, cx)
            }
            _ => div().into_any_element(),
        }
    }

    fn approval_panel(
        &mut self,
        request: &ThreadItem,
        tool_name: &str,
        target: String,
        reason: Option<&str>,
        choices: &[ApprovalDecision],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.hoshi().clone();
        let buttons = choices.iter().map(|&decision| {
            let (id, label, variant) = match decision {
                ApprovalDecision::Cancel => ("cancel", m::cancel(), ButtonVariant::Ghost),
                ApprovalDecision::Decline => {
                    ("decline", m::request_decline(), ButtonVariant::Ghost)
                }
                ApprovalDecision::AcceptForSession => (
                    "approve-session",
                    m::request_approve_session(),
                    ButtonVariant::Outline,
                ),
                ApprovalDecision::Accept => {
                    ("approve", m::request_approve(), ButtonVariant::Primary)
                }
            };
            let request = request.clone();
            Button::new(id)
                .label(label)
                .size(ButtonSize::Sm)
                .variant(variant)
                .on_click(
                    cx.listener(move |this, _, _, cx| this.decide(&request, decision, None, cx)),
                )
        });
        Panel::new()
            .child(panel_title(
                IconName::ShieldUser,
                m::request_approval_title(tool_name),
                None,
                &colors,
            ))
            .child(
                div()
                    .id("approval-target")
                    .max_h(px(128.))
                    .overflow_y_scroll()
                    .rounded_md()
                    .bg(colors.muted.opacity(0.4))
                    .px_3()
                    .py_2()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_xs()
                    .text_color(colors.foreground.opacity(0.85))
                    .child(target),
            )
            .when_some(reason, |panel, reason| {
                panel.child(muted_note(reason.to_owned(), &colors))
            })
            .child(actions().children(buttons))
            .into_any_element()
    }

    fn plan_panel(
        &mut self,
        request: &ThreadItem,
        plan: &str,
        task: Option<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.hoshi().clone();
        let implement = request.clone();
        let keep_planning = Button::new("keep-planning")
            .label(m::request_plan_keep_planning())
            .size(ButtonSize::Sm)
            .variant(ButtonVariant::Ghost);
        let keep_planning = match task {
            Some(task) => keep_planning.on_click(crate::delegations::open_thread(task)),
            None => {
                keep_planning.on_click(cx.listener(|this, _, window, cx| this.focus(window, cx)))
            }
        };
        Panel::new()
            .child(panel_title(
                IconName::ListChecks,
                m::request_plan_title(),
                None,
                &colors,
            ))
            .child(markdown_box("plan", &request.id, plan, px(288.), &colors))
            .child(
                actions().child(keep_planning).child(
                    Button::new("implement")
                        .label(m::request_plan_implement())
                        .size(ButtonSize::Sm)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.decide(&implement, ApprovalDecision::Accept, None, cx)
                        })),
                ),
            )
            .into_any_element()
    }

    fn questions_panel(
        &mut self,
        request: &ThreadItem,
        questions: &[Question],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.hoshi().clone();
        let progress = self.progress(&request.id);
        let Some(question) = questions.get(progress.index) else {
            return div().into_any_element();
        };
        let total = questions.len();
        let last = progress.index + 1 == total;
        let multi = question.multi_select;
        let preview = question
            .options
            .iter()
            .find(|option| option.preview.is_some() && progress.selected.contains(&option.label))
            .and_then(|option| option.preview.clone());

        let topic = match question.header.trim() {
            "" => m::request_question_title(),
            header => header.to_owned(),
        };
        let title = div().truncate().child(topic);
        let body = div()
            .id(SharedString::from(format!(
                "question-body-{}-{}",
                request.id, progress.index
            )))
            .debug_selector(|| "question-text".into())
            .max_h(px(192.))
            .overflow_y_scroll()
            .pl_6()
            .text_sm()
            .text_color(colors.foreground.opacity(0.85))
            .child(crate::markdown::markdown(
                SharedString::from(format!("question-text-{}-{}", request.id, progress.index)),
                question.question.clone(),
                &colors,
            ));
        let count = (total > 1).then(|| m::request_question_count(progress.index + 1, total));

        let options = question.options.iter().enumerate().map(|(index, option)| {
            let selected = progress.selected.contains(&option.label);
            let (request, label) = (request.clone(), option.label.clone());
            div()
                .id(("question-option", index))
                .flex()
                .items_start()
                .gap_2p5()
                .rounded_lg()
                .px_2()
                .py_1p5()
                .text_sm()
                .cursor_pointer()
                .when(selected, |row| row.bg(colors.accent))
                .hover(|style| style.bg(colors.muted))
                .on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.pick_option(&request, &label, multi, cx)
                    }),
                )
                .when(index < 9, |row| {
                    row.child(div().mt_0p5().child(Kbd::new((index + 1).to_string())))
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(
                            div()
                                .text_color(colors.foreground)
                                .child(option.label.clone()),
                        )
                        .when(
                            !option.description.is_empty() && option.description != option.label,
                            |column| {
                                column.child(
                                    div()
                                        .text_xs()
                                        .text_color(colors.muted_foreground)
                                        .child(option.description.clone()),
                                )
                            },
                        ),
                )
        });

        let (back, next) = (request.clone(), request.clone());
        Panel::new()
            .child(panel_title_element(
                IconName::MessageCircleQuestionMark,
                title,
                count,
                &colors,
            ))
            .child(body)
            .when(multi, |panel| {
                panel.child(muted_note(m::request_question_multi(), &colors))
            })
            .child(div().flex().flex_col().gap_1().children(options))
            .when_some(preview, |panel, preview| {
                panel.child(markdown_box(
                    "preview",
                    &request.id,
                    &preview,
                    px(192.),
                    &colors,
                ))
            })
            .when(progress.index > 0 || multi, |panel| {
                panel.child(
                    actions()
                        .when(progress.index > 0, |row| {
                            row.child(
                                Button::new("question-back")
                                    .label(m::request_question_back())
                                    .size(ButtonSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(
                                        cx.listener(move |this, _, _, cx| this.back(&back, cx)),
                                    ),
                            )
                        })
                        .when(multi, |row| {
                            let from = progress.clone();
                            row.child(
                                Button::new("question-next")
                                    .label(if last {
                                        m::request_question_submit()
                                    } else {
                                        m::request_question_next()
                                    })
                                    .size(ButtonSize::Sm)
                                    .disabled(progress.selected.is_empty())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.advance_from(&next, &from, "", cx);
                                    })),
                            )
                        }),
                )
            })
            .into_any_element()
    }

    fn secret_panel(
        &mut self,
        request: &ThreadItem,
        label: &str,
        reason: &str,
        field: Entity<InputState>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.hoshi().clone();
        let empty = field.read(cx).value().trim().is_empty();
        let (decline, save, value) = (request.clone(), request.clone(), field.clone());
        Panel::new()
            .child(panel_title(
                IconName::KeyRound,
                label.to_owned(),
                None,
                &colors,
            ))
            .child(muted_note(reason.to_owned(), &colors))
            .child(text_input(&field))
            .child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(colors.muted_foreground)
                    .child(m::secret_request_note()),
            )
            .child(
                actions()
                    .child(
                        Button::new("secret-decline")
                            .label(m::request_decline())
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .disabled(self.sending)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.answer_secret(&decline, None, cx)
                            })),
                    )
                    .child(
                        Button::new("secret-save")
                            .label(m::secret_request_save())
                            .size(ButtonSize::Sm)
                            .disabled(self.sending || empty)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let secret = value.read(cx).value().to_string();
                                this.answer_secret(&save, Some(secret), cx)
                            })),
                    ),
            )
            .into_any_element()
    }

    fn cwd(&self, cx: &App) -> Option<String> {
        let workspace = self.workspace.read(cx);
        workspace
            .open
            .as_ref()
            .and_then(|open| open.read(cx).cwd.clone())
    }
}

fn panel_title(icon: IconName, text: String, aside: Option<String>, colors: &Colors) -> Div {
    panel_title_element(icon, div().child(text), aside, colors)
}

fn panel_title_element(icon: IconName, text: Div, aside: Option<String>, colors: &Colors) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_sm()
        .child(
            Icon::new(icon)
                .size_4()
                .flex_none()
                .text_color(colors.status_waiting),
        )
        .child(
            text.min_w_0()
                .flex_1()
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors.foreground),
        )
        .when_some(aside, |row, aside| {
            row.child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .child(aside),
            )
        })
}

fn muted_note(text: String, colors: &Colors) -> Div {
    div()
        .text_xs()
        .text_color(colors.muted_foreground)
        .child(text)
}

fn actions() -> Div {
    div().flex().flex_wrap().justify_end().gap_1p5()
}

fn markdown_box(
    kind: &str,
    id: &str,
    text: &str,
    max_height: Pixels,
    colors: &Colors,
) -> Stateful<Div> {
    div()
        .id(SharedString::from(format!("{kind}-{id}")))
        .max_h(max_height)
        .overflow_y_scroll()
        .rounded_md()
        .bg(colors.muted.opacity(0.4))
        .px_3()
        .py_2()
        .text_sm()
        .child(crate::markdown::markdown(
            SharedString::from(format!("{kind}-text-{id}")),
            text.to_owned(),
            colors,
        ))
}
