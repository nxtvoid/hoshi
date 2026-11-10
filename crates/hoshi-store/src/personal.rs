//! What the user keeps for themselves: their notes, and which secrets the vault holds. A
//! database of its own, apart from the event log: nothing here is an event, so what the
//! user deletes is gone. A secret's value is never here; the system's credential store
//! has it.

use crate::{StoreError, enum_text, json_column, json_document_column};
use hoshi_core::{NoteSearch, NoteSummary, StoredNote, VaultAccess, VaultSecret};
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::path::Path;

const MIGRATIONS: &[&str] = &[
    r"
    CREATE TABLE notes (
        id TEXT PRIMARY KEY,
        project_id TEXT,
        text TEXT NOT NULL,
        ignored_json TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE INDEX notes_by_project ON notes (project_id, updated_at);
    CREATE TABLE vault_secrets (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        description TEXT NOT NULL,
        project_id TEXT,
        access TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        last_used_at TEXT
    );
    ",
    // A secret from before it had fields is the one value kept under its own id.
    r#"
    ALTER TABLE vault_secrets ADD COLUMN fields_json TEXT NOT NULL
        DEFAULT '[{"id":"","label":"","masked":true}]';
    "#,
    r"
    ALTER TABLE notes ADD COLUMN document_json TEXT;
    ",
    // How a list shows a note is kept beside it, in an index a list reads alone.
    r"
    ALTER TABLE notes ADD COLUMN title TEXT NOT NULL DEFAULT '';
    ALTER TABLE notes ADD COLUMN preview TEXT NOT NULL DEFAULT '';
    ALTER TABLE notes ADD COLUMN credentials INTEGER NOT NULL DEFAULT 0;
    DROP INDEX notes_by_project;
    CREATE INDEX notes_listed
        ON notes (project_id, updated_at DESC, id, title, preview, credentials);
    ",
];

/// The migration that added what lists show of a note. A database from before it has its
/// notes summarized once. What a summary masks follows how credentials are found, so a
/// change to that needs a migration of its own and this set to it.
const SUMMARIZED_AT: i64 = 4;

const NOTE_COLUMNS: &str =
    "id, project_id, text, ignored_json, created_at, updated_at, document_json";
const LISTED_NOTES: &str = "
    SELECT id, project_id, title, preview, updated_at, credentials FROM notes
    WHERE project_id IS ?1 ORDER BY updated_at DESC, id LIMIT ?2";
const SEARCHED_NOTES: &str = "
    SELECT id, project_id, title, preview, updated_at, credentials, text FROM notes
    WHERE project_id IS ?1 ORDER BY updated_at DESC, id";
const REPLACED_NOTE: &str = "
    SELECT ignored_json, created_at, updated_at,
        text = ?2 AND project_id IS ?3 AND document_json IS ?4
    FROM notes WHERE id = ?1";
const SECRET_COLUMNS: &str =
    "id, name, description, project_id, access, created_at, updated_at, last_used_at, fields_json";

pub struct PersonalStore {
    conn: Connection,
}

/// What a save needs of the note it replaces, without reading its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacedNote {
    pub ignored: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    /// It already holds the text, the document and the project the save brings.
    pub same: bool,
}

