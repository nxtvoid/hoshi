//! Secrets an agent asks for: the user types one in a card, or the vault gives one it
//! holds, and the agent gets a file to read inside its commands, never the value. Values
//! that go together, such as a login's email and password, are one secret: one card, and
//! a file for each. Values the user already wrote in a message need no typing: they go to
//! files at once, and to the vault only once the user says so in a card.

use crate::{Engine, State, delegation, lock};
use hoshi_core::{
    Command, CommandError, ItemContent, ProviderOutput, RequestResponse, RequestStatus,
    SecretAnswer, SecretField, VaultFieldInput, VaultKeep, VaultSecret, VaultSecretInput,
    VaultSecretRef,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime};

const DEFAULT_WAIT: Duration = Duration::from_secs(10 * 60);
const MAX_WAIT: Duration = Duration::from_secs(60 * 60);
const CHECK_EVERY: Duration = Duration::from_secs(1);
/// Secret files go once they are this old.
const KEEP: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_FIELDS: usize = 6;

const SAVED_NOTE: &str = "Read a file only inside the command that needs it, such as \"$(cat <file>)\", and never print or copy its contents; to type a value into a page, pass its file to preview_act as secretFile. Files are deleted after 24 hours. `inVault` tells whether the vault holds the secret now: tell the user it is saved there only when it is true.";
const NOT_KEPT_NOTE: &str = "The user chose not to keep it in the vault: tell them it was not saved there. If a command needs the values, pass them again without keep.";
const DECLINED_NOTE: &str = "The user declined. Don't ask for it in chat or another way; say what's blocked and go on without it.";
const CANCELLED_NOTE: &str =
    "The card closed with the turn. Ask again only if the secret is still needed.";
const TIMED_OUT_NOTE: &str =
    "No answer in time. Don't ask in chat; ask again only if it is still needed.";
const NO_TURN: &str = "this thread has no running turn to ask in";
const NOT_IN_VAULT: &str =
    "the vault offers this thread no secret with that id: call vault_list for the ones it does";
const VALUE_GONE: &str = "the system's credential store no longer holds this secret's value: ask the user for it with request_secret, without vaultId";
const VAULT_NOTE: &str = "Names only, never values. Take one by passing its id to request_secret as vaultId: an \"allowed\" one comes at once, an \"ask\" one after the user approves. One with `fields` holds several values that go together and comes as a file for each. One that isn't here: request_secret asks the user for it.";

/// A secret's values, each with the label of its field.
type Labelled = Vec<(String, String)>;

/// The files holding a secret's values, each with the label of its field.
type Delivered = Vec<(String, PathBuf)>;

/// A card's wait for its answer.
pub(crate) struct SecretWait {
    /// The files the agent gets and whether the vault now holds the secret, or `None`
    /// when the user declined.
    reply: mpsc::Sender<Option<(Delivered, bool)>>,
    /// The values the user already wrote in a message, for a card that only asks whether
    /// to keep them.
    pasted: Option<Labelled>,
}

/// Writes `value` to a new file of its own under `dir`, after clearing out old ones.
pub(crate) fn save(dir: &Path, value: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    forget_old(dir);
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    let name: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let path = dir.join(name);
    std::fs::write(&path, value).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    Ok(path)
}

/// Deletes the secret files older than a day.
pub(crate) fn forget_old(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > KEEP);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The value in a file [`save`] wrote under `dir`, for a tool that uses it in the agent's
/// place. Any other file is refused.
pub(crate) fn read_saved(dir: &Path, file: &str) -> Result<String, String> {
    let refused = || "`secretFile` is not a file request_secret returned".to_owned();
    let path = dunce::canonicalize(file).map_err(|_| refused())?;
    let dir = dunce::canonicalize(dir).map_err(|_| refused())?;
    if path.parent() != Some(dir.as_path()) {
        return Err(refused());
    }
    std::fs::read_to_string(path).map_err(|_| refused())
}

