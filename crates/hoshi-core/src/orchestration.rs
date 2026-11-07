use crate::{
    ItemContent, Project, ProviderOutput, Thread, ThreadItem, ThreadStatus, TurnFailure,
    TurnOutcome,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

const TITLE_MAX_CHARS: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum Command {
    ProjectCreate {
        project_id: String,
        title: String,
        workspace_root: String,
    },
    ProjectRemove {
        project_id: String,
    },
    ThreadCreate {
        thread_id: String,
        project_id: String,
        title: String,
    },
    ThreadDelete {
        thread_id: String,
    },
    MessageSend {
        thread_id: String,
        message_id: String,
        text: String,
    },
    TurnInterrupt {
        thread_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum Event {
    ProjectCreated {
        project: Project,
    },
    ProjectRemoved {
        project_id: String,
    },
    ThreadCreated {
        thread: Thread,
    },
    ThreadDeleted {
        thread_id: String,
    },
    ThreadTitleSet {
        thread_id: String,
        title: String,
    },
    TurnStarted {
        thread_id: String,
        turn_id: String,
        at: String,
    },
    SessionAttached {
        thread_id: String,
        session_id: String,
    },
    ItemAdded {
        item: ThreadItem,
    },
    ToolCallCompleted {
        thread_id: String,
        item_id: String,
        output: String,
        is_error: bool,
    },
    TurnCompleted {
        thread_id: String,
        turn_id: String,
        outcome: TurnOutcome,
        at: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum CommandError {
    #[error("project {project_id} already exists")]
    DuplicateProjectId { project_id: String },
    #[error("workspace root is already project {project_id}")]
    DuplicateWorkspaceRoot { project_id: String },
    #[error("project {project_id} not found")]
    ProjectNotFound { project_id: String },
    #[error("thread {thread_id} already exists")]
    DuplicateThreadId { thread_id: String },
    #[error("thread {thread_id} not found")]
    ThreadNotFound { thread_id: String },
    #[error("thread {thread_id} is already running a turn")]
    ThreadBusy { thread_id: String },
    #[error("thread {thread_id} is not running a turn")]
    ThreadNotWorking { thread_id: String },
    #[error("message is empty")]
    EmptyMessage,
    #[error("folder {path} does not exist")]
    WorkspaceRootNotFound { path: String },
    /// Storage or other failures outside the domain; `detail` is for logs, not the UI.
    #[error("internal error: {detail}")]
    Internal { detail: String },
}

/// What the UI reads at startup: every project and thread, without transcripts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Snapshot {
    pub sequence: u64,
    pub projects: Vec<Project>,
    pub threads: Vec<Thread>,
}

/// An event as delivered to the UI, with its position in the event log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EventEnvelope {
    pub sequence: u64,
    pub event: Event,
}

/// Text an assistant is still writing. Sent to the UI as it streams, never stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StreamDelta {
    pub thread_id: String,
    pub item_id: String,
    pub delta: String,
}

#[derive(Debug, Clone, Default)]
pub struct ReadModel {
    projects: BTreeMap<String, Project>,
    threads: BTreeMap<String, Thread>,
}

impl ReadModel {
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        Self {
            projects: snapshot
                .projects
                .iter()
                .map(|project| (project.id.clone(), project.clone()))
                .collect(),
            threads: snapshot
                .threads
                .iter()
                .map(|thread| (thread.id.clone(), thread.clone()))
                .collect(),
        }
    }

    pub fn projects(&self) -> impl Iterator<Item = &Project> {
        self.projects.values()
    }

    pub fn threads(&self) -> impl Iterator<Item = &Thread> {
        self.threads.values()
    }

    pub fn thread(&self, thread_id: &str) -> Option<&Thread> {
        self.threads.get(thread_id)
    }

    /// Where the thread's agent runs: its worktree when it has one, never the
    /// project root in that case.
    pub fn thread_cwd(&self, thread_id: &str) -> Option<&str> {
        let thread = self.threads.get(thread_id)?;
        match &thread.worktree_path {
            Some(path) => Some(path),
            None => self
                .projects
                .get(&thread.project_id)
                .map(|project| project.workspace_root.as_str()),
        }
    }

    pub fn apply(&mut self, event: &Event) {
        match event {
            Event::ProjectCreated { project } => {
                self.projects.insert(project.id.clone(), project.clone());
            }
            Event::ProjectRemoved { project_id } => {
                self.projects.remove(project_id);
                self.threads
                    .retain(|_, thread| &thread.project_id != project_id);
            }
            Event::ThreadCreated { thread } => {
                self.threads.insert(thread.id.clone(), thread.clone());
            }
            Event::ThreadDeleted { thread_id } => {
                self.threads.remove(thread_id);
            }
            Event::ThreadTitleSet { thread_id, title } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.title.clone_from(title);
                }
            }
            Event::TurnStarted {
                thread_id,
                turn_id,
                at,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.status = ThreadStatus::Working;
                    thread.active_turn_id = Some(turn_id.clone());
                    thread.updated_at.clone_from(at);
                }
            }
            Event::SessionAttached {
                thread_id,
                session_id,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.session_id = Some(session_id.clone());
                }
            }
            Event::ItemAdded { item } => {
                if let Some(thread) = self.threads.get_mut(&item.thread_id) {
                    thread.updated_at.clone_from(&item.created_at);
                }
            }
            Event::ToolCallCompleted { .. } => {}
            Event::TurnCompleted {
                thread_id,
                outcome,
                at,
                ..
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.active_turn_id = None;
                    thread.status = match outcome {
                        TurnOutcome::Failed { .. } => ThreadStatus::Error,
                        TurnOutcome::Completed | TurnOutcome::Interrupted => ThreadStatus::Idle,
                    };
                    thread.updated_at.clone_from(at);
                }
            }
        }
    }
}

