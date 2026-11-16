//! What Codex asks the user mid-turn, as hoshi's requests, and the answers it takes back.

use super::items::{ToolCall, unwrap_shell};
use crate::truncate_json;
use hoshi_core::{
    ApprovalDecision, ItemContent, Question, QuestionOption, RequestResponse, RequestStatus,
};
use serde_json::{Map, Value, json};

const APPROVAL_INPUT_MAX_CHARS: usize = 2_000;

/// What a request asks for, which says how its answer is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    Command,
    FileChange,
    Permissions,
    Questions,
    Elicitation,
}

impl Kind {
    pub fn of(method: &str) -> Option<Self> {
        match method {
            "item/commandExecution/requestApproval" => Some(Self::Command),
            "item/fileChange/requestApproval" => Some(Self::FileChange),
            "item/permissions/requestApproval" => Some(Self::Permissions),
            "item/tool/requestUserInput" => Some(Self::Questions),
            "mcpServer/elicitation/request" => Some(Self::Elicitation),
            _ => None,
        }
    }

    /// Whether accepting one for the session accepts the rest of its kind without asking.
    pub fn lasts_the_session(self) -> bool {
        matches!(self, Self::Command | Self::FileChange)
    }
}

/// hoshi's id for the request: the item it holds back, when it names one.
pub(crate) fn request_id(kind: Kind, params: &Value, rpc_id: &Value) -> String {
    match params["itemId"].as_str() {
        Some(item) if kind == Kind::Questions => format!("questions-{item}"),
        Some(item) => format!("approval-{item}"),
        None => format!(
            "elicitation-{}-{rpc_id}",
            params["turnId"].as_str().unwrap_or_default()
        ),
    }
}

/// The request as the transcript shows it. `tool` is the call it holds back, when known.
pub(crate) fn content(kind: Kind, params: &Value, tool: Option<ToolCall>) -> ItemContent {
    if kind == Kind::Questions {
        return ItemContent::Questions {
            questions: questions(params),
            status: RequestStatus::Pending,
            answers: None,
        };
    }
    let reason = params["reason"]
        .as_str()
        .or_else(|| params["grantRoot"].as_str())
        .or_else(|| params["message"].as_str())
        .filter(|reason| !reason.trim().is_empty())
        .map(str::to_owned);
    let (tool_name, input) =
        tool.map_or_else(|| untracked(kind, params), |tool| (tool.name, tool.input));
    ItemContent::Approval {
        tool_call_id: params["itemId"].as_str().map(str::to_owned),
        tool_name,
        input: truncate_json(&input, APPROVAL_INPUT_MAX_CHARS),
        reason,
        can_accept_for_session: kind != Kind::Elicitation || accepts(params, true).is_some(),
        status: RequestStatus::Pending,
        decision: None,
        feedback: None,
    }
}

/// What a request holds back when no call of the transcript shows it.
fn untracked(kind: Kind, params: &Value) -> (String, Value) {
    match kind {
        Kind::Command => (
            "Bash".into(),
            json!({ "command": unwrap_shell(params["command"].as_str().unwrap_or_default()) }),
        ),
        Kind::FileChange => ("Edit".into(), json!({ "file_path": params["grantRoot"] })),
        Kind::Permissions => ("Permissions".into(), params["permissions"].clone()),
        Kind::Elicitation => {
            let input = match &params["requestedSchema"] {
                Value::Null => json!({ "url": params["url"] }),
                schema => schema.clone(),
            };
            (
                format!("mcp__{}", params["serverName"].as_str().unwrap_or_default()),
                input,
            )
        }
        Kind::Questions => ("AskUserQuestion".into(), params["questions"].clone()),
    }
}

fn questions(params: &Value) -> Vec<Question> {
    params["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|question| Question {
            header: question["header"].as_str().unwrap_or_default().to_owned(),
            question: question["question"].as_str().unwrap_or_default().to_owned(),
            options: question["options"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|option| QuestionOption {
                    label: option["label"].as_str().unwrap_or_default().to_owned(),
                    description: option["description"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    preview: None,
                })
                .collect(),
            multi_select: false,
        })
        .collect()
}

/// The answer to a command or file change the user already accepted for the session.
pub(crate) fn accepted_for_session() -> Value {
    json!({ "decision": "acceptForSession" })
}

