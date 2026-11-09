//! Secrets an agent asks the user for, as t3code's `request_secret`: the user types it in a
//! card, and the agent gets a file to read inside its commands, never the value.

use crate::{Engine, State, delegation, lock};
use hoshi_core::{
    Command, CommandError, ItemContent, ProviderOutput, RequestResponse, RequestStatus,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Mutex, Weak};
use std::time::{Duration, Instant, SystemTime};

const DEFAULT_WAIT: Duration = Duration::from_secs(10 * 60);
const MAX_WAIT: Duration = Duration::from_secs(60 * 60);
const CHECK_EVERY: Duration = Duration::from_secs(1);
/// Secret files go once they are this old.
const KEEP: Duration = Duration::from_secs(24 * 60 * 60);

const SAVED_NOTE: &str = "Read the file only inside the command that needs it, such as \"$(cat <file>)\", and never print or copy its contents. It is deleted after 24 hours.";
const DECLINED_NOTE: &str = "The user declined. Don't ask for it in chat or another way; say what's blocked and go on without it.";
const CANCELLED_NOTE: &str =
    "The card closed with the turn. Ask again only if the secret is still needed.";
const TIMED_OUT_NOTE: &str =
    "No answer in time. Don't ask in chat; ask again only if it is still needed.";

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

fn saved(path: &Path) -> Value {
    json!({ "status": "saved", "file": path.to_string_lossy(), "note": SAVED_NOTE })
}

fn unsaved(status: &str, note: &str) -> Value {
    json!({ "status": status, "note": note })
}

/// The `request_secret` tool: shows the card in the caller's thread and waits for the user.
/// A `value` the user already pasted is saved at once, with no card.
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
    if let Some(value) = text("value") {
        let dir = lock(&shared).secrets_dir.clone();
        return save(&dir, &value).map(|path| saved(&path));
    }
    let label = text("label").ok_or("label is required")?;
    let reason = text("reason").ok_or("reason is required")?;
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
            .ok_or("this thread has no running turn to ask in")?;
        state.secret_waits.insert(request_id.clone(), sender);
        let content = ItemContent::SecretRequest {
            label,
            reason,
            placeholder: text("placeholder"),
            status: RequestStatus::Pending,
            saved: None,
        };
        let opened = ProviderOutput::RequestOpened {
            request_id: request_id.clone(),
            content,
        };
        state.record(caller, &turn_id, opened);
        turn_id
    };
    let deadline = Instant::now() + wait;
    loop {
        match receiver.recv_timeout(CHECK_EVERY) {
            Ok(Some(path)) => return Ok(saved(&path)),
            Ok(None) | Err(RecvTimeoutError::Disconnected) => {
                return Ok(unsaved("declined", DECLINED_NOTE));
            }
            Err(RecvTimeoutError::Timeout) => {
                let mut state = lock(&shared);
                if state.model.pending_request(&request_id).is_none() {
                    state.secret_waits.remove(&request_id);
                    drop(state);
                    return Ok(match receiver.try_recv() {
                        Ok(Some(path)) => saved(&path),
                        Ok(None) => unsaved("declined", DECLINED_NOTE),
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

impl Engine {
    /// The user's answer to a secret request: the value to keep, or `None` to decline. The
    /// value only reaches a file; the transcript records whether one was saved.
    pub fn answer_secret_request(
        &self,
        thread_id: &str,
        request_id: &str,
        secret: Option<&str>,
    ) -> Result<(), CommandError> {
        let mut state = lock(&self.state);
        let asked = state.model.pending_request(request_id).is_some_and(|item| {
            item.thread_id == thread_id && matches!(item.content, ItemContent::SecretRequest { .. })
        });
        if !asked {
            return Err(CommandError::RequestNotPending {
                request_id: request_id.to_owned(),
            });
        }
        let path = match secret.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => Some(
                save(&state.secrets_dir, value)
                    .map_err(|detail| CommandError::Internal { detail })?,
            ),
            None => None,
        };
        let saved = path.is_some();
        if let Some(waiter) = state.secret_waits.remove(request_id) {
            let _ = waiter.send(path);
        }
        state.run_command(Command::RequestRespond {
            thread_id: thread_id.to_owned(),
            request_id: request_id.to_owned(),
            response: RequestResponse::Secret { saved },
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
}
