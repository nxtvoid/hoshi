//! Codex's ChatGPT sign-in, run through its app server, and its `auth.json`, read and never
//! written.

use super::install::CodexSetup;
use crate::jsonrpc::{Connection, Incoming};
use crate::process::{hide_console, kill_tree};
use base64::Engine as _;
use hoshi_core::{ChatGptPlan, CodexAccount, ProviderSignInFailure};
use serde_json::{Value, json};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const POLL: Duration = Duration::from_millis(200);

/// Where Codex keeps its configuration and sign-in: the settings' folder, else `CODEX_HOME`,
/// else `~/.codex`.
pub fn codex_home(configured: Option<&Path>) -> Option<PathBuf> {
    configured
        .map(Path::to_owned)
        .or_else(|| {
            std::env::var_os("CODEX_HOME")
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| std::env::home_dir().map(|home| home.join(".codex")))
}

/// Whether Codex is signed in, as its files say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Auth {
    SignedIn(CodexAccount),
    SignedOut,
    /// The files can't be read, or the sign-in is kept in the system's keyring.
    Unknown,
}

pub(crate) fn auth(configured_home: Option<&Path>) -> Auth {
    let Some(home) = codex_home(configured_home) else {
        return Auth::Unknown;
    };
    match std::fs::read_to_string(home.join("auth.json")) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(file) => auth_of(&file),
            Err(_) => Auth::Unknown,
        },
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let config = std::fs::read_to_string(home.join("config.toml")).unwrap_or_default();
            if keeps_sign_in_elsewhere(&config) {
                Auth::Unknown
            } else {
                Auth::SignedOut
            }
        }
        Err(_) => Auth::Unknown,
    }
}

/// The account Codex is signed in with; `None` when signed out or it can't tell.
pub fn account(configured_home: Option<&Path>) -> Option<CodexAccount> {
    match auth(configured_home) {
        Auth::SignedIn(account) => Some(account),
        Auth::SignedOut | Auth::Unknown => None,
    }
}

fn keeps_sign_in_elsewhere(config: &str) -> bool {
    config.lines().any(|line| {
        let line = line.trim();
        line.strip_prefix("cli_auth_credentials_store")
            .map(|rest| rest.trim_start())
            .and_then(|rest| rest.strip_prefix('='))
            .is_some_and(|value| {
                let value = value.trim().trim_matches(['"', '\'']);
                value == "keyring" || value == "auto"
            })
    })
}

fn auth_of(file: &Value) -> Auth {
    let api_key = file["OPENAI_API_KEY"]
        .as_str()
        .is_some_and(|key| !key.trim().is_empty());
    let tokens = &file["tokens"];
    let has_tokens = tokens["access_token"]
        .as_str()
        .is_some_and(|token| !token.is_empty())
        || tokens["refresh_token"]
            .as_str()
            .is_some_and(|token| !token.is_empty());
    let mode = file["auth_mode"].as_str().unwrap_or_default();
    if has_tokens && mode != "apikey" {
        let claims = tokens["id_token"]
            .as_str()
            .and_then(jwt_claims)
            .unwrap_or(Value::Null);
        return Auth::SignedIn(CodexAccount {
            chatgpt: true,
            email: claims["email"]
                .as_str()
                .map(str::trim)
                .filter(|email| email.contains('@'))
                .map(str::to_owned),
            plan: claims["https://api.openai.com/auth"]["chatgpt_plan_type"]
                .as_str()
                .and_then(plan_of),
        });
    }
    if api_key {
        return Auth::SignedIn(CodexAccount {
            chatgpt: false,
            email: None,
            plan: None,
        });
    }
    Auth::SignedOut
}