impl PersonalStore {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "secure_delete", "ON")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let tx = conn.transaction()?;
        for migration in MIGRATIONS.iter().skip(version as usize) {
            tx.execute_batch(migration)?;
        }
        if version < SUMMARIZED_AT {
            summarize_notes(&tx)?;
        }
        tx.pragma_update(None, "user_version", MIGRATIONS.len() as i64)?;
        tx.commit()?;
        Ok(Self { conn })
    }

    /// A project's notes, or the general ones for `None`, as a list shows them, the latest
    /// edited first: those whose text `search` finds, at most `limit` of them. Without a
    /// search no note's text is read.
    pub fn notes(
        &self,
        project_id: Option<&str>,
        search: Option<&NoteSearch>,
        limit: Option<usize>,
    ) -> Result<Vec<NoteSummary>, StoreError> {
        let Some(search) = search else {
            let limit = limit.map_or(-1, |limit| limit as i64);
            return Ok(self
                .conn
                .prepare_cached(LISTED_NOTES)?
                .query_map(params![project_id, limit], summary)?
                .collect::<Result<Vec<_>, _>>()?);
        };
        let limit = limit.unwrap_or(usize::MAX);
        let mut found = Vec::new();
        let mut statement = self.conn.prepare_cached(SEARCHED_NOTES)?;
        let mut rows = statement.query([project_id])?;
        while found.len() < limit
            && let Some(row) = rows.next()?
        {
            let text = row.get_ref(6)?.as_str().map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(6, Type::Text, Box::new(error))
            })?;
            if search.finds(text) {
                found.push(summary(row)?);
            }
        }
        Ok(found)
    }

    pub fn note(&self, id: &str) -> Result<Option<StoredNote>, StoreError> {
        Ok(self
            .conn
            .prepare_cached(&format!("SELECT {NOTE_COLUMNS} FROM notes WHERE id = ?1"))?
            .query_row([id], note)
            .optional()?)
    }

    /// What the note `id` leaves to a save that brings `text`, `document` and `project_id`,
    /// compared where they are kept; `None` for a note not written yet.
    pub fn replaced_note(
        &self,
        id: &str,
        project_id: Option<&str>,
        text: &str,
        document: Option<&str>,
    ) -> Result<Option<ReplacedNote>, StoreError> {
        Ok(self
            .conn
            .prepare_cached(REPLACED_NOTE)?
            .query_row(params![id, text, project_id, document], |row| {
                Ok(ReplacedNote {
                    ignored: json_document_column(row, 0)?,
                    created_at: row.get(1)?,
                    updated_at: row.get(2)?,
                    same: row.get(3)?,
                })
            })
            .optional()?)
    }

    /// Adds the note, or replaces the one with its id. `summary` is how a list shows it.
    pub fn save_note(
        &mut self,
        note: &StoredNote,
        summary: &NoteSummary,
    ) -> Result<(), StoreError> {
        self.conn
            .prepare_cached(&format!(
                "INSERT OR REPLACE INTO notes ({NOTE_COLUMNS}, title, preview, credentials)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
            ))?
            .execute(params![
                note.id,
                note.project_id,
                note.text,
                serde_json::to_string(&note.ignored)?,
                note.created_at,
                note.updated_at,
                note.document,
                summary.title,
                summary.preview,
                summary.credentials,
            ])?;
        Ok(())
    }

    /// Whether there was one to delete.
    pub fn delete_note(&mut self, id: &str) -> Result<bool, StoreError> {
        let deleted = self.conn.execute("DELETE FROM notes WHERE id = ?1", [id])? > 0;
        if deleted {
            self.forget_removed();
        }
        Ok(deleted)
    }

    /// Every secret of the vault, by name.
    pub fn vault_secrets(&self) -> Result<Vec<VaultSecret>, StoreError> {
        let secrets = self
            .conn
            .prepare_cached(&format!(
                "SELECT {SECRET_COLUMNS} FROM vault_secrets ORDER BY name COLLATE NOCASE, id"
            ))?
            .query_map([], secret)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(secrets)
    }

    pub fn vault_secret(&self, id: &str) -> Result<Option<VaultSecret>, StoreError> {
        Ok(self
            .conn
            .prepare_cached(&format!(
                "SELECT {SECRET_COLUMNS} FROM vault_secrets WHERE id = ?1"
            ))?
            .query_row([id], secret)
            .optional()?)
    }

    /// Adds the secret, or replaces the one with its id.
    pub fn save_vault_secret(&mut self, secret: &VaultSecret) -> Result<(), StoreError> {
        self.conn
            .prepare_cached(&format!(
                "INSERT OR REPLACE INTO vault_secrets ({SECRET_COLUMNS})
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
            ))?
            .execute(params![
                secret.id,
                secret.name,
                secret.description,
                secret.project_id,
                enum_text(secret.access)?,
                secret.created_at,
                secret.updated_at,
                secret.last_used_at,
                serde_json::to_string(&secret.fields)?,
            ])?;
        Ok(())
    }

    /// Whether there was one to delete.
    pub fn delete_vault_secret(&mut self, id: &str) -> Result<bool, StoreError> {
        let deleted = self
            .conn
            .execute("DELETE FROM vault_secrets WHERE id = ?1", [id])?
            > 0;
        Ok(deleted)
    }

    /// A removed project's notes and secrets become general ones, so none is lost. Its
    /// secrets turn hidden: every project would see them now, so no agent does until the
    /// user says which may.
    pub fn release_project(&mut self, project_id: &str) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE notes SET project_id = NULL WHERE project_id = ?1",
            [project_id],
        )?;
        tx.execute(
            "UPDATE vault_secrets SET project_id = NULL, access = ?2 WHERE project_id = ?1",
            params![project_id, enum_text(VaultAccess::Hidden)?],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Empties the write-ahead log, which would otherwise keep a deleted note's text, or
    /// what was taken out of one, until its next checkpoint.
    pub fn forget_removed(&self) {
        let _ = self
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    }
}

