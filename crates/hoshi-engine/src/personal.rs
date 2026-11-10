//! What the user keeps for themselves: their notes, and the vault of secrets their agents
//! may be given. Apart from the engine's state and its lock, as nothing here waits on a
//! turn. Notes reach an agent only when the user attaches one to a message; a secret's
//! value lives in the system's credential store and reaches an agent only as a file.

use crate::secrets::SecretValues;
use crate::{Notification, Notify, delegation, internal, now};
use hoshi_core::{
    CommandError, MessageAttachment, Note, NoteSummary, StoredNote, VaultAccess, VaultField,
    VaultFieldInput, VaultSecret, VaultSecretInput,
};
use hoshi_store::PersonalStore;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

pub(crate) struct Personal {
    store: Mutex<PersonalStore>,
    values: Box<dyn SecretValues>,
    /// Agents take any secret that isn't hidden without asking.
    always_allow: AtomicBool,
    notify: Notify,
}

fn trimmed(text: &str) -> Option<&str> {
    Some(text.trim()).filter(|text| !text.is_empty())
}

fn unavailable(detail: String) -> CommandError {
    CommandError::VaultUnavailable { detail }
}

/// What the system's credential store keeps a field's value under. A secret from before
/// it had fields kept its one value under its own id.
fn value_key(secret_id: &str, field: &VaultField) -> String {
    if field.id.is_empty() {
        secret_id.to_owned()
    } else {
        format!("{secret_id}:{}", field.id)
    }
}

impl Personal {
    pub fn new(store: PersonalStore, values: Box<dyn SecretValues>, notify: Notify) -> Self {
        Self {
            store: Mutex::new(store),
            values,
            always_allow: AtomicBool::new(false),
            notify,
        }
    }

    fn store(&self) -> MutexGuard<'_, PersonalStore> {
        self.store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn vault_changed(&self) {
        (self.notify)(Notification::VaultChanged);
    }

    /// A project's notes holding `query`, or the general ones for `None`, latest first.
    pub fn notes(
        &self,
        project_id: Option<&str>,
        query: &str,
    ) -> Result<Vec<NoteSummary>, CommandError> {
        let notes = self.store().notes(project_id).map_err(internal)?;
        Ok(notes
            .iter()
            .filter(|note| note.matches(query))
            .map(StoredNote::summary)
            .collect())
    }

    fn stored_note(&self, id: &str) -> Result<StoredNote, CommandError> {
        Self::held_note(&self.store(), id)
    }

    /// The note, read under a lock the caller keeps to write it back.
    fn held_note(store: &PersonalStore, id: &str) -> Result<StoredNote, CommandError> {
        store
            .note(id)
            .map_err(internal)?
            .ok_or_else(|| CommandError::NoteNotFound {
                note_id: id.to_owned(),
            })
    }

    pub fn note(&self, id: &str) -> Result<Note, CommandError> {
        Ok(self.stored_note(id)?.open())
    }

    /// Writes a note's text, as its editor shows it in `document`, and where it belongs,
    /// adding the note if `id` is new.
    pub fn save_note(
        &self,
        id: &str,
        project_id: Option<String>,
        text: String,
        document: Option<String>,
    ) -> Result<Note, CommandError> {
        let mut store = self.store();
        let now = now();
        let note = match store.note(id).map_err(internal)? {
            Some(existing)
                if existing.text == text
                    && existing.project_id == project_id
                    && existing.document == document =>
            {
                return Ok(existing.open());
            }
            Some(existing) => StoredNote {
                ignored: existing
                    .ignored
                    .into_iter()
                    .filter(|value| text.contains(value.as_str()))
                    .collect(),
                project_id,
                text,
                document,
                updated_at: now,
                ..existing
            },
            None => StoredNote {
                id: id.to_owned(),
                project_id,
                text,
                document,
                created_at: now.clone(),
                updated_at: now,
                ignored: Vec::new(),
            },
        };
        store.save_note(&note).map_err(internal)?;
        Ok(note.open())
    }

    pub fn delete_note(&self, id: &str) -> Result<(), CommandError> {
        self.store().delete_note(id).map_err(internal)?;
        Ok(())
    }

    /// The text of the note's `index`th credential, as [`Note::credentials`] counts them.
    fn credential(note: &StoredNote, index: usize) -> Result<std::ops::Range<usize>, CommandError> {
        note.credentials()
            .into_iter()
            .nth(index)
            .ok_or_else(|| CommandError::NoteNotFound {
                note_id: note.id.clone(),
            })
    }