/// The payload of a JWT, unverified: it only names the account.
fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn plan_of(plan: &str) -> Option<ChatGptPlan> {
    let plan = plan.trim().to_ascii_lowercase();
    Some(match plan.as_str() {
        "free" => ChatGptPlan::Free,
        "go" => ChatGptPlan::Go,
        "plus" => ChatGptPlan::Plus,
        "pro" | "prolite" | "promax" => ChatGptPlan::Pro,
        "team" => ChatGptPlan::Team,
        "business" => ChatGptPlan::Business,
        "enterprise" | "ent26" => ChatGptPlan::Enterprise,
        "edu" => ChatGptPlan::Edu,
        other if other.starts_with("self_serve_business") => ChatGptPlan::Business,
        other if other.starts_with("enterprise") => ChatGptPlan::Enterprise,
        other if other.starts_with("edu") => ChatGptPlan::Edu,
        _ => return None,
    })
}

/// What the app server says once a sign-in ends.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Completion {
    Finished {
        login_id: Option<String>,
        success: bool,
        error: Option<String>,
    },
    /// The app server ended.
    Ended,
}

fn completion_of(incoming: &Incoming) -> Option<Completion> {
    let Incoming::Notification { method, params } = incoming else {
        return None;
    };
    (method == "account/login/completed").then(|| Completion::Finished {
        login_id: params["loginId"].as_str().map(str::to_owned),
        success: params["success"].as_bool().unwrap_or(false),
        error: params["error"]
            .as_str()
            .map(str::trim)
            .filter(|error| !error.is_empty())
            .map(str::to_owned),
    })
}

/// A Codex app server, for its account methods.
struct Session {
    connection: Connection,
    child: Arc<Mutex<Child>>,
    pid: u32,
}

impl Session {
    fn open(
        setup: &CodexSetup,
        incoming: impl FnMut(Incoming) + Send + 'static,
        ended: impl FnOnce() + Send + 'static,
    ) -> Result<Self, ProviderSignInFailure> {
        let binary = setup.binary().ok_or(ProviderSignInFailure::NotInstalled)?;
        let mut command = Command::new(binary.path());
        crate::updater::with_search_path(&mut command);
        command
            .arg("app-server")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(home) = &setup.home {
            command.env("CODEX_HOME", home);
        }
        hide_console(&mut command);
        let mut child = command
            .spawn()
            .map_err(|_| ProviderSignInFailure::SpawnFailed)?;
        let pid = child.id();
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            return Err(ProviderSignInFailure::SpawnFailed);
        };
        let child = Arc::new(Mutex::new(child));
        let session = Self {
            connection: Connection::start(stdin, stdout, incoming, ended),
            child,
            pid,
        };
        session
            .connection
            .call(
                "initialize",
                json!({
                    "clientInfo": {
                        "name": "hoshi",
                        "title": "hoshi",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                }),
                HANDSHAKE_TIMEOUT,
            )
            .map_err(|_| ProviderSignInFailure::Ended)?;
        session.connection.notify("initialized", json!({}));
        Ok(session)
    }

    fn call(&self, method: &str, params: Value) -> Result<Value, ProviderSignInFailure> {
        self.connection
            .call(method, params, HANDSHAKE_TIMEOUT)
            .map_err(|error| {
                if error.code == 0 {
                    ProviderSignInFailure::Ended
                } else {
                    ProviderSignInFailure::Rejected {
                        detail: Some(error.detail()).filter(|detail| !detail.is_empty()),
                    }
                }
            })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.connection.close();
        kill_tree(self.pid, &self.child);
    }
}

/// A ChatGPT sign-in waiting for the user to finish it in their browser.
pub struct CodexSignIn {
    session: Session,
    url: String,
    login_id: String,
    completions: Receiver<Completion>,
}

/// Starts signing Codex in with ChatGPT; the user then signs in on [`CodexSignIn::url`].
pub fn start_sign_in(setup: &CodexSetup) -> Result<CodexSignIn, ProviderSignInFailure> {
    let (sender, completions) = mpsc::channel();
    let ended = sender.clone();
    let session = Session::open(
        setup,
        move |incoming| {
            if let Some(completion) = completion_of(&incoming) {
                let _ = sender.send(completion);
            }
        },
        move || {
            let _ = ended.send(Completion::Ended);
        },
    )?;
    let started = session.call("account/login/start", json!({ "type": "chatgpt" }))?;
    let (Some(url), Some(login_id)) = (
        started["authUrl"]
            .as_str()
            .filter(|url| url.starts_with("https://")),
        started["loginId"].as_str(),
    ) else {
        return Err(ProviderSignInFailure::Rejected { detail: None });
    };
    Ok(CodexSignIn {
        url: url.to_owned(),
        login_id: login_id.to_owned(),
        session,
        completions,
    })
}

