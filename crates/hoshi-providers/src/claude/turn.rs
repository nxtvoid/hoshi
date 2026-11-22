use super::*;

pub(super) fn turn_model(request: &TurnRequest, default_model: Option<&str>) -> TurnModel {
    manifest().resolve(
        request.model.as_deref().or(default_model),
        request.effort,
        request.fast_mode,
        request.ultracode,
    )
}

/// What the turn adds to the agent's system prompt: hoshi's instructions, then how to choose
/// the model and effort of subagents.
pub(super) fn system_prompt(
    request: &TurnRequest,
    default_model: Option<&str>,
    tool_search: ToolSearch,
) -> Option<String> {
    let own = request.instructions.as_ref().map(|own| match tool_search {
        ToolSearch::Off => own.replacen(DEFERRED_TOOLS, "", 1),
        _ => own.clone(),
    });
    let subagents = subagents::instructions(
        request.subagent_effort,
        request.model.as_deref().or(default_model),
        request.effort,
        request.mcp.is_some(),
    );
    match (own, subagents) {
        (Some(own), Some(subagents)) => Some(format!("{own}\n{subagents}")),
        (own, subagents) => own.or(subagents),
    }
}

pub(super) fn access(request: &TurnRequest) -> Access {
    Access {
        mode: request.access_mode,
        plan_mode: request.plan_mode,
    }
}

/// The message that opens a turn, which `queue` now waits for the agent to begin.
pub(super) fn user_message(
    prompt: &str,
    attachments: &[TurnAttachment],
    media: &mut MediaBudget,
    queue: &mut QueueState,
) -> String {
    let message_id = uuid::Uuid::new_v4().to_string();
    let message = json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": prompt::message_content(prompt, attachments, media),
        },
        "parent_tool_use_id": null,
        "uuid": message_id,
    })
    .to_string();
    queue.sent = HashSet::from([message_id.clone()]);
    queue.unbegun = true;
    queue.prompt = Some(message_id);
    queue.prompt_started = false;
    message
}

