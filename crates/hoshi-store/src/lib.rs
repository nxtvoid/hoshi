//! SQLite persistence for events and the projections the UI reads.
//!
//! Every append writes the events and updates the projection tables in one
//! transaction, so the projections never disagree with the event log. The
//! thread projection must match `ReadModel::apply`; a test checks that.

use hoshi_core::{
    ContextUsage, Effort, Event, MessageAttachment, ModelChoice, OpenToolCall, Project,
    PullRequestDetail, PullRequestWatch, Snapshot, TaskPreview, Thread, ThreadContextUsage,
    ThreadItem, ThreadPullRequest, ThreadStatus, TurnOutcome, UsageLimits, task_preview,
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params, types::Type};
use std::collections::HashMap;
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
    r"
    ALTER TABLE context_usages ADD COLUMN cache_ttl_secs INTEGER;
    ALTER TABLE context_usages ADD COLUMN cache_expires_at INTEGER;
    ",
    r"
    ALTER TABLE projects ADD COLUMN worktree_setup TEXT;
    ",
    r"
    CREATE INDEX thread_items_thread_ordinal ON thread_items (thread_id, ordinal);
    CREATE INDEX thread_items_thread_kind
        ON thread_items (thread_id, json_extract(content_json, '$.kind'), ordinal);
    CREATE INDEX thread_items_pending ON thread_items (ordinal)
        WHERE json_extract(content_json, '$.status') = 'pending';
    CREATE INDEX thread_items_open_tool_calls ON thread_items (ordinal)
        WHERE json_extract(content_json, '$.kind') = 'toolCall'
          AND json_extract(content_json, '$.output') IS NULL
          AND coalesce(json_extract(content_json, '$.interrupted'), 0) = 0;
    CREATE INDEX thread_items_task_results ON thread_items (ordinal)
        WHERE json_extract(content_json, '$.taskResult') IS NOT NULL;
    ",
    r"
    ALTER TABLE events ADD COLUMN thread_id TEXT GENERATED ALWAYS AS (coalesce(
        json_extract(payload_json, '$.threadId'),
        json_extract(payload_json, '$.item.threadId'),
        json_extract(payload_json, '$.thread.id')
    )) VIRTUAL;
    CREATE INDEX events_thread_id ON events (thread_id);
    ",
    r"
    CREATE TABLE pull_request_watches (
        thread_id TEXT PRIMARY KEY,
        watch_json TEXT NOT NULL
    );
    ",
    r"
    DELETE FROM events
    WHERE thread_id IS NOT NULL
      AND event_type <> 'threadDeleted'
      AND coalesce(substr(thread_id, 1, nullif(instr(thread_id, ':'), 0) - 1), thread_id)
          NOT IN (SELECT id FROM threads);
    ",
    r"
    ALTER TABLE threads ADD COLUMN provider TEXT NOT NULL DEFAULT 'claudeCode';
    ",
    r"
    CREATE TABLE provider_usage_limits (
        provider TEXT PRIMARY KEY,
        limits_json TEXT NOT NULL
    );
    INSERT INTO provider_usage_limits (provider, limits_json)
        SELECT 'claudeCode', limits_json FROM usage_limits WHERE id = 1;
    ",
    r"
    CREATE TABLE thread_recoveries (
        thread_id TEXT PRIMARY KEY,
        recovery_json TEXT NOT NULL
    );
    ",
    r"
    ALTER TABLE threads ADD COLUMN snoozed_until TEXT;
    ALTER TABLE threads ADD COLUMN snoozed_at TEXT;
    ALTER TABLE threads ADD COLUMN last_snooze_wake_at TEXT;
    ALTER TABLE threads ADD COLUMN last_visited_at TEXT;
    ALTER TABLE threads ADD COLUMN last_turn_ended_at TEXT;
    ",
    r"
    ALTER TABLE projects ADD COLUMN settings TEXT;
    ",
    r"
    ALTER TABLE threads ADD COLUMN forked_from TEXT;
    ALTER TABLE threads ADD COLUMN pending_model TEXT;
    ALTER TABLE threads ADD COLUMN provider_sessions TEXT;
    ALTER TABLE threads ADD COLUMN pending_handoffs TEXT;
    ",
    r"
    CREATE TABLE merged_cleanup (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        cleanup_json TEXT NOT NULL
    );
    ",
    r"
    ALTER TABLE threads ADD COLUMN last_turn_model TEXT;
    ",
];

const JOURNAL_SIZE_LIMIT: i64 = 8 * 1024 * 1024;
const STATEMENT_CACHE_CAPACITY: usize = 64;

const PENDING_REQUESTS: &str = "
    SELECT id, thread_id, created_at, content_json FROM thread_items
    WHERE json_extract(content_json, '$.status') = 'pending'
      AND json_extract(content_json, '$.kind')
          IN ('approval', 'questions', 'proposedPlan', 'secretRequest')
    ORDER BY ordinal";