impl CodexSignIn {
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Waits for the user to finish signing in, for `cancel` to turn true, or for the time to
    /// run out.
    pub fn wait(self, cancel: &AtomicBool) -> Result<(), ProviderSignInFailure> {
        let deadline = Instant::now() + SIGN_IN_TIMEOUT;
        loop {
            if cancel.load(Ordering::SeqCst) {
                self.cancel();
                return Err(ProviderSignInFailure::Cancelled);
            }
            if Instant::now() >= deadline {
                self.cancel();
                return Err(ProviderSignInFailure::TimedOut);
            }
            match self.completions.recv_timeout(POLL) {
                Ok(completion) => {
                    if let Some(result) = outcome(&completion, &self.login_id) {
                        return result;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(ProviderSignInFailure::Ended),
            }
        }
    }

    fn cancel(&self) {
        let _ = self.session.connection.call(
            "account/login/cancel",
            json!({ "loginId": self.login_id }),
            Duration::from_secs(5),
        );
    }
}

/// `None` while the completion is another sign-in's.
fn outcome(completion: &Completion, login_id: &str) -> Option<Result<(), ProviderSignInFailure>> {
    match completion {
        Completion::Ended => Some(Err(ProviderSignInFailure::Ended)),
        Completion::Finished {
            login_id: Some(other),
            ..
        } if other != login_id => None,
        Completion::Finished { success: true, .. } => Some(Ok(())),
        Completion::Finished { error, .. } => Some(Err(ProviderSignInFailure::Rejected {
            detail: error.clone(),
        })),
    }
}

/// Signs Codex out, through Codex itself.
pub fn sign_out(setup: &CodexSetup) -> Result<(), ProviderSignInFailure> {
    let session = Session::open(setup, |_| {}, || {})?;
    session.call("account/logout", json!({}))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(claims: &Value) -> String {
        let encode = |value: &Value| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.to_string())
        };
        format!(
            "{}.{}.signature",
            encode(&json!({ "alg": "RS256" })),
            encode(claims)
        )
    }

    #[test]
    fn a_chatgpt_sign_in_names_the_email_and_plan() {
        let claims = json!({
            "email": "nxtvoid@example.com",
            "https://api.openai.com/auth": { "chatgpt_plan_type": "plus" },
        });
        let file = json!({
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": null,
            "tokens": {
                "id_token": token(&claims),
                "access_token": "access",
                "refresh_token": "refresh",
                "account_id": "account",
            },
        });

        assert_eq!(
            auth_of(&file),
            Auth::SignedIn(CodexAccount {
                chatgpt: true,
                email: Some("nxtvoid@example.com".into()),
                plan: Some(ChatGptPlan::Plus),
            })
        );
    }

    #[test]
    fn an_api_key_signs_in_without_an_account() {
        let file = json!({ "auth_mode": "apikey", "OPENAI_API_KEY": "sk-test", "tokens": null });

        assert_eq!(
            auth_of(&file),
            Auth::SignedIn(CodexAccount {
                chatgpt: false,
                email: None,
                plan: None,
            })
        );
    }

    #[test]
    fn a_file_without_tokens_or_key_is_signed_out() {
        assert_eq!(auth_of(&json!({})), Auth::SignedOut);
        assert_eq!(
            auth_of(&json!({ "OPENAI_API_KEY": " ", "tokens": { "access_token": "" } })),
            Auth::SignedOut
        );
    }

    #[test]
    fn an_unreadable_id_token_still_signs_in() {
        let file = json!({ "tokens": { "id_token": "not-a-jwt", "access_token": "access" } });

        assert_eq!(
            auth_of(&file),
            Auth::SignedIn(CodexAccount {
                chatgpt: true,
                email: None,
                plan: None,
            })
        );
    }