/// Decides the events a command produces. `now` is an RFC 3339 timestamp.
pub fn decide(model: &ReadModel, command: Command, now: &str) -> Result<Vec<Event>, CommandError> {
    match command {
        Command::ProjectCreate {
            project_id,
            title,
            workspace_root,
        } => {
            if model.projects.contains_key(&project_id) {
                return Err(CommandError::DuplicateProjectId { project_id });
            }
            if let Some(existing) = model
                .projects()
                .find(|project| same_path(&project.workspace_root, &workspace_root))
            {
                return Err(CommandError::DuplicateWorkspaceRoot {
                    project_id: existing.id.clone(),
                });
            }
            Ok(vec![Event::ProjectCreated {
                project: Project {
                    id: project_id,
                    title,
                    workspace_root,
                    created_at: now.to_owned(),
                    updated_at: now.to_owned(),
                },
            }])
        }
        Command::ProjectRemove { project_id } => {
            if !model.projects.contains_key(&project_id) {
                return Err(CommandError::ProjectNotFound { project_id });
            }
            Ok(vec![Event::ProjectRemoved { project_id }])
        }
        Command::ThreadCreate {
            thread_id,
            project_id,
            title,
        } => {
            if model.threads.contains_key(&thread_id) {
                return Err(CommandError::DuplicateThreadId { thread_id });
            }
            if !model.projects.contains_key(&project_id) {
                return Err(CommandError::ProjectNotFound { project_id });
            }
            Ok(vec![Event::ThreadCreated {
                thread: Thread {
                    id: thread_id,
                    project_id,
                    title,
                    status: ThreadStatus::Idle,
                    branch: None,
                    worktree_path: None,
                    session_id: None,
                    active_turn_id: None,
                    created_at: now.to_owned(),
                    updated_at: now.to_owned(),
                },
            }])
        }
        Command::ThreadDelete { thread_id } => {
            if !model.threads.contains_key(&thread_id) {
                return Err(CommandError::ThreadNotFound { thread_id });
            }
            Ok(vec![Event::ThreadDeleted { thread_id }])
        }
        Command::MessageSend {
            thread_id,
            message_id,
            text,
        } => {
            let Some(thread) = model.threads.get(&thread_id) else {
                return Err(CommandError::ThreadNotFound { thread_id });
            };
            if thread.active_turn_id.is_some() {
                return Err(CommandError::ThreadBusy { thread_id });
            }
            let text = text.trim().to_owned();
            if text.is_empty() {
                return Err(CommandError::EmptyMessage);
            }
            let mut events = Vec::with_capacity(3);
            if thread.title.is_empty() {
                events.push(Event::ThreadTitleSet {
                    thread_id: thread_id.clone(),
                    title: title_from(&text),
                });
            }
            events.push(Event::ItemAdded {
                item: ThreadItem {
                    id: message_id.clone(),
                    thread_id: thread_id.clone(),
                    created_at: now.to_owned(),
                    content: ItemContent::UserMessage { text },
                },
            });
            events.push(Event::TurnStarted {
                thread_id,
                turn_id: message_id,
                at: now.to_owned(),
            });
            Ok(events)
        }
        Command::TurnInterrupt { thread_id } => {
            let Some(thread) = model.threads.get(&thread_id) else {
                return Err(CommandError::ThreadNotFound { thread_id });
            };
            if thread.active_turn_id.is_none() {
                return Err(CommandError::ThreadNotWorking { thread_id });
            }
            Ok(Vec::new())
        }
    }
}

