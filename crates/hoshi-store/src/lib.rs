//! SQLite persistence for events and the projections the UI reads.
//!
//! Every append writes the events and updates the projection tables in one
//! transaction, so the projections never disagree with the event log. The
//! thread projection must match `ReadModel::apply`; a test checks that.

use hoshi_core::{
    ContextUsage, Effort, Event, OpenToolCall, Project, PullRequestDetail, Snapshot, Thread,
    ThreadContextUsage, ThreadItem, ThreadPullRequest, ThreadStatus, TurnOutcome, UsageLimits,
};
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
    r"
    ALTER TABLE threads ADD COLUMN archived INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE threads ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
    ",
    r"
    CREATE TABLE pull_requests (
        thread_id TEXT PRIMARY KEY,
        pull_request_json TEXT NOT NULL
    );
    ",
    r"
    ALTER TABLE threads ADD COLUMN fast_mode INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE threads ADD COLUMN ultracode INTEGER NOT NULL DEFAULT 0;
    ",
    r"
    ALTER TABLE threads ADD COLUMN access_mode TEXT NOT NULL DEFAULT 'acceptEdits';
    ALTER TABLE threads ADD COLUMN plan_mode INTEGER NOT NULL DEFAULT 0;
    ",
    r"
    UPDATE threads SET model = CASE model
        WHEN 'opus' THEN 'claude-opus-5-5'
        WHEN 'sonnet' THEN 'claude-sonnet-5-5'
        WHEN 'fable' THEN 'claude-fable-5-1'
        WHEN 'haiku' THEN 'claude-haiku-5-5'
        ELSE model
    END;
    ",
    r"
    CREATE TABLE pull_request_details (
        thread_id TEXT PRIMARY KEY,
        number INTEGER NOT NULL,
        version TEXT NOT NULL,
        detail_json TEXT NOT NULL
    );
    ",
    r"
    CREATE TABLE context_usages (
        thread_id TEXT PRIMARY KEY,
        used_tokens INTEGER NOT NULL,
        max_tokens INTEGER
    );
    ",
    r"
    ALTER TABLE threads ADD COLUMN worktree_released INTEGER NOT NULL DEFAULT 0;
    ",
    r"
    CREATE TABLE threads_next (
        id TEXT PRIMARY KEY,
        project_id TEXT,
        title TEXT NOT NULL,
        status TEXT NOT NULL,
        branch TEXT,
        worktree_path TEXT,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        session_id TEXT,
        active_turn_id TEXT,
        model TEXT,
        effort TEXT,
        archived INTEGER NOT NULL DEFAULT 0,
        pinned INTEGER NOT NULL DEFAULT 0,
        fast_mode INTEGER NOT NULL DEFAULT 0,
        ultracode INTEGER NOT NULL DEFAULT 0,
        access_mode TEXT NOT NULL DEFAULT 'acceptEdits',
        plan_mode INTEGER NOT NULL DEFAULT 0,
        worktree_released INTEGER NOT NULL DEFAULT 0
    );
    INSERT INTO threads_next (
        id, project_id, title, status, branch, worktree_path, created_at, updated_at,
        session_id, active_turn_id, model, effort, archived, pinned, fast_mode, ultracode,
        access_mode, plan_mode, worktree_released
    )
    SELECT
        id, project_id, title, status, branch, worktree_path, created_at, updated_at,
        session_id, active_turn_id, model, effort, archived, pinned, fast_mode, ultracode,
        access_mode, plan_mode, worktree_released
    FROM threads;
    DROP TABLE threads;
    ALTER TABLE threads_next RENAME TO threads;
    CREATE INDEX threads_project_id ON threads (project_id);
    ",
    r"
    CREATE TABLE usage_limits (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        limits_json TEXT NOT NULL
    );
    ",
    r"
    ALTER TABLE threads ADD COLUMN worktree_adopted INTEGER NOT NULL DEFAULT 0;
    ",
    // Tool calls left without output by turns that ended before calls were marked interrupted.
    r"
    UPDATE thread_items
    SET content_json = json_set(content_json, '$.interrupted', json('true'))
    WHERE json_extract(content_json, '$.kind') = 'toolCall'
      AND json_extract(content_json, '$.output') IS NULL
      AND NOT EXISTS (
        SELECT 1 FROM threads
        WHERE threads.active_turn_id IS NOT NULL
          AND (threads.id = thread_items.thread_id
               OR substr(thread_items.thread_id, 1, length(threads.id) + 1) = threads.id || ':')
      );
    ",
    r"
    ALTER TABLE threads ADD COLUMN parent_thread_id TEXT;
    ",
    r"
    ALTER TABLE threads ADD COLUMN task_cancelled INTEGER NOT NULL DEFAULT 0;
    ",
    r"
    ALTER TABLE threads ADD COLUMN task_answered INTEGER NOT NULL DEFAULT 0;
    ",
    r"
    ALTER TABLE threads ADD COLUMN conversation_cursor TEXT;
    ALTER TABLE threads ADD COLUMN resume_at TEXT;
    ",
    r"
    ALTER TABLE threads ADD COLUMN watching_pull_request INTEGER NOT NULL DEFAULT 0;
    ",
];

