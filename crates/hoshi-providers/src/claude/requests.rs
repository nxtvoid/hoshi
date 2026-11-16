//! Requests Claude Code makes mid-turn over its control protocol, started by
//! `--permission-prompt-tool stdio`: tool approvals, `AskUserQuestion`, `ExitPlanMode`, and
//! the questions MCP servers ask the user through it.

use crate::{MCP_SERVER_NAME, truncate, truncate_json};
use hoshi_core::{
    AccessMode, ApprovalDecision, ItemContent, ProviderOutput, Question, QuestionOption,
    RequestResponse, RequestStatus,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const TOOL_INPUT_MAX_CHARS: usize = 2_000;

/// What the turn runs with, which decides what is asked and how answers apply.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Access {
    pub mode: AccessMode,
    pub plan_mode: bool,
}

impl Access {
    pub fn permission_mode(self) -> &'static str {
        if self.plan_mode {
            return "plan";
        }
        mode_name(self.mode)
    }

    /// Auto mode leaves the call to Claude's classifier; asking on top would ask twice.
    /// hoshi's own tools never need the user's approval.
    fn approves_on_its_own(self, tool_name: &str) -> bool {
        let own = tool_name
            .strip_prefix("mcp__")
            .and_then(|rest| rest.strip_prefix(MCP_SERVER_NAME))
            .is_some_and(|rest| rest.starts_with("__"));
        own || (self.mode == AccessMode::Auto
            && !self.plan_mode
            && !matches!(tool_name, "AskUserQuestion" | "ExitPlanMode"))
    }
}

fn mode_name(mode: AccessMode) -> &'static str {
    match mode {
        AccessMode::Ask => "default",
        AccessMode::AcceptEdits => "acceptEdits",
        AccessMode::Auto => "auto",
        AccessMode::FullAccess => "bypassPermissions",
    }
}

/// A request the agent waits on, as Claude sent it.
#[derive(Debug, Clone)]
pub(crate) struct PendingRequest {
    tool_name: String,
    input: Value,
    tool_use_id: Option<String>,
    suggestions: Vec<Value>,
    /// Set when an MCP server asks rather than a tool call.
    elicitation: Option<Elicitation>,
}

/// An MCP server's question, which the user answers as an approval.
#[derive(Debug, Clone)]
struct Elicitation {
    /// What accepting it for the session records.
    grant: Value,
    /// The answer accepting it sends.
    acceptance: Value,
}

#[derive(Debug, Default)]
struct TurnState {
    session_id: Option<String>,
    pending: HashMap<String, PendingRequest>,
    grants: Grants,
}

/// The turn's open requests, shared by the stdout reader and the running turn.
#[derive(Debug, Clone, Default)]
pub(crate) struct Requests(Arc<Mutex<TurnState>>);

impl Requests {
    /// Requests that answer on their own what `grants` holds for the session.
    pub fn new(grants: Grants) -> Self {
        Self(Arc::new(Mutex::new(TurnState {
            grants,
            ..TurnState::default()
        })))
    }

    fn state(&self) -> std::sync::MutexGuard<'_, TurnState> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn set_session(&self, session_id: &str) {
        self.state().session_id = Some(session_id.to_owned());
    }

    pub fn session(&self) -> Option<String> {
        self.state().session_id.clone()
    }

    pub fn take(&self, request_id: &str) -> Option<PendingRequest> {
        self.state().pending.remove(request_id)
    }
}

/// Permission rules accepted for a session, by session id. Each turn is its own process,
/// so they are passed again whenever the session resumes.
#[derive(Debug, Clone, Default)]
pub(crate) struct Grants(Arc<Mutex<HashMap<String, Vec<Value>>>>);