/// What Codex takes back for the user's `response`.
pub(crate) fn answer(kind: Kind, params: &Value, response: &RequestResponse) -> Value {
    let decision = match response {
        RequestResponse::Decision { decision, .. } => *decision,
        RequestResponse::Answers { answers } if kind == Kind::Questions => {
            return json!({ "answers": question_answers(params, answers) });
        }
        _ => ApprovalDecision::Decline,
    };
    match kind {
        Kind::Command | Kind::FileChange => json!({
            "decision": match decision {
                ApprovalDecision::Accept => "accept",
                ApprovalDecision::AcceptForSession => "acceptForSession",
                ApprovalDecision::Decline => "decline",
                ApprovalDecision::Cancel => "cancel",
            },
        }),
        Kind::Permissions if decision.accepts() => json!({
            "permissions": params["permissions"],
            "scope": if decision == ApprovalDecision::AcceptForSession { "session" } else { "turn" },
        }),
        Kind::Permissions => json!({ "permissions": {}, "scope": "turn" }),
        Kind::Elicitation if decision.accepts() => {
            accepts(params, decision == ApprovalDecision::AcceptForSession)
                .unwrap_or_else(|| json!({ "action": "decline" }))
        }
        Kind::Elicitation if decision == ApprovalDecision::Cancel => json!({ "action": "cancel" }),
        Kind::Elicitation => json!({ "action": "decline" }),
        Kind::Questions => json!({ "answers": {} }),
    }
}

/// What a request the turn no longer waits for is answered with.
pub(crate) fn withdrawn(kind: Kind) -> Value {
    match kind {
        Kind::Command | Kind::FileChange => json!({ "decision": "cancel" }),
        Kind::Permissions => json!({ "permissions": {}, "scope": "turn" }),
        Kind::Elicitation => json!({ "action": "cancel" }),
        Kind::Questions => json!({ "answers": {} }),
    }
}

fn question_answers(params: &Value, answers: &[String]) -> Value {
    let ids = params["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|question| question["id"].as_str());
    ids.zip(answers)
        .map(|(id, answer)| (id.to_owned(), json!({ "answers": [answer] })))
        .collect::<Map<_, _>>()
        .into()
}

/// Accepts an MCP server's form, filled in with the choices that allow what it asks, once or
/// for the session; `None` when it can't be accepted so, as a page to open or a form that
/// needs the user's own words.
fn accepts(params: &Value, for_session: bool) -> Option<Value> {
    if params["mode"] == "url" {
        return None;
    }
    let schema = &params["requestedSchema"];
    let mut content = Map::new();
    for (key, field) in schema["properties"].as_object().into_iter().flatten() {
        let choices: Vec<&str> = field["oneOf"]
            .as_array()
            .map(|options| {
                options
                    .iter()
                    .filter_map(|option| option["const"].as_str())
                    .collect()
            })
            .or_else(|| {
                field["enum"]
                    .as_array()
                    .map(|options| options.iter().filter_map(Value::as_str).collect())
            })
            .unwrap_or_default();
        let chosen = choices.into_iter().find(|choice| {
            let choice = choice.to_ascii_lowercase();
            if for_session {
                choice.contains("session")
            } else {
                !choice.contains("session")
                    && !choice.contains("always")
                    && ["once", "accept", "approve", "allow"]
                        .iter()
                        .any(|word| choice.starts_with(word))
            }
        });
        if let Some(chosen) = chosen {
            content.insert(key.clone(), json!(chosen));
        } else if field["type"] == "boolean" && persists(key, field) {
            content.insert(key.clone(), json!(false));
        } else if !field["default"].is_null() {
            content.insert(key.clone(), field["default"].clone());
        }
    }
    let missing = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|key| !content.contains_key(key));
    if missing {
        return None;
    }
    let mut answer = json!({ "action": "accept" });
    if !schema.is_null() {
        answer["content"] = content.into();
    }
    if for_session {
        answer["_meta"] = json!({ "persist": "session" });
    }
    Some(answer)
}

