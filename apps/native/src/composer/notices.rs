//! One-line notices above the composer about the open thread: the project root moved off the
//! branch it worked on, and review comments wait on it. A merged pull request needs none: the
//! thread moves to Done, and its worktree goes by itself when the settings ask for it.

use super::{Composer, command_error_message};
use crate::m;
use crate::sidebar::branch_mismatch;
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::{Command, PullRequest, PullRequestState, ReviewThread, Thread, ThreadStatus};
use hoshi_ui::assets::IconName;
use hoshi_ui::prelude::*;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct Notices {
    /// Branch notices closed this session, by [`branch_key`].
    branch_dismissed: HashSet<String>,
    /// The branch notice shown, which stays until it is closed or the branches change.
    branch_shown: Option<String>,
    /// The thread an action of a notice runs for.
    running: Option<String>,
    /// How many review comments waited when the user asked the agent to address them.
    addressed: HashMap<String, u32>,
    /// Threads whose pull request detail was asked for, to address their comments.
    detail_asked: HashSet<String>,
}

/// What the branch notice is about; a new pair of branches shows it again.
fn branch_key(thread: &Thread, root: &str) -> String {
    format!(
        "{}:{}:{root}",
        thread.id,
        thread.branch.as_deref().unwrap_or_default()
    )
}

/// The review comments that wait on an open pull request, unless the agent is already on them.
pub fn waiting_comments(
    state: PullRequestState,
    count: u32,
    addressed: Option<u32>,
) -> Option<u32> {
    (state == PullRequestState::Open && count > 0 && addressed != Some(count)).then_some(count)
}

fn line(selector: &'static str, icon: IconName, color: Hsla, text: String) -> Div {
    div()
        .debug_selector(move || selector.into())
        .flex()
        .items_center()
        .gap_2()
        .px_1()
        .pb_1()
        .text_xs()
        .child(Icon::new(icon).size_3p5().flex_none().text_color(color))
        .child(div().min_w_0().flex_1().truncate().child(text))
}

fn action(id: &'static str, label: String) -> Button {
    Button::new(id)
        .label(label)
        .size(ButtonSize::Xs)
        .variant(ButtonVariant::Ghost)
}

impl Composer {
    pub(super) fn render_notices(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let workspace = self.workspace.read(cx);
        let Some(thread) = workspace
            .open_thread_id(cx)
            .and_then(|id| workspace.model.thread(&id))
            .cloned()
        else {
            return Vec::new();
        };
        let root = thread
            .project_id
            .as_ref()
            .and_then(|id| workspace.project_branches.get(id).cloned().flatten());
        let pull_request = workspace.pull_requests.get(&thread.id).cloned();
        let working = matches!(
            thread.status,
            ThreadStatus::Working | ThreadStatus::WaitingForInput
        );
        let mut notices = Vec::new();
        notices.extend(self.branch_notice(&thread, root, working, cx));
        if let Some(pull_request) = &pull_request {
            notices.extend(self.review_notice(&thread, pull_request, working, cx));
        }
        notices
    }