impl Grants {
    fn add(&self, session_id: &str, updates: &[Value]) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(session_id.to_owned())
            .or_default()
            .extend(updates.iter().cloned());
    }

    fn holds(&self, session_id: &str, grant: &Value) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(session_id)
            .is_some_and(|grants| grants.contains(grant))
    }

    /// `--allowedTools` rules and `--add-dir` folders for a resumed session.
    pub fn arguments(&self, session_id: &str) -> (Vec<String>, Vec<String>) {
        let grants = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut rules = Vec::new();
        let mut directories = Vec::new();
        for update in grants.get(session_id).into_iter().flatten() {
            match update["type"].as_str() {
                Some("addRules") if update["behavior"] == "allow" => {
                    for rule in update["rules"].as_array().into_iter().flatten() {
                        let Some(tool) = rule["toolName"].as_str() else {
                            continue;
                        };
                        rules.push(match rule["ruleContent"].as_str() {
                            Some(content) => format!("{tool}({content})"),
                            None => tool.to_owned(),
                        });
                    }
                }
                Some("addDirectories") => directories.extend(
                    update["directories"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned),
                ),
                _ => {}
            }
        }
        rules.dedup();
        directories.dedup();
        (rules, directories)
    }
}

/// What a `control_request` from the agent leads to: what the thread records, and the
/// line to write back at once, if any.
pub(crate) fn receive(
    value: &Value,
    access: Access,
    requests: &Requests,
) -> (Vec<ProviderOutput>, Option<String>) {
    let Some(request_id) = value["request_id"].as_str() else {
        return (Vec::new(), None);
    };
    let request = &value["request"];
    match request["subtype"].as_str() {
        Some("can_use_tool") => {}
        Some("elicitation") => return elicitation(request_id, request, requests),
        _ => {
            return (
                Vec::new(),
                Some(error_line(request_id, "unsupported request")),
            );
        }
    }
    let tool_name = request["tool_name"].as_str().unwrap_or_default().to_owned();
    let input = request["input"].clone();
    let tool_use_id = request["tool_use_id"].as_str().map(str::to_owned);
    if access.approves_on_its_own(&tool_name) {
        let allow = json!({ "behavior": "allow", "updatedInput": input, "toolUseID": tool_use_id });
        return (Vec::new(), Some(success_line(request_id, allow)));
    }
    let content = match tool_name.as_str() {
        "AskUserQuestion" => match questions(&input) {
            Some(questions) => ItemContent::Questions {
                questions,
                status: RequestStatus::Pending,
                answers: None,
            },
            None => {
                let deny = json!({
                    "behavior": "deny",
                    "message": "The questions could not be read. Ask in plain text instead.",
                    "toolUseID": tool_use_id,
                });
                return (Vec::new(), Some(success_line(request_id, deny)));
            }
        },
        "ExitPlanMode" => ItemContent::ProposedPlan {
            plan: input["plan"].as_str().unwrap_or_default().to_owned(),
            status: RequestStatus::Pending,
            decision: None,
            feedback: None,
        },
        _ => ItemContent::Approval {
            tool_call_id: tool_use_id.clone(),
            tool_name: tool_name.clone(),
            input: truncate_json(&input, TOOL_INPUT_MAX_CHARS),
            reason: request["decision_reason"]
                .as_str()
                .map(strip_ansi)
                .filter(|reason| !reason.trim().is_empty()),
            can_accept_for_session: request["suppress_always_allow_rule"] != true,
            status: RequestStatus::Pending,
            decision: None,
            feedback: None,
        },
    };
    let suggestions = request["permission_suggestions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    requests.state().pending.insert(
        request_id.to_owned(),
        PendingRequest {
            tool_name,
            input,
            tool_use_id,
            suggestions,
            elicitation: None,
        },
    );
    let opened = ProviderOutput::RequestOpened {
        request_id: request_id.to_owned(),
        content,
    };
    (vec![opened], None)
}