/// A field that asks whether the approval should last.
fn persists(key: &str, field: &Value) -> bool {
    [
        key,
        field["title"].as_str().unwrap_or_default(),
        field["description"].as_str().unwrap_or_default(),
    ]
    .iter()
    .any(|text| {
        let text = text.to_ascii_lowercase();
        ["persist", "always", "session", "remember"]
            .iter()
            .any(|word| text.contains(word))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_request() -> Value {
        serde_json::from_str(r#"{"kind":"command","threadId":"01a1db17-c7a7","turnId":"01a1db17-c7e2","itemId":"exec-61116b80","startedAtMs":1794677136120,"environmentId":"local","command":"\"C:\\\\WINDOWS\\\\System32\\\\WindowsPowerShell\\\\v1.0\\\\powershell.exe\" -Command 'type note.txt'","cwd":"C:\\work","commandActions":[{"type":"read","command":"type note.txt","name":"note.txt","path":"C:\\work\\note.txt"}],"proposedExecpolicyAmendment":["type","note.txt"],"availableDecisions":["accept",{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["type","note.txt"]}},"cancel"]}"#).unwrap()
    }

    #[test]
    fn a_command_approval_shows_the_call_it_holds_back() {
        let params = command_request();
        let kind = Kind::of("item/commandExecution/requestApproval").unwrap();
        let tool = ToolCall {
            name: "Read".into(),
            input: json!({ "file_path": "C:\\work\\note.txt" }),
        };

        assert_eq!(
            request_id(kind, &params, &json!(0)),
            "approval-exec-61116b80"
        );
        assert_eq!(
            content(kind, &params, Some(tool)),
            ItemContent::Approval {
                tool_call_id: Some("exec-61116b80".into()),
                tool_name: "Read".into(),
                input: r#"{"file_path":"C:\\work\\note.txt"}"#.into(),
                reason: None,
                can_accept_for_session: true,
                status: RequestStatus::Pending,
                decision: None,
                feedback: None,
            }
        );
        let ItemContent::Approval {
            tool_name, input, ..
        } = content(kind, &params, None)
        else {
            panic!("an approval");
        };
        assert_eq!(tool_name, "Bash");
        assert_eq!(input, r#"{"command":"type note.txt"}"#);
    }

    #[test]
    fn decisions_answer_in_codexs_words() {
        let params = command_request();
        let decide = |decision| {
            answer(
                Kind::Command,
                &params,
                &RequestResponse::Decision {
                    decision,
                    feedback: None,
                },
            )
        };
        assert_eq!(
            decide(ApprovalDecision::Accept),
            json!({ "decision": "accept" })
        );
        assert_eq!(
            decide(ApprovalDecision::AcceptForSession),
            json!({ "decision": "acceptForSession" })
        );
        assert_eq!(
            decide(ApprovalDecision::Decline),
            json!({ "decision": "decline" })
        );
        assert_eq!(
            decide(ApprovalDecision::Cancel),
            json!({ "decision": "cancel" })
        );
        assert_eq!(withdrawn(Kind::FileChange), json!({ "decision": "cancel" }));
        assert_eq!(
            accepted_for_session(),
            json!({ "decision": "acceptForSession" })
        );
    }

    #[test]
    fn questions_are_asked_and_answered_by_their_ids() {
        let params: Value = serde_json::from_str(r#"{"threadId":"t","turnId":"u","itemId":"call_wGA02D7X","questions":[{"id":"schema_preference","header":"Fixture","question":"Should this fixture prefer strict schemas or UI flexibility?","isOther":true,"isSecret":false,"options":[{"label":"Strict schemas (Recommended)","description":"Keep fixture data tightly validated."},{"label":"UI flexibility","description":"Allow looser shapes."}]}],"isBlocking":true,"autoResolutionMs":null}"#).unwrap();
        let kind = Kind::of("item/tool/requestUserInput").unwrap();

        assert_eq!(
            request_id(kind, &params, &json!(0)),
            "questions-call_wGA02D7X"
        );
        let ItemContent::Questions { questions, .. } = content(kind, &params, None) else {
            panic!("questions");
        };
        assert_eq!(questions[0].header, "Fixture");
        assert_eq!(questions[0].options[1].label, "UI flexibility");
        assert_eq!(
            answer(
                kind,
                &params,
                &RequestResponse::Answers {
                    answers: vec!["Strict schemas (Recommended)".into()]
                }
            ),
            json!({ "answers": { "schema_preference": { "answers": ["Strict schemas (Recommended)"] } } })
        );
    }

    #[test]
    fn an_mcp_form_is_accepted_with_the_choice_that_allows_it() {
        let params = json!({
            "serverName": "codex_apps",
            "threadId": "t",
            "turnId": "u",
            "mode": "form",
            "message": "Allow ChatGPT to use Gmail?",
            "requestedSchema": {
                "type": "object",
                "properties": {
                    "decision": { "type": "string", "enum": ["accept", "accept_for_session", "decline"] },
                },
                "required": ["decision"],
            },
        });
        let decide = |decision| {
            answer(
                Kind::Elicitation,
                &params,
                &RequestResponse::Decision {
                    decision,
                    feedback: None,
                },
            )
        };

        assert_eq!(
            decide(ApprovalDecision::Accept),
            json!({ "action": "accept", "content": { "decision": "accept" } })
        );
        assert_eq!(
            decide(ApprovalDecision::AcceptForSession),
            json!({ "action": "accept", "content": { "decision": "accept_for_session" }, "_meta": { "persist": "session" } })
        );
        assert_eq!(
            decide(ApprovalDecision::Decline),
            json!({ "action": "decline" })
        );
        let url = json!({ "serverName": "s", "mode": "url", "url": "https://example.com" });
        assert_eq!(
            answer(
                Kind::Elicitation,
                &url,
                &RequestResponse::Decision {
                    decision: ApprovalDecision::Accept,
                    feedback: None
                }
            ),
            json!({ "action": "decline" })
        );
    }

    #[test]
    fn permissions_are_granted_for_the_turn_or_the_session() {
        let params =
            json!({ "itemId": "exec-1", "permissions": { "network": { "enabled": true } } });
        let decide = |decision| {
            answer(
                Kind::Permissions,
                &params,
                &RequestResponse::Decision {
                    decision,
                    feedback: None,
                },
            )
        };

        assert_eq!(
            decide(ApprovalDecision::AcceptForSession),
            json!({ "permissions": { "network": { "enabled": true } }, "scope": "session" })
        );
        assert_eq!(
            decide(ApprovalDecision::Decline),
            json!({ "permissions": {}, "scope": "turn" })
        );
    }
}