/// The arguments of a `preview_act` that types a secret, with the secret as its text: the
/// browser types it and the agent never reads it. `None` for any other call.
pub(crate) fn with_secret_text(
    dir: &Path,
    tool: &str,
    arguments: &Value,
) -> Result<Option<Value>, String> {
    let Some(file) = arguments["secretFile"].as_str() else {
        return Ok(None);
    };
    if tool != "preview_act" || arguments["action"] != "type" {
        return Err("`secretFile` goes only with preview_act's type action".into());
    }
    let mut typed = arguments.clone();
    typed["text"] = json!(read_saved(dir, file)?);
    Ok(Some(typed))
}

/// A file under `dir` for each of the values, which come with the labels of their fields.
fn deliver(dir: &Path, values: &[(String, String)]) -> Result<Delivered, String> {
    values
        .iter()
        .map(|(label, value)| Ok((label.clone(), save(dir, value)?)))
        .collect()
}

fn saved(files: &Delivered, in_vault: bool) -> Value {
    let mut result = json!({ "status": "saved", "inVault": in_vault, "note": SAVED_NOTE });
    if let [(_, path)] = &files[..] {
        result["file"] = json!(path.to_string_lossy());
    } else {
        let files: Vec<Value> = files
            .iter()
            .map(|(label, path)| json!({ "field": label, "file": path.to_string_lossy() }))
            .collect();
        result["files"] = json!(files);
    }
    result
}