/// An MCP server asks the user to confirm, as an approval: it is answered at once when the
/// user accepted the same question for the session, and declined when its form can't be
/// filled by consenting alone.
fn elicitation(
    request_id: &str,
    request: &Value,
    requests: &Requests,
) -> (Vec<ProviderOutput>, Option<String>) {
    let Some(acceptance) = elicitation_acceptance(request) else {
        let decline = json!({ "action": "decline" });
        return (Vec::new(), Some(success_line(request_id, decline)));
    };
    let server = request["mcp_server_name"].as_str().unwrap_or_default();
    let message = request["message"].as_str().unwrap_or_default();
    let grant = json!({ "type": "mcpElicitation", "server": server, "message": message });
    let mut state = requests.state();
    let granted = state
        .session_id
        .as_deref()
        .is_some_and(|session_id| state.grants.holds(session_id, &grant));
    if granted {
        return (Vec::new(), Some(success_line(request_id, acceptance)));
    }
    let name = ["display_name", "title", "mcp_server_name"]
        .iter()
        .find_map(|key| request[key].as_str().filter(|name| !name.trim().is_empty()))
        .unwrap_or("MCP")
        .to_owned();
    let content = ItemContent::Approval {
        tool_call_id: None,
        tool_name: name.clone(),
        input: truncate(message, TOOL_INPUT_MAX_CHARS),
        reason: request["description"]
            .as_str()
            .map(strip_ansi)
            .filter(|reason| !reason.trim().is_empty()),
        can_accept_for_session: true,
        status: RequestStatus::Pending,
        decision: None,
        feedback: None,
    };
    state.pending.insert(
        request_id.to_owned(),
        PendingRequest {
            tool_name: name,
            input: Value::Null,
            tool_use_id: None,
            suggestions: Vec::new(),
            elicitation: Some(Elicitation { grant, acceptance }),
        },
    );
    let opened = ProviderOutput::RequestOpened {
        request_id: request_id.to_owned(),
        content,
    };
    (vec![opened], None)
}

/// The answer that accepts an elicitation: its form filled with the choices that consent
/// once, never with the server's defaults, which could grant what the user did not see.
/// `None` when consent alone can't fill it, as a URL to open or a required field with no
/// such choice.
fn elicitation_acceptance(request: &Value) -> Option<Value> {
    if request["mode"] == "url" {
        return None;
    }
    let schema = request["requested_schema"].as_object()?;
    let properties = match schema.get("properties") {
        None => serde_json::Map::new(),
        Some(properties) => properties.as_object()?.clone(),
    };
    let mut content = serde_json::Map::new();
    for (key, field) in &properties {
        let field = field.as_object()?;
        let text = |name: &str| field.get(name).and_then(Value::as_str).unwrap_or_default();
        let options: Vec<&str> = match field.get("oneOf").and_then(Value::as_array) {
            Some(choices) => choices
                .iter()
                .filter_map(|choice| choice["const"].as_str())
                .collect(),
            None => field
                .get("enum")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect(),
        };
        if let Some(once) = options.into_iter().find(|option| consents_once(option)) {
            content.insert(key.clone(), json!(once));
        } else if text("type") == "boolean"
            && (persists(key)
                || key.eq_ignore_ascii_case("persist")
                || persists(text("title"))
                || persists(text("description")))
        {
            content.insert(key.clone(), json!(false));
        }
    }
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str);
    for key in required {
        if !content.contains_key(key) {
            return None;
        }
    }
    Some(json!({ "action": "accept", "content": content }))
}

/// Whether a choice keeps the consent beyond this once.
fn persists(text: &str) -> bool {
    let text = text.to_lowercase();
    ["session", "always", "permanent", "forever", "persistent"]
        .iter()
        .any(|word| text.contains(word))
}

/// Whether a choice consents this once: `accept`, `allow once`, `approve this time`...
fn consents_once(option: &str) -> bool {
    let option = option.trim().to_lowercase();
    let separator = |char: char| char == '_' || char == '-' || char.is_whitespace();
    let Some(rest) = ["once", "accept", "approve", "allow"]
        .iter()
        .find_map(|verb| option.strip_prefix(verb))
    else {
        return false;
    };
    let suffix = rest.trim_start_matches(separator);
    let qualified = match suffix.strip_prefix("once") {
        Some(after) => after.is_empty() || after.trim_start_matches(separator) == "only",
        None => suffix
            .strip_prefix("this")
            .is_some_and(|after| after.trim_start_matches(separator) == "time"),
    };
    (rest.is_empty() || qualified) && !persists(&option)
}

