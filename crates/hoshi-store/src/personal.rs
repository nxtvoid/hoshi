//! What the user keeps for themselves: their notes, and which secrets the vault holds. A
//! database of its own, apart from the event log: nothing here is an event, so what the
//! user deletes is gone. A secret's value is never here; the system's credential store
//! has it.

use crate::{StoreError, enum_text, json_column, json_document_column};
use hoshi_core::{StoredNote, VaultAccess, VaultSecret};
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
];

const NOTE_COLUMNS: &str =
    "id, project_id, text, ignored_json, created_at, updated_at, document_json";
/// A list never shows what a note's editor holds, so it isn't read.
const LISTED_NOTE_COLUMNS: &str =
    "id, project_id, text, ignored_json, created_at, updated_at, NULL";
const SECRET_COLUMNS: &str =
    "id, name, description, project_id, access, created_at, updated_at, last_used_at, fields_json";

pub struct PersonalStore {
    conn: Connection,
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
        tx.pragma_update(None, "user_version", MIGRATIONS.len() as i64)?;
        tx.commit()?;
        Ok(Self { conn })
    }

    /// A project's notes, or the general ones for `None`, the latest edited first, each
    /// without its editor's document.
    pub fn notes(&self, project_id: Option<&str>) -> Result<Vec<StoredNote>, StoreError> {
        let notes = self
            .conn
            .prepare_cached(&format!(
                "SELECT {LISTED_NOTE_COLUMNS} FROM notes WHERE project_id IS ?1
                 ORDER BY updated_at DESC, id"
            ))?
            .query_map([project_id], note)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(notes)
    }

    pub fn note(&self, id: &str) -> Result<Option<StoredNote>, StoreError> {
        Ok(self
            .conn
            .prepare_cached(&format!("SELECT {NOTE_COLUMNS} FROM notes WHERE id = ?1"))?
            .query_row([id], note)
            .optional()?)
    }

    /// Adds the note, or replaces the one with its id.
    pub fn save_note(&mut self, note: &StoredNote) -> Result<(), StoreError> {
        self.conn
            .prepare_cached(&format!(
                "INSERT OR REPLACE INTO notes ({NOTE_COLUMNS})
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
            ))?
            .execute(params![
                note.id,
                note.project_id,
                note.text,
                serde_json::to_string(&note.ignored)?,
                note.created_at,
                note.updated_at,
                note.document,
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

    #[test]
    fn notes_are_listed_by_scope_with_the_latest_edit_first() {
        let mut store = PersonalStore::open_in_memory().unwrap();
        store
            .save_note(&stored("a", Some("p"), "old", "2026-11-09T10:00:00.000Z"))
            .unwrap();
        store
            .save_note(&stored("b", Some("p"), "new", "2026-11-09T11:00:00.000Z"))
            .unwrap();
        store
            .save_note(&stored("c", None, "general", "2026-11-09T12:00:00.000Z"))
            .unwrap();

        let ids =
            |notes: Vec<StoredNote>| notes.into_iter().map(|note| note.id).collect::<Vec<_>>();

        let mut rich = stored("b", Some("p"), "new", "2026-11-09T11:00:00.000Z");
        rich.document = Some(r#"{"type":"doc"}"#.into());
        store.save_note(&rich).unwrap();

        assert_eq!(ids(store.notes(Some("p")).unwrap()), ["b", "a"]);
        assert_eq!(store.notes(Some("p")).unwrap()[0].document, None);
        assert_eq!(store.note("b").unwrap().unwrap().document, rich.document);
        assert_eq!(ids(store.notes(None).unwrap()), ["c"]);
        assert!(store.notes(Some("other")).unwrap().is_empty());
    }

    #[test]
    fn saving_a_note_again_replaces_it_and_deleting_it_leaves_nothing() {
        let mut store = PersonalStore::open_in_memory().unwrap();
        let mut note = stored("a", None, "password: hunter2", "2026-11-09T10:00:00.000Z");
        store.save_note(&note).unwrap();
        note.text = "edited".into();
        note.document = Some(r#"{"type":"doc"}"#.into());
        note.ignored = vec!["hunter2".into()];
        store.save_note(&note).unwrap();

        assert_eq!(store.note("a").unwrap(), Some(note));
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
        store
            .save_note(&stored("a", Some("p"), "kept", "2026-11-09T10:00:00.000Z"))
            .unwrap();
        store
            .save_note(&stored("b", Some("q"), "other", "2026-11-09T10:00:00.000Z"))
            .unwrap();
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

        assert_eq!(store.notes(None).unwrap()[0].id, "a");
        assert_eq!(store.notes(Some("q")).unwrap()[0].id, "b");
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
        store
            .save_note(&stored("a", None, "kept", "2026-11-09T10:00:00.000Z"))
            .unwrap();
        let secure: i64 = store
            .conn
            .pragma_query_value(None, "secure_delete", |row| row.get(0))
            .unwrap();
        drop(store);

        let reopened = PersonalStore::open(&path).unwrap();

        assert_eq!(secure, 1);
        assert_eq!(reopened.notes(None).unwrap()[0].text, "kept");
        drop(reopened);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