/// Turns provider output into events. Output from a turn that is no longer
/// active (interrupted, or the thread was deleted) is dropped.
pub fn record_output(
    model: &ReadModel,
    thread_id: &str,
    turn_id: &str,
    output: ProviderOutput,
    now: &str,
) -> Vec<Event> {
    let active = model
        .threads
        .get(thread_id)
        .is_some_and(|thread| thread.active_turn_id.as_deref() == Some(turn_id));
    if !active {
        return Vec::new();
    }
    let item = |id: String, content: ItemContent| Event::ItemAdded {
        item: ThreadItem {
            id,
            thread_id: thread_id.to_owned(),
            created_at: now.to_owned(),
            content,
        },
    };
    match output {
        ProviderOutput::SessionStarted { session_id } => vec![Event::SessionAttached {
            thread_id: thread_id.to_owned(),
            session_id,
        }],
        ProviderOutput::TextDelta { .. } => Vec::new(),
        ProviderOutput::AssistantMessage { item_id, text } => {
            vec![item(item_id, ItemContent::AssistantMessage { text })]
        }
        ProviderOutput::ToolCallStarted {
            item_id,
            name,
            input,
        } => vec![item(
            item_id,
            ItemContent::ToolCall {
                name,
                input,
                output: None,
                is_error: false,
            },
        )],
        ProviderOutput::ToolCallCompleted {
            item_id,
            output,
            is_error,
        } => vec![Event::ToolCallCompleted {
            thread_id: thread_id.to_owned(),
            item_id,
            output,
            is_error,
        }],
        ProviderOutput::TurnCompleted { outcome } => {
            let mut events = Vec::with_capacity(2);
            if let TurnOutcome::Failed { reason } = &outcome {
                events.push(item(
                    format!("{turn_id}:failed"),
                    ItemContent::TurnFailed {
                        reason: reason.clone(),
                    },
                ));
            }
            events.push(Event::TurnCompleted {
                thread_id: thread_id.to_owned(),
                turn_id: turn_id.to_owned(),
                outcome,
                at: now.to_owned(),
            });
            events
        }
    }
}

/// The failure recorded when a turn cannot start at all.
pub fn turn_failed(reason: TurnFailure) -> ProviderOutput {
    ProviderOutput::TurnCompleted {
        outcome: TurnOutcome::Failed { reason },
    }
}

fn title_from(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    line.chars().take(TITLE_MAX_CHARS).collect()
}