/// The agent withdrew a request, as when a hook answered it.
pub(crate) fn withdraw(value: &Value, requests: &Requests) -> Vec<ProviderOutput> {
    let Some(request_id) = value["request_id"].as_str() else {
        return Vec::new();
    };
    match requests.take(request_id) {
        Some(_) => vec![ProviderOutput::RequestWithdrawn {
            request_id: request_id.to_owned(),
        }],
        None => Vec::new(),
    }
}

/// The `control_response` line answering `pending`. Rules accepted for the session go to
/// `grants` too.
pub(crate) fn reply(
    request_id: &str,
    pending: &PendingRequest,
    response: &RequestResponse,
    access: Access,
    session: Option<(&str, &Grants)>,
) -> String {
    if let Some(elicitation) = &pending.elicitation {
        let answer = match response {
            RequestResponse::Decision {
                decision: ApprovalDecision::Accept,
                ..
            } => elicitation.acceptance.clone(),
            RequestResponse::Decision {
                decision: ApprovalDecision::AcceptForSession,
                ..
            } => {
                if let Some((session_id, grants)) = session {
                    grants.add(session_id, std::slice::from_ref(&elicitation.grant));
                }
                elicitation.acceptance.clone()
            }
            RequestResponse::Decision {
                decision: ApprovalDecision::Cancel,
                ..
            } => json!({ "action": "cancel" }),
            _ => json!({ "action": "decline" }),
        };
        return success_line(request_id, answer);
    }
    let tool_use_id = &pending.tool_use_id;
    let result = match response {
        // Secrets are hoshi's own requests; none reaches the agent's.
        RequestResponse::Secret { .. } => {
            json!({ "behavior": "deny", "message": "", "toolUseID": tool_use_id })
        }
        RequestResponse::Answers { answers } => {
            let mut input = pending.input.clone();
            let answered: serde_json::Map<String, Value> = input["questions"]
                .as_array()
                .into_iter()
                .flatten()
                .zip(answers)
                .filter_map(|(question, answer)| {
                    Some((question["question"].as_str()?.to_owned(), json!(answer)))
                })
                .collect();
            input["answers"] = Value::Object(answered);
            json!({ "behavior": "allow", "updatedInput": input, "toolUseID": tool_use_id })
        }
        RequestResponse::Decision { decision, feedback } => {
            let feedback = feedback
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty());
            let plan = pending.tool_name == "ExitPlanMode";
            match decision {
                ApprovalDecision::Accept | ApprovalDecision::AcceptForSession => {
                    let mut allow = json!({
                        "behavior": "allow",
                        "updatedInput": pending.input,
                        "toolUseID": tool_use_id,
                        "decisionClassification": "user_temporary",
                    });
                    if plan {
                        allow["updatedPermissions"] = json!([{
                            "type": "setMode",
                            "mode": mode_name(access.mode),
                            "destination": "session",
                        }]);
                    } else if *decision == ApprovalDecision::AcceptForSession {
                        let updates = session_updates(pending);
                        if let Some((session_id, grants)) = session {
                            grants.add(session_id, &updates);
                        }
                        allow["updatedPermissions"] = Value::Array(updates);
                        allow["decisionClassification"] = json!("user_permanent");
                    }
                    allow
                }
                ApprovalDecision::Decline => {
                    let message = match (plan, feedback) {
                        (true, Some(feedback)) => {
                            format!("The user wants changes to the plan before it is carried out: {feedback}")
                        }
                        (true, None) => "The user did not accept the plan. Stop here and wait for their feedback.".to_owned(),
                        (false, Some(feedback)) => {
                            format!("The user declined this tool call and said: {feedback}")
                        }
                        (false, None) => "The user declined this tool call.".to_owned(),
                    };
                    json!({
                        "behavior": "deny",
                        "message": message,
                        "toolUseID": tool_use_id,
                        "decisionClassification": "user_reject",
                    })
                }
                ApprovalDecision::Cancel => json!({
                    "behavior": "deny",
                    "message": "The user stopped the turn.",
                    "interrupt": true,
                    "toolUseID": tool_use_id,
                    "decisionClassification": "user_reject",
                }),
            }
        }
    };
    success_line(request_id, result)
}