    /// The user says the note's `index`th credential is none: it stops being flagged, and
    /// agents read it when the note is attached.
    pub fn ignore_credential(&self, id: &str, index: usize) -> Result<Note, CommandError> {
        let mut store = self.store();
        let mut note = Self::held_note(&store, id)?;
        let range = Self::credential(&note, index)?;
        note.ignored.push(note.text[range].to_owned());
        store.save_note(&note).map_err(internal)?;
        Ok(note.open())
    }

    /// Moves the note's `index`th credential into the vault as `input` describes, leaving
    /// its name in the note where the value was. The editor's document, which held the
    /// value too, goes: the editor writes it again from what it shows. The note is read
    /// again once the vault has the value, so text saved meanwhile is kept.
    pub fn move_credential_to_vault(
        &self,
        id: &str,
        index: usize,
        input: VaultSecretInput,
    ) -> Result<Note, CommandError> {
        let value = {
            let note = self.stored_note(id)?;
            let range = Self::credential(&note, index)?;
            note.text[range].to_owned()
        };
        let secret = self.save_secret(
            None,
            VaultSecretInput {
                fields: vec![VaultFieldInput::single(value.clone())],
                ..input
            },
        )?;
        let mut store = self.store();
        let mut note = Self::held_note(&store, id)?;
        let held = note
            .credentials()
            .into_iter()
            .find(|range| note.text[range.clone()] == value);
        if let Some(range) = held {
            note.text
                .replace_range(range, &format!("[vault: {}]", secret.name));
            note.document = None;
            note.updated_at = now();
            store.save_note(&note).map_err(internal)?;
            store.forget_removed();
        }
        Ok(note.open())
    }

    /// The note as a message carries it to an agent, without its credentials.
    pub fn note_attachment(&self, id: &str) -> Result<MessageAttachment, CommandError> {
        Ok(self.stored_note(id)?.attachment())
    }

    pub fn set_always_allow(&self, always_allow: bool) {
        self.always_allow.store(always_allow, Ordering::Relaxed);
    }

    pub fn secrets(&self) -> Result<Vec<VaultSecret>, CommandError> {
        self.store().vault_secrets().map_err(internal)
    }

    /// Adds a secret, or changes the one with `id`: a field without a value keeps its own,
    /// and one the input leaves out goes with its value. Several fields each need a label.
    pub fn save_secret(
        &self,
        id: Option<&str>,
        input: VaultSecretInput,
    ) -> Result<VaultSecret, CommandError> {
        let name = trimmed(&input.name).ok_or(CommandError::VaultSecretIncomplete)?;
        let now = now();
        let existing = match id {
            Some(id) => Some(self.secret(id)?),
            None => None,
        };
        let had: &[VaultField] = existing.as_ref().map_or(&[], |secret| &secret.fields);
        let secret_id = existing
            .as_ref()
            .map_or_else(delegation::new_id, |secret| secret.id.clone());
        let labelled = input.fields.len() > 1;
        let mut fields: Vec<VaultField> = Vec::with_capacity(input.fields.len());
        let mut written = Vec::new();
        for field in &input.fields {
            let label = field.label.trim();
            let known = had.iter().find(|known| {
                field.id.as_deref() == Some(known.id.as_str())
                    && fields.iter().all(|taken| taken.id != known.id)
            });
            let value = field.value.as_deref().and_then(trimmed);
            if (labelled && label.is_empty()) || (known.is_none() && value.is_none()) {
                return Err(CommandError::VaultSecretIncomplete);
            }
            let stored = VaultField {
                id: known.map_or_else(delegation::new_id, |known| known.id.clone()),
                label: label.to_owned(),
                masked: field.masked,
            };
            if let Some(value) = value {
                written.push((value_key(&secret_id, &stored), value, known.is_none()));
            }
            fields.push(stored);
        }
        if fields.is_empty() {
            return Err(CommandError::VaultSecretIncomplete);
        }
        let secret = VaultSecret {
            id: secret_id,
            name: name.to_owned(),
            description: input.description.trim().to_owned(),
            project_id: input.project_id,
            access: input.access,
            fields,
            created_at: existing
                .as_ref()
                .map_or_else(|| now.clone(), |secret| secret.created_at.clone()),
            updated_at: now,
            last_used_at: existing
                .as_ref()
                .and_then(|secret| secret.last_used_at.clone()),
        };
        for (index, (key, value, _)) in written.iter().enumerate() {
            if let Err(detail) = self.values.write(key, value) {
                for (key, _, added) in &written[..index] {
                    if *added {
                        let _ = self.values.delete(key);
                    }
                }
                return Err(unavailable(detail));
            }
        }
        if let Err(error) = self.store().save_vault_secret(&secret) {
            for (key, _, added) in &written {
                if *added {
                    let _ = self.values.delete(key);
                }
            }
            return Err(internal(error));
        }
        let gone = had
            .iter()
            .filter(|field| secret.fields.iter().all(|kept| kept.id != field.id));
        for field in gone {
            if let Err(error) = self.values.delete(&value_key(&secret.id, field)) {
                eprintln!("hoshi: failed to delete a removed field's value: {error}");
            }
        }
        self.vault_changed();
        Ok(secret)
    }