    #[test]
    fn each_plan_maps_to_its_kind() {
        assert_eq!(plan_of("free"), Some(ChatGptPlan::Free));
        assert_eq!(plan_of("Pro"), Some(ChatGptPlan::Pro));
        assert_eq!(plan_of("prolite"), Some(ChatGptPlan::Pro));
        assert_eq!(
            plan_of("self_serve_business_usage_based"),
            Some(ChatGptPlan::Business)
        );
        assert_eq!(
            plan_of("enterprise_cbp_automation"),
            Some(ChatGptPlan::Enterprise)
        );
        assert_eq!(plan_of("edu_plus"), Some(ChatGptPlan::Edu));
        assert_eq!(plan_of("unknown"), None);
    }

    #[test]
    fn a_keyring_store_leaves_a_missing_file_unknown() {
        assert!(keeps_sign_in_elsewhere(
            "model = \"gpt-5\"\ncli_auth_credentials_store = \"keyring\"\n"
        ));
        assert!(keeps_sign_in_elsewhere("cli_auth_credentials_store='auto'"));
        assert!(!keeps_sign_in_elsewhere(
            "cli_auth_credentials_store = \"file\""
        ));
        assert!(!keeps_sign_in_elsewhere(""));
    }

    #[test]
    fn a_missing_auth_file_is_signed_out() {
        let home = std::env::temp_dir().join(format!("hoshi-codex-login-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();

        assert_eq!(auth(Some(&home)), Auth::SignedOut);
        std::fs::write(home.join("auth.json"), "{ not json").unwrap();
        assert_eq!(auth(Some(&home)), Auth::Unknown);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_sign_in_ends_with_its_own_completion() {
        let finished =
            |login_id: Option<&str>, success: bool, error: Option<&str>| Completion::Finished {
                login_id: login_id.map(str::to_owned),
                success,
                error: error.map(str::to_owned),
            };

        assert_eq!(outcome(&finished(Some("other"), true, None), "mine"), None);
        assert_eq!(
            outcome(&finished(Some("mine"), true, None), "mine"),
            Some(Ok(()))
        );
        assert_eq!(outcome(&finished(None, true, None), "mine"), Some(Ok(())));
        assert_eq!(
            outcome(&finished(Some("mine"), false, Some("denied")), "mine"),
            Some(Err(ProviderSignInFailure::Rejected {
                detail: Some("denied".into())
            }))
        );
        assert_eq!(
            outcome(&Completion::Ended, "mine"),
            Some(Err(ProviderSignInFailure::Ended))
        );
    }

    #[test]
    #[ignore = "runs Codex (CODEX_BINARY, else codex on the PATH) in a new CODEX_HOME"]
    fn a_sign_in_gives_the_chatgpt_page_and_can_be_cancelled() {
        let home = std::env::temp_dir().join(format!("hoshi-codex-sign-in-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let setup = CodexSetup {
            binary_path: std::env::var("CODEX_BINARY").ok(),
            home: Some(home.clone()),
            install: crate::CodexInstall::new(&home),
        };

        let sign_in = start_sign_in(&setup).unwrap();

        assert!(sign_in.url().starts_with("https://auth.openai.com/"));
        let cancel = AtomicBool::new(true);
        assert_eq!(sign_in.wait(&cancel), Err(ProviderSignInFailure::Cancelled));
        assert_eq!(auth(Some(&home)), Auth::SignedOut);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_completion_notification_is_read() {
        let incoming = Incoming::Notification {
            method: "account/login/completed".into(),
            params: json!({ "loginId": "mine", "success": false, "error": " denied " }),
        };
        let other = Incoming::Notification {
            method: "account/updated".into(),
            params: json!({}),
        };

        assert_eq!(
            completion_of(&incoming),
            Some(Completion::Finished {
                login_id: Some("mine".into()),
                success: false,
                error: Some("denied".into()),
            })
        );
        assert_eq!(completion_of(&other), None);
    }
}
