//! SQLite persistence for events and the projections the UI reads.
//!
//! Every append writes the events and updates the projection tables in one
//! transaction, so the projections never disagree with the event log. The
//! thread projection must match `ReadModel::apply`; a test checks that.

use hoshi_core::{Effort, Event, Project, Snapshot, Thread, ThreadItem, ThreadStatus, TurnOutcome};
use rusqlite::{Connection, Row, Transaction, params, types::Type};
use std::path::Path;

const MIGRATIONS: &[&str] = &[
    r"
    CREATE TABLE events (
        sequence INTEGER PRIMARY KEY AUTOINCREMENT,
        event_type TEXT NOT NULL,
        payload_json TEXT NOT NULL,
        occurred_at TEXT NOT NULL
    );
    CREATE TABLE projects (
        id TEXT PRIMARY KEY,
        title TEXT NOT NULL,
        workspace_root TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE threads (
        id TEXT PRIMARY KEY,
        project_id TEXT NOT NULL,
        title TEXT NOT NULL,
        status TEXT NOT NULL,
        branch TEXT,
        worktree_path TEXT,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE INDEX threads_project_id ON threads (project_id);
    ",
    r"
    ALTER TABLE threads ADD COLUMN session_id TEXT;
    ALTER TABLE threads ADD COLUMN active_turn_id TEXT;
    CREATE TABLE thread_items (
        ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
        id TEXT NOT NULL,
        thread_id TEXT NOT NULL,
        created_at TEXT NOT NULL,
        content_json TEXT NOT NULL,
        UNIQUE (thread_id, id)
    );
    ",
    r"
    ALTER TABLE threads ADD COLUMN model TEXT;
    ALTER TABLE threads ADD COLUMN effort TEXT;
    ",
];

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self, StoreError> {
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let tx = conn.transaction()?;
        for migration in MIGRATIONS.iter().skip(version as usize) {
            tx.execute_batch(migration)?;
        }
        tx.pragma_update(None, "user_version", MIGRATIONS.len() as i64)?;
        tx.commit()?;
        Ok(Self { conn })
    }

    /// Appends events and updates the projections. Returns each event's sequence.
    pub fn append(&mut self, events: &[Event], occurred_at: &str) -> Result<Vec<u64>, StoreError> {
        let tx = self.conn.transaction()?;
        let mut sequences = Vec::with_capacity(events.len());
        for event in events {
            let payload = serde_json::to_value(event)?;
            let event_type = payload["type"].as_str().unwrap_or_default().to_owned();
            tx.execute(
                "INSERT INTO events (event_type, payload_json, occurred_at) VALUES (?1, ?2, ?3)",
                params![event_type, payload.to_string(), occurred_at],
            )?;
            sequences.push(tx.last_insert_rowid() as u64);
            project(&tx, event)?;
        }
        tx.commit()?;
        Ok(sequences)
    }

    pub fn snapshot(&self) -> Result<Snapshot, StoreError> {
        let sequence: i64 =
            self.conn
                .query_row("SELECT COALESCE(MAX(sequence), 0) FROM events", [], |row| {
                    row.get(0)
                })?;
        let projects = self
            .conn
            .prepare(
                "SELECT id, title, workspace_root, created_at, updated_at
                 FROM projects ORDER BY created_at, id",
            )?
            .query_map([], |row| {
                Ok(Project {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    workspace_root: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let threads = self
            .conn
            .prepare(
                "SELECT id, project_id, title, status, branch, worktree_path, session_id,
                        active_turn_id, created_at, updated_at, model, effort
                 FROM threads ORDER BY created_at, id",
            )?
            .query_map([], |row| {
                Ok(Thread {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    title: row.get(2)?,
                    status: json_column(row, 3)?,
                    branch: row.get(4)?,
                    worktree_path: row.get(5)?,
                    session_id: row.get(6)?,
                    active_turn_id: row.get(7)?,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                    model: row.get(10)?,
                    effort: optional_json_column(row, 11)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Snapshot {
            sequence: sequence as u64,
            projects,
            threads,
        })
    }

    /// A thread's transcript, oldest first.
    pub fn thread_items(&self, thread_id: &str) -> Result<Vec<ThreadItem>, StoreError> {
        Ok(self
            .conn
            .prepare(
                "SELECT id, thread_id, created_at, content_json
                 FROM thread_items WHERE thread_id = ?1 ORDER BY ordinal",
            )?
            .query_map([thread_id], |row| {
                let content: String = row.get(3)?;
                Ok(ThreadItem {
                    id: row.get(0)?,
                    thread_id: row.get(1)?,
                    created_at: row.get(2)?,
                    content: serde_json::from_str(&content).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(error))
                    })?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }
}

fn json_column<T: serde::de::DeserializeOwned>(row: &Row, index: usize) -> rusqlite::Result<T> {
    let text: String = row.get(index)?;
    serde_json::from_value(text.into()).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
    })
}

fn optional_json_column<T: serde::de::DeserializeOwned>(
    row: &Row,
    index: usize,
) -> rusqlite::Result<Option<T>> {
    match row.get::<_, Option<String>>(index)? {
        Some(_) => json_column(row, index).map(Some),
        None => Ok(None),
    }
}

fn effort_text(effort: Option<Effort>) -> Option<&'static str> {
    effort.map(Effort::as_str)
}

fn status_text(status: ThreadStatus) -> Result<String, StoreError> {
    Ok(serde_json::to_value(status)?
        .as_str()
        .unwrap_or_default()
        .to_owned())
}

fn project(tx: &Transaction, event: &Event) -> Result<(), StoreError> {
    match event {
        Event::ProjectCreated { project } => {
            tx.execute(
                "INSERT INTO projects (id, title, workspace_root, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    project.id,
                    project.title,
                    project.workspace_root,
                    project.created_at,
                    project.updated_at
                ],
            )?;
        }
        Event::ProjectRemoved { project_id } => {
            tx.execute(
                "DELETE FROM thread_items WHERE thread_id IN
                 (SELECT id FROM threads WHERE project_id = ?1)",
                [project_id],
            )?;
            tx.execute("DELETE FROM threads WHERE project_id = ?1", [project_id])?;
            tx.execute("DELETE FROM projects WHERE id = ?1", [project_id])?;
        }
        Event::ThreadCreated { thread } => {
            tx.execute(
                "INSERT INTO threads
                 (id, project_id, title, status, branch, worktree_path, session_id,
                  active_turn_id, created_at, updated_at, model, effort)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    thread.id,
                    thread.project_id,
                    thread.title,
                    status_text(thread.status)?,
                    thread.branch,
                    thread.worktree_path,
                    thread.session_id,
                    thread.active_turn_id,
                    thread.created_at,
                    thread.updated_at,
                    thread.model,
                    effort_text(thread.effort)
                ],
            )?;
        }
        Event::ThreadDeleted { thread_id } => {
            tx.execute("DELETE FROM thread_items WHERE thread_id = ?1", [thread_id])?;
            tx.execute("DELETE FROM threads WHERE id = ?1", [thread_id])?;
        }
        Event::ThreadTitleSet { thread_id, title } => {
            tx.execute(
                "UPDATE threads SET title = ?2 WHERE id = ?1",
                params![thread_id, title],
            )?;
        }
        Event::ThreadBranchSet { thread_id, branch } => {
            tx.execute(
                "UPDATE threads SET branch = ?2 WHERE id = ?1",
                params![thread_id, branch],
            )?;
        }
        Event::ThreadModelSet {
            thread_id,
            model,
            effort,
        } => {
            tx.execute(
                "UPDATE threads SET model = ?2, effort = ?3 WHERE id = ?1",
                params![thread_id, model, effort_text(*effort)],
            )?;
        }
        Event::TurnStarted {
            thread_id,
            turn_id,
            at,
        } => {
            tx.execute(
                "UPDATE threads SET status = ?2, active_turn_id = ?3, updated_at = ?4 WHERE id = ?1",
                params![thread_id, status_text(ThreadStatus::Working)?, turn_id, at],
            )?;
        }
        Event::SessionAttached {
            thread_id,
            session_id,
        } => {
            tx.execute(
                "UPDATE threads SET session_id = ?2 WHERE id = ?1",
                params![thread_id, session_id],
            )?;
        }
        Event::ItemAdded { item } => {
            tx.execute(
                "INSERT INTO thread_items (id, thread_id, created_at, content_json)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (thread_id, id) DO UPDATE SET content_json = excluded.content_json",
                params![
                    item.id,
                    item.thread_id,
                    item.created_at,
                    serde_json::to_string(&item.content)?
                ],
            )?;
            tx.execute(
                "UPDATE threads SET updated_at = ?2 WHERE id = ?1",
                params![item.thread_id, item.created_at],
            )?;
        }
        Event::ToolCallCompleted {
            thread_id,
            item_id,
            output,
            is_error,
        } => {
            tx.execute(
                "UPDATE thread_items
                 SET content_json = json_set(content_json, '$.output', ?3, '$.isError', json(?4))
                 WHERE thread_id = ?1 AND id = ?2",
                params![thread_id, item_id, output, is_error.to_string()],
            )?;
        }
        Event::TurnCompleted {
            thread_id,
            outcome,
            at,
            ..
        } => {
            let status = match outcome {
                TurnOutcome::Failed { .. } => ThreadStatus::Error,
                TurnOutcome::Completed | TurnOutcome::Interrupted => ThreadStatus::Idle,
            };
            tx.execute(
                "UPDATE threads SET status = ?2, active_turn_id = NULL, updated_at = ?3
                 WHERE id = ?1",
                params![thread_id, status_text(status)?, at],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoshi_core::{Command, ItemContent, ProviderOutput, ReadModel, decide, record_output};

    const NOW: &str = "2026-11-07T12:00:00.000Z";

    fn project_created(id: &str) -> Event {
        Event::ProjectCreated {
            project: Project {
                id: id.into(),
                title: id.into(),
                workspace_root: format!("/repo/{id}"),
                created_at: NOW.into(),
                updated_at: NOW.into(),
            },
        }
    }

    fn thread_created(id: &str, project_id: &str) -> Event {
        Event::ThreadCreated {
            thread: Thread {
                id: id.into(),
                project_id: project_id.into(),
                title: String::new(),
                status: ThreadStatus::WaitingForInput,
                branch: Some("feat/x".into()),
                worktree_path: Some("/worktrees/x".into()),
                session_id: None,
                model: Some("sonnet".into()),
                effort: Some(Effort::High),
                active_turn_id: None,
                created_at: NOW.into(),
                updated_at: NOW.into(),
            },
        }
    }

    #[test]
    fn snapshot_reflects_appended_events() {
        let mut store = Store::open_in_memory().unwrap();

        let sequences = store
            .append(&[project_created("p"), thread_created("t", "p")], NOW)
            .unwrap();

        assert_eq!(sequences, [1, 2]);
        let snapshot = store.snapshot().unwrap();
        assert_eq!(snapshot.sequence, 2);
        assert_eq!(snapshot.projects.len(), 1);
        let Event::ThreadCreated { thread } = thread_created("t", "p") else {
            unreachable!()
        };
        assert_eq!(snapshot.threads, [thread]);
    }

    fn commit(store: &mut Store, model: &mut ReadModel, events: Vec<Event>) {
        events.iter().for_each(|event| model.apply(event));
        store.append(&events, NOW).unwrap();
    }

    fn store_with_thread() -> (Store, ReadModel) {
        let mut store = Store::open_in_memory().unwrap();
        store
            .append(&[project_created("p"), thread_created("t", "p")], NOW)
            .unwrap();
        let model = ReadModel::from_snapshot(&store.snapshot().unwrap());
        (store, model)
    }

    fn send(model: &ReadModel) -> Vec<Event> {
        let command = Command::MessageSend {
            thread_id: "t".into(),
            message_id: "m1".into(),
            text: "Read the file".into(),
        };
        decide(model, command, NOW).unwrap()
    }

    #[test]
    fn removing_a_project_deletes_its_threads_and_items() {
        let (mut store, mut model) = store_with_thread();
        let events = send(&model);
        commit(&mut store, &mut model, events);

        let remove = Command::ProjectRemove {
            project_id: "p".into(),
        };
        let events = decide(&model, remove, NOW).unwrap();
        commit(&mut store, &mut model, events);

        assert!(store.snapshot().unwrap().threads.is_empty());
        assert!(store.thread_items("t").unwrap().is_empty());
    }

    #[test]
    fn thread_projection_matches_the_read_model_through_a_turn() {
        let (mut store, mut model) = store_with_thread();
        let events = send(&model);
        commit(&mut store, &mut model, events);
        let model_set = Command::ThreadModelSet {
            thread_id: "t".into(),
            model: Some("opus".into()),
            effort: Some(Effort::Max),
        };
        let events = decide(&model, model_set, NOW).unwrap();
        commit(&mut store, &mut model, events);
        let events = hoshi_core::record_branch(&model, "t", Some("main".into()));
        commit(&mut store, &mut model, events);
        let outputs = [
            ProviderOutput::SessionStarted {
                session_id: "s1".into(),
            },
            ProviderOutput::ToolCallStarted {
                item_id: "tool".into(),
                name: "Read".into(),
                input: "{}".into(),
            },
            ProviderOutput::ToolCallCompleted {
                item_id: "tool".into(),
                output: "hello".into(),
                is_error: false,
            },
            ProviderOutput::AssistantMessage {
                item_id: "a".into(),
                text: "It says hello".into(),
            },
            ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Completed,
            },
        ];

        for output in outputs {
            let events = record_output(&model, "t", "m1", output, NOW);
            commit(&mut store, &mut model, events);
            let expected: Vec<_> = model.threads().cloned().collect();
            assert_eq!(store.snapshot().unwrap().threads, expected);
        }

        let thread = model.thread("t").unwrap();
        assert_eq!(thread.status, ThreadStatus::Idle);
        assert_eq!(thread.session_id.as_deref(), Some("s1"));
        let items = store.thread_items("t").unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(
            items[1].content,
            ItemContent::ToolCall {
                name: "Read".into(),
                input: "{}".into(),
                output: Some("hello".into()),
                is_error: false,
            }
        );
    }

    #[test]
    fn reopening_keeps_data_and_does_not_rerun_migrations() {
        let dir = std::env::temp_dir().join(format!("hoshi-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.sqlite");
        let _ = std::fs::remove_file(&path);

        Store::open(&path)
            .unwrap()
            .append(&[project_created("p")], NOW)
            .unwrap();
        let snapshot = Store::open(&path).unwrap().snapshot().unwrap();

        assert_eq!(snapshot.projects.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn upgrades_a_version_one_database() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute(
            "INSERT INTO threads VALUES ('t', 'p', 'old', 'idle', NULL, NULL, ?1, ?1)",
            [NOW],
        )
        .unwrap();

        let store = Store::init(conn).unwrap();

        let thread = &store.snapshot().unwrap().threads[0];
        assert_eq!(thread.title, "old");
        assert_eq!(thread.session_id, None);
        assert_eq!(thread.model, None);
    }
}
