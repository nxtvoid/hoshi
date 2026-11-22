use super::*;

/// How the thread's turns run.
pub(super) struct TurnSettings {
    pub(super) model: String,
    pub(super) effort: Option<String>,
    pub(super) fast_tier: Option<String>,
    pub(super) access: AccessMode,
    pub(super) cwd: String,
}

impl TurnSettings {
    pub(super) fn turn_params(&self, thread_id: &str, input: &[Value], planning: bool) -> Value {
        let (approval_policy, reviewer, sandbox) = policy(self.access);
        let mut params = json!({
            "threadId": thread_id,
            "input": input,
            "cwd": self.cwd,
            "summary": "auto",
            "approvalPolicy": approval_policy,
            "approvalsReviewer": reviewer,
            "sandboxPolicy": sandbox,
        });
        if !self.model.is_empty() {
            params["model"] = json!(self.model);
            params["collaborationMode"] = json!({
                "mode": if planning { "plan" } else { "default" },
                "settings": {
                    "model": self.model,
                    "reasoning_effort": self.effort,
                    "developer_instructions": null,
                },
            });
        }
        if let Some(effort) = &self.effort {
            params["effort"] = json!(effort);
        }
        if let Some(tier) = &self.fast_tier {
            params["serviceTier"] = json!(tier);
        }
        params
    }
}

/// Codex's approval policy, who reviews its approvals and its sandbox, for hoshi's access mode.
pub(super) fn policy(access: AccessMode) -> (&'static str, &'static str, Value) {
    let workspace = json!({
        "type": "workspaceWrite",
        "writableRoots": [],
        "networkAccess": false,
        "excludeTmpdirEnvVar": false,
        "excludeSlashTmp": false,
    });
    match access {
        AccessMode::Ask => (
            "untrusted",
            "user",
            json!({ "type": "readOnly", "networkAccess": false }),
        ),
        AccessMode::AcceptEdits => ("on-request", "user", workspace),
        AccessMode::Auto => ("on-request", "auto_review", workspace),
        AccessMode::FullAccess => ("never", "user", json!({ "type": "dangerFullAccess" })),
    }
}

/// Starts the thread, or resumes the thread's earlier one, forked after the message a rewind
/// goes back to; its id and the model it runs. A thread that can't be resumed fails the turn:
/// a new one would not know the conversation.
pub(super) fn open_thread(
    call: &dyn Fn(&str, Value) -> Result<Value, RpcError>,
    request: &TurnRequest,
    closing_wait: Duration,
) -> Result<(String, Option<String>), TurnFailure> {
    let mut params = json!({
        "cwd": request.cwd,
        "config": thread_config(request),
    });
    if let Some(model) = &request.model {
        params["model"] = json!(model);
    }
    if let Some(instructions) = &request.instructions {
        params["developerInstructions"] = json!(codex_instructions(instructions));
    }
    let opened = |answer: Value| {
        let id = answer["thread"]["id"].as_str()?.to_owned();
        Some((id, answer["model"].as_str().map(str::to_owned)))
    };
    if let Some(resumed) = &request.resume_session_id {
        let mut resume = params.clone();
        resume["threadId"] = json!(resumed);
        resume["excludeTurns"] = json!(true);
        if let Some(turn) = &request.resume_session_at {
            resume["lastTurnId"] = json!(turn);
        }
        let method = if request.resume_session_at.is_some() || request.fork_session {
            "thread/fork"
        } else {
            "thread/resume"
        };
        let answer = resume_thread(call, method, &resume, resumed, closing_wait)
            .map_err(|error| resume_failure(&error))?;
        return opened(answer).ok_or_else(|| TurnFailure::ResumeFailed {
            detail: format!("{method} answered no thread"),
        });
    }
    let answer = call("thread/start", params).map_err(|error| TurnFailure::ProviderError {
        detail: error.detail(),
    })?;
    opened(answer).ok_or_else(|| TurnFailure::ProviderError {
        detail: "the app-server started no thread".into(),
    })
}

/// Resumes or forks a thread: one archived elsewhere once unarchived, one Codex still closes
/// or another server still holds once let go, as the server of the thread's last turn that
/// ends just now.
pub(super) fn resume_thread(
    call: &dyn Fn(&str, Value) -> Result<Value, RpcError>,
    method: &str,
    params: &Value,
    thread_id: &str,
    closing_wait: Duration,
) -> Result<Value, RpcError> {
    let mut answer = call(method, params.clone());
    if answer.as_ref().is_err_and(archived) {
        call("thread/unarchive", json!({ "threadId": thread_id }))?;
        answer = call(method, params.clone());
    }
    for _ in 0..CLOSING_TRIES {
        if !answer
            .as_ref()
            .is_err_and(|error| closing(error) || in_use(error))
        {
            break;
        }
        thread::sleep(closing_wait);
        answer = call(method, params.clone());
    }
    answer
}

