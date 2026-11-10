//! A thread's real Claude Code agent uses the vault by itself: it finds a credential it
//! needs there and asks to use it, it opens the card that keeps one the user wants saved,
//! and it asks for a login's values in a single card. Costs a few cents and writes to the system's credential store, so it only runs on
//! request: `cargo test -p hoshi-engine --test vault_live -- --ignored`

use hoshi_core::{
    AccessMode, ApprovalDecision, Command, Event, ItemContent, RequestResponse, RequestStatus,
    SecretAnswer, ThreadWorkspace, TurnOutcome, VaultAccess, VaultFieldInput, VaultKeep,
    VaultSecretInput,
};
use hoshi_engine::{Engine, Notification, Notify};
use hoshi_providers::ClaudeCodeAdapter;
use hoshi_store::Store;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

const TOKEN: &str = "tok-live-7391-zx";

/// Takes the test's secrets out of the system's credential store however the test ends.
struct Cleanup<'a> {
    engine: &'a Engine,
    root: PathBuf,
}

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        for secret in self.engine.vault_secrets().unwrap_or_default() {
            let _ = self.engine.delete_vault_secret(&secret.id);
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// An engine with project `p` and a thread `t` of a real agent, and the events it emits.
fn engine_with_thread(name: &str) -> (Engine, Receiver<Event>, PathBuf) {
    let root = std::env::temp_dir().join(format!("hoshi-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("project")).unwrap();
    let (sender, events) = mpsc::channel();
    let notify: Notify = Arc::new(move |notification| {
        if let Notification::Event(envelope) = notification {
            let _ = sender.send(envelope.event);
        }
    });
    let engine =
        Engine::from_store(Store::open_in_memory().unwrap(), &root.join("data"), notify).unwrap();
    engine.set_adapter(Some(Arc::new(ClaudeCodeAdapter::new(
        None,
        Some("haiku".into()),
    ))));
    engine
        .dispatch(Command::ProjectCreate {
            project_id: "p".into(),
            title: "Project".into(),
            workspace_root: root.join("project").to_string_lossy().into_owned(),
        })
        .unwrap();
    engine
        .dispatch(Command::ThreadCreate {
            thread_id: "t".into(),
            project_id: Some("p".into()),
            title: "Vault".into(),
            workspace: ThreadWorkspace::Local,
            model: None,
            effort: Some(hoshi_core::Effort::Low),
            fast_mode: false,
            ultracode: false,
            access_mode: AccessMode::AcceptEdits,
            plan_mode: false,
        })
        .unwrap();
    (engine, events, root)
}

fn send(engine: &Engine, text: &str) {
    engine
        .dispatch(Command::MessageSend {
            thread_id: "t".into(),
            message_id: "22222222-2222-4222-8222-222222222222".into(),
            text: text.into(),
            attachments: Vec::new(),
        })
        .unwrap();
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn an_agent_takes_a_credential_from_the_vault_once_the_user_approves() {
    let (engine, events, root) = engine_with_thread("vault-take");
    let _cleanup = Cleanup {
        engine: &engine,
        root,
    };
    let secret = engine
        .save_vault_secret(
            None,
            VaultSecretInput {
                name: "Staging API token".into(),
                description: "Bearer token of the staging API".into(),
                project_id: Some("p".into()),
                access: VaultAccess::Ask,
                fields: vec![VaultFieldInput::single(TOKEN.into())],
            },
        )
        .unwrap();
    send(
        &engine,
        "I need to call the staging API. Get its bearer token and tell me how many characters it has, counting them with a shell command. Never show me the token itself.",
    );

    let mut asked = false;
    let mut commands = Vec::new();
    loop {
        match events.recv_timeout(Duration::from_secs(240)) {
            Ok(Event::ItemAdded { item }) => match &item.content {
                ItemContent::SecretRequest {
                    status: RequestStatus::Pending,
                    vault: Some(vault),
                    ..
                } if vault.id == secret.id => {
                    asked = true;
                    let approve = SecretAnswer::Vault {
                        secret_id: secret.id.clone(),
                        always: false,
                    };
                    engine
                        .answer_secret_request("t", &item.id, approve)
                        .unwrap();
                }
                ItemContent::Approval {
                    status: RequestStatus::Pending,
                    input,
                    ..
                } => {
                    commands.push(input.clone());
                    engine
                        .dispatch(Command::RequestRespond {
                            thread_id: "t".into(),
                            request_id: item.id.clone(),
                            response: RequestResponse::Decision {
                                decision: ApprovalDecision::Accept,
                                feedback: None,
                            },
                        })
                        .unwrap();
                }
                _ => {}
            },
            Ok(Event::TurnCompleted { outcome, .. }) => {
                assert_eq!(outcome, TurnOutcome::Completed);
                break;
            }
            Ok(_) => {}
            Err(_) => panic!("timed out; asked for the secret: {asked}; ran: {commands:?}"),
        }
    }

    let items = engine.transcript("t", None).unwrap().items;
    let answer = items.iter().rev().find_map(|item| match &item.content {
        ItemContent::AssistantMessage { text } => Some(text.clone()),
        _ => None,
    });
    let recorded = serde_json::to_string(&items).unwrap();

    assert!(asked, "the agent never asked for the vault's secret");
    assert!(
        items.iter().any(|item| matches!(
            &item.content,
            ItemContent::SecretRequest { saved: Some(true), vault: Some(vault), .. }
                if vault.name == "Staging API token"
        )),
        "{recorded}"
    );
    assert!(
        !recorded.contains(TOKEN),
        "the token reached the transcript"
    );
    let answer = answer.expect("an answer");
    assert!(
        answer.contains(&TOKEN.len().to_string()),
        "{answer}; ran: {commands:?}"
    );
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn an_agent_asked_to_save_a_secret_opens_the_card_that_keeps_it() {
    let (engine, events, root) = engine_with_thread("vault-save");
    let _cleanup = Cleanup {
        engine: &engine,
        root,
    };
    send(
        &engine,
        "Guarda en el vault mi token de la API de staging, te lo doy yo.",
    );

    let mut card = None;
    loop {
        match events.recv_timeout(Duration::from_secs(240)) {
            Ok(Event::ItemAdded { item }) => {
                if let ItemContent::SecretRequest {
                    status: RequestStatus::Pending,
                    label,
                    fields,
                    keep_by_default,
                    ..
                } = &item.content
                {
                    assert!(fields.is_empty(), "one token asked as {fields:?}");
                    card = Some((label.clone(), *keep_by_default));
                    let typed = SecretAnswer::Value {
                        values: vec![TOKEN.into()],
                        keep: keep_by_default.then(|| VaultKeep {
                            name: label.clone(),
                            access: VaultAccess::Ask,
                        }),
                    };
                    engine.answer_secret_request("t", &item.id, typed).unwrap();
                }
            }
            Ok(Event::TurnCompleted { outcome, .. }) => {
                assert_eq!(outcome, TurnOutcome::Completed);
                break;
            }
            Ok(_) => {}
            Err(_) => panic!("timed out; the card: {card:?}"),
        }
    }

    let (label, keeps) = card.expect("the agent opened no card");
    assert!(keeps, "the card for {label} did not start set to keep it");
    let kept = engine.vault_secrets().unwrap();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0].project_id.as_deref(), Some("p"));
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn an_agent_asked_to_save_a_login_asks_for_it_in_one_card() {
    let (engine, events, root) = engine_with_thread("vault-login");
    let _cleanup = Cleanup {
        engine: &engine,
        root,
    };
    send(
        &engine,
        "Guarda en el vault mi inicio de sesion de prueba, te lo doy yo",
    );

    let mut cards = Vec::new();
    loop {
        match events.recv_timeout(Duration::from_secs(240)) {
            Ok(Event::ItemAdded { item }) => {
                if let ItemContent::SecretRequest {
                    status: RequestStatus::Pending,
                    label,
                    fields,
                    keep_by_default,
                    ..
                } = &item.content
                {
                    cards.push((label.clone(), fields.clone()));
                    let typed = SecretAnswer::Value {
                        values: (0..fields.len().max(1))
                            .map(|index| format!("value-{index}"))
                            .collect(),
                        keep: keep_by_default.then(|| VaultKeep {
                            name: label.clone(),
                            access: VaultAccess::Ask,
                        }),
                    };
                    engine.answer_secret_request("t", &item.id, typed).unwrap();
                }
            }
            Ok(Event::TurnCompleted { outcome, .. }) => {
                assert_eq!(outcome, TurnOutcome::Completed);
                break;
            }
            Ok(_) => {}
            Err(_) => panic!("timed out; the cards: {cards:?}"),
        }
    }

    let [(label, fields)] = &cards[..] else {
        panic!("expected one card: {cards:?}");
    };
    assert!(fields.len() >= 2, "{label} asked for {fields:?}");
    assert!(
        fields.iter().any(|field| field.masked) && fields.iter().any(|field| !field.masked),
        "{fields:?}"
    );
    let kept = engine.vault_secrets().unwrap();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0].fields.len(), fields.len());
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn an_agent_given_a_login_in_the_chat_keeps_it_once_the_user_confirms() {
    let (engine, events, root) = engine_with_thread("vault-pasted");
    let _cleanup = Cleanup {
        engine: &engine,
        root,
    };
    send(
        &engine,
        "ahora si yo te digo, guarda mi usuario de inicio de sesion\n\nemail: hola@example.dev\npass: tok-live-7391-zx\n\nla puedes guardar?",
    );

    let mut cards = Vec::new();
    loop {
        match events.recv_timeout(Duration::from_secs(240)) {
            Ok(Event::ItemAdded { item }) => {
                if let ItemContent::SecretRequest {
                    status: RequestStatus::Pending,
                    label,
                    fields,
                    pasted,
                    ..
                } = &item.content
                {
                    cards.push((label.clone(), fields.len(), *pasted));
                    let answer = if *pasted {
                        SecretAnswer::Keep {
                            keep: VaultKeep {
                                name: label.clone(),
                                access: VaultAccess::Ask,
                            },
                        }
                    } else {
                        SecretAnswer::Decline
                    };
                    engine.answer_secret_request("t", &item.id, answer).unwrap();
                }
            }
            Ok(Event::TurnCompleted { outcome, .. }) => {
                assert_eq!(outcome, TurnOutcome::Completed);
                break;
            }
            Ok(_) => {}
            Err(_) => panic!("timed out; the cards: {cards:?}"),
        }
    }

    assert_eq!(cards.len(), 1, "{cards:?}");
    assert!(cards[0].2, "the card asked to type the values again");
    let kept = engine.vault_secrets().unwrap();
    assert_eq!(kept.len(), 1, "{kept:?}");
    let values: Vec<String> = kept[0]
        .fields
        .iter()
        .map(|field| engine.reveal_vault_secret(&kept[0].id, &field.id).unwrap())
        .collect();
    assert_eq!(values, ["hola@example.dev", TOKEN]);
}