const OPEN_TOOL_CALLS: &str = "
    SELECT thread_id, id FROM thread_items
    WHERE json_extract(content_json, '$.kind') = 'toolCall'
      AND json_extract(content_json, '$.output') IS NULL
      AND coalesce(json_extract(content_json, '$.interrupted'), 0) = 0
    ORDER BY ordinal";

const THREAD_ITEM: &str = "
    SELECT id, thread_id, created_at, content_json
    FROM thread_items WHERE thread_id = ?1 AND id = ?2";

const ORDINAL_OF: &str = "SELECT ordinal FROM thread_items WHERE thread_id = ?1 AND id = ?2";

const TURN_START: &str = "
    SELECT ordinal FROM thread_items
    WHERE thread_id = ?1 AND json_extract(content_json, '$.kind') = 'userMessage'
      AND ordinal < ?2
    ORDER BY ordinal DESC LIMIT 1 OFFSET ?3";

const PAGE_ITEMS: &str = "
    SELECT id, thread_id, created_at, content_json FROM thread_items
    WHERE thread_id = ?1 AND ordinal >= ?2 AND ordinal < ?3
    ORDER BY ordinal DESC LIMIT ?4";

const HAS_OLDER: &str = "
    SELECT EXISTS (SELECT 1 FROM thread_items WHERE thread_id = ?1 AND ordinal < ?2)";

const LAST_ANSWER: &str = "
    SELECT json_extract(content_json, '$.text') FROM thread_items
    WHERE thread_id = ?1 AND json_extract(content_json, '$.kind') = 'assistantMessage'
    ORDER BY ordinal DESC LIMIT 1";

const LAST_PLAN: &str = "
    SELECT json_extract(content_json, '$.plan') FROM thread_items
    WHERE thread_id = ?1 AND json_extract(content_json, '$.kind') = 'proposedPlan'
    ORDER BY ordinal DESC LIMIT 1";

const LAST_DECISION: &str = "
    SELECT json_extract(content_json, '$.decision') FROM thread_items
    WHERE ordinal = max(
        coalesce((SELECT max(ordinal) FROM thread_items
                  WHERE thread_id = ?1
                    AND json_extract(content_json, '$.kind') = 'userMessage'), 0),
        coalesce((SELECT max(ordinal) FROM thread_items
                  WHERE thread_id = ?1
                    AND json_extract(content_json, '$.kind') = 'proposedPlan'), 0))";

const ANSWERED_TASKS: &str = "
    SELECT json_extract(content_json, '$.taskResult') FROM thread_items
    WHERE json_extract(content_json, '$.kind') = 'userMessage'
      AND json_extract(content_json, '$.taskResult') IS NOT NULL";

const MEDIA_SINCE_COMPACTION: &str = "
    SELECT json_extract(content_json, '$.attachments')
    FROM thread_items INDEXED BY thread_items_thread_kind
    WHERE thread_id = ?1 AND json_extract(content_json, '$.kind') = 'userMessage'
      AND ordinal > coalesce((
          SELECT max(ordinal) FROM thread_items INDEXED BY thread_items_thread_kind
          WHERE thread_id = ?1 AND json_extract(content_json, '$.kind') = 'compaction'
            AND json_extract(content_json, '$.status') = 'done'), 0)
      AND json_array_length(content_json, '$.attachments') > 0
    ORDER BY ordinal";

const THREAD_ITEMS: &str = "
    SELECT id, thread_id, created_at, content_json
    FROM thread_items WHERE thread_id = ?1 ORDER BY ordinal";

const LATEST_MESSAGES: &str = "
    SELECT id, thread_id, created_at, content_json FROM thread_items
    WHERE ordinal IN (
        SELECT ordinal FROM thread_items INDEXED BY thread_items_thread_kind
        WHERE thread_id = ?1 AND json_extract(content_json, '$.kind')
              IN ('userMessage', 'assistantMessage', 'proposedPlan')
        ORDER BY ordinal DESC LIMIT ?2)
    ORDER BY ordinal";

const MESSAGE_COUNT: &str = "
    SELECT count(*) FROM thread_items
    WHERE thread_id = ?1 AND json_extract(content_json, '$.kind')
          IN ('userMessage', 'assistantMessage', 'proposedPlan')";

/// `?1` is a thread; `?1:...` are its subagents' transcripts. `;` follows `:` in ASCII.
const DELETE_THREAD_ITEMS: &str = "
    DELETE FROM thread_items
    WHERE thread_id = ?1 OR (thread_id >= ?1 || ':' AND thread_id < ?1 || ';')";

const DELETE_THREAD_EVENTS: &str = "
    DELETE FROM events
    WHERE (thread_id = ?1 OR (thread_id >= ?1 || ':' AND thread_id < ?1 || ';'))
      AND sequence < ?2";

