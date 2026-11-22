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