/// The last detail read of a thread's pull request, to show at once on the next start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedPullRequestDetail {
    pub thread_id: String,
    pub number: u32,
    /// The pull request's `updated_at` when it was read.
    pub version: String,
    pub detail: PullRequestDetail,
}

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
                        active_turn_id, created_at, updated_at, model, effort, archived, pinned,
                        fast_mode, ultracode, access_mode, plan_mode, worktree_released,
                        worktree_adopted, parent_thread_id, task_cancelled, task_answered,
                        conversation_cursor, resume_at, watching_pull_request
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
                    archived: row.get(12)?,
                    pinned: row.get(13)?,
                    fast_mode: row.get(14)?,
                    ultracode: row.get(15)?,
                    access_mode: json_column(row, 16)?,
                    plan_mode: row.get(17)?,
                    worktree_released: row.get(18)?,
                    worktree_adopted: row.get(19)?,
                    parent_thread_id: row.get(20)?,
                    task_cancelled: row.get(21)?,
                    task_answered: row.get(22)?,
                    conversation_cursor: row.get(23)?,
                    resume_at: row.get(24)?,
                    watching_pull_request: row.get(25)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let pull_requests = self
            .conn
            .prepare("SELECT thread_id, pull_request_json FROM pull_requests")?
            .query_map([], |row| {
                Ok(ThreadPullRequest {
                    thread_id: row.get(0)?,
                    pull_request: Some(json_document_column(row, 1)?),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let context_usages = self
            .conn
            .prepare("SELECT thread_id, used_tokens, max_tokens FROM context_usages")?
            .query_map([], |row| {
                Ok(ThreadContextUsage {
                    thread_id: row.get(0)?,
                    usage: ContextUsage {
                        used_tokens: row.get::<_, i64>(1)? as u64,
                        max_tokens: row.get::<_, Option<i64>>(2)?.map(|tokens| tokens as u64),
                    },
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let pending_requests = self.items(
            "SELECT id, thread_id, created_at, content_json FROM thread_items
             WHERE json_extract(content_json, '$.status') = 'pending'
               AND json_extract(content_json, '$.kind') IN ('approval', 'questions', 'proposedPlan')
             ORDER BY ordinal",
            [],
        )?;
        let open_tool_calls = self
            .conn
            .prepare(
                "SELECT thread_id, id FROM thread_items
                 WHERE json_extract(content_json, '$.kind') = 'toolCall'
                   AND json_extract(content_json, '$.output') IS NULL
                   AND coalesce(json_extract(content_json, '$.interrupted'), 0) = 0
                 ORDER BY ordinal",
            )?
            .query_map([], |row| {
                Ok(OpenToolCall {
                    thread_id: row.get(0)?,
                    item_id: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Snapshot {
            sequence: sequence as u64,
            projects,
            threads,
            pull_requests,
            context_usages,
            pending_requests,
            open_tool_calls,
            // The engine knows which tasks still owe an answer.
            pending_tasks: Vec::new(),
        })
    }

    /// Caches a thread's context usage. Not an event: it changes on every model call.
    pub fn save_context_usage(&mut self, update: &ThreadContextUsage) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO context_usages (thread_id, used_tokens, max_tokens)
             VALUES (?1, ?2, ?3)",
            params![
                update.thread_id,
                update.usage.used_tokens as i64,
                update.usage.max_tokens.map(|tokens| tokens as i64)
            ],
        )?;
        Ok(())
    }

    /// The provider plan's limits as last read, to show at once on the next start.
    pub fn usage_limits(&self) -> Result<Option<UsageLimits>, StoreError> {
        let limits = self
            .conn
            .prepare("SELECT limits_json FROM usage_limits WHERE id = 1")?
            .query_map([], |row| json_document_column(row, 0))?
            .next()
            .transpose()?;
        Ok(limits)
    }

    pub fn save_usage_limits(&mut self, limits: &UsageLimits) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO usage_limits (id, limits_json) VALUES (1, ?1)",
            [serde_json::to_string(limits)?],
        )?;
        Ok(())
    }

    pub fn save_pull_request_detail(
        &mut self,
        cached: &CachedPullRequestDetail,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO pull_request_details (thread_id, number, version, detail_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                cached.thread_id,
                cached.number,
                cached.version,
                serde_json::to_string(&cached.detail)?
            ],
        )?;
        Ok(())
    }

    pub fn pull_request_details(&self) -> Result<Vec<CachedPullRequestDetail>, StoreError> {
        let details = self
            .conn
            .prepare("SELECT thread_id, number, version, detail_json FROM pull_request_details")?
            .query_map([], |row| {
                Ok(CachedPullRequestDetail {
                    thread_id: row.get(0)?,
                    number: row.get(1)?,
                    version: row.get(2)?,
                    detail: json_document_column(row, 3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(details)
    }

    /// Caches what a forge last said about a thread's pull request. Not an event: it is
    /// read again from the forge, and polling would flood the event log.
    pub fn save_pull_request(&mut self, update: &ThreadPullRequest) -> Result<(), StoreError> {
        match &update.pull_request {
            Some(pull_request) => self.conn.execute(
                "INSERT OR REPLACE INTO pull_requests (thread_id, pull_request_json)
                 VALUES (?1, ?2)",
                params![update.thread_id, serde_json::to_string(pull_request)?],
            )?,
            None => self.conn.execute(
                "DELETE FROM pull_requests WHERE thread_id = ?1",
                [&update.thread_id],
            )?,
        };
        Ok(())
    }

    /// One item of a thread's transcript, whole.
    pub fn thread_item(
        &self,
        thread_id: &str,
        item_id: &str,
    ) -> Result<Option<ThreadItem>, StoreError> {
        Ok(self
            .items(
                "SELECT id, thread_id, created_at, content_json
                 FROM thread_items WHERE thread_id = ?1 AND id = ?2",
                [thread_id, item_id],
            )?
            .pop())
    }

    /// The last `turns` turns before item `before` (the end when `None`), at most
    /// `max_items` of their items, oldest first; and whether older items remain.
    pub fn transcript_page(
        &self,
        thread_id: &str,
        before: Option<&str>,
        turns: usize,
        max_items: usize,
    ) -> Result<(Vec<ThreadItem>, bool), StoreError> {
        let ordinal_of = |id: &str| -> Result<Option<i64>, StoreError> {
            Ok(self
                .conn
                .prepare("SELECT ordinal FROM thread_items WHERE thread_id = ?1 AND id = ?2")?
                .query_map([thread_id, id], |row| row.get(0))?
                .next()
                .transpose()?)
        };
        let end = match before {
            Some(id) => ordinal_of(id)?.unwrap_or(i64::MAX),
            None => i64::MAX,
        };
        let start: i64 = self
            .conn
            .prepare(
                "SELECT ordinal FROM thread_items
                 WHERE thread_id = ?1 AND ordinal < ?2
                   AND json_extract(content_json, '$.kind') = 'userMessage'
                 ORDER BY ordinal DESC LIMIT 1 OFFSET ?3",
            )?
            .query_map(
                params![thread_id, end, turns.saturating_sub(1) as i64],
                |row| row.get(0),
            )?
            .next()
            .transpose()?
            .unwrap_or(i64::MIN);
        let mut items = self.items(
            "SELECT id, thread_id, created_at, content_json FROM thread_items
             WHERE thread_id = ?1 AND ordinal >= ?2 AND ordinal < ?3
             ORDER BY ordinal DESC LIMIT ?4",
            params![thread_id, start, end, max_items as i64],
        )?;
        items.reverse();
        let first = match items.first() {
            Some(item) => ordinal_of(&item.id)?.unwrap_or(i64::MIN),
            None => end,
        };
        let has_older: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM thread_items WHERE thread_id = ?1 AND ordinal < ?2)",
            params![thread_id, first],
            |row| row.get(0),
        )?;
        Ok((items, has_older))
    }

    /// The thread's latest answer, which is what a delegated task reports back.
    pub fn last_answer(&self, thread_id: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .prepare(
                "SELECT json_extract(content_json, '$.text') FROM thread_items
                 WHERE thread_id = ?1
                   AND json_extract(content_json, '$.kind') = 'assistantMessage'
                 ORDER BY ordinal DESC LIMIT 1",
            )?
            .query_map([thread_id], |row| row.get(0))?
            .next()
            .transpose()?)
    }

    /// The thread's latest proposed plan, in markdown.
    pub fn last_plan(&self, thread_id: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .prepare(
                "SELECT json_extract(content_json, '$.plan') FROM thread_items
                 WHERE thread_id = ?1
                   AND json_extract(content_json, '$.kind') = 'proposedPlan'
                 ORDER BY ordinal DESC LIMIT 1",
            )?
            .query_map([thread_id], |row| row.get(0))?
            .next()
            .transpose()?)
    }

    /// Whether the user sent the thread's plan back to planning since their last message.
    pub fn plan_declined_last(&self, thread_id: &str) -> Result<bool, StoreError> {
        let decision: Option<Option<String>> = self
            .conn
            .prepare(
                "SELECT json_extract(content_json, '$.decision') FROM thread_items
                 WHERE thread_id = ?1
                   AND json_extract(content_json, '$.kind') IN ('userMessage', 'proposedPlan')
                 ORDER BY ordinal DESC LIMIT 1",
            )?
            .query_map([thread_id], |row| row.get(0))?
            .next()
            .transpose()?;
        Ok(decision.flatten().as_deref() == Some("decline"))
    }

    /// The delegated tasks whose answer already went back to the thread that delegated them.
    pub fn answered_tasks(&self) -> Result<Vec<String>, StoreError> {
        Ok(self
            .conn
            .prepare(
                "SELECT json_extract(content_json, '$.taskResult') FROM thread_items
                 WHERE json_extract(content_json, '$.kind') = 'userMessage'
                   AND json_extract(content_json, '$.taskResult') IS NOT NULL",
            )?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?)
    }

    /// A thread's transcript, oldest first.
    pub fn thread_items(&self, thread_id: &str) -> Result<Vec<ThreadItem>, StoreError> {
        self.items(
            "SELECT id, thread_id, created_at, content_json
             FROM thread_items WHERE thread_id = ?1 ORDER BY ordinal",
            [thread_id],
        )
    }

    fn items(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
    ) -> Result<Vec<ThreadItem>, StoreError> {
        Ok(self
            .conn
            .prepare(sql)?
            .query_map(params, |row| {
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

/// A column holding a whole JSON document, not a single JSON string.
fn json_document_column<T: serde::de::DeserializeOwned>(
    row: &Row,
    index: usize,
) -> rusqlite::Result<T> {
    let text: String = row.get(index)?;
    serde_json::from_str(&text).map_err(|error| {
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

fn enum_text(value: impl serde::Serialize) -> Result<String, StoreError> {
    Ok(serde_json::to_value(value)?
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
                 (SELECT id FROM threads WHERE project_id = ?1)
                 OR substr(thread_id, 1, instr(thread_id, ':') - 1) IN
                 (SELECT id FROM threads WHERE project_id = ?1)",
                [project_id],
            )?;
            tx.execute(
                "DELETE FROM pull_requests WHERE thread_id IN
                 (SELECT id FROM threads WHERE project_id = ?1)",
                [project_id],
            )?;
            tx.execute(
                "DELETE FROM pull_request_details WHERE thread_id IN
                 (SELECT id FROM threads WHERE project_id = ?1)",
                [project_id],
            )?;
            tx.execute(
                "DELETE FROM context_usages WHERE thread_id IN
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
                  active_turn_id, created_at, updated_at, model, effort, archived, pinned,
                  fast_mode, ultracode, access_mode, plan_mode, worktree_released,
                  worktree_adopted, parent_thread_id, task_cancelled, task_answered,
                  conversation_cursor, resume_at, watching_pull_request)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                         ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
                params![
                    thread.id,
                    thread.project_id,
                    thread.title,
                    enum_text(thread.status)?,
                    thread.branch,
                    thread.worktree_path,
                    thread.session_id,
                    thread.active_turn_id,
                    thread.created_at,
                    thread.updated_at,
                    thread.model,
                    effort_text(thread.effort),
                    thread.archived,
                    thread.pinned,
                    thread.fast_mode,
                    thread.ultracode,
                    enum_text(thread.access_mode)?,
                    thread.plan_mode,
                    thread.worktree_released,
                    thread.worktree_adopted,
                    thread.parent_thread_id,
                    thread.task_cancelled,
                    thread.task_answered,
                    thread.conversation_cursor,
                    thread.resume_at,
                    thread.watching_pull_request
                ],
            )?;
        }
        Event::ThreadDeleted { thread_id } => {
            tx.execute(
                "DELETE FROM thread_items
                 WHERE thread_id = ?1 OR substr(thread_id, 1, length(?1) + 1) = ?1 || ':'",
                [thread_id],
            )?;
            tx.execute(
                "DELETE FROM pull_requests WHERE thread_id = ?1",
                [thread_id],
            )?;
            tx.execute(
                "DELETE FROM pull_request_details WHERE thread_id = ?1",
                [thread_id],
            )?;
            tx.execute(
                "DELETE FROM context_usages WHERE thread_id = ?1",
                [thread_id],
            )?;
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
        Event::ThreadWorktreeAttached {
            thread_id,
            worktree_path,
            branch,
            adopted,
        } => {
            tx.execute(
                "UPDATE threads SET worktree_path = ?2, branch = ?3, worktree_released = 0,
                 worktree_adopted = ?4 WHERE id = ?1",
                params![thread_id, worktree_path, branch, adopted],
            )?;
        }
        Event::ThreadWorktreeDetached { thread_id, branch } => {
            tx.execute(
                "UPDATE threads SET worktree_path = NULL, branch = ?2, worktree_released = 0,
                 worktree_adopted = 0 WHERE id = ?1",
                params![thread_id, branch],
            )?;
        }
        Event::ThreadWorktreeReleased { thread_id } => {
            tx.execute(
                "UPDATE threads SET worktree_released = 1 WHERE id = ?1",
                [thread_id],
            )?;
        }
        Event::ThreadWorktreeRestored { thread_id } => {
            tx.execute(
                "UPDATE threads SET worktree_released = 0 WHERE id = ?1",
                [thread_id],
            )?;
        }
        Event::ThreadCleanedUp { thread_id } => {
            tx.execute(
                "UPDATE threads SET worktree_path = NULL, branch = NULL, session_id = NULL,
                 worktree_released = 0, worktree_adopted = 0 WHERE id = ?1",
                [thread_id],
            )?;
        }
        Event::ThreadArchivedSet {
            thread_id,
            archived,
        } => {
            tx.execute(
                "UPDATE threads SET archived = ?2 WHERE id = ?1",
                params![thread_id, archived],
            )?;
        }
        Event::ThreadPinnedSet { thread_id, pinned } => {
            tx.execute(
                "UPDATE threads SET pinned = ?2 WHERE id = ?1",
                params![thread_id, pinned],
            )?;
        }
        Event::ThreadModelSet {
            thread_id,
            model,
            effort,
            fast_mode,
            ultracode,
        } => {
            tx.execute(
                "UPDATE threads SET model = ?2, effort = ?3, fast_mode = ?4, ultracode = ?5
                 WHERE id = ?1",
                params![thread_id, model, effort_text(*effort), fast_mode, ultracode],
            )?;
        }
        Event::ThreadAccessSet {
            thread_id,
            access_mode,
            plan_mode,
        } => {
            tx.execute(
                "UPDATE threads SET access_mode = ?2, plan_mode = ?3 WHERE id = ?1",
                params![thread_id, enum_text(*access_mode)?, plan_mode],
            )?;
        }
        Event::TurnStarted {
            thread_id,
            turn_id,
            at,
        } => {
            tx.execute(
                "UPDATE threads SET status = ?2, active_turn_id = ?3, updated_at = ?4 WHERE id = ?1",
                params![thread_id, enum_text(ThreadStatus::Working)?, turn_id, at],
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
            detail,
        } => {
            tx.execute(
                "UPDATE thread_items
                 SET content_json = json_set(content_json, '$.output', ?3, '$.isError', json(?4),
                                             '$.detail', json(?5))
                 WHERE thread_id = ?1 AND id = ?2",
                params![
                    thread_id,
                    item_id,
                    output,
                    is_error.to_string(),
                    serde_json::to_string(detail)?
                ],
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
                params![thread_id, enum_text(status)?, at],
            )?;
        }
        Event::ToolCallInterrupted { thread_id, item_id } => {
            tx.execute(
                "UPDATE thread_items
                 SET content_json = json_set(content_json, '$.interrupted', json('true'))
                 WHERE thread_id = ?1 AND id = ?2",
                params![thread_id, item_id],
            )?;
        }
        Event::ThreadDelegated {
            thread_id,
            parent_thread_id,
            shares_worktree,
        } => {
            tx.execute(
                "UPDATE threads SET parent_thread_id = ?2,
                 worktree_adopted = worktree_adopted OR ?3 WHERE id = ?1",
                params![thread_id, parent_thread_id, shares_worktree],
            )?;
        }
        Event::TaskCancelled { thread_id } => {
            tx.execute(
                "UPDATE threads SET task_cancelled = 1 WHERE id = ?1",
                [thread_id],
            )?;
        }
        Event::TaskAnswered { thread_id } => {
            tx.execute(
                "UPDATE threads SET task_answered = 1 WHERE id = ?1",
                [thread_id],
            )?;
        }
        Event::ThreadStatusSet { thread_id, status } => {
            tx.execute(
                "UPDATE threads SET status = ?2 WHERE id = ?1",
                params![thread_id, enum_text(*status)?],
            )?;
        }
        Event::PullRequestWatchSet {
            thread_id,
            watching,
        } => {
            tx.execute(
                "UPDATE threads SET watching_pull_request = ?2 WHERE id = ?1",
                params![thread_id, watching],
            )?;
        }
        Event::ConversationCursorSet { thread_id, cursor } => {
            tx.execute(
                "UPDATE threads SET conversation_cursor = ?2, resume_at = NULL WHERE id = ?1",
                params![thread_id, cursor],
            )?;
        }
        Event::ThreadRewound {
            thread_id,
            message_id,
            resume_at,
        } => {
            tx.execute(
                "DELETE FROM thread_items
                 WHERE (thread_id = ?1 OR substr(thread_id, 1, length(?1) + 1) = ?1 || ':')
                   AND ordinal >= (SELECT ordinal FROM thread_items
                                   WHERE thread_id = ?1 AND id = ?2)",
                params![thread_id, message_id],
            )?;
            tx.execute(
                "UPDATE threads SET conversation_cursor = ?2, resume_at = ?2,
                 session_id = CASE WHEN ?2 IS NULL THEN NULL ELSE session_id END
                 WHERE id = ?1",
                params![thread_id, resume_at],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoshi_core::{
        AccessMode, ChecksState, Command, ForgeKind, ItemContent, Mergeability, ProviderOutput,
        PullRequest, PullRequestState, ReadModel, ToolDetail, UsageWindow, UsageWindowKind, decide,
        record_output,
    };

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
                project_id: Some(project_id.into()),
                title: String::new(),
                status: ThreadStatus::WaitingForInput,
                branch: Some("feat/x".into()),
                worktree_path: Some("/worktrees/x".into()),
                session_id: None,
                conversation_cursor: None,
                resume_at: None,
                watching_pull_request: false,
                model: Some("sonnet".into()),
                effort: Some(Effort::High),
                fast_mode: true,
                ultracode: false,
                access_mode: AccessMode::Auto,
                plan_mode: true,
                active_turn_id: None,
                archived: false,
                pinned: true,
                worktree_released: false,
                worktree_adopted: false,
                parent_thread_id: None,
                task_cancelled: false,
                task_answered: false,
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
    fn a_chat_is_kept_without_a_project_and_outlives_project_removal() {
        let mut store = Store::open_in_memory().unwrap();
        let Event::ThreadCreated { mut thread } = thread_created("chat", "p") else {
            unreachable!()
        };
        thread.project_id = None;
        store
            .append(
                &[
                    project_created("p"),
                    Event::ThreadCreated {
                        thread: thread.clone(),
                    },
                    Event::ProjectRemoved {
                        project_id: "p".into(),
                    },
                ],
                NOW,
            )
            .unwrap();

        assert_eq!(store.snapshot().unwrap().threads, [thread]);
    }

    #[test]
    fn threads_from_before_chats_keep_their_project() {
        let mut conn = Connection::open_in_memory().unwrap();
        let tx = conn.transaction().unwrap();
        for migration in &MIGRATIONS[..MIGRATIONS.len() - 1] {
            tx.execute_batch(migration).unwrap();
        }
        tx.execute(
            "INSERT INTO threads (id, project_id, title, status, created_at, updated_at, pinned)
             VALUES ('t', 'p', 'old', 'idle', ?1, ?1, 1)",
            [NOW],
        )
        .unwrap();
        tx.pragma_update(None, "user_version", MIGRATIONS.len() as i64 - 1)
            .unwrap();
        tx.commit().unwrap();

        let store = Store::init(conn).unwrap();

        let threads = store.snapshot().unwrap().threads;
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].project_id.as_deref(), Some("p"));
        assert!(threads[0].pinned);
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

    #[test]
    fn a_cached_detail_survives_a_restart_and_goes_with_its_thread() {
        let (mut store, model) = store_with_thread();
        let cached = CachedPullRequestDetail {
            thread_id: "t".into(),
            number: 3,
            version: NOW.into(),
            detail: PullRequestDetail {
                supports: hoshi_core::PullRequestSupport::all(),
                forge_id: "PR_1".into(),
                repository: "o/r".into(),
                mergeability: Mergeability::Mergeable,
                merge_methods: Vec::new(),
                can_edit: true,
                body: "## Summary".into(),
                changed_files: 2,
                reviewers: Vec::new(),
                labels: Vec::new(),
                checks: Vec::new(),
                review_threads: Vec::new(),
                timeline: Vec::new(),
            },
        };

        store.save_pull_request_detail(&cached).unwrap();
        assert_eq!(store.pull_request_details().unwrap(), [cached]);

        let delete = Command::ThreadDelete {
            thread_id: "t".into(),
        };
        store
            .append(&decide(&model, delete, NOW).unwrap(), NOW)
            .unwrap();
        assert!(store.pull_request_details().unwrap().is_empty());
    }

    #[test]
    fn a_cached_pull_request_survives_a_restart_and_goes_with_its_thread() {
        let (mut store, model) = store_with_thread();
        let update = ThreadPullRequest {
            thread_id: "t".into(),
            pull_request: Some(PullRequest {
                forge: ForgeKind::Github,
                number: 42,
                title: "Fix".into(),
                url: "https://github.com/o/r/pull/42".into(),
                author: Some("me".into()),
                head_branch: "feat/x".into(),
                head_sha: None,
                base_branch: "main".into(),
                state: PullRequestState::Open,
                checks: Some(ChecksState::Failing),
                unresolved_comments: 3,
                additions: 1,
                deletions: 2,
                updated_at: NOW.into(),
            }),
        };

        store.save_pull_request(&update).unwrap();
        assert_eq!(store.snapshot().unwrap().pull_requests, [update]);

        let delete = Command::ThreadDelete {
            thread_id: "t".into(),
        };
        store
            .append(&decide(&model, delete, NOW).unwrap(), NOW)
            .unwrap();
        assert!(store.snapshot().unwrap().pull_requests.is_empty());
    }

    #[test]
    fn a_context_usage_survives_a_restart_and_goes_with_its_thread() {
        let (mut store, model) = store_with_thread();
        let first = ThreadContextUsage {
            thread_id: "t".into(),
            usage: ContextUsage {
                used_tokens: 9_000,
                max_tokens: None,
            },
        };
        let latest = ThreadContextUsage {
            thread_id: "t".into(),
            usage: ContextUsage {
                used_tokens: 12_000,
                max_tokens: Some(1_000_000),
            },
        };

        store.save_context_usage(&first).unwrap();
        store.save_context_usage(&latest).unwrap();
        assert_eq!(store.snapshot().unwrap().context_usages, [latest]);

        let delete = Command::ThreadDelete {
            thread_id: "t".into(),
        };
        store
            .append(&decide(&model, delete, NOW).unwrap(), NOW)
            .unwrap();
        assert!(store.snapshot().unwrap().context_usages.is_empty());
    }

    #[test]
    fn the_last_usage_limits_replace_the_previous_ones() {
        let mut store = Store::open_in_memory().unwrap();
        assert_eq!(store.usage_limits().unwrap(), None);
        let window = |used_percent| UsageLimits {
            windows: vec![UsageWindow {
                kind: UsageWindowKind::Session,
                model: None,
                used_percent,
                window_minutes: 300,
                resets_at: Some("2026-11-08T12:00:00Z".into()),
            }],
        };

        store.save_usage_limits(&window(20)).unwrap();
        store.save_usage_limits(&window(35)).unwrap();

        assert_eq!(store.usage_limits().unwrap(), Some(window(35)));
    }

    #[test]
    fn an_adopted_worktree_survives_a_restart_and_leaving_it_clears_it() {
        let (mut store, mut model) = store_with_thread();
        let attached = hoshi_core::record_worktree(
            &model,
            "t",
            "/home/me/repo-pr".into(),
            Some("feat/pr".into()),
            true,
        );
        commit(&mut store, &mut model, attached);

        let thread = &store.snapshot().unwrap().threads[0];
        assert_eq!(thread.worktree_path.as_deref(), Some("/home/me/repo-pr"));
        assert!(thread.worktree_adopted);

        let left = hoshi_core::record_worktree_left(&model, "t", Some("main".into()));
        commit(&mut store, &mut model, left);

        let thread = &store.snapshot().unwrap().threads[0];
        assert_eq!(thread.worktree_path, None);
        assert_eq!(thread.branch.as_deref(), Some("main"));
        assert!(!thread.worktree_adopted);
    }

    #[test]
    fn archiving_survives_a_restart() {
        let (mut store, mut model) = store_with_thread();
        let command = Command::ThreadArchive {
            thread_id: "t".into(),
            archived: true,
        };

        let events = decide(&model, command, NOW).unwrap();
        commit(&mut store, &mut model, events);

        let thread = &store.snapshot().unwrap().threads[0];
        assert!(thread.archived);
        assert!(!thread.pinned);
    }

    fn send_text(message_id: &str) -> Command {
        Command::MessageSend {
            thread_id: "t".into(),
            message_id: message_id.into(),
            text: "Read the file".into(),
            attachments: Vec::new(),
        }
    }

    fn send(model: &ReadModel) -> Vec<Event> {
        let command = Command::MessageSend {
            thread_id: "t".into(),
            message_id: "m1".into(),
            text: "Read the file".into(),
            attachments: Vec::new(),
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
            fast_mode: true,
            ultracode: true,
        };
        let events = decide(&model, model_set, NOW).unwrap();
        commit(&mut store, &mut model, events);
        let access_set = Command::ThreadAccessSet {
            thread_id: "t".into(),
            access_mode: AccessMode::FullAccess,
            plan_mode: false,
        };
        let events = decide(&model, access_set, NOW).unwrap();
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
                detail: Some(change()),
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
                detail: Some(change()),
                output_omitted: false,
                interrupted: false,
            }
        );
    }

    #[test]
    fn a_rewind_drops_the_message_and_what_followed_and_survives_a_restart() {
        let (mut store, mut model) = store_with_thread();
        let turn = |model: &ReadModel, message_id: &str| {
            let mut events = decide(model, send_text(message_id), NOW).unwrap();
            events.push(Event::ConversationCursorSet {
                thread_id: "t".into(),
                cursor: format!("u-{message_id}"),
            });
            events
        };
        let events = turn(&model, "m1");
        commit(&mut store, &mut model, events);
        let finished = |model: &ReadModel, turn_id: &str| {
            let done = ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Completed,
            };
            record_output(model, "t", turn_id, done, NOW)
        };
        let events = finished(&model, "m1");
        commit(&mut store, &mut model, events);
        let events = turn(&model, "m2");
        commit(&mut store, &mut model, events);
        let inside = ProviderOutput::Subagent {
            tool_use_id: "toolu_agent".into(),
            output: Box::new(ProviderOutput::AssistantMessage {
                item_id: "a".into(),
                text: "later".into(),
            }),
        };
        let events = record_output(&model, "t", "m2", inside, NOW);
        commit(&mut store, &mut model, events);
        let events = finished(&model, "m2");
        commit(&mut store, &mut model, events);

        let rewind = Command::ThreadRewind {
            thread_id: "t".into(),
            message_id: "m2".into(),
            rewind: hoshi_core::Rewind::After {
                cursor: "u-m1".into(),
            },
        };
        let events = decide(&model, rewind, NOW).unwrap();
        commit(&mut store, &mut model, events);

        let ids: Vec<_> = store
            .thread_items("t")
            .unwrap()
            .into_iter()
            .map(|item| item.id)
            .collect();
        assert_eq!(ids, ["m1"]);
        assert!(store.thread_items("t:toolu_agent").unwrap().is_empty());
        let thread = store.snapshot().unwrap().threads.remove(0);
        assert_eq!(&thread, model.thread("t").unwrap());
        assert_eq!(thread.resume_at.as_deref(), Some("u-m1"));
    }

    #[test]
    fn deleting_a_thread_deletes_its_subagent_transcripts() {
        let (mut store, mut model) = store_with_thread();
        let events = decide(&model, send_text("m1"), NOW).unwrap();
        commit(&mut store, &mut model, events);
        let inside = ProviderOutput::Subagent {
            tool_use_id: "toolu_agent".into(),
            output: Box::new(ProviderOutput::AssistantMessage {
                item_id: "a".into(),
                text: "3 files".into(),
            }),
        };
        let events = record_output(&model, "t", "m1", inside, NOW);
        commit(&mut store, &mut model, events);
        assert_eq!(store.thread_items("t:toolu_agent").unwrap().len(), 1);

        let events = decide(
            &model,
            Command::ThreadDelete {
                thread_id: "t".into(),
            },
            NOW,
        )
        .unwrap();
        commit(&mut store, &mut model, events);

        assert!(store.thread_items("t:toolu_agent").unwrap().is_empty());
    }

    #[test]
    fn a_page_holds_the_latest_turns_and_says_whether_older_ones_remain() {
        let (mut store, mut model) = store_with_thread();
        for turn in 0..5 {
            let events = decide(&model, send_text(&format!("m{turn}")), NOW).unwrap();
            commit(&mut store, &mut model, events);
            let reply = ProviderOutput::AssistantMessage {
                item_id: format!("a{turn}"),
                text: "ok".into(),
            };
            let done = ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Completed,
            };
            for output in [reply, done] {
                let events = record_output(&model, "t", &format!("m{turn}"), output, NOW);
                commit(&mut store, &mut model, events);
            }
        }
        let ids = |items: &[ThreadItem]| -> Vec<String> {
            items.iter().map(|item| item.id.clone()).collect()
        };

        let (latest, older) = store.transcript_page("t", None, 2, 100).unwrap();
        assert_eq!(ids(&latest), ["m3", "a3", "m4", "a4"]);
        assert!(older);

        let (earlier, older) = store.transcript_page("t", Some("m3"), 2, 100).unwrap();
        assert_eq!(ids(&earlier), ["m1", "a1", "m2", "a2"]);
        assert!(older);

        let (first, older) = store.transcript_page("t", Some("m1"), 2, 100).unwrap();
        assert_eq!(ids(&first), ["m0", "a0"]);
        assert!(!older);

        let (capped, older) = store.transcript_page("t", None, 2, 3).unwrap();
        assert_eq!(ids(&capped), ["a3", "m4", "a4"]);
        assert!(older);
    }

    fn change() -> ToolDetail {
        ToolDetail::FileChange {
            path: "/repo/a.txt".into(),
            additions: 1,
            deletions: 0,
            diff: Some("@@ -0,0 +1,1 @@\n+a".into()),
        }
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