    /// Like the reference, it shows once the user writes, so it never nags an idle thread.
    fn branch_notice(
        &mut self,
        thread: &Thread,
        root: Option<String>,
        working: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let root = root.filter(|root| branch_mismatch(thread, Some(root)))?;
        let branch = thread.branch.clone()?;
        let key = branch_key(thread, &root);
        if self.notices.branch_dismissed.contains(&key) {
            return None;
        }
        let written = !self.text(cx).trim().is_empty();
        if !written && self.notices.branch_shown.as_deref() != Some(key.as_str()) {
            return None;
        }
        self.notices.branch_shown = Some(key.clone());
        let colors = cx.hoshi().clone();
        let running = self.notices.running.as_deref() == Some(thread.id.as_str());
        let thread_id = thread.id.clone();
        let switch_to = branch.clone();
        Some(
            line(
                "branch-notice",
                IconName::GitBranch,
                colors.link,
                m::branch_changed_from(&branch, &root),
            )
            .text_color(colors.muted_foreground)
            .child(
                action("branch-notice-switch", m::branch_switch_to(&branch))
                    .disabled(working || running)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.switch_to_branch(thread_id.clone(), switch_to.clone(), cx)
                    })),
            )
            .child(
                Button::new("branch-notice-dismiss")
                    .icon(IconName::X)
                    .size(ButtonSize::IconXs)
                    .variant(ButtonVariant::Ghost)
                    .tooltip(Tooltip::new(m::branch_notice_dismiss()).side(Side::Top))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.notices.branch_dismissed.insert(key.clone());
                        this.notices.branch_shown = None;
                        cx.notify();
                    })),
            )
            .into_any_element(),
        )
    }

    fn switch_to_branch(&mut self, thread_id: String, branch: String, cx: &mut Context<Self>) {
        let engine = self.workspace.read(cx).engine.clone();
        self.notices.running = Some(thread_id.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { engine.switch_branch(&thread_id, &branch) })
                .await;
            this.update(cx, |this, cx| {
                this.notices.running = None;
                if let Err(error) = result {
                    this.error = Some(command_error_message(&error).into());
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn review_notice(
        &mut self,
        thread: &Thread,
        pull_request: &PullRequest,
        working: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if working {
            return None;
        }
        let count = waiting_comments(
            pull_request.state,
            pull_request.unresolved_comments,
            self.notices.addressed.get(&thread.id).copied(),
        )?;
        let detail = self
            .workspace
            .read(cx)
            .pull_request_details
            .get(&thread.id)
            .cloned()
            .flatten();
        if detail.is_none() && self.notices.detail_asked.insert(thread.id.clone()) {
            self.read_pull_request_detail(thread.id.clone(), cx);
        }
        let prompt = detail.map(|detail| {
            let unresolved: Vec<&ReviewThread> = detail
                .review_threads
                .iter()
                .filter(|thread| !thread.resolved)
                .collect();
            m::address_comments_prompt(
                crate::panel::format_review_threads(&unresolved),
                pull_request.number,
            )
        });
        let colors = cx.hoshi().clone();
        let running = self.notices.running.as_deref() == Some(thread.id.as_str());
        let thread_id = thread.id.clone();
        Some(
            line(
                "review-notice",
                IconName::MessageCircle,
                colors.muted_foreground,
                m::review_comments_waiting(count as usize, pull_request.number),
            )
            .text_color(colors.muted_foreground)
            .child(
                action("review-notice-address", m::address_comments())
                    .disabled(prompt.is_none() || running)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(prompt) = prompt.clone() {
                            this.address_comments(thread_id.clone(), prompt, count, cx)
                        }
                    })),
            )
            .into_any_element(),
        )
    }

    fn read_pull_request_detail(&self, thread_id: String, cx: &mut Context<Self>) {
        let engine = self.workspace.read(cx).engine.clone();
        let workspace = self.workspace.downgrade();
        cx.spawn(async move |_, cx| {
            let read = thread_id.clone();
            let detail = cx
                .background_spawn(async move { engine.pull_request_detail(&read) })
                .await;
            let Some(detail) = detail else {
                return;
            };
            workspace
                .update(cx, |workspace, cx| {
                    workspace
                        .pull_request_details
                        .insert(thread_id, Some(detail));
                    cx.notify();
                })
                .ok();
        })
        .detach();
    }

    fn address_comments(
        &mut self,
        thread_id: String,
        prompt: String,
        count: u32,
        cx: &mut Context<Self>,
    ) {
        let engine = self.workspace.read(cx).engine.clone();
        self.notices.running = Some(thread_id.clone());
        cx.notify();
        let send = Command::MessageSend {
            thread_id: thread_id.clone(),
            message_id: uuid::Uuid::new_v4().to_string(),
            text: prompt,
            attachments: Vec::new(),
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { engine.dispatch(send) })
                .await;
            this.update(cx, |this, cx| {
                this.notices.running = None;
                match result {
                    Ok(()) => {
                        this.notices.addressed.insert(thread_id, count);
                    }
                    Err(error) => this.error = Some(command_error_message(&error).into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::waiting_comments;
    use hoshi_core::PullRequestState::{Merged, Open};

    #[test]
    fn comments_wait_on_an_open_pull_request_until_the_agent_is_asked_about_them() {
        assert_eq!(waiting_comments(Open, 3, None), Some(3));
        assert_eq!(waiting_comments(Open, 3, Some(3)), None);
        assert_eq!(
            waiting_comments(Open, 4, Some(3)),
            Some(4),
            "a new comment shows the notice again"
        );
        assert_eq!(waiting_comments(Open, 0, None), None);
        assert_eq!(waiting_comments(Merged, 3, None), None);
    }
}