/// Claude's own suggestions, kept to the session rather than written to settings files;
/// a rule for the whole tool when it made none.
fn session_updates(pending: &PendingRequest) -> Vec<Value> {
    let updates: Vec<Value> = pending
        .suggestions
        .iter()
        .cloned()
        .map(|mut update| {
            update["destination"] = json!("session");
            update
        })
        .collect();
    if !updates.is_empty() {
        return updates;
    }
    vec![json!({
        "type": "addRules",
        "rules": [{ "toolName": pending.tool_name }],
        "behavior": "allow",
        "destination": "session",
    })]
}

fn questions(input: &Value) -> Option<Vec<Question>> {
    let questions: Vec<Question> = input["questions"]
        .as_array()?
        .iter()
        .enumerate()
        .filter_map(|(index, question)| {
            let text = question["question"].as_str()?.trim().to_owned();
            let options = question["options"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|option| {
                    let label = option["label"].as_str()?.trim().to_owned();
                    Some(QuestionOption {
                        description: option["description"]
                            .as_str()
                            .unwrap_or_default()
                            .trim()
                            .to_owned(),
                        preview: option["preview"].as_str().map(str::to_owned),
                        label,
                    })
                })
                .collect();
            Some(Question {
                header: question["header"]
                    .as_str()
                    .map(str::trim)
                    .filter(|header| !header.is_empty())
                    .map_or_else(|| format!("{}", index + 1), str::to_owned),
                question: text,
                options,
                multi_select: question["multiSelect"] == true,
            })
        })
        .collect();
    (!questions.is_empty()).then_some(questions)
}

fn success_line(request_id: &str, response: Value) -> String {
    json!({
        "type": "control_response",
        "response": { "subtype": "success", "request_id": request_id, "response": response },
    })
    .to_string()
}

fn error_line(request_id: &str, error: &str) -> String {
    json!({
        "type": "control_response",
        "response": { "subtype": "error", "request_id": request_id, "error": error },
    })
    .to_string()
}