/// Codex keeps an archived session from being resumed until it is unarchived.
pub(super) fn archived(error: &RpcError) -> bool {
    let message = error.message.to_ascii_lowercase();
    (message.contains("session ") && message.contains(" is archived"))
        || message.contains("codex unarchive")
}

/// Codex still unloads the thread, as after an unsubscribe.
pub(super) fn closing(error: &RpcError) -> bool {
    let message = error.message.to_ascii_lowercase();
    message.contains("thread ") && message.contains(" is closing")
}

/// Another client writes to the thread.
pub(super) fn in_use(error: &RpcError) -> bool {
    error.code == -32600 && error.message.contains(ACTIVE_WRITER)
}

/// Why a thread could not be resumed: another client holds it, or Codex's own error.
pub(super) fn resume_failure(error: &RpcError) -> TurnFailure {
    let detail = if in_use(error) {
        format!("thread in use: {}", error.detail())
    } else {
        error.detail()
    };
    TurnFailure::ResumeFailed { detail }
}

/// What every thread runs with: hoshi's MCP server, its tools allowed without asking, and the
/// environment the commands it runs see.
pub(super) fn thread_config(request: &TurnRequest) -> Value {
    let mut config = json!({
        "tools.update_plan.enabled": true,
        "features.concurrent_reasoning_summaries": true,
    });
    if let Some(server) = &request.mcp {
        config[format!("mcp_servers.{}", crate::MCP_SERVER_NAME)] = mcp(server);
    }
    for (name, value) in &request.environment {
        config[format!("shell_environment_policy.set.{name}")] = json!(value);
    }
    config
}

/// hoshi's MCP server as Codex's config describes one reached over HTTP.
pub(super) fn mcp(server: &McpServer) -> Value {
    json!({
        "url": server.url,
        "http_headers": { "Authorization": server.authorization },
        "default_tools_approval_mode": "approve",
        "tool_timeout_sec": MCP_TOOL_TIMEOUT_SECS,
    })
}

/// hoshi's instructions as Codex reads them: its tools are never deferred.
pub(super) fn codex_instructions(instructions: &str) -> String {
    instructions.replace(crate::DEFERRED_TOOLS, "")
}

/// Where the skills the prompt mentions are, by name.
pub(super) fn skill_paths(
    connection: &Connection,
    request: &TurnRequest,
) -> HashMap<String, String> {
    let wanted: HashSet<&str> = request
        .attachments
        .iter()
        .filter_map(|attachment| match &attachment.attachment {
            MessageAttachment::Skill { name } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    if wanted.is_empty() {
        return HashMap::new();
    }
    skills(connection, Path::new(&request.cwd))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|skill| {
            let name = skill["name"]
                .as_str()
                .filter(|name| wanted.contains(name))?;
            Some((name.to_owned(), skill["path"].as_str()?.to_owned()))
        })
        .collect()
}

/// A context handoff as items of the conversation: what it covers as the user's, then each
/// message as its role's.
pub(super) fn history_items(handoff: &ContextHandoff) -> Vec<Value> {
    let item = |role: &str, text: String| {
        let kind = if role == "user" {
            "input_text"
        } else {
            "output_text"
        };
        json!({ "type": "message", "role": role, "content": [{ "type": kind, "text": text }] })
    };
    std::iter::once(item("user", handoff.context.clone()))
        .chain(
            handoff
                .messages
                .iter()
                .map(|message| item(message.role.as_str(), message.rendered())),
        )
        .collect()
}

pub(super) fn text_input(text: &str) -> Value {
    json!({ "type": "text", "text": text, "text_elements": [] })
}

/// The message as Codex takes it: its text, with a line saying where each image and file is
/// saved, then its images and the skills it mentions.
pub(super) fn user_input(
    prompt: &str,
    attachments: &[TurnAttachment],
    skills: &HashMap<String, String>,
) -> Vec<Value> {
    let text = attachments::with_context(prompt.to_owned(), attachments);
    let mut input = vec![text_input(&text)];
    for attachment in attachments {
        match (&attachment.attachment, &attachment.path) {
            (MessageAttachment::Image { .. }, Some(path)) if path.is_file() => {
                input.push(json!({ "type": "localImage", "path": path }));
            }
            (MessageAttachment::Skill { name }, _) => {
                if let Some(path) = skills.get(name) {
                    input.push(json!({ "type": "skill", "name": name, "path": path }));
                }
            }
            _ => {}
        }
    }
    input
}
