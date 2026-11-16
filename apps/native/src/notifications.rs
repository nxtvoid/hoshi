//! System notifications for threads that finish, fail or ask for input while the window is in
//! the background, unless turned off in the settings or the thread is snoozed. Clicking one
//! brings the window forward on that thread.

use crate::m;
use crate::state::Workspace;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui_kit::*;
use hoshi_core::{ReadModel, Thread, ThreadStatus, is_snoozed};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    Finished,
    Failed,
    Waiting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub thread_id: String,
    pub kind: NoticeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub thread_id: String,
    pub title: String,
    pub body: String,
}

pub type Statuses = HashMap<String, ThreadStatus>;

pub fn statuses<'a>(threads: impl IntoIterator<Item = &'a Thread>) -> Statuses {
    threads
        .into_iter()
        .map(|thread| (thread.id.clone(), thread.status))
        .collect()
}

pub fn notices<'a>(previous: &Statuses, next: impl IntoIterator<Item = &'a Thread>) -> Vec<Notice> {
    next.into_iter()
        .filter_map(|thread| {
            let was = *previous.get(&thread.id)?;
            if was == thread.status {
                return None;
            }
            let kind = match thread.status {
                ThreadStatus::Error => NoticeKind::Failed,
                ThreadStatus::WaitingForInput => NoticeKind::Waiting,
                ThreadStatus::Idle if was == ThreadStatus::Working => NoticeKind::Finished,
                _ => return None,
            };
            Some(Notice {
                thread_id: thread.id.clone(),
                kind,
            })
        })
        .collect()
}

/// Leaves out the threads snoozed at `now`, which tell of nothing until they wake.
pub fn unsnoozed(found: Vec<Notice>, model: &ReadModel, now: DateTime<Utc>) -> Vec<Notice> {
    found
        .into_iter()
        .filter(|notice| {
            model
                .thread(&notice.thread_id)
                .is_none_or(|thread| !is_snoozed(model, thread, now))
        })
        .collect()
}

pub fn message(notice: &Notice, model: &ReadModel) -> Message {
    let thread = model.thread(&notice.thread_id);
    let title = thread
        .map(|thread| thread.title.clone())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(m::untitled_thread);
    let project = thread
        .and_then(|thread| thread.project_id.as_deref())
        .and_then(|id| model.project(id))
        .map(|project| project.title.clone())
        .unwrap_or_else(m::chat);
    let body = match notice.kind {
        NoticeKind::Finished => m::notification_finished(project),
        NoticeKind::Failed => m::notification_failed(project),
        NoticeKind::Waiting => m::notification_waiting(project),
    };
    Message {
        thread_id: notice.thread_id.clone(),
        title,
        body,
    }
}

