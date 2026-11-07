//! SQLite persistence for events and the projections the UI reads.
//!
//! Every append writes the events and updates the projection tables in one
//! transaction, so the projections never disagree with the event log.

use hoshi_core::{Event, Project, Snapshot, Thread};
use rusqlite::{Connection, Transaction, params, types::Type};
use std::path::Path;

const MIGRATIONS: &[&str] = &[r"
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
"];

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
                "SELECT id, project_id, title, status, branch, worktree_path, created_at, updated_at
                 FROM threads ORDER BY created_at, id",
            )?
            .query_map([], |row| {
                let status: String = row.get(3)?;
                Ok(Thread {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    title: row.get(2)?,
                    status: serde_json::from_value(status.into()).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(error))
                    })?,
                    branch: row.get(4)?,
                    worktree_path: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Snapshot {
            sequence: sequence as u64,
            projects,
            threads,
        })
    }
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
            tx.execute("DELETE FROM threads WHERE project_id = ?1", [project_id])?;
            tx.execute("DELETE FROM projects WHERE id = ?1", [project_id])?;
        }
        Event::ThreadCreated { thread } => {
            let status = serde_json::to_value(thread.status)?;
            tx.execute(
                "INSERT INTO threads
                 (id, project_id, title, status, branch, worktree_path, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    thread.id,
                    thread.project_id,
                    thread.title,
                    status.as_str(),
                    thread.branch,
                    thread.worktree_path,
                    thread.created_at,
                    thread.updated_at
                ],
            )?;
        }
        Event::ThreadDeleted { thread_id } => {
            tx.execute("DELETE FROM threads WHERE id = ?1", [thread_id])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoshi_core::ThreadStatus;

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

    #[test]
    fn removing_a_project_deletes_its_threads() {
        let mut store = Store::open_in_memory().unwrap();
        store
            .append(
                &[
                    project_created("p"),
                    project_created("q"),
                    thread_created("t1", "p"),
                    thread_created("t2", "q"),
                ],
                NOW,
            )
            .unwrap();

        store
            .append(
                &[Event::ProjectRemoved {
                    project_id: "p".into(),
                }],
                NOW,
            )
            .unwrap();

        let snapshot = store.snapshot().unwrap();
        assert_eq!(snapshot.sequence, 5);
        assert_eq!(snapshot.projects[0].id, "q");
        assert_eq!(snapshot.threads.len(), 1);
        assert_eq!(snapshot.threads[0].id, "t2");
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
}