fn summarize_notes(conn: &Connection) -> Result<(), StoreError> {
    let notes = conn
        .prepare(&format!("SELECT {NOTE_COLUMNS} FROM notes"))?
        .query_map([], note)?
        .collect::<Result<Vec<_>, _>>()?;
    let mut update =
        conn.prepare("UPDATE notes SET title = ?2, preview = ?3, credentials = ?4 WHERE id = ?1")?;
    for note in notes {
        let summary = note.summary();
        update.execute(params![
            note.id,
            summary.title,
            summary.preview,
            summary.credentials
        ])?;
    }
    Ok(())
}

fn summary(row: &Row) -> rusqlite::Result<NoteSummary> {
    Ok(NoteSummary {
        id: row.get(0)?,
        project_id: row.get(1)?,
        title: row.get(2)?,
        preview: row.get(3)?,
        updated_at: row.get(4)?,
        credentials: row.get(5)?,
    })
}

fn note(row: &Row) -> rusqlite::Result<StoredNote> {
    Ok(StoredNote {
        id: row.get(0)?,
        project_id: row.get(1)?,
        text: row.get(2)?,
        ignored: json_document_column(row, 3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        document: row.get(6)?,
    })
}

fn secret(row: &Row) -> rusqlite::Result<VaultSecret> {
    Ok(VaultSecret {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        project_id: row.get(3)?,
        access: json_column(row, 4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        last_used_at: row.get(7)?,
        fields: json_document_column(row, 8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoshi_core::VaultField;

    fn stored(id: &str, project_id: Option<&str>, text: &str, updated_at: &str) -> StoredNote {
        StoredNote {
            id: id.into(),
            project_id: project_id.map(str::to_owned),
            text: text.into(),
            document: None,
            created_at: "2026-11-09T10:00:00.000Z".into(),
            updated_at: updated_at.into(),
            ignored: Vec::new(),
        }
    }

    fn kept(id: &str, project_id: Option<&str>, access: VaultAccess) -> VaultSecret {
        VaultSecret {
            id: id.into(),
            name: format!("Secret {id}"),
            description: "for tests".into(),
            project_id: project_id.map(str::to_owned),
            access,
            fields: vec![
                VaultField {
                    id: "f1".into(),
                    label: "Email".into(),
                    masked: false,
                },
                VaultField {
                    id: "f2".into(),
                    label: "Password".into(),
                    masked: true,
                },
            ],
            created_at: "2026-11-09T10:00:00.000Z".into(),
            updated_at: "2026-11-09T10:00:00.000Z".into(),
            last_used_at: None,
        }
    }

    fn save(store: &mut PersonalStore, note: &StoredNote) {
        store.save_note(note, &note.summary()).unwrap();
    }

    fn ids(notes: Vec<NoteSummary>) -> Vec<String> {
        notes.into_iter().map(|note| note.id).collect()
    }

    fn query_plan(store: &PersonalStore, sql: &str) -> Vec<String> {
        store
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap()
            .query_map(params![Some("p"), 10], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn notes_are_listed_by_scope_with_the_latest_edit_first() {
        let mut store = PersonalStore::open_in_memory().unwrap();
        let old = stored("a", Some("p"), "old", "2026-11-09T10:00:00.000Z");
        save(&mut store, &old);
        save(
            &mut store,
            &stored("b", Some("p"), "new", "2026-11-09T11:00:00.000Z"),
        );
        save(
            &mut store,
            &stored("c", None, "general", "2026-11-09T12:00:00.000Z"),
        );

        let mut rich = stored("b", Some("p"), "new", "2026-11-09T11:00:00.000Z");
        rich.document = Some(r#"{"type":"doc"}"#.into());
        save(&mut store, &rich);

        let listed = store.notes(Some("p"), None, None).unwrap();
        assert_eq!(listed[1], old.summary());
        assert_eq!(ids(listed), ["b", "a"]);
        assert_eq!(ids(store.notes(Some("p"), None, Some(1)).unwrap()), ["b"]);
        assert_eq!(store.note("b").unwrap().unwrap().document, rich.document);
        assert_eq!(ids(store.notes(None, None, None).unwrap()), ["c"]);
        assert!(store.notes(Some("other"), None, None).unwrap().is_empty());
    }

    #[test]
    fn a_list_reads_its_index_alone_and_never_a_note() {
        let store = PersonalStore::open_in_memory().unwrap();

        let plan = query_plan(&store, LISTED_NOTES);

        assert!(
            plan.iter()
                .any(|step| step.contains("COVERING INDEX notes_listed")),
            "{plan:#?}"
        );
        assert!(
            !plan.iter().any(|step| step.contains("TEMP B-TREE")),
            "{plan:#?}"
        );
    }

    #[test]
    fn a_search_finds_notes_by_their_text_up_to_a_limit() {
        let mut store = PersonalStore::open_in_memory().unwrap();
        for (id, text, at) in [
            ("a", "Cache ideas\ntry a TTL", "2026-11-09T10:00:00.000Z"),
            ("b", "Deploy steps", "2026-11-09T11:00:00.000Z"),
            ("c", "the CACHE again", "2026-11-09T12:00:00.000Z"),
        ] {
            save(&mut store, &stored(id, Some("p"), text, at));
        }
        let search = NoteSearch::new("cache");

        let found = |limit| ids(store.notes(Some("p"), search.as_ref(), limit).unwrap());

        assert_eq!(found(None), ["c", "a"]);
        assert_eq!(found(Some(1)), ["c"]);
        assert!(store.notes(None, search.as_ref(), None).unwrap().is_empty());
    }

    #[test]
    fn a_save_learns_of_the_note_it_replaces_without_reading_it() {
        let mut store = PersonalStore::open_in_memory().unwrap();
        let mut note = stored("a", Some("p"), "kept", "2026-11-09T10:00:00.000Z");
        note.document = Some(r#"{"type":"doc"}"#.into());
        note.ignored = vec!["hunter2".into()];
        save(&mut store, &note);

        let replaced = |project_id, text, document| {
            store
                .replaced_note("a", project_id, text, document)
                .unwrap()
                .unwrap()
        };

        assert_eq!(
            replaced(Some("p"), "kept", note.document.as_deref()),
            ReplacedNote {
                ignored: note.ignored.clone(),
                created_at: note.created_at.clone(),
                updated_at: note.updated_at.clone(),
                same: true,
            }
        );
        assert!(!replaced(Some("p"), "edited", note.document.as_deref()).same);
        assert!(!replaced(Some("p"), "kept", None).same);
        assert!(!replaced(None, "kept", note.document.as_deref()).same);
        assert_eq!(store.replaced_note("new", None, "", None).unwrap(), None);
    }

    #[test]
    fn notes_from_before_lists_kept_a_summary_get_one() {
        let conn = Connection::open_in_memory().unwrap();
        for migration in &MIGRATIONS[..3] {
            conn.execute_batch(migration).unwrap();
        }
        conn.pragma_update(None, "user_version", 3).unwrap();
        conn.execute(
            "INSERT INTO notes (id, project_id, text, ignored_json, created_at, updated_at)
             VALUES ('old', NULL, 'Staging\npassword: hunter2', '[]', 't', 't')",
            [],
        )
        .unwrap();

        let store = PersonalStore::init(conn).unwrap();

        let [listed] = &store.notes(None, None, None).unwrap()[..] else {
            panic!("one note");
        };
        assert_eq!(
            (
                listed.title.as_str(),
                listed.preview.as_str(),
                listed.credentials
            ),
            ("Staging", "password: ••••••", 1)
        );
    }

    #[test]
    fn saving_a_note_again_replaces_it_and_deleting_it_leaves_nothing() {
        let mut store = PersonalStore::open_in_memory().unwrap();
        let mut note = stored("a", None, "password: hunter2", "2026-11-09T10:00:00.000Z");
        save(&mut store, &note);
        note.text = "edited".into();
        note.document = Some(r#"{"type":"doc"}"#.into());
        note.ignored = vec!["hunter2".into()];
        save(&mut store, &note);

        assert_eq!(store.note("a").unwrap(), Some(note));
        assert_eq!(store.notes(None, None, None).unwrap()[0].title, "edited");
        assert!(store.delete_note("a").unwrap());
        assert!(!store.delete_note("a").unwrap());
        assert_eq!(store.note("a").unwrap(), None);
    }

    #[test]
    fn a_secret_keeps_its_fields_and_goes_when_deleted() {
        let mut store = PersonalStore::open_in_memory().unwrap();
        let mut secret = kept("s", Some("p"), VaultAccess::Hidden);
        store.save_vault_secret(&secret).unwrap();
        secret.access = VaultAccess::Allow;
        secret.last_used_at = Some("2026-11-09T12:00:00.000Z".into());
        store.save_vault_secret(&secret).unwrap();

        assert_eq!(store.vault_secrets().unwrap(), [secret.clone()]);
        assert_eq!(store.vault_secret("s").unwrap(), Some(secret));
        assert!(store.delete_vault_secret("s").unwrap());
        assert!(store.vault_secrets().unwrap().is_empty());
    }

    #[test]
    fn a_secret_from_before_fields_is_the_one_value_under_its_own_id() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute(
            "INSERT INTO vault_secrets
             (id, name, description, project_id, access, created_at, updated_at)
             VALUES ('old', 'Token', '', NULL, 'ask', 't', 't')",
            [],
        )
        .unwrap();

        let store = PersonalStore::init(conn).unwrap();

        assert_eq!(
            store.vault_secret("old").unwrap().unwrap().fields,
            [VaultField {
                id: String::new(),
                label: String::new(),
                masked: true,
            }]
        );
    }

    #[test]
    fn a_removed_project_leaves_its_notes_and_secrets_as_general_ones() {
        let mut store = PersonalStore::open_in_memory().unwrap();
        save(
            &mut store,
            &stored("a", Some("p"), "kept", "2026-11-09T10:00:00.000Z"),
        );
        save(
            &mut store,
            &stored("b", Some("q"), "other", "2026-11-09T10:00:00.000Z"),
        );
        store
            .save_vault_secret(&kept("free", Some("p"), VaultAccess::Allow))
            .unwrap();
        store
            .save_vault_secret(&kept("hidden", Some("p"), VaultAccess::Hidden))
            .unwrap();
        store
            .save_vault_secret(&kept("other", Some("q"), VaultAccess::Allow))
            .unwrap();

        store.release_project("p").unwrap();

        assert_eq!(ids(store.notes(None, None, None).unwrap()), ["a"]);
        assert_eq!(ids(store.notes(Some("q"), None, None).unwrap()), ["b"]);
        let access = |id: &str| {
            let secret = store.vault_secret(id).unwrap().unwrap();
            (secret.project_id, secret.access)
        };
        assert_eq!(access("free"), (None, VaultAccess::Hidden));
        assert_eq!(access("hidden"), (None, VaultAccess::Hidden));
        assert_eq!(access("other"), (Some("q".into()), VaultAccess::Allow));
    }

    #[test]
    fn a_file_database_keeps_what_was_saved_and_deletes_securely() {
        let dir = std::env::temp_dir().join(format!("hoshi-personal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("personal.sqlite");
        let mut store = PersonalStore::open(&path).unwrap();
        save(
            &mut store,
            &stored("a", None, "kept", "2026-11-09T10:00:00.000Z"),
        );
        let secure: i64 = store
            .conn
            .pragma_query_value(None, "secure_delete", |row| row.get(0))
            .unwrap();
        drop(store);

        let reopened = PersonalStore::open(&path).unwrap();

        assert_eq!(secure, 1);
        assert_eq!(reopened.note("a").unwrap().unwrap().text, "kept");
        drop(reopened);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