pub fn watch(workspace: &Entity<Workspace>, window: &mut Window, cx: &mut App) {
    let (clicked, mut clicks) = unbounded::<String>();
    let mut previous = statuses(workspace.read(cx).model.threads());
    if std::env::var_os("HOSHI_NATIVE_NOTIFY").is_some() {
        show(sample(&workspace.read(cx).model), clicked.clone());
    }
    window
        .observe(workspace, cx, move |workspace, window, cx| {
            let workspace = workspace.read(cx);
            let changed = workspace.model.threads().count() != previous.len()
                || workspace
                    .model
                    .threads()
                    .any(|thread| previous.get(&thread.id) != Some(&thread.status));
            if !changed {
                return;
            }
            let found = unsnoozed(
                notices(&previous, workspace.model.threads()),
                &workspace.model,
                Utc::now(),
            );
            previous = statuses(workspace.model.threads());
            if found.is_empty() || !workspace.settings.general.notifications {
                return;
            }
            if window.is_window_active() {
                return;
            }
            for notice in &found {
                show(message(notice, &workspace.model), clicked.clone());
            }
        })
        .detach();
    let workspace = workspace.downgrade();
    window
        .spawn(cx, async move |cx| {
            while let Some(thread_id) = clicks.next().await {
                let opened = cx.update(|window, cx| {
                    window.activate_window();
                    workspace
                        .update(cx, |workspace, cx| {
                            if !thread_id.is_empty() {
                                workspace.open_thread(&thread_id, cx)
                            }
                        })
                        .is_ok()
                });
                if !matches!(opened, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
}

/// `HOSHI_NATIVE_NOTIFY`: a notification on start for the latest thread, to check that toasts show.
fn sample(model: &ReadModel) -> Message {
    let latest = model
        .threads()
        .max_by(|a, b| a.updated_at.cmp(&b.updated_at));
    match latest {
        Some(thread) => message(
            &Notice {
                thread_id: thread.id.clone(),
                kind: NoticeKind::Finished,
            },
            model,
        ),
        None => Message {
            thread_id: String::new(),
            title: m::untitled_thread(),
            body: m::notification_finished(m::chat()),
        },
    }
}

fn show(message: Message, clicked: UnboundedSender<String>) {
    let mut notification = notify_rust::Notification::new();
    notification
        .summary(&message.title)
        .body(&message.body)
        .auto_icon();
    #[cfg(all(unix, not(target_os = "macos")))]
    notification.action("default", "");
    #[cfg(windows)]
    notification.app_id(crate::app_user_model::APP_ID);
    #[cfg(target_os = "macos")]
    let _ = notify_rust::set_application(if cfg!(debug_assertions) {
        "com.apple.Terminal"
    } else {
        crate::app_icon::APP_ID
    });
    let thread_id = message.thread_id;
    std::thread::Builder::new()
        .name("notification".into())
        .spawn(move || {
            let Ok(handle) = notification.show() else {
                return;
            };
            let _ = handle.wait_for_response(|response: &notify_rust::NotificationResponse| {
                if matches!(response, notify_rust::NotificationResponse::Default) {
                    let _ = clicked.unbounded_send(thread_id);
                }
            });
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::{Notice, NoticeKind, message, notices, statuses, unsnoozed};
    use hoshi_core::{AccessMode, Event, Project, ReadModel, Thread, ThreadStatus};

    fn thread(id: &str, status: ThreadStatus) -> Thread {
        Thread {
            id: id.into(),
            project_id: None,
            title: String::new(),
            status,
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
            parent_thread_id: None,
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
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn kinds(before: &[Thread], after: &[Thread]) -> Vec<(String, NoticeKind)> {
        notices(&statuses(before), after)
            .into_iter()
            .map(|Notice { thread_id, kind }| (thread_id, kind))
            .collect()
    }

    #[test]
    fn notifications_follow_status_changes() {
        let before = [
            thread("a", ThreadStatus::Working),
            thread("b", ThreadStatus::Working),
            thread("c", ThreadStatus::Idle),
            thread("d", ThreadStatus::WaitingForInput),
            thread("e", ThreadStatus::Working),
        ];
        let after = [
            thread("a", ThreadStatus::Idle),
            thread("b", ThreadStatus::Error),
            thread("c", ThreadStatus::WaitingForInput),
            thread("d", ThreadStatus::Idle),
            thread("e", ThreadStatus::Working),
            thread("new", ThreadStatus::Error),
        ];
        assert_eq!(
            kinds(&before, &after),
            [
                ("a".to_owned(), NoticeKind::Finished),
                ("b".to_owned(), NoticeKind::Failed),
                ("c".to_owned(), NoticeKind::Waiting),
            ]
        );
    }

    #[test]
    fn snoozed_threads_finish_without_a_notification() {
        let mut model = ReadModel::default();
        let mut snoozed = thread("snoozed", ThreadStatus::Idle);
        snoozed.snoozed_until = Some("2026-11-15T12:00:00Z".into());
        snoozed.snoozed_at = Some("2026-11-15T09:00:00Z".into());
        for thread in [snoozed, thread("awake", ThreadStatus::Idle)] {
            model.apply(&Event::ThreadCreated {
                thread: Box::new(thread),
            });
        }
        let finished = |id: &str| Notice {
            thread_id: id.into(),
            kind: NoticeKind::Finished,
        };
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-15T10:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(
            unsnoozed(vec![finished("snoozed"), finished("awake")], &model, now),
            [finished("awake")]
        );
        let later = now + chrono::Duration::hours(3);
        assert_eq!(
            unsnoozed(vec![finished("snoozed")], &model, later),
            [finished("snoozed")]
        );
    }

    #[test]
    fn notifications_name_the_thread_and_project() {
        let mut model = ReadModel::default();
        model.apply(&Event::ProjectCreated {
            project: Project {
                id: "p".into(),
                title: "hoshi".into(),
                workspace_root: "/code/hoshi".into(),
                created_at: String::new(),
                updated_at: String::new(),
                worktree_setup: None,
                settings: Default::default(),
            },
        });
        let mut titled = thread("t", ThreadStatus::Idle);
        titled.title = "Fix the sidebar".into();
        titled.project_id = Some("p".into());
        model.apply(&Event::ThreadCreated {
            thread: Box::new(titled),
        });
        model.apply(&Event::ThreadCreated {
            thread: Box::new(thread("chat", ThreadStatus::Error)),
        });

        let finished = message(
            &Notice {
                thread_id: "t".into(),
                kind: NoticeKind::Finished,
            },
            &model,
        );
        assert_eq!(finished.title, "Fix the sidebar");
        assert_eq!(finished.body, "Finished in hoshi");

        let failed = message(
            &Notice {
                thread_id: "chat".into(),
                kind: NoticeKind::Failed,
            },
            &model,
        );
        assert_eq!(failed.title, "Untitled thread");
        assert_eq!(failed.body, "Failed in Chat");

        let waiting = message(
            &Notice {
                thread_id: "t".into(),
                kind: NoticeKind::Waiting,
            },
            &model,
        );
        assert_eq!(waiting.body, "Needs your input in hoshi");
    }
}
