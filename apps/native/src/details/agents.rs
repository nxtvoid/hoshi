use crate::tool_calls::is_subagent;
use hoshi_core::{Event, ItemContent, Thread, ThreadItem, ThreadStatus};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentState {
    Running,
    Waiting,
    Done,
    Stopped,
    Failed,
}

impl AgentState {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::Waiting)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentKind {
    Task,
    Subagent,
}

/// An agent that worked for a thread: a task it delegated, or a subagent of its turns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadAgent {
    pub kind: AgentKind,
    /// The child thread of a task; the call that started a subagent.
    pub id: String,
    pub title: String,
    pub state: AgentState,
    /// When it started, as RFC 3339.
    pub at: String,
}

/// Keeps the delegated tasks whose answer hasn't gone back yet, as the engine's snapshot
/// lists them, up to date with `event`.
pub fn track_pending_tasks(pending: &mut HashSet<String>, event: &Event) {
    match event {
        Event::ThreadDelegated { thread_id, .. } | Event::TaskReopened { thread_id } => {
            pending.insert(thread_id.clone());
        }
        Event::TaskCancelled { thread_id } | Event::TaskAnswered { thread_id } => {
            pending.remove(thread_id);
        }
        Event::ItemAdded { item } => {
            if let ItemContent::UserMessage {
                task_result: Some(task),
                ..
            } = &item.content
            {
                pending.remove(task);
            }
        }
        _ => {}
    }
}

/// A task that waits for the user's changes to its plan: still planning, at rest, its answer
/// not given back.
fn planning_with_user(thread: &Thread, pending: &HashSet<String>) -> bool {
    thread.parent_thread_id.is_some()
        && thread.plan_mode
        && thread.status == ThreadStatus::Idle
        && pending.contains(&thread.id)
}

/// Whether a task is still to answer the thread that delegated it: its answer hasn't gone
/// back yet, or it has and what the task left going will have it answer again.
fn in_flight(thread: &Thread, pending: &HashSet<String>, busy: &HashSet<String>) -> bool {
    thread.parent_thread_id.is_some()
        && !thread.task_cancelled
        && (pending.contains(&thread.id) || busy.contains(&thread.id))
}

/// The threads at rest that still have something going: those with commands
/// `in_background`, and those a task in flight is still to answer, however deep.
pub fn busy_threads<'a>(
    threads: &[&Thread],
    pending: &HashSet<String>,
    in_background: impl IntoIterator<Item = &'a String>,
) -> HashSet<String> {
    let mut busy: HashSet<String> = in_background.into_iter().cloned().collect();
    let mut grew = true;
    while grew {
        grew = false;
        for thread in threads {
            if let Some(parent) = &thread.parent_thread_id
                && !busy.contains(parent)
                && in_flight(thread, pending, &busy)
            {
                busy.insert(parent.clone());
                grew = true;
            }
        }
    }
    busy
}

/// How a delegated task stands, read from its thread. At rest it still runs while it is in
/// flight: its answer is on its way, or it will answer again.
pub fn task_state(
    thread: &Thread,
    pending: &HashSet<String>,
    busy: &HashSet<String>,
) -> AgentState {
    if planning_with_user(thread, pending) {
        return AgentState::Waiting;
    }
    if thread.task_cancelled && thread.status != ThreadStatus::Working {
        return AgentState::Stopped;
    }
    match thread.status {
        ThreadStatus::Working => AgentState::Running,
        ThreadStatus::WaitingForInput => AgentState::Waiting,
        ThreadStatus::Error => AgentState::Failed,
        ThreadStatus::Idle if in_flight(thread, pending, busy) => AgentState::Running,
        ThreadStatus::Idle => AgentState::Done,
    }
}

/// How a subagent stands, read from the call that started it.
pub fn subagent_state(has_output: bool, is_error: bool, interrupted: bool) -> AgentState {
    if interrupted {
        AgentState::Stopped
    } else if !has_output {
        AgentState::Running
    } else if is_error {
        AgentState::Failed
    } else {
        AgentState::Done
    }
}

/// What the agent asked a subagent: its description, else its kind.
fn subagent_title(input: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(input).unwrap_or_default();
    let text = |key: &str| value.get(key).and_then(|v| v.as_str()).unwrap_or_default();
    let description = text("description");
    if description.is_empty() {
        text("subagent_type").to_owned()
    } else {
        description.to_owned()
    }
}