    pub fn delete_secret(&self, id: &str) -> Result<(), CommandError> {
        let secret = self.store().vault_secret(id).map_err(internal)?;
        for field in secret.iter().flat_map(|secret| &secret.fields) {
            self.values
                .delete(&value_key(id, field))
                .map_err(unavailable)?;
        }
        self.store().delete_vault_secret(id).map_err(internal)?;
        self.vault_changed();
        Ok(())
    }

    pub fn secret(&self, id: &str) -> Result<VaultSecret, CommandError> {
        self.store()
            .vault_secret(id)
            .map_err(internal)?
            .ok_or_else(|| CommandError::VaultSecretNotFound {
                secret_id: id.to_owned(),
            })
    }

    /// The value of one of a secret's fields, for the user to copy.
    pub fn reveal(&self, id: &str, field_id: &str) -> Result<String, CommandError> {
        let secret = self.secret(id)?;
        let gone = || CommandError::VaultValueMissing {
            secret_id: id.to_owned(),
        };
        let field = secret
            .fields
            .iter()
            .find(|field| field.id == field_id)
            .ok_or_else(gone)?;
        self.values.read(&value_key(id, field)).ok_or_else(gone)
    }

    /// The secrets the agent of a thread in `project_id` may know of.
    pub fn offered(&self, project_id: Option<&str>) -> Vec<VaultSecret> {
        self.secrets()
            .unwrap_or_default()
            .into_iter()
            .filter(|secret| secret.offered_to(project_id))
            .collect()
    }

    /// Whether an agent takes the secret without the user approving it.
    pub fn allows(&self, secret: &VaultSecret) -> bool {
        match secret.access {
            VaultAccess::Allow => true,
            VaultAccess::Ask => self.always_allow.load(Ordering::Relaxed),
            VaultAccess::Hidden => false,
        }
    }

    /// From now on agents take the secret without asking.
    pub fn allow(&self, id: &str) -> Result<(), CommandError> {
        let mut secret = self.secret(id)?;
        secret.access = VaultAccess::Allow;
        secret.updated_at = now();
        self.store().save_vault_secret(&secret).map_err(internal)?;
        self.vault_changed();
        Ok(())
    }

    /// The values to hand an agent, each with the label of its field, noting that they
    /// were taken. `None` when the system's credential store no longer has one of them.
    pub fn take(&self, id: &str) -> Option<Vec<(String, String)>> {
        let mut secret = self.secret(id).ok()?;
        let values = secret
            .fields
            .iter()
            .map(|field| {
                let value = self.values.read(&value_key(id, field))?;
                Some((field.label.clone(), value))
            })
            .collect::<Option<Vec<_>>>()?;
        secret.last_used_at = Some(now());
        if let Err(error) = self.store().save_vault_secret(&secret) {
            eprintln!("hoshi: failed to note a secret's use: {error}");
        }
        self.vault_changed();
        Some(values)
    }

