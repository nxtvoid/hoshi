//! What a delegated task waits on, answered from the thread that delegated it, with the
//! way to the task's own thread.

use super::Composer;
use crate::delegations::open_thread;
use crate::m;
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::{Thread, ThreadItem};
use hoshi_ui::assets::IconName;
use hoshi_ui::prelude::*;
use std::collections::HashSet;

/// The oldest of `pending` that a task `thread_id` delegated waits on, however deep the
/// task, with the task's thread.
pub fn delegated_request<'a>(
    thread_id: &str,
    pending: &[ThreadItem],
    thread: impl Fn(&str) -> Option<&'a Thread>,
) -> Option<(ThreadItem, &'a Thread)> {
    let works_for = |task: &Thread| {
        let mut seen = HashSet::new();
        let mut parent = task.parent_thread_id.as_deref();
        while let Some(id) = parent {
            if id == thread_id {
                return true;
            }
            if !seen.insert(id.to_owned()) {
                return false;
            }
            parent = thread(id).and_then(|thread| thread.parent_thread_id.as_deref());
        }
        false
    };
    pending.iter().find_map(|request| {
        let task = thread(&request.thread_id)?;
        works_for(task).then(|| (request.clone(), task))
    })
}

impl Composer {
    /// The request a task of the open thread waits on, with the task.
    pub(super) fn delegated_request(&self, cx: &App) -> Option<(ThreadItem, Thread)> {
        let workspace = self.workspace.read(cx);
        let thread_id = workspace.open_thread_id(cx)?;
        let pending = workspace.model.all_pending_requests();
        delegated_request(&thread_id, &pending, |id| workspace.model.thread(id))
            .map(|(request, task)| (request, task.clone()))
    }

    pub(super) fn render_delegated(
        &mut self,
        request: &ThreadItem,
        task: &Thread,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.hoshi().clone();
        let panel = self.render_request(request, Some(task), window, cx);
        div()
            .debug_selector(|| "delegated-request".into())
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .pb_1()
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .child(Icon::new(IconName::GitFork).size_3p5().flex_none())
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .child(m::delegated_request_from(&task.title)),
                    )
                    .child(
                        Button::new("delegated-request-open")
                            .label(m::delegation_open())
                            .suffix(Icon::new(IconName::ChevronRight).size_3p5())
                            .size(ButtonSize::Xs)
                            .variant(ButtonVariant::Ghost)
                            .on_click(open_thread(task.id.clone())),
                    ),
            )
            .child(panel)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::delegated_request;
    use hoshi_core::{AccessMode, ItemContent, Thread, ThreadItem, ThreadStatus};
    use std::collections::HashMap;

    fn thread(id: &str, parent: Option<&str>) -> Thread {
        Thread {
            id: id.into(),
            project_id: Some("p".into()),
            title: id.into(),
            status: ThreadStatus::Working,
            branch: None,
            worktree_path: None,
            session_id: None,
            conversation_cursor: None,
            resume_at: None,
            watching_pull_request: false,
            provider: hoshi_core::default_provider(),
            model: None,
            effort: None,
            fast_mode: false,
            ultracode: false,
            access_mode: AccessMode::default(),
            plan_mode: false,
            active_turn_id: None,
            archived: false,
            pinned: false,
            worktree_released: false,
            worktree_adopted: false,
            parent_thread_id: parent.map(Into::into),
            task_cancelled: false,
            task_answered: false,
            snoozed_until: None,
            snoozed_at: None,
            last_snooze_wake_at: None,
            last_visited_at: None,
            last_turn_ended_at: None,
            forked_from: None,
            pending_model: None,
            provider_sessions: Vec::new(),
            pending_handoffs: Vec::new(),
            created_at: "2026-11-08T10:00:00Z".into(),
            updated_at: "2026-11-08T10:00:00Z".into(),
            last_turn_model: None,
        }
    }

    fn request(id: &str, thread_id: &str) -> ThreadItem {
        ThreadItem {
            id: id.into(),
            thread_id: thread_id.into(),
            created_at: "2026-11-08T10:00:00Z".into(),
            content: ItemContent::RunEnded,
        }
    }

    fn threads() -> HashMap<String, Thread> {
        [
            thread("root", None),
            thread("child", Some("root")),
            thread("grandchild", Some("child")),
            thread("other", None),
            thread("other-child", Some("other")),
        ]
        .into_iter()
        .map(|thread| (thread.id.clone(), thread))
        .collect()
    }

    fn found(thread_id: &str, pending: &[ThreadItem]) -> Option<(String, String)> {
        let threads = threads();
        delegated_request(thread_id, pending, |id| threads.get(id))
            .map(|(request, task)| (request.id, task.id.clone()))
    }

    #[test]
    fn a_task_of_a_task_counts_and_the_oldest_request_wins() {
        let pending = [
            request("r-other", "other-child"),
            request("r-deep", "grandchild"),
            request("r-child", "child"),
        ];
        assert_eq!(
            found("root", &pending),
            Some(("r-deep".into(), "grandchild".into()))
        );
        assert_eq!(
            found("child", &pending),
            Some(("r-deep".into(), "grandchild".into()))
        );
    }

    #[test]
    fn the_thread_s_own_requests_and_other_threads_tasks_do_not_count() {
        let pending = [request("own", "root"), request("r-other", "other-child")];
        assert_eq!(found("root", &pending), None);
        assert_eq!(found("grandchild", &[request("r", "child")]), None);
    }
}