/// Where the calls that started the thread's subagents are in its loaded transcript.
pub fn subagent_calls(thread_id: &str, items: &[ThreadItem]) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.thread_id == thread_id)
        .filter(|(_, item)| {
            matches!(&item.content, ItemContent::ToolCall { name, .. } if is_subagent(name))
        })
        .map(|(index, _)| index)
        .collect()
}

/// The thread's agents, newest first: the tasks it delegated, and the subagents of `calls`,
/// the calls that started them.
pub fn thread_agents<'a>(
    thread_id: &str,
    threads: impl IntoIterator<Item = &'a Thread>,
    calls: impl IntoIterator<Item = &'a ThreadItem>,
    pending: &HashSet<String>,
    busy: &HashSet<String>,
) -> Vec<ThreadAgent> {
    let tasks = threads
        .into_iter()
        .filter(|thread| thread.parent_thread_id.as_deref() == Some(thread_id))
        .map(|thread| ThreadAgent {
            kind: AgentKind::Task,
            id: thread.id.clone(),
            title: thread.title.clone(),
            state: task_state(thread, pending, busy),
            at: thread.created_at.clone(),
        });
    let subagents = calls
        .into_iter()
        .filter(|item| item.thread_id == thread_id)
        .filter_map(|item| match &item.content {
            ItemContent::ToolCall {
                name,
                input,
                output,
                is_error,
                interrupted,
                ..
            } if is_subagent(name) => Some(ThreadAgent {
                kind: AgentKind::Subagent,
                id: item.id.clone(),
                title: subagent_title(input),
                state: subagent_state(output.is_some(), *is_error, *interrupted),
                at: item.created_at.clone(),
            }),
            _ => None,
        });
    let mut agents: Vec<_> = tasks.chain(subagents).collect();
    agents.sort_by(|a, b| b.at.cmp(&a.at));
    agents
}

#[cfg(test)]
mod tests {
    use super::{
        AgentKind, AgentState, busy_threads, subagent_calls, task_state, thread_agents,
        track_pending_tasks,
    };
    use hoshi_core::{AccessMode, Event, ItemContent, Thread, ThreadItem, ThreadStatus};
    use std::collections::HashSet;

    fn thread(id: &str, parent: Option<&str>) -> Thread {
        Thread {
            id: id.into(),
            project_id: Some("p".into()),
            title: id.into(),
            status: ThreadStatus::Idle,
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
            task_answered: true,
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
        }
    }