/// The messages of ours a line of the agent's says it is about.
pub(super) fn echoed_messages(value: &Value) -> Vec<&str> {
    let mut ids: Vec<&str> = value["user_message_uuids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    ids.extend(value["user_message_uuid"].as_str());
    ids
}

pub(super) const INTERRUPT_REQUEST_ID: &str = "hoshi-interrupt";

pub(super) const MODE_REQUEST_ID: &str = "hoshi-mode";

pub(super) const MODEL_REQUEST_ID: &str = "hoshi-model";

pub(super) const FLAGS_REQUEST_ID: &str = "hoshi-flags";

pub(super) const MCP_REQUEST_ID: &str = "hoshi-mcp";

/// How long a turn asked to stop may take to wind down before its process is killed.
pub(super) const INTERRUPT_GRACE: Duration = Duration::from_secs(5);

pub(super) struct ClaudeTurn {
    pub(super) process: LiveProcess,
    pub(super) requests: Requests,
    pub(super) grants: Grants,
    pub(super) live_output: LiveOutput,
    pub(super) queue: Queue,
    pub(super) launch: Launch,
    /// The adapter's model, for requests that name none.
    pub(super) default_model: Option<String>,
}

impl RunningTurn for ClaudeTurn {
    /// Asks Claude Code to stop, which lets it record the stop in its session; kills it when it
    /// takes too long.
    fn interrupt(&self) {
        self.queue.state().interrupted = true;
        ask_to_stop(&self.process, INTERRUPT_GRACE);
    }

    /// `at_once`, the agent reads the message now: what runs in the foreground moves to the
    /// background first and keeps running, rather than being cut by the message. Otherwise,
    /// and until it answers what the turn began with, it reads it once its step is over.
    fn steer(
        &self,
        message_id: &str,
        prompt: &str,
        attachments: &[TurnAttachment],
        at_once: bool,
    ) -> bool {
        let mut queue = self.queue.state();
        if queue.closing || queue.interrupted || !queue.turn_open {
            return false;
        }
        let at_once = at_once && queue.answering;
        let message = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": prompt::message_content(prompt, attachments, &mut queue.media),
            },
            "parent_tool_use_id": null,
            "uuid": message_id,
            "priority": if at_once { "now" } else { "next" },
        })
        .to_string();
        let stdin = self.process.stdin();
        if at_once && self.live_output.anything_running() {
            let move_aside = json!({
                "type": "control_request",
                "request_id": format!("hoshi-background:{message_id}"),
                "request": { "subtype": "background_tasks" },
            });
            stdin.write_line(&move_aside.to_string());
        }
        let sent = stdin.write_line(&message);
        if sent {
            queue.waiting.insert(message_id.to_owned());
            queue.sent.insert(message_id.to_owned());
        }
        sent
    }

    fn respond(&self, request_id: &str, response: &RequestResponse) {
        let Some(pending) = self.requests.take(request_id) else {
            return;
        };
        let session = self.requests.session();
        let session = session
            .as_deref()
            .map(|session_id| (session_id, &self.grants));
        let access = self.queue.state().access;
        let line = requests::reply(request_id, &pending, response, access, session);
        self.process.stdin().write_line(&line);
    }

    fn stop_subagent(&self, tool_use_id: &str) {
        if let Some(task_id) = self.live_output.agent_task(tool_use_id) {
            self.stop_task(&task_id);
        }
    }

    fn stop_background_task(&self, task_id: &str) {
        self.stop_task(task_id);
    }

    fn await_background_tasks(&self, task_ids: &[String], awaited: bool) -> Option<Vec<String>> {
        Some(self.queue.await_tasks(task_ids, awaited))
    }

    fn lingers(&self) -> bool {
        self.process.stdin().is_open()
    }

    fn awaits_work(&self) -> bool {
        self.queue.state().wakeup
    }

    fn continue_with(&self, request: &TurnRequest) -> bool {
        let session = self.requests.session();
        let mut queue = self.queue.state();
        let request = request.clone().with_inline_handoff();
        let Some(lines) = next_turn(
            &self.launch,
            self.default_model.as_deref(),
            session.as_deref(),
            &mut queue,
            &request,
        ) else {
            return false;
        };
        let stdin = self.process.stdin();
        if lines.iter().all(|line| stdin.write_line(line)) {
            return true;
        }
        queue.turn_open = false;
        false
    }

    /// Between turns the process gets its stdin closed and a moment to exit on its own, as it
    /// may be refreshing its sign-in; a turn still open is stopped at once.
    fn close(&self) {
        let mut queue = self.queue.state();
        queue.closing = true;
        let between_turns = !queue.turn_open;
        drop(queue);
        if between_turns {
            self.process.stdin().close();
        } else {
            self.process.interrupt();
        }
    }
}

impl ClaudeTurn {
    pub(super) fn stop_task(&self, task_id: &str) {
        let request = json!({
            "type": "control_request",
            "request_id": format!("hoshi-stop:{task_id}"),
            "request": { "subtype": "stop_task", "task_id": task_id },
        });
        self.process.stdin().write_line(&request.to_string());
    }
}

/// An hour for the user to answer a card, and some slack, as t3code gives its tools.
pub(super) const MCP_TOOL_TIMEOUT_MS: u64 = 65 * 60 * 1000;

pub(super) fn mcp_servers(server: &McpServer) -> Value {
    json!({
        "type": "control_request",
        "request_id": MCP_REQUEST_ID,
        "request": {
            "subtype": "mcp_set_servers",
            "servers": {
                MCP_SERVER_NAME: {
                    "type": "http",
                    "url": server.url,
                    "headers": { "Authorization": server.authorization },
                    // A tool can wait on the user, as `request_secret` does.
                    "timeout": MCP_TOOL_TIMEOUT_MS,
                },
            },
        },
    })
}

/// Ultracode runs through the `Workflow` tool, which `-p` would refuse
/// without an allow rule since it never prompts. A settings `env` beats the process's own.
pub(super) fn session_settings(model: &TurnModel, tool_search: ToolSearch) -> Option<Value> {
    let defer = tool_search == ToolSearch::Auto;
    if !model.fast_mode && !model.ultracode && !defer {
        return None;
    }
    let mut settings = json!({});
    if defer {
        settings["env"] = json!({ "ENABLE_TOOL_SEARCH": "true" });
    }
    if model.fast_mode {
        settings["fastMode"] = json!(true);
    }
    if model.ultracode {
        settings["ultracode"] = json!(true);
        settings["permissions"] = json!({ "allow": ["Workflow"] });
    }
    Some(settings)
}