/// Claude colors some reasons for its terminal.
pub(crate) fn strip_ansi(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(char) = chars.next() {
        if char != '\u{1b}' {
            plain.push(char);
            continue;
        }
        if chars.next() == Some('[') {
            for code in chars.by_ref() {
                if code.is_ascii_alphabetic() {
                    break;
                }
            }
        }
    }
    plain
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASK: Access = Access {
        mode: AccessMode::Ask,
        plan_mode: false,
    };

    fn can_use_tool(tool_name: &str, input: Value) -> Value {
        json!({
            "type": "control_request",
            "request_id": "r1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": tool_name,
                "input": input,
                "tool_use_id": "toolu_1",
                "permission_suggestions": [
                    { "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "npm test:*" }],
                      "behavior": "allow", "destination": "localSettings" },
                    { "type": "setMode", "mode": "acceptEdits", "destination": "session" }
                ],
                "decision_reason": "\u{1b}[1mBash\u{1b}[0m needs approval",
            },
        })
    }

    fn response(line: &str) -> Value {
        let value: Value = serde_json::from_str(line).unwrap();
        assert_eq!(value["type"], "control_response");
        assert_eq!(value["response"]["request_id"], "r1");
        value["response"]["response"].clone()
    }

    #[test]
    fn a_tool_call_waits_for_approval() {
        let requests = Requests::default();
        let (outputs, line) = receive(
            &can_use_tool("Bash", json!({ "command": "npm test" })),
            ASK,
            &requests,
        );

        assert_eq!(line, None);
        assert_eq!(
            outputs,
            vec![ProviderOutput::RequestOpened {
                request_id: "r1".into(),
                content: ItemContent::Approval {
                    tool_call_id: Some("toolu_1".into()),
                    tool_name: "Bash".into(),
                    input: r#"{"command":"npm test"}"#.into(),
                    reason: Some("Bash needs approval".into()),
                    can_accept_for_session: true,
                    status: RequestStatus::Pending,
                    decision: None,
                    feedback: None,
                },
            }]
        );
        assert!(requests.take("r1").is_some());
    }

    #[test]
    fn hoshis_own_tools_need_no_approval() {
        let requests = Requests::default();
        let (outputs, line) = receive(
            &can_use_tool("mcp__hoshi__delegate_task", json!({ "task": "x" })),
            ASK,
            &requests,
        );
        assert!(outputs.is_empty());
        assert_eq!(response(&line.unwrap())["behavior"], "allow");

        let (outputs, _) = receive(
            &can_use_tool("mcp__hoshi_fake__delegate_task", json!({})),
            ASK,
            &requests,
        );
        assert_eq!(outputs.len(), 1, "another server's tool still asks");
    }

    #[test]
    fn auto_mode_lets_claude_decide_but_still_asks_questions() {
        let auto = Access {
            mode: AccessMode::Auto,
            plan_mode: false,
        };
        let requests = Requests::default();
        let (outputs, line) = receive(
            &can_use_tool("Bash", json!({ "command": "ls" })),
            auto,
            &requests,
        );
        assert!(outputs.is_empty());
        assert_eq!(response(&line.unwrap())["behavior"], "allow");

        let question = json!({ "questions": [{ "question": "Color?", "header": "Color",
            "options": [{ "label": "Red", "description": "" }], "multiSelect": false }] });
        let (outputs, line) = receive(&can_use_tool("AskUserQuestion", question), auto, &requests);
        assert_eq!(line, None);
        assert!(matches!(
            outputs.as_slice(),
            [ProviderOutput::RequestOpened {
                content: ItemContent::Questions { .. },
                ..
            }]
        ));
    }

    #[test]
    fn answers_go_back_keyed_by_question() {
        let requests = Requests::default();
        let input = json!({ "questions": [
            { "question": "Color?", "header": "Color", "multiSelect": false,
              "options": [{ "label": "Red", "description": "Warm" }, { "label": "Blue", "description": "Cold" }] },
            { "question": "Sizes?", "header": "", "multiSelect": true,
              "options": [{ "label": "S", "description": "" }, { "label": "M", "description": "" }] }
        ] });
        let (outputs, _) = receive(&can_use_tool("AskUserQuestion", input), ASK, &requests);
        let [
            ProviderOutput::RequestOpened {
                content: ItemContent::Questions { questions, .. },
                ..
            },
        ] = outputs.as_slice()
        else {
            panic!("expected questions, got {outputs:?}");
        };
        assert_eq!(questions[1].header, "2");
        assert!(questions[1].multi_select);

        let pending = requests.take("r1").unwrap();
        let answers = RequestResponse::Answers {
            answers: vec!["Red".into(), "S, M".into()],
        };
        let reply = response(&reply("r1", &pending, &answers, ASK, None));

        assert_eq!(reply["behavior"], "allow");
        assert_eq!(
            reply["updatedInput"]["answers"],
            json!({ "Color?": "Red", "Sizes?": "S, M" })
        );
        assert_eq!(reply["updatedInput"]["questions"][0]["question"], "Color?");
    }

    #[test]
    fn accepting_for_the_session_keeps_the_rules_to_the_session() {
        let requests = Requests::default();
        receive(
            &can_use_tool("Bash", json!({ "command": "npm test" })),
            ASK,
            &requests,
        );
        let pending = requests.take("r1").unwrap();
        let grants = Grants::default();
        let decision = RequestResponse::Decision {
            decision: ApprovalDecision::AcceptForSession,
            feedback: None,
        };
        let reply = response(&reply(
            "r1",
            &pending,
            &decision,
            ASK,
            Some(("s1", &grants)),
        ));

        assert_eq!(reply["behavior"], "allow");
        assert_eq!(
            reply["updatedPermissions"],
            json!([
                { "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "npm test:*" }],
                  "behavior": "allow", "destination": "session" },
                { "type": "setMode", "mode": "acceptEdits", "destination": "session" }
            ])
        );
        assert_eq!(
            grants.arguments("s1"),
            (vec!["Bash(npm test:*)".to_owned()], Vec::new())
        );
    }

    #[test]
    fn declining_passes_the_feedback_and_cancelling_stops_the_turn() {
        let requests = Requests::default();
        receive(
            &can_use_tool("Bash", json!({ "command": "rm -rf x" })),
            ASK,
            &requests,
        );
        let pending = requests.take("r1").unwrap();
        let decline = RequestResponse::Decision {
            decision: ApprovalDecision::Decline,
            feedback: Some(" use git clean ".into()),
        };
        let declined = response(&reply("r1", &pending, &decline, ASK, None));
        assert_eq!(declined["behavior"], "deny");
        assert!(
            declined["message"]
                .as_str()
                .unwrap()
                .ends_with("use git clean")
        );
        assert_eq!(declined.get("interrupt"), None);

        let cancel = RequestResponse::Decision {
            decision: ApprovalDecision::Cancel,
            feedback: None,
        };
        assert_eq!(
            response(&reply("r1", &pending, &cancel, ASK, None))["interrupt"],
            true
        );
    }

    #[test]
    fn an_accepted_plan_leaves_plan_mode_for_the_thread_access() {
        let planning = Access {
            mode: AccessMode::AcceptEdits,
            plan_mode: true,
        };
        let requests = Requests::default();
        let (outputs, _) = receive(
            &can_use_tool("ExitPlanMode", json!({ "plan": "1. Do it" })),
            planning,
            &requests,
        );
        assert!(matches!(
            outputs.as_slice(),
            [ProviderOutput::RequestOpened {
                content: ItemContent::ProposedPlan { plan, .. },
                ..
            }] if plan == "1. Do it"
        ));
        let pending = requests.take("r1").unwrap();
        let accept = RequestResponse::Decision {
            decision: ApprovalDecision::Accept,
            feedback: None,
        };
        let reply = response(&reply("r1", &pending, &accept, planning, None));

        assert_eq!(reply["behavior"], "allow");
        assert_eq!(
            reply["updatedPermissions"],
            json!([{ "type": "setMode", "mode": "acceptEdits", "destination": "session" }])
        );
    }

    #[test]
    fn a_withdrawn_request_is_forgotten() {
        let requests = Requests::default();
        receive(&can_use_tool("Bash", json!({})), ASK, &requests);
        let cancel = json!({ "type": "control_cancel_request", "request_id": "r1" });

        assert_eq!(
            withdraw(&cancel, &requests),
            vec![ProviderOutput::RequestWithdrawn {
                request_id: "r1".into()
            }]
        );
        assert!(withdraw(&cancel, &requests).is_empty());
    }

    fn elicitation(schema: Value) -> Value {
        json!({
            "type": "control_request",
            "request_id": "r1",
            "request": {
                "subtype": "elicitation",
                "mcp_server_name": "computer-use",
                "message": "Allow Claude to control Finder?",
                "mode": "form",
                "requested_schema": schema,
                "display_name": "Computer Use",
            },
        })
    }

    fn consent_schema() -> Value {
        json!({ "type": "object", "required": ["choice"], "properties": {
            "choice": { "type": "string", "enum": ["allow_once", "allow_session", "deny"],
                        "default": "allow_session" },
            "remember": { "type": "boolean", "title": "Always allow", "default": true },
            "note": { "type": "string", "default": "granted" },
        } })
    }

    fn decide(decision: ApprovalDecision) -> RequestResponse {
        RequestResponse::Decision {
            decision,
            feedback: None,
        }
    }

    #[test]
    fn an_mcp_server_s_question_asks_as_an_approval() {
        let requests = Requests::default();
        requests.set_session("s1");
        let (outputs, line) = receive(&elicitation(consent_schema()), ASK, &requests);

        assert_eq!(line, None);
        assert_eq!(
            outputs,
            vec![ProviderOutput::RequestOpened {
                request_id: "r1".into(),
                content: ItemContent::Approval {
                    tool_call_id: None,
                    tool_name: "Computer Use".into(),
                    input: "Allow Claude to control Finder?".into(),
                    reason: None,
                    can_accept_for_session: true,
                    status: RequestStatus::Pending,
                    decision: None,
                    feedback: None,
                },
            }]
        );
        let pending = requests.take("r1").unwrap();
        assert_eq!(
            response(&reply(
                "r1",
                &pending,
                &decide(ApprovalDecision::Accept),
                ASK,
                None
            )),
            json!({ "action": "accept", "content": { "choice": "allow_once", "remember": false } })
        );
        assert_eq!(
            response(&reply(
                "r1",
                &pending,
                &decide(ApprovalDecision::Decline),
                ASK,
                None
            )),
            json!({ "action": "decline" })
        );
        assert_eq!(
            response(&reply(
                "r1",
                &pending,
                &decide(ApprovalDecision::Cancel),
                ASK,
                None
            )),
            json!({ "action": "cancel" })
        );
    }

    #[test]
    fn an_mcp_question_accepted_for_the_session_is_not_asked_again() {
        let grants = Grants::default();
        let requests = Requests::new(grants.clone());
        requests.set_session("s1");
        receive(&elicitation(consent_schema()), ASK, &requests);
        let pending = requests.take("r1").unwrap();
        reply(
            "r1",
            &pending,
            &decide(ApprovalDecision::AcceptForSession),
            ASK,
            Some(("s1", &grants)),
        );

        let (outputs, line) = receive(&elicitation(consent_schema()), ASK, &requests);
        assert!(outputs.is_empty());
        assert_eq!(response(&line.unwrap())["action"], "accept");

        let other = Requests::new(grants);
        other.set_session("s2");
        let (outputs, _) = receive(&elicitation(consent_schema()), ASK, &other);
        assert_eq!(outputs.len(), 1, "another session still asks");
    }

    #[test]
    fn an_mcp_question_consent_alone_cannot_answer_is_declined() {
        let requests = Requests::default();
        let url = json!({ "type": "control_request", "request_id": "r1", "request": {
            "subtype": "elicitation", "mcp_server_name": "github", "message": "Sign in",
            "mode": "url", "url": "https://example.com/login" } });
        let required = elicitation(json!({ "type": "object", "required": ["token"],
            "properties": { "token": { "type": "string", "default": "abc" } } }));
        let schemaless = json!({ "type": "control_request", "request_id": "r1", "request": {
            "subtype": "elicitation", "mcp_server_name": "x", "message": "Go on?" } });

        for request in [url, required, schemaless] {
            let (outputs, line) = receive(&request, ASK, &requests);
            assert!(outputs.is_empty());
            assert_eq!(response(&line.unwrap()), json!({ "action": "decline" }));
        }
    }

    #[test]
    fn only_choices_that_consent_once_fill_the_form() {
        for (option, once) in [
            ("allow", true),
            ("Allow once", true),
            ("approve-this-time", true),
            ("accept_once_only", true),
            ("allow_session", false),
            ("allow_", false),
            ("allowed", false),
            ("always allow", false),
        ] {
            assert_eq!(consents_once(option), once, "{option}");
        }
    }

    #[test]
    fn other_requests_are_refused() {
        let hook = json!({ "type": "control_request", "request_id": "r1",
            "request": { "subtype": "hook_callback" } });
        let (outputs, line) = receive(&hook, ASK, &Requests::default());

        assert!(outputs.is_empty());
        let value: Value = serde_json::from_str(&line.unwrap()).unwrap();
        assert_eq!(value["response"]["subtype"], "error");
    }
}