    fn agent_call(id: &str, at: &str, output: Option<&str>, interrupted: bool) -> ThreadItem {
        ThreadItem {
            id: id.into(),
            thread_id: "t".into(),
            created_at: at.into(),
            content: ItemContent::ToolCall {
                name: "Agent".into(),
                input: format!(r#"{{"description":"look {id}","subagent_type":"Explore"}}"#),
                output: output.map(Into::into),
                is_error: false,
                detail: None,
                output_omitted: false,
                interrupted,
            },
        }
    }

    fn answer(id: &str) -> ThreadItem {
        ThreadItem {
            id: id.into(),
            thread_id: "t".into(),
            created_at: "2026-11-08T10:02:00Z".into(),
            content: ItemContent::AssistantMessage { text: id.into() },
        }
    }

    #[test]
    fn lists_the_tasks_a_thread_delegated_and_its_subagents_newest_first() {
        let mut task = thread("task", Some("t"));
        task.title = "Count files".into();
        task.status = ThreadStatus::WaitingForInput;
        task.created_at = "2026-11-08T10:05:00Z".into();
        let threads = [task, thread("other", Some("x"))];
        let items = [
            agent_call("a1", "2026-11-08T10:01:00Z", Some("done"), false),
            answer("m1"),
            agent_call("a2", "2026-11-08T10:09:00Z", None, false),
            agent_call("a3", "2026-11-08T10:03:00Z", None, true),
        ];

        let calls = subagent_calls("t", &items);
        assert_eq!(calls, [0, 2, 3]);
        let none = HashSet::new();
        let agents = thread_agents(
            "t",
            &threads,
            calls.iter().map(|index| &items[*index]),
            &none,
            &none,
        );

        let summary: Vec<_> = agents
            .iter()
            .map(|agent| {
                (
                    agent.kind,
                    agent.id.as_str(),
                    agent.title.as_str(),
                    agent.state,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (AgentKind::Subagent, "a2", "look a2", AgentState::Running),
                (AgentKind::Task, "task", "Count files", AgentState::Waiting),
                (AgentKind::Subagent, "a3", "look a3", AgentState::Stopped),
                (AgentKind::Subagent, "a1", "look a1", AgentState::Done),
            ]
        );
    }

    #[test]
    fn a_task_reads_as_running_waiting_finished_stopped_or_failed() {
        let none = HashSet::new();
        let task = |status: ThreadStatus| Thread {
            status,
            ..thread("task", Some("t"))
        };
        let state = |thread: &Thread| task_state(thread, &none, &none);
        assert_eq!(state(&task(ThreadStatus::Working)), AgentState::Running);
        assert_eq!(
            state(&task(ThreadStatus::WaitingForInput)),
            AgentState::Waiting
        );
        assert_eq!(state(&task(ThreadStatus::Idle)), AgentState::Done);
        assert_eq!(state(&task(ThreadStatus::Error)), AgentState::Failed);
        let cancelled = |status| Thread {
            task_cancelled: true,
            ..task(status)
        };
        assert_eq!(state(&cancelled(ThreadStatus::Idle)), AgentState::Stopped);
        assert_eq!(
            state(&cancelled(ThreadStatus::Working)),
            AgentState::Running
        );
    }

    #[test]
    fn a_task_at_rest_runs_while_its_answer_is_pending_or_it_keeps_commands_going() {
        let none = HashSet::new();
        let pending = HashSet::from(["task".to_owned()]);
        let task = thread("task", Some("t"));
        assert_eq!(task_state(&task, &none, &none), AgentState::Done);
        assert_eq!(task_state(&task, &pending, &none), AgentState::Running);
        let planning = Thread {
            plan_mode: true,
            ..task.clone()
        };
        assert_eq!(task_state(&planning, &pending, &none), AgentState::Waiting);
        assert_eq!(task_state(&planning, &none, &none), AgentState::Done);
        let busy = HashSet::from(["task".to_owned()]);
        assert_eq!(task_state(&task, &none, &busy), AgentState::Running);
    }

    #[test]
    fn a_task_is_pending_from_its_delegation_until_its_answer_goes_back() {
        let mut pending = HashSet::new();
        track_pending_tasks(
            &mut pending,
            &Event::ThreadDelegated {
                thread_id: "task".into(),
                parent_thread_id: "t".into(),
                shares_worktree: false,
            },
        );
        assert!(pending.contains("task"));
        track_pending_tasks(
            &mut pending,
            &Event::TaskAnswered {
                thread_id: "task".into(),
            },
        );
        assert!(pending.is_empty());
        track_pending_tasks(
            &mut pending,
            &Event::TaskReopened {
                thread_id: "task".into(),
            },
        );
        assert!(pending.contains("task"));
        let result = ThreadItem {
            content: ItemContent::UserMessage {
                text: "done".into(),
                attachments: Vec::new(),
                steer: false,
                task_result: Some("task".into()),
                pull_request_update: None,
                resumed: None,
                rewind: None,
                dropped: false,
            },
            ..answer("r1")
        };
        track_pending_tasks(&mut pending, &Event::ItemAdded { item: result });
        assert!(pending.is_empty());
    }

    #[test]
    fn a_thread_stays_busy_while_a_task_of_its_own_is_in_flight_however_deep() {
        let threads = [
            thread("root", None),
            thread("child", Some("root")),
            thread("grandchild", Some("child")),
        ];
        let pending = HashSet::from(["grandchild".to_owned()]);
        let threads: Vec<&Thread> = threads.iter().collect();
        let busy = busy_threads(&threads, &pending, []);
        assert_eq!(busy, HashSet::from(["child".to_owned(), "root".to_owned()]));
        let background = "grandchild".to_owned();
        let busy = busy_threads(&threads, &HashSet::new(), [&background]);
        assert_eq!(busy.len(), 3);
    }
}