/// The fields a request names for values that go together, none for a single value; and
/// the values themselves when the user already wrote them in a message.
fn asked(arguments: &Value) -> Result<(Vec<SecretField>, Option<Labelled>), String> {
    let value_of = |holder: &Value| {
        holder["value"]
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let Some(fields) = arguments["fields"]
        .as_array()
        .filter(|fields| fields.len() > 1)
    else {
        let pasted = value_of(arguments).map(|value| vec![(String::new(), value)]);
        return Ok((Vec::new(), pasted));
    };
    if fields.len() > MAX_FIELDS {
        return Err(format!("a secret has at most {MAX_FIELDS} fields"));
    }
    let mut asked: Vec<SecretField> = Vec::with_capacity(fields.len());
    let mut pasted = Labelled::new();
    for field in fields {
        let label = field["label"]
            .as_str()
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .filter(|label| asked.iter().all(|other| other.label != *label))
            .ok_or("each field needs a label of its own")?;
        if let Some(value) = value_of(field) {
            pasted.push((label.to_owned(), value));
        }
        asked.push(SecretField {
            label: label.to_owned(),
            masked: field["secret"] != false,
        });
    }
    if !pasted.is_empty() && pasted.len() != asked.len() {
        return Err("give the value of every field, or of none".into());
    }
    Ok((asked, Some(pasted).filter(|pasted| !pasted.is_empty())))
}

/// What keeps `values` in the vault as `keep` says, each as the field it was asked as.
fn kept_as(
    keep: VaultKeep,
    project_id: Option<String>,
    fields: &[SecretField],
    values: &Labelled,
) -> VaultSecretInput {
    let fields = fields
        .iter()
        .zip(values)
        .map(|(field, (_, value))| VaultFieldInput {
            id: None,
            label: field.label.clone(),
            masked: field.masked,
            value: Some(value.clone()),
        })
        .collect();
    VaultSecretInput {
        name: keep.name,
        description: String::new(),
        project_id,
        access: keep.access,
        fields,
    }
}

fn unsaved(status: &str, note: &str) -> Value {
    json!({ "status": status, "note": note })
}

/// The `vault_list` tool: the secrets the vault offers the caller's agent, without values.
pub(crate) fn vault_list(state: &Weak<Mutex<State>>, caller: &str) -> Result<Value, String> {
    let shared = state.upgrade().ok_or("hoshi is closing")?;
    let (personal, project_id) = {
        let state = lock(&shared);
        let thread = state
            .model
            .thread(caller)
            .ok_or("this thread no longer exists")?;
        (state.personal.clone(), thread.project_id.clone())
    };
    let secrets: Vec<Value> = personal
        .offered(project_id.as_deref())
        .iter()
        .map(|secret| {
            let mut listed = json!({
                "id": secret.id,
                "name": secret.name,
                "description": secret.description,
                "scope": if secret.project_id.is_some() { "project" } else { "general" },
                "access": if personal.allows(secret) { "allowed" } else { "ask" },
            });
            let fields: Vec<&str> = secret
                .fields
                .iter()
                .map(|field| field.label.as_str())
                .filter(|label| !label.is_empty())
                .collect();
            if !fields.is_empty() {
                listed["fields"] = json!(fields);
            }
            listed
        })
        .collect();
    Ok(json!({ "secrets": secrets, "note": VAULT_NOTE }))
}

/// The `request_secret` tool: shows the card in the caller's thread and waits for the user.
/// Values the user already pasted go to files at once, with no card unless they are to be
/// kept in the vault, and so does a secret of the vault agents may take without asking.
pub(crate) fn request_secret(
    state: &Weak<Mutex<State>>,
    caller: &str,
    arguments: &Value,
) -> Result<Value, String> {
    let text = |key: &str| {
        arguments[key]
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let shared = state.upgrade().ok_or("hoshi is closing")?;
    let keeps = arguments["keep"] == true;
    let (fields, pasted) = asked(arguments)?;
    if let Some(values) = pasted.as_ref().filter(|_| !keeps) {
        let dir = lock(&shared).secrets_dir.clone();
        return deliver(&dir, values).map(|files| saved(&files, false));
    }
    let confirming = pasted.is_some();
    let reason = text("reason").ok_or("reason is required")?;
    let (personal, project_id) = {
        let state = lock(&shared);
        let thread = state.model.thread(caller).ok_or(NO_TURN)?;
        if thread.active_turn_id.is_none() {
            return Err(NO_TURN.into());
        }
        (state.personal.clone(), thread.project_id.clone())
    };
    let stored = match text("vaultId").filter(|_| !confirming) {
        Some(id) => Some(
            personal
                .secret(&id)
                .ok()
                .filter(|secret| secret.offered_to(project_id.as_deref()))
                .ok_or(NOT_IN_VAULT)?,
        ),
        None => None,
    };
    if let Some(secret) = stored.as_ref().filter(|secret| personal.allows(secret)) {
        return provide(&shared, caller, secret, reason);
    }
    let (label, fields) = match &stored {
        Some(secret) => (secret.name.clone(), Vec::new()),
        None => (text("label").ok_or("label is required")?, fields),
    };
    let wait = arguments["timeoutMs"]
        .as_u64()
        .map_or(DEFAULT_WAIT, Duration::from_millis)
        .clamp(CHECK_EVERY, MAX_WAIT);
    let request_id = delegation::new_id();
    let (sender, receiver) = mpsc::channel();
    let turn_id = {
        let mut state = lock(&shared);
        let turn_id = state
            .model
            .thread(caller)
            .and_then(|thread| thread.active_turn_id.clone())
            .ok_or(NO_TURN)?;
        let wait = SecretWait {
            reply: sender,
            pasted,
        };
        state.secret_waits.insert(request_id.clone(), wait);
        let content = ItemContent::SecretRequest {
            label,
            reason,
            placeholder: text("placeholder"),
            fields,
            status: RequestStatus::Pending,
            saved: None,
            vault: stored.as_ref().map(VaultSecret::reference),
            kept: false,
            keep_by_default: stored.is_none() && keeps,
            pasted: confirming,
        };
        let opened = ProviderOutput::RequestOpened {
            request_id: request_id.clone(),
            content,
        };
        state.record(caller, &turn_id, opened);
        turn_id
    };
    let deadline = Instant::now() + wait;
    let declined = || {
        let note = if confirming {
            NOT_KEPT_NOTE
        } else {
            DECLINED_NOTE
        };
        unsaved("declined", note)
    };
    loop {
        match receiver.recv_timeout(CHECK_EVERY) {
            Ok(Some((files, in_vault))) => return Ok(saved(&files, in_vault)),
            Ok(None) | Err(RecvTimeoutError::Disconnected) => return Ok(declined()),
            Err(RecvTimeoutError::Timeout) => {
                let mut state = lock(&shared);
                if state.model.pending_request(&request_id).is_none() {
                    state.secret_waits.remove(&request_id);
                    drop(state);
                    return Ok(match receiver.try_recv() {
                        Ok(Some((files, in_vault))) => saved(&files, in_vault),
                        Ok(None) => declined(),
                        Err(_) => unsaved("cancelled", CANCELLED_NOTE),
                    });
                }
                if Instant::now() >= deadline {
                    state.secret_waits.remove(&request_id);
                    let withdrawn = ProviderOutput::RequestWithdrawn {
                        request_id: request_id.clone(),
                    };
                    state.record(caller, &turn_id, withdrawn);
                    return Ok(unsaved("timed_out", TIMED_OUT_NOTE));
                }
            }
        }
    }
}

/// Hands the agent a secret the vault lets it take, and records the use in its thread.
fn provide(
    shared: &Arc<Mutex<State>>,
    caller: &str,
    secret: &VaultSecret,
    reason: String,
) -> Result<Value, String> {
    let personal = lock(shared).personal.clone();
    let values = personal.take(&secret.id).ok_or(VALUE_GONE)?;
    let mut state = lock(shared);
    let turn_id = state
        .model
        .thread(caller)
        .and_then(|thread| thread.active_turn_id.clone())
        .ok_or(NO_TURN)?;
    let files = deliver(&state.secrets_dir, &values)?;
    let provided = ProviderOutput::SecretProvided {
        item_id: delegation::new_id(),
        reason,
        vault: secret.reference(),
    };
    state.record(caller, &turn_id, provided);
    Ok(saved(&files, true))
}

impl Engine {
    /// The user's answer to a secret request: what they typed, a secret the vault holds,
    /// to keep what they already wrote in a message, or none. A value only reaches a file; the transcript records whether the agent got
    /// the secret, and which of the vault's.
    pub fn answer_secret_request(
        &self,
        thread_id: &str,
        request_id: &str,
        answer: SecretAnswer,
    ) -> Result<(), CommandError> {
        let not_pending = || CommandError::RequestNotPending {
            request_id: request_id.to_owned(),
        };
        let asked = |state: &State| {
            let item = state
                .model
                .pending_request(request_id)
                .filter(|item| item.thread_id == thread_id)?;
            match &item.content {
                ItemContent::SecretRequest { vault, fields, .. } => {
                    Some((vault.clone(), fields.clone()))
                }
                _ => None,
            }
        };
        let invalid = || CommandError::RequestResponseInvalid {
            request_id: request_id.to_owned(),
        };
        let ((asked_for, fields), project_id, pasted) = {
            let state = lock(&self.state);
            let asked_for = asked(&state).ok_or_else(not_pending)?;
            let project_id = state
                .model
                .thread(thread_id)
                .and_then(|thread| thread.project_id.clone());
            let pasted = state
                .secret_waits
                .get(request_id)
                .and_then(|wait| wait.pasted.clone());
            (asked_for, project_id, pasted)
        };
        let fields = if fields.is_empty() {
            vec![SecretField {
                label: String::new(),
                masked: true,
            }]
        } else {
            fields
        };
        let given: Option<(Labelled, Option<VaultSecretRef>, bool)> = match (answer, pasted) {
            (SecretAnswer::Decline, _) => None,
            (SecretAnswer::Keep { keep }, Some(values)) => {
                let input = kept_as(keep, project_id, &fields, &values);
                let kept = self.personal.save_secret(None, input)?.reference();
                Some((values, Some(kept), true))
            }
            (SecretAnswer::Keep { .. }, None) | (_, Some(_)) => return Err(invalid()),
            (SecretAnswer::Value { values, keep }, None) => {
                let values: Vec<&str> = values.iter().map(|value| value.trim()).collect();
                if values.len() != fields.len() || values.iter().any(|value| value.is_empty()) {
                    return Err(invalid());
                }
                let typed: Labelled = fields
                    .iter()
                    .zip(values)
                    .map(|(field, value)| (field.label.clone(), value.to_owned()))
                    .collect();
                let kept = match keep {
                    Some(keep) => {
                        let input = kept_as(keep, project_id, &fields, &typed);
                        Some(self.personal.save_secret(None, input)?.reference())
                    }
                    None => None,
                };
                let is_kept = kept.is_some();
                Some((typed, kept, is_kept))
            }
            (SecretAnswer::Vault { secret_id, always }, None) => {
                let secret = self
                    .personal
                    .secret(&secret_id)
                    .ok()
                    .filter(|secret| secret.in_reach_of(project_id.as_deref()))
                    .ok_or(CommandError::VaultSecretNotFound {
                        secret_id: secret_id.clone(),
                    })?;
                let values =
                    self.personal
                        .take(&secret_id)
                        .ok_or(CommandError::VaultValueMissing {
                            secret_id: secret_id.clone(),
                        })?;
                if always {
                    self.personal.allow(&secret_id)?;
                }
                Some((values, Some(secret.reference()), false))
            }
        };
        let mut state = lock(&self.state);
        asked(&state).ok_or_else(not_pending)?;
        let (reply, vault, kept) = match given {
            Some((values, vault, kept)) => {
                let files = deliver(&state.secrets_dir, &values)
                    .map_err(|detail| CommandError::Internal { detail })?;
                (Some((files, vault.is_some())), vault, kept)
            }
            None => (None, asked_for, false),
        };
        let saved = reply.is_some();
        if let Some(wait) = state.secret_waits.remove(request_id) {
            let _ = wait.reply.send(reply);
        }
        state.run_command(Command::RequestRespond {
            thread_id: thread_id.to_owned(),
            request_id: request_id.to_owned(),
            response: RequestResponse::Secret { saved, vault, kept },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_gets_a_file_of_its_own_and_old_ones_go() {
        let dir = std::env::temp_dir().join(format!("hoshi-secrets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = save(&dir, "token-1").unwrap();
        let second = save(&dir, "token-2").unwrap();
        let old = std::fs::File::options().write(true).open(&first).unwrap();
        old.set_modified(SystemTime::now() - KEEP - Duration::from_secs(60))
            .unwrap();
        drop(old);

        let third = save(&dir, "token-3").unwrap();

        assert_ne!(second, third);
        assert!(!first.exists());
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "token-2");
        assert_eq!(std::fs::read_to_string(&third).unwrap(), "token-3");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_saved_secret_is_read_back_for_a_tool() {
        let dir = std::env::temp_dir().join(format!("hoshi-secrets-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = save(&dir, "token-1").unwrap();
        let outside = dir.parent().unwrap().join("hoshi-secrets-outside.txt");
        std::fs::write(&outside, "not a secret").unwrap();
        let sneaky = dir.join("..").join("hoshi-secrets-outside.txt");

        assert_eq!(
            read_saved(&dir, &file.to_string_lossy()).as_deref(),
            Ok("token-1")
        );
        assert!(read_saved(&dir, &outside.to_string_lossy()).is_err());
        assert!(read_saved(&dir, &sneaky.to_string_lossy()).is_err());
        assert!(read_saved(&dir, &dir.join("missing").to_string_lossy()).is_err());
        let _ = std::fs::remove_file(outside);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_page_is_typed_a_secret_the_agent_only_names_by_its_file() {
        let dir = std::env::temp_dir().join(format!("hoshi-secrets-type-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = save(&dir, "hunter2").unwrap();
        let file = file.to_string_lossy();
        let typing = json!({ "action": "type", "ref": "e4", "secretFile": file, "clear": true });

        let typed = with_secret_text(&dir, "preview_act", &typing)
            .unwrap()
            .unwrap();

        assert_eq!(typed["text"], "hunter2");
        assert_eq!(typed["ref"], "e4");
        assert_eq!(
            with_secret_text(
                &dir,
                "preview_act",
                &json!({ "action": "type", "text": "hi" })
            ),
            Ok(None)
        );
        let clicking = json!({ "action": "click", "secretFile": file });
        assert!(with_secret_text(&dir, "preview_act", &clicking).is_err());
        let evaluating = json!({ "expression": "1", "secretFile": file });
        assert!(with_secret_text(&dir, "preview_evaluate", &evaluating).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