const REWIND_ITEMS: &str = "
    DELETE FROM thread_items
    WHERE (thread_id = ?1 OR (thread_id >= ?1 || ':' AND thread_id < ?1 || ';'))
      AND ordinal >= (SELECT ordinal FROM thread_items WHERE thread_id = ?1 AND id = ?2)";

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
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        conn.pragma_update(None, "journal_size_limit", JOURNAL_SIZE_LIMIT)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self, StoreError> {
        conn.set_prepared_statement_cache_capacity(STATEMENT_CACHE_CAPACITY);
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
            execute(
                &tx,
                "INSERT INTO events (event_type, payload_json, occurred_at)
                 VALUES (json_extract(?1, '$.type'), ?1, ?2)",
                params![serde_json::to_string(event)?, occurred_at],
            )?;
            let sequence = tx.last_insert_rowid();
            sequences.push(sequence as u64);
            project(&tx, event, sequence)?;
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
                "SELECT id, title, workspace_root, created_at, updated_at, worktree_setup,
                        settings
                 FROM projects ORDER BY created_at, id",
            )?
            .query_map([], |row| {
                Ok(Project {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    workspace_root: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    worktree_setup: optional_json_document_column(row, 5)?,
                    settings: optional_json_document_column(row, 6)?.unwrap_or_default(),
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
                        conversation_cursor, resume_at, watching_pull_request, provider,
                        snoozed_until, snoozed_at, last_snooze_wake_at, last_visited_at,
                        last_turn_ended_at,
                        forked_from, pending_model, provider_sessions, pending_handoffs,
                        last_turn_model
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
                    provider: row.get(26)?,
                    snoozed_until: row.get(27)?,
                    snoozed_at: row.get(28)?,
                    last_snooze_wake_at: row.get(29)?,
                    last_visited_at: row.get(30)?,
                    last_turn_ended_at: row.get(31)?,
                    forked_from: optional_json_document_column(row, 32)?,
                    pending_model: optional_json_document_column(row, 33)?,
                    provider_sessions: optional_json_document_column(row, 34)?.unwrap_or_default(),
                    pending_handoffs: optional_json_document_column(row, 35)?.unwrap_or_default(),
                    last_turn_model: optional_json_document_column(row, 36)?,
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
            .prepare(
                "SELECT thread_id, used_tokens, max_tokens, cache_ttl_secs, cache_expires_at
                 FROM context_usages",
            )?
            .query_map([], |row| {
                let number = |index: usize| {
                    row.get::<_, Option<i64>>(index)
                        .map(|value| value.map(|value| value as u64))
                };
                Ok(ThreadContextUsage {
                    thread_id: row.get(0)?,
                    usage: ContextUsage {
                        used_tokens: row.get::<_, i64>(1)? as u64,
                        max_tokens: number(2)?,
                        cache_ttl_secs: number(3)?,
                        cache_expires_at: number(4)?,
                    },
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let pending_requests = self.items(PENDING_REQUESTS, [])?;
        let open_tool_calls = self
            .conn
            .prepare(OPEN_TOOL_CALLS)?
            .query_map([], |row| {
                Ok(OpenToolCall {
                    thread_id: row.get(0)?,
                    item_id: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let task_previews = threads
            .iter()
            .filter(|thread| thread.parent_thread_id.is_some())
            .map(|thread| {
                Ok(self.last_answer(&thread.id)?.map(|text| TaskPreview {
                    thread_id: thread.id.clone(),
                    text: task_preview(&text),
                }))
            })
            .filter_map(Result::transpose)
            .collect::<Result<Vec<_>, StoreError>>()?;
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
            background_tasks: Vec::new(),
            task_previews,
            held_messages: Vec::new(),
            project_branches: Vec::new(),
        })
    }

    /// Caches a thread's context usage. Not an event: it changes on every model call.
    pub fn save_context_usage(&mut self, update: &ThreadContextUsage) -> Result<(), StoreError> {
        self.conn
            .prepare_cached(
                "INSERT OR REPLACE INTO context_usages
                 (thread_id, used_tokens, max_tokens, cache_ttl_secs, cache_expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?
            .execute(params![
                update.thread_id,
                update.usage.used_tokens as i64,
                update.usage.max_tokens.map(|tokens| tokens as i64),
                update.usage.cache_ttl_secs.map(|secs| secs as i64),
                update.usage.cache_expires_at.map(|at| at as i64)
            ])?;
        Ok(())
    }

    /// Each provider plan's limits as last read, by provider id, to show at once on the next
    /// start.
    pub fn provider_usage_limits(&self) -> Result<HashMap<String, UsageLimits>, StoreError> {
        let limits = self
            .conn
            .prepare("SELECT provider, limits_json FROM provider_usage_limits")?
            .query_map([], |row| Ok((row.get(0)?, json_document_column(row, 1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(limits)
    }

    pub fn save_provider_usage_limits(
        &mut self,
        provider: &str,
        limits: &UsageLimits,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO provider_usage_limits (provider, limits_json) VALUES (?1, ?2)",
            params![provider, serde_json::to_string(limits)?],
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

    /// What each thread's agent was told about its pull request, watched now or before.
    pub fn pull_request_watches(&self) -> Result<Vec<(String, PullRequestWatch)>, StoreError> {
        let watches = self
            .conn
            .prepare("SELECT thread_id, watch_json FROM pull_request_watches")?
            .query_map([], |row| Ok((row.get(0)?, json_document_column(row, 1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(watches)
    }

    /// Not an event: it changes on reads of the forge, which would flood the event log.
    pub fn save_pull_request_watch(
        &mut self,
        thread_id: &str,
        watch: &PullRequestWatch,
    ) -> Result<(), StoreError> {
        self.conn
            .prepare_cached(
                "INSERT OR REPLACE INTO pull_request_watches (thread_id, watch_json)
                 VALUES (?1, ?2)",
            )?
            .execute(params![thread_id, serde_json::to_string(watch)?])?;
        Ok(())
    }

    /// What the engine keeps of each thread to pick it up again after a restart, such as the
    /// messages still to reach its agent, by thread id. Gone with its thread.
    pub fn thread_recoveries<T: serde::de::DeserializeOwned>(
        &self,
    ) -> Result<Vec<(String, T)>, StoreError> {
        let recoveries = self
            .conn
            .prepare("SELECT thread_id, recovery_json FROM thread_recoveries")?
            .query_map([], |row| Ok((row.get(0)?, json_document_column(row, 1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(recoveries)
    }

    /// Not an event: it changes as messages queue and background work starts and ends.
    /// `None` forgets it.
    pub fn save_thread_recovery<T: serde::Serialize>(
        &self,
        thread_id: &str,
        recovery: Option<&T>,
    ) -> Result<(), StoreError> {
        match recovery {
            Some(recovery) => self
                .conn
                .prepare_cached(
                    "INSERT OR REPLACE INTO thread_recoveries (thread_id, recovery_json)
                     VALUES (?1, ?2)",
                )?
                .execute(params![thread_id, serde_json::to_string(recovery)?])?,
            None => self
                .conn
                .prepare_cached("DELETE FROM thread_recoveries WHERE thread_id = ?1")?
                .execute([thread_id])?,
        };
        Ok(())
    }

    /// What the last sweep of merged worktrees did, to show after a restart.
    pub fn merged_cleanup<T: serde::de::DeserializeOwned>(&self) -> Result<Option<T>, StoreError> {
        let cleanup = self
            .conn
            .query_row(
                "SELECT cleanup_json FROM merged_cleanup WHERE id = 1",
                [],
                |row| json_document_column(row, 0),
            )
            .optional()?;
        Ok(cleanup)
    }

    pub fn save_merged_cleanup<T: serde::Serialize>(&self, cleanup: &T) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO merged_cleanup (id, cleanup_json) VALUES (1, ?1)",
            [serde_json::to_string(cleanup)?],
        )?;
        Ok(())
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

    /// What the last turn of thread `thread_id` that recorded it ran on, among those started
    /// before item `item_id` first came, since the last rewind before it last did, and not
    /// taken away by a rewind since; in the order the turns ran, which a message that waited
    /// its turn doesn't follow. `None` when none did.
    pub fn turn_model_before(
        &self,
        thread_id: &str,
        item_id: &str,
    ) -> Result<Option<ModelChoice>, StoreError> {
        let model: Option<String> = self
            .conn
            .query_row(
                "WITH latest AS (
                     SELECT sequence FROM events
                     WHERE thread_id = ?1 AND event_type = 'itemAdded'
                       AND json_extract(payload_json, '$.item.id') = ?2
                     ORDER BY sequence DESC LIMIT 1),
                 rewound AS (
                     SELECT coalesce(max(sequence), 0) AS sequence FROM events
                     WHERE thread_id = ?1 AND event_type = 'threadRewound'
                       AND sequence < (SELECT sequence FROM latest)),
                 came AS (
                     SELECT sequence FROM events
                     WHERE thread_id = ?1 AND event_type = 'itemAdded'
                       AND sequence > (SELECT sequence FROM rewound)
                       AND json_extract(payload_json, '$.item.id') = ?2
                     ORDER BY sequence LIMIT 1)
                 SELECT json_extract(e.payload_json, '$.model')
                 FROM events e
                 JOIN thread_items i
                   ON i.thread_id = e.thread_id
                  AND i.id = json_extract(e.payload_json, '$.turnId')
                 WHERE e.thread_id = ?1
                   AND e.event_type = 'turnStarted'
                   AND json_extract(e.payload_json, '$.model') IS NOT NULL
                   AND e.sequence < (SELECT sequence FROM came)
                   AND i.ordinal <= coalesce((SELECT ordinal FROM thread_items
                                              WHERE thread_id = ?1 AND id = ?2), i.ordinal)
                   AND NOT EXISTS (
                       SELECT 1 FROM events r
                       WHERE r.thread_id = ?1 AND r.event_type = 'threadRewound'
                         AND r.sequence > e.sequence
                         AND (SELECT a.sequence FROM events a
                              WHERE a.thread_id = ?1 AND a.event_type = 'itemAdded'
                                AND a.sequence < r.sequence
                                AND json_extract(a.payload_json, '$.item.id')
                                    = json_extract(r.payload_json, '$.messageId')
                              ORDER BY a.sequence DESC LIMIT 1) < e.sequence)
                 ORDER BY e.sequence DESC
                 LIMIT 1",
                params![thread_id, item_id],
                |row| row.get(0),
            )
            .optional()?;
        model
            .map(|model| serde_json::from_str(&model))
            .transpose()
            .map_err(Into::into)
    }

    /// One item of a thread's transcript, whole.
    pub fn thread_item(
        &self,
        thread_id: &str,
        item_id: &str,
    ) -> Result<Option<ThreadItem>, StoreError> {
        Ok(self.items(THREAD_ITEM, [thread_id, item_id])?.pop())
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
        self.transcript_page_through(thread_id, None, before, turns, max_items)
    }

    /// As [`Store::transcript_page`], of the items up to `through` and that item itself when
    /// set; nothing when the thread has no such item.
    pub fn transcript_page_through(
        &self,
        thread_id: &str,
        through: Option<&str>,
        before: Option<&str>,
        turns: usize,
        max_items: usize,
    ) -> Result<(Vec<ThreadItem>, bool), StoreError> {
        let ordinal_of = |id: &str| self.first::<i64>(ORDINAL_OF, [thread_id, id]);
        let mut end = match before {
            Some(id) => ordinal_of(id)?.unwrap_or(i64::MAX),
            None => i64::MAX,
        };
        if let Some(through) = through {
            let Some(last) = ordinal_of(through)? else {
                return Ok((Vec::new(), false));
            };
            end = end.min(last + 1);
        }
        let start = self
            .first::<i64>(
                TURN_START,
                params![thread_id, end, turns.saturating_sub(1) as i64],
            )?
            .unwrap_or(i64::MIN);
        let mut items = self.items(PAGE_ITEMS, params![thread_id, start, end, max_items as i64])?;
        items.reverse();
        let first = match items.first() {
            Some(item) => ordinal_of(&item.id)?.unwrap_or(i64::MIN),
            None => end,
        };
        let has_older = self
            .first::<bool>(HAS_OLDER, params![thread_id, first])?
            .unwrap_or(false);
        Ok((items, has_older))
    }

    /// The thread's latest answer, which is what a delegated task reports back.
    pub fn last_answer(&self, thread_id: &str) -> Result<Option<String>, StoreError> {
        self.first(LAST_ANSWER, [thread_id])
    }

    /// The thread's latest proposed plan, in markdown.
    pub fn last_plan(&self, thread_id: &str) -> Result<Option<String>, StoreError> {
        self.first(LAST_PLAN, [thread_id])
    }

    /// Whether the user sent the thread's plan back to planning since their last message.
    pub fn plan_declined_last(&self, thread_id: &str) -> Result<bool, StoreError> {
        let decision = self.first::<Option<String>>(LAST_DECISION, [thread_id])?;
        Ok(decision.flatten().as_deref() == Some("decline"))
    }

    /// The delegated tasks whose answer already went back to the thread that delegated them.
    pub fn answered_tasks(&self) -> Result<Vec<String>, StoreError> {
        Ok(self
            .conn
            .prepare(ANSWERED_TASKS)?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?)
    }

    /// The images and files the thread's messages carried since its conversation last
    /// compacted, oldest first.
    pub fn media_since_compaction(
        &self,
        thread_id: &str,
    ) -> Result<Vec<MessageAttachment>, StoreError> {
        let lists = self
            .conn
            .prepare_cached(MEDIA_SINCE_COMPACTION)?
            .query_map([thread_id], |row| {
                json_document_column::<Vec<MessageAttachment>>(row, 0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(lists
            .into_iter()
            .flatten()
            .filter(|attachment| {
                matches!(
                    attachment,
                    MessageAttachment::Image { .. } | MessageAttachment::File { .. }
                )
            })
            .collect())
    }

    /// A thread's transcript, oldest first.
    pub fn thread_items(&self, thread_id: &str) -> Result<Vec<ThreadItem>, StoreError> {
        self.items(THREAD_ITEMS, [thread_id])
    }

    /// The thread's last `limit` messages (the user's, the agent's answers and plans),
    /// oldest first, and how many came before them.
    pub fn latest_messages(
        &self,
        thread_id: &str,
        limit: usize,
    ) -> Result<(Vec<ThreadItem>, usize), StoreError> {
        let items = self.items(LATEST_MESSAGES, params![thread_id, limit as i64])?;
        let count = self.first::<i64>(MESSAGE_COUNT, [thread_id])?.unwrap_or(0) as usize;
        let earlier = count.saturating_sub(items.len());
        Ok((items, earlier))
    }

    fn first<T: rusqlite::types::FromSql>(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
    ) -> Result<Option<T>, StoreError> {
        Ok(self
            .conn
            .prepare_cached(sql)?
            .query_row(params, |row| row.get(0))
            .optional()?)
    }

    fn items(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
    ) -> Result<Vec<ThreadItem>, StoreError> {
        Ok(self
            .conn
            .prepare_cached(sql)?
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

fn optional_json_document_column<T: serde::de::DeserializeOwned>(
    row: &Row,
    index: usize,
) -> rusqlite::Result<Option<T>> {
    match row.get::<_, Option<String>>(index)? {
        Some(_) => json_document_column(row, index).map(Some),
        None => Ok(None),
    }
}

fn optional_json(value: Option<&impl serde::Serialize>) -> Result<Option<String>, StoreError> {
    Ok(value.map(serde_json::to_string).transpose()?)
}

/// `NULL` for an empty list.
fn list_json<T: serde::Serialize>(list: &[T]) -> Result<Option<String>, StoreError> {
    optional_json(Some(&list).filter(|list| !list.is_empty()))
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

fn execute(tx: &Transaction, sql: &str, params: impl rusqlite::Params) -> Result<(), StoreError> {
    tx.prepare_cached(sql)?.execute(params)?;
    Ok(())
}

/// Drops a thread's rows, its subagents' transcripts, and the events before `sequence`
/// that carried them.
fn forget_thread(tx: &Transaction, thread_id: &str, sequence: i64) -> Result<(), StoreError> {
    execute(tx, DELETE_THREAD_ITEMS, [thread_id])?;
    execute(tx, DELETE_THREAD_EVENTS, params![thread_id, sequence])?;
    execute(
        tx,
        "DELETE FROM pull_requests WHERE thread_id = ?1",
        [thread_id],
    )?;
    execute(
        tx,
        "DELETE FROM pull_request_details WHERE thread_id = ?1",
        [thread_id],
    )?;
    execute(
        tx,
        "DELETE FROM context_usages WHERE thread_id = ?1",
        [thread_id],
    )?;
    execute(
        tx,
        "DELETE FROM pull_request_watches WHERE thread_id = ?1",
        [thread_id],
    )?;
    execute(
        tx,
        "DELETE FROM thread_recoveries WHERE thread_id = ?1",
        [thread_id],
    )?;
    execute(tx, "DELETE FROM threads WHERE id = ?1", [thread_id])
}

fn project(tx: &Transaction, event: &Event, sequence: i64) -> Result<(), StoreError> {
    match event {
        Event::ProjectCreated { project } => {
            execute(
                tx,
                "INSERT INTO projects
                 (id, title, workspace_root, created_at, updated_at, worktree_setup, settings)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    project.id,
                    project.title,
                    project.workspace_root,
                    project.created_at,
                    project.updated_at,
                    optional_json(project.worktree_setup.as_ref())?,
                    serde_json::to_string(&project.settings)?
                ],
            )?;
        }
        Event::ProjectSettingsSet {
            project_id,
            settings,
        } => {
            execute(
                tx,
                "UPDATE projects SET settings = ?2 WHERE id = ?1",
                params![project_id, serde_json::to_string(settings)?],
            )?;
        }
        Event::ProjectWorktreeSetupSet { project_id, setup } => {
            execute(
                tx,
                "UPDATE projects SET worktree_setup = ?2 WHERE id = ?1",
                params![project_id, optional_json(setup.as_ref())?],
            )?;
        }
        Event::ProjectRemoved { project_id } => {
            let thread_ids = tx
                .prepare_cached("SELECT id FROM threads WHERE project_id = ?1")?
                .query_map([project_id], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            for thread_id in thread_ids {
                forget_thread(tx, &thread_id, sequence)?;
            }
            execute(tx, "DELETE FROM projects WHERE id = ?1", [project_id])?;
        }
        Event::ThreadCreated { thread } => {
            execute(
                tx,
                "INSERT INTO threads
                 (id, project_id, title, status, branch, worktree_path, session_id,
                  active_turn_id, created_at, updated_at, model, effort, archived, pinned,
                  fast_mode, ultracode, access_mode, plan_mode, worktree_released,
                  worktree_adopted, parent_thread_id, task_cancelled, task_answered,
                  conversation_cursor, resume_at, watching_pull_request, provider,
                  snoozed_until, snoozed_at, last_snooze_wake_at, last_visited_at,
                  last_turn_ended_at, forked_from, pending_model, provider_sessions,
                  pending_handoffs, last_turn_model)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                         ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30,
                         ?31, ?32, ?33, ?34, ?35, ?36, ?37)",
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
                    thread.watching_pull_request,
                    thread.provider,
                    thread.snoozed_until,
                    thread.snoozed_at,
                    thread.last_snooze_wake_at,
                    thread.last_visited_at,
                    thread.last_turn_ended_at,
                    optional_json(thread.forked_from.as_ref())?,
                    optional_json(thread.pending_model.as_ref())?,
                    list_json(&thread.provider_sessions)?,
                    list_json(&thread.pending_handoffs)?,
                    optional_json(thread.last_turn_model.as_ref())?
                ],
            )?;
        }
        Event::ThreadDeleted { thread_id } => forget_thread(tx, thread_id, sequence)?,
        Event::ThreadTitleSet { thread_id, title } => {
            execute(
                tx,
                "UPDATE threads SET title = ?2 WHERE id = ?1",
                params![thread_id, title],
            )?;
        }
        Event::ThreadBranchSet { thread_id, branch } => {
            execute(
                tx,
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
            execute(
                tx,
                "UPDATE threads SET worktree_path = ?2, branch = ?3, worktree_released = 0,
                 worktree_adopted = ?4 WHERE id = ?1",
                params![thread_id, worktree_path, branch, adopted],
            )?;
        }
        Event::ThreadWorktreeDetached { thread_id, branch } => {
            execute(
                tx,
                "UPDATE threads SET worktree_path = NULL, branch = ?2, worktree_released = 0,
                 worktree_adopted = 0 WHERE id = ?1",
                params![thread_id, branch],
            )?;
        }
        Event::ThreadWorktreeReleased { thread_id } => {
            execute(
                tx,
                "UPDATE threads SET worktree_released = 1 WHERE id = ?1",
                [thread_id],
            )?;
        }
        Event::ThreadWorktreeRestored { thread_id } => {
            execute(
                tx,
                "UPDATE threads SET worktree_released = 0 WHERE id = ?1",
                [thread_id],
            )?;
        }
        Event::ThreadCleanedUp { thread_id } => {
            execute(
                tx,
                "UPDATE threads SET worktree_path = NULL, branch = NULL, session_id = NULL,
                 provider_sessions = NULL, worktree_released = 0, worktree_adopted = 0
                 WHERE id = ?1",
                [thread_id],
            )?;
        }
        Event::ThreadArchivedSet {
            thread_id,
            archived,
        } => {
            execute(
                tx,
                "UPDATE threads SET archived = ?2 WHERE id = ?1",
                params![thread_id, archived],
            )?;
        }
        Event::ThreadPinnedSet { thread_id, pinned } => {
            execute(
                tx,
                "UPDATE threads SET pinned = ?2 WHERE id = ?1",
                params![thread_id, pinned],
            )?;
        }
        Event::ThreadSnoozed {
            thread_id,
            until,
            at,
        } => {
            execute(
                tx,
                "UPDATE threads SET snoozed_until = ?2, snoozed_at = ?3 WHERE id = ?1",
                params![thread_id, until, at],
            )?;
        }
        Event::ThreadUnsnoozed { thread_id, woke_at } => {
            execute(
                tx,
                "UPDATE threads SET snoozed_until = NULL, snoozed_at = NULL,
                 last_snooze_wake_at = coalesce(?2, last_snooze_wake_at) WHERE id = ?1",
                params![thread_id, woke_at],
            )?;
        }
        Event::ThreadVisited { thread_id, at } => {
            execute(
                tx,
                "UPDATE threads SET last_visited_at = ?2 WHERE id = ?1",
                params![thread_id, at],
            )?;
        }
        Event::ThreadModelSet {
            thread_id,
            provider,
            model,
            effort,
            fast_mode,
            ultracode,
        } => {
            execute(
                tx,
                "UPDATE threads SET provider = ?2, model = ?3, effort = ?4, fast_mode = ?5,
                 ultracode = ?6 WHERE id = ?1",
                params![
                    thread_id,
                    provider,
                    model,
                    effort_text(*effort),
                    fast_mode,
                    ultracode
                ],
            )?;
        }
        Event::ThreadAccessSet {
            thread_id,
            access_mode,
            plan_mode,
        } => {
            execute(
                tx,
                "UPDATE threads SET access_mode = ?2, plan_mode = ?3 WHERE id = ?1",
                params![thread_id, enum_text(*access_mode)?, plan_mode],
            )?;
        }
        Event::TurnStarted {
            thread_id,
            turn_id,
            at,
            model,
        } => {
            execute(
                tx,
                "UPDATE threads SET status = ?2, active_turn_id = ?3, updated_at = ?4,
                 last_turn_model = COALESCE(?5, last_turn_model) WHERE id = ?1",
                params![
                    thread_id,
                    enum_text(ThreadStatus::Working)?,
                    turn_id,
                    at,
                    optional_json(model.as_ref())?
                ],
            )?;
        }
        Event::SessionAttached {
            thread_id,
            session_id,
        } => {
            execute(
                tx,
                "UPDATE threads SET session_id = ?2 WHERE id = ?1",
                params![thread_id, session_id],
            )?;
        }
        Event::ItemAdded { item } => {
            execute(
                tx,
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
            execute(
                tx,
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
            execute(
                tx,
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
            execute(
                tx,
                "UPDATE threads SET status = ?2, active_turn_id = NULL, updated_at = ?3,
                 last_turn_ended_at = ?3 WHERE id = ?1",
                params![thread_id, enum_text(status)?, at],
            )?;
        }
        Event::ToolCallDetailSet {
            thread_id,
            item_id,
            detail,
        } => {
            execute(
                tx,
                "UPDATE thread_items SET content_json = json_set(content_json, '$.detail', json(?3))
                 WHERE thread_id = ?1 AND id = ?2",
                params![thread_id, item_id, serde_json::to_string(detail)?],
            )?;
        }
        Event::ToolCallInterrupted { thread_id, item_id } => {
            execute(
                tx,
                "UPDATE thread_items
                 SET content_json = json_set(content_json, '$.interrupted', json('true'))
                 WHERE thread_id = ?1 AND id = ?2",
                params![thread_id, item_id],
            )?;
        }
        Event::MessagesDropped {
            thread_id,
            message_ids,
        } => {
            for message_id in message_ids {
                execute(
                    tx,
                    "UPDATE thread_items
                     SET content_json = json_set(content_json, '$.dropped', json('true'))
                     WHERE thread_id = ?1 AND id = ?2
                       AND json_extract(content_json, '$.kind') = 'userMessage'",
                    params![thread_id, message_id],
                )?;
            }
        }
        Event::TurnRetrying { .. } => {}
        Event::ThreadDelegated {
            thread_id,
            parent_thread_id,
            shares_worktree,
        } => {
            execute(
                tx,
                "UPDATE threads SET parent_thread_id = ?2,
                 worktree_adopted = worktree_adopted OR ?3 WHERE id = ?1",
                params![thread_id, parent_thread_id, shares_worktree],
            )?;
        }
        Event::TaskCancelled { thread_id } => {
            execute(
                tx,
                "UPDATE threads SET task_cancelled = 1 WHERE id = ?1",
                [thread_id],
            )?;
        }
        Event::TaskAnswered { thread_id } => {
            execute(
                tx,
                "UPDATE threads SET task_answered = 1 WHERE id = ?1",
                [thread_id],
            )?;
        }
        Event::TaskReopened { thread_id } => {
            execute(
                tx,
                "UPDATE threads SET task_answered = 0 WHERE id = ?1",
                [thread_id],
            )?;
        }
        Event::ThreadStatusSet { thread_id, status } => {
            execute(
                tx,
                "UPDATE threads SET status = ?2 WHERE id = ?1",
                params![thread_id, enum_text(*status)?],
            )?;
        }
        Event::PullRequestWatchSet {
            thread_id,
            watching,
        } => {
            execute(
                tx,
                "UPDATE threads SET watching_pull_request = ?2 WHERE id = ?1",
                params![thread_id, watching],
            )?;
        }
        Event::ConversationCursorSet { thread_id, cursor } => {
            execute(
                tx,
                "UPDATE threads SET conversation_cursor = ?2, resume_at = NULL WHERE id = ?1",
                params![thread_id, cursor],
            )?;
        }
        Event::ThreadRewound {
            thread_id,
            message_id,
            resume_at,
            last_turn_model,
        } => {
            execute(tx, REWIND_ITEMS, params![thread_id, message_id])?;
            execute(
                tx,
                "UPDATE threads SET last_turn_model = ?2 WHERE id = ?1",
                params![thread_id, optional_json(last_turn_model.as_ref())?],
            )?;
            execute(
                tx,
                "UPDATE threads SET conversation_cursor = ?2, resume_at = ?2,
                 session_id = CASE WHEN ?2 IS NULL THEN NULL ELSE session_id END,
                 provider_sessions = CASE WHEN ?2 IS NULL THEN NULL ELSE provider_sessions END
                 WHERE id = ?1",
                params![thread_id, resume_at],
            )?;
        }
        Event::ThreadModelPending { thread_id, choice } => {
            execute(
                tx,
                "UPDATE threads SET pending_model = ?2 WHERE id = ?1",
                params![thread_id, optional_json(choice.as_ref())?],
            )?;
        }
        Event::ProviderSwitched {
            thread_id,
            to,
            session_id,
            conversation_cursor,
            resume_at,
            provider_sessions,
            pending_handoffs,
        } => {
            execute(
                tx,
                "UPDATE threads SET provider = ?2, model = ?3, effort = ?4, fast_mode = ?5,
                 ultracode = ?6, session_id = ?7, conversation_cursor = ?8, resume_at = ?9,
                 provider_sessions = ?10, pending_handoffs = ?11, pending_model = NULL
                 WHERE id = ?1",
                params![
                    thread_id,
                    to.provider,
                    to.model,
                    effort_text(to.effort),
                    to.fast_mode,
                    to.ultracode,
                    session_id,
                    conversation_cursor,
                    resume_at,
                    list_json(provider_sessions)?,
                    list_json(pending_handoffs)?
                ],
            )?;
        }
        Event::ContextHandoffsSet {
            thread_id,
            handoffs,
        } => {
            execute(
                tx,
                "UPDATE threads SET pending_handoffs = ?2 WHERE id = ?1",
                params![thread_id, list_json(handoffs)?],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