/// Windows paths are case-insensitive and accept either separator.
fn same_path(a: &str, b: &str) -> bool {
    let normalize = |path: &str| {
        let path = path.replace('\\', "/");
        let path = path.trim_end_matches('/');
        if cfg!(windows) {
            path.to_lowercase()
        } else {
            path.to_owned()
        }
    };
    normalize(a) == normalize(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-11-07T12:00:00.000Z";

    fn run(model: &mut ReadModel, command: Command) -> Result<Vec<Event>, CommandError> {
        let events = decide(model, command, NOW)?;
        for event in &events {
            model.apply(event);
        }
        Ok(events)
    }

    fn record(model: &mut ReadModel, turn_id: &str, output: ProviderOutput) -> Vec<Event> {
        let events = record_output(model, "t", turn_id, output, NOW);
        for event in &events {
            model.apply(event);
        }
        events
    }

    fn create_project(id: &str, root: &str) -> Command {
        Command::ProjectCreate {
            project_id: id.into(),
            title: id.into(),
            workspace_root: root.into(),
        }
    }

    fn create_thread(id: &str, project_id: &str) -> Command {
        Command::ThreadCreate {
            thread_id: id.into(),
            project_id: project_id.into(),
            title: String::new(),
        }
    }

    fn send(message_id: &str, text: &str) -> Command {
        Command::MessageSend {
            thread_id: "t".into(),
            message_id: message_id.into(),
            text: text.into(),
        }
    }

    fn model_with_thread() -> ReadModel {
        let mut model = ReadModel::default();
        run(&mut model, create_project("p", "/repo")).unwrap();
        run(&mut model, create_thread("t", "p")).unwrap();
        model
    }

    #[test]
    fn rejects_a_second_project_on_the_same_folder() {
        let mut model = ReadModel::default();
        run(&mut model, create_project("a", "/repo/hoshi")).unwrap();

        let error = run(&mut model, create_project("b", "/repo/hoshi/")).unwrap_err();

        assert_eq!(
            error,
            CommandError::DuplicateWorkspaceRoot {
                project_id: "a".into()
            }
        );
    }

    #[test]
    fn rejects_a_thread_for_a_missing_project() {
        let mut model = ReadModel::default();

        let error = run(&mut model, create_thread("t", "nope")).unwrap_err();

        assert_eq!(
            error,
            CommandError::ProjectNotFound {
                project_id: "nope".into()
            }
        );
    }

    #[test]
    fn removing_a_project_removes_its_threads_and_frees_the_folder() {
        let mut model = ReadModel::default();
        run(&mut model, create_project("p", "/repo")).unwrap();
        run(&mut model, create_project("q", "/other")).unwrap();
        run(&mut model, create_thread("t1", "p")).unwrap();
        run(&mut model, create_thread("t2", "q")).unwrap();

        run(
            &mut model,
            Command::ProjectRemove {
                project_id: "p".into(),
            },
        )
        .unwrap();

        let thread_ids: Vec<_> = model.threads().map(|thread| thread.id.as_str()).collect();
        assert_eq!(thread_ids, ["t2"]);
        run(&mut model, create_project("p2", "/repo")).unwrap();
    }

    #[test]
    fn first_message_titles_the_thread_and_starts_a_turn() {
        let mut model = model_with_thread();

        run(&mut model, send("m1", "  Fix the login bug\nwith details")).unwrap();

        let thread = model.thread("t").unwrap();
        assert_eq!(thread.title, "Fix the login bug");
        assert_eq!(thread.status, ThreadStatus::Working);
        assert_eq!(thread.active_turn_id.as_deref(), Some("m1"));
    }

    #[test]
    fn a_busy_thread_rejects_another_message() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "first")).unwrap();

        let error = run(&mut model, send("m2", "second")).unwrap_err();

        assert_eq!(
            error,
            CommandError::ThreadBusy {
                thread_id: "t".into()
            }
        );
    }

    #[test]
    fn rejects_an_empty_message() {
        let mut model = model_with_thread();

        assert_eq!(
            run(&mut model, send("m1", "   ")).unwrap_err(),
            CommandError::EmptyMessage
        );
    }

    #[test]
    fn interrupting_an_idle_thread_fails() {
        let mut model = model_with_thread();

        let error = run(
            &mut model,
            Command::TurnInterrupt {
                thread_id: "t".into(),
            },
        )
        .unwrap_err();

        assert_eq!(
            error,
            CommandError::ThreadNotWorking {
                thread_id: "t".into()
            }
        );
    }

    #[test]
    fn output_from_a_finished_turn_is_dropped() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "hi")).unwrap();
        record(
            &mut model,
            "m1",
            ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Interrupted,
            },
        );

        let late = record(
            &mut model,
            "m1",
            ProviderOutput::AssistantMessage {
                item_id: "a".into(),
                text: "too late".into(),
            },
        );

        assert!(late.is_empty());
        assert_eq!(model.thread("t").unwrap().status, ThreadStatus::Idle);
    }

    #[test]
    fn a_failed_turn_records_the_reason_and_marks_the_thread() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "hi")).unwrap();

        let events = record(
            &mut model,
            "m1",
            turn_failed(TurnFailure::SpawnFailed {
                detail: "not found".into(),
            }),
        );

        assert!(matches!(
            &events[0],
            Event::ItemAdded { item } if matches!(item.content, ItemContent::TurnFailed { .. })
        ));
        let thread = model.thread("t").unwrap();
        assert_eq!(thread.status, ThreadStatus::Error);
        assert_eq!(thread.active_turn_id, None);
    }

    #[test]
    fn the_session_is_kept_for_the_next_turn() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "hi")).unwrap();

        record(
            &mut model,
            "m1",
            ProviderOutput::SessionStarted {
                session_id: "s1".into(),
            },
        );

        assert_eq!(model.thread("t").unwrap().session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn a_thread_runs_in_its_worktree_when_it_has_one() {
        let mut model = model_with_thread();
        assert_eq!(model.thread_cwd("t"), Some("/repo"));

        model.threads.get_mut("t").unwrap().worktree_path = Some("/worktrees/t".into());

        assert_eq!(model.thread_cwd("t"), Some("/worktrees/t"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_compare_case_and_separator_insensitively() {
        assert!(same_path(r"C:\Repo\Hoshi", "c:/repo/hoshi/"));
    }
}