    /// A removed project's notes and secrets stay, as general ones.
    pub fn release_project(&self, project_id: &str) {
        if let Err(error) = self.store().release_project(project_id) {
            eprintln!("hoshi: failed to keep a removed project's notes: {error}");
        }
        self.vault_changed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryValues;
    use std::sync::Arc;

    fn personal() -> Personal {
        Personal::new(
            PersonalStore::open_in_memory().unwrap(),
            Box::<MemoryValues>::default(),
            Arc::new(|_| {}),
        )
    }

    fn input(name: &str, value: Option<&str>) -> VaultSecretInput {
        VaultSecretInput {
            name: name.into(),
            description: String::new(),
            project_id: Some("p".into()),
            access: VaultAccess::Ask,
            fields: vec![VaultFieldInput {
                value: value.map(str::to_owned),
                ..VaultFieldInput::single(String::new())
            }],
        }
    }

    fn field(id: Option<&str>, label: &str, value: Option<&str>) -> VaultFieldInput {
        VaultFieldInput {
            id: id.map(str::to_owned),
            label: label.into(),
            masked: label != "Email",
            value: value.map(str::to_owned),
        }
    }

    fn only_value(personal: &Personal, secret: &VaultSecret) -> Result<String, CommandError> {
        personal.reveal(&secret.id, &secret.fields[0].id)
    }

    #[test]
    fn a_note_is_added_edited_and_found_by_its_text() {
        let personal = personal();
        let created = personal
            .save_note("n", Some("p".into()), "Cache ideas\ntry a TTL".into(), None)
            .unwrap();
        let edited = personal
            .save_note(
                "n",
                Some("p".into()),
                "Cache ideas\ntry a 5 min TTL".into(),
                None,
            )
            .unwrap();

        assert_eq!(edited.created_at, created.created_at);
        assert_eq!(
            personal.note("n").unwrap().text,
            "Cache ideas\ntry a 5 min TTL"
        );
        let found = personal.notes(Some("p"), "5 MIN").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Cache ideas");
        assert!(personal.notes(Some("p"), "deploy").unwrap().is_empty());
        assert!(personal.notes(None, "").unwrap().is_empty());

        personal.delete_note("n").unwrap();
        assert_eq!(
            personal.note("n"),
            Err(CommandError::NoteNotFound {
                note_id: "n".into()
            })
        );
    }

    #[test]
    fn a_credential_the_user_dismisses_stops_being_flagged_until_it_changes() {
        let personal = personal();
        let note = personal
            .save_note("n", None, "port: 5432\npassword: hunter2".into(), None)
            .unwrap();
        assert_eq!(note.credentials, ["hun…"]);

        let dismissed = personal.ignore_credential("n", 0).unwrap();
        assert!(dismissed.credentials.is_empty());
        let MessageAttachment::Note { text, hidden, .. } = personal.note_attachment("n").unwrap()
        else {
            panic!("a note attachment");
        };
        assert_eq!(
            (text.as_str(), hidden),
            ("port: 5432\npassword: hunter2", 0)
        );

        let changed = personal
            .save_note("n", None, "port: 5432\npassword: hunter3".into(), None)
            .unwrap();
        assert_eq!(changed.credentials, ["hun…"]);
    }

    #[test]
    fn a_credential_moved_to_the_vault_leaves_its_name_in_the_note() {
        let personal = personal();
        personal
            .save_note(
                "n",
                Some("p".into()),
                "staging\npassword: hunter2\nok".into(),
                Some(r#"{"text":"password: hunter2"}"#.into()),
            )
            .unwrap();

        let note = personal
            .move_credential_to_vault("n", 0, input("Staging password", None))
            .unwrap();

        assert_eq!(
            note.text,
            "staging\npassword: [vault: Staging password]\nok"
        );
        assert!(note.credentials.is_empty());
        let [secret] = &personal.secrets().unwrap()[..] else {
            panic!("one secret");
        };
        assert_eq!(secret.project_id.as_deref(), Some("p"));
        assert_eq!(only_value(&personal, secret).unwrap(), "hunter2");
        assert_eq!(note.document, None);
    }

    #[test]
    fn a_note_keeps_what_its_editor_shows_beside_its_text() {
        let personal = personal();
        let document = Some(r#"{"type":"doc","content":[]}"#.to_owned());
        personal
            .save_note("n", None, "**bold**".into(), None)
            .unwrap();

        let rich = personal
            .save_note("n", None, "**bold**".into(), document.clone())
            .unwrap();

        assert_eq!(rich.document, document);
        assert_eq!(personal.note("n").unwrap().document, document);
    }

    #[test]
    fn a_secret_needs_a_name_and_a_value_and_keeps_its_value_when_edited() {
        let personal = personal();
        assert_eq!(
            personal.save_secret(None, input(" ", Some("v"))),
            Err(CommandError::VaultSecretIncomplete)
        );
        assert_eq!(
            personal.save_secret(None, input("Token", Some("  "))),
            Err(CommandError::VaultSecretIncomplete)
        );
        let secret = personal
            .save_secret(None, input("Token", Some(" abc123 \n")))
            .unwrap();

        let renamed = personal
            .save_secret(
                Some(&secret.id),
                VaultSecretInput {
                    fields: vec![field(Some(&secret.fields[0].id), "", None)],
                    ..input("Deploy token", None)
                },
            )
            .unwrap();

        assert_eq!(renamed.id, secret.id);
        assert_eq!(renamed.created_at, secret.created_at);
        assert_eq!(renamed.fields, secret.fields);
        assert_eq!(only_value(&personal, &secret).unwrap(), "abc123");
        personal.delete_secret(&secret.id).unwrap();
        assert!(personal.secrets().unwrap().is_empty());
        assert!(only_value(&personal, &secret).is_err());
        assert_eq!(
            personal
                .values
                .read(&value_key(&secret.id, &secret.fields[0])),
            None
        );
    }

    #[test]
    fn a_login_is_one_secret_whose_fields_each_keep_a_value() {
        let personal = personal();
        let login = |fields: Vec<VaultFieldInput>| VaultSecretInput {
            fields,
            ..input("Test login", None)
        };
        assert_eq!(
            personal.save_secret(
                None,
                login(vec![
                    field(None, " ", Some("a@b.dev")),
                    field(None, "Password", Some("hunter2")),
                ])
            ),
            Err(CommandError::VaultSecretIncomplete)
        );
        assert_eq!(
            personal.save_secret(None, login(Vec::new())),
            Err(CommandError::VaultSecretIncomplete)
        );
        let saved = personal
            .save_secret(
                None,
                login(vec![
                    field(None, "Email", Some("a@b.dev")),
                    field(None, "Password", Some("hunter2")),
                ]),
            )
            .unwrap();
        let [email, password] = &saved.fields[..] else {
            panic!("two fields");
        };
        assert_eq!((email.label.as_str(), email.masked), ("Email", false));
        assert_eq!(
            personal.take(&saved.id),
            Some(vec![
                ("Email".into(), "a@b.dev".into()),
                ("Password".into(), "hunter2".into()),
            ])
        );

        let edited = personal
            .save_secret(
                Some(&saved.id),
                login(vec![
                    field(Some(&password.id), "Pass", None),
                    field(None, "Code", Some("123456")),
                ]),
            )
            .unwrap();

        assert_eq!(edited.fields[0].id, password.id);
        assert_eq!(personal.reveal(&saved.id, &password.id).unwrap(), "hunter2");
        assert!(personal.reveal(&saved.id, &email.id).is_err());
        assert_eq!(personal.values.read(&value_key(&saved.id, email)), None);
        assert_eq!(
            personal.take(&saved.id),
            Some(vec![
                ("Pass".into(), "hunter2".into()),
                ("Code".into(), "123456".into()),
            ])
        );
        assert_eq!(
            personal.save_secret(
                Some(&saved.id),
                login(vec![field(Some("unknown"), "Other", None)])
            ),
            Err(CommandError::VaultSecretIncomplete)
        );
    }

    #[test]
    fn a_secret_the_store_refuses_is_not_listed() {
        let values = Box::<MemoryValues>::default();
        values.refuses.store(true, Ordering::Relaxed);
        let personal = Personal::new(
            PersonalStore::open_in_memory().unwrap(),
            values,
            Arc::new(|_| {}),
        );

        let refused = personal.save_secret(None, input("Token", Some("abc")));

        assert!(matches!(
            refused,
            Err(CommandError::VaultUnavailable { .. })
        ));
        assert!(personal.secrets().unwrap().is_empty());
    }

    #[test]
    fn agents_are_offered_what_is_in_reach_and_take_freely_only_what_is_allowed() {
        let personal = personal();
        let ask = personal.save_secret(None, input("Ask", Some("a"))).unwrap();
        let allowed = personal
            .save_secret(
                None,
                VaultSecretInput {
                    access: VaultAccess::Allow,
                    project_id: None,
                    ..input("Allowed", Some("b"))
                },
            )
            .unwrap();
        let hidden = personal
            .save_secret(
                None,
                VaultSecretInput {
                    access: VaultAccess::Hidden,
                    ..input("Hidden", Some("c"))
                },
            )
            .unwrap();

        let names = |project_id: Option<&str>| -> Vec<String> {
            personal
                .offered(project_id)
                .into_iter()
                .map(|secret| secret.name)
                .collect()
        };
        assert_eq!(names(Some("p")), ["Allowed", "Ask"]);
        assert_eq!(names(Some("other")), ["Allowed"]);
        assert!(!personal.allows(&ask));
        assert!(personal.allows(&allowed));

        personal.set_always_allow(true);
        assert!(personal.allows(&ask));
        assert!(!personal.allows(&hidden));

        assert_eq!(
            personal.take(&ask.id),
            Some(vec![(String::new(), "a".into())])
        );
        assert!(personal.secret(&ask.id).unwrap().last_used_at.is_some());
    }
}
