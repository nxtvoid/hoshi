use super::*;

/// A permission the agent waits for.
pub(super) struct Permission {
    id: Value,
    /// Option ids by kind, such as `allow_once`.
    options: HashMap<String, String>,
}

/// A turn's state, shared by the thread that runs it and the one that reads the agent.
pub(super) struct Turn {
    sink: Mutex<OutputSink>,
    translator: Mutex<Translator>,
    pub(super) session_id: Mutex<Option<String>>,
    /// While a session loads, the agent replays its history, which the transcript has.
    loading: AtomicBool,
    pub(super) finished: AtomicBool,
    pub(super) cancelled: AtomicBool,
    /// Ended by [`RunningTurn::close`], so its process is not kept.
    pub(super) closed: AtomicBool,
    /// Runs in a process an earlier turn of its session left.
    pub(super) reused: AtomicBool,
    /// Whether the agent showed anything of the turn yet.
    progressed: AtomicBool,
    permissions: Mutex<HashMap<String, Permission>>,
    access: AccessMode,
    pub(super) connection: Mutex<Option<Connection>>,
    /// The plan file the agent wrote in plan mode, which it proposes once its prompt ends.
    plan_file: Mutex<Option<std::path::PathBuf>>,
    /// Answers the plan proposal the turn waits on.
    plan_answer: Mutex<Option<(String, mpsc::Sender<RequestResponse>)>>,
    pub(super) subagents: Mutex<Subagents>,
}

/// The subagents running in the turn, by Copilot's agent id.
#[derive(Default)]
pub(super) struct Subagents {
    /// The call that started each one.
    calls: HashMap<String, String>,
    translators: HashMap<String, Translator>,
}

impl Subagents {
    /// What `update` makes inside the subagent it belongs to; `None` for the agent's own.
    pub(super) fn update(&mut self, update: &Value) -> Option<Vec<ProviderOutput>> {
        let agent = update["_meta"]["github.com/copilot"]["agentId"].as_str()?;
        let call = self.calls.get(agent)?.clone();
        let outputs = self
            .translators
            .entry(agent.to_owned())
            .or_default()
            .update(update);
        Some(within(&call, outputs))
    }

    pub(super) fn started(&mut self, agent: &str, call: &str) {
        self.calls.insert(agent.to_owned(), call.to_owned());
    }

    pub(super) fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    /// The subagent's last streamed text, as it ends.
    pub(super) fn ended(&mut self, agent: &str) -> Vec<ProviderOutput> {
        let (Some(call), Some(mut translator)) =
            (self.calls.remove(agent), self.translators.remove(agent))
        else {
            return Vec::new();
        };
        let mut outputs = Vec::new();
        translator.flush(&mut outputs);
        within(&call, outputs)
    }
}

/// `outputs` as what the subagent `call` started did.
pub(super) fn within(call: &str, outputs: Vec<ProviderOutput>) -> Vec<ProviderOutput> {
    outputs
        .into_iter()
        .filter(|output| {
            !matches!(
                output,
                ProviderOutput::ContextUsage(_) | ProviderOutput::TaskList { .. }
            )
        })
        .map(|output| ProviderOutput::Subagent {
            tool_use_id: call.to_owned(),
            output: Box::new(output),
        })
        .collect()
}

impl Turn {
    pub(super) fn new(sink: OutputSink, access: AccessMode) -> Self {
        Self {
            sink: Mutex::new(sink),
            translator: Mutex::default(),
            session_id: Mutex::default(),
            loading: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            reused: AtomicBool::new(false),
            progressed: AtomicBool::new(false),
            permissions: Mutex::default(),
            access,
            connection: Mutex::new(None),
            plan_file: Mutex::default(),
            plan_answer: Mutex::default(),
            subagents: Mutex::default(),
        }
    }

    pub(super) fn emit(&self, outputs: Vec<ProviderOutput>) {
        if outputs.is_empty() {
            return;
        }
        self.progressed.store(true, Ordering::SeqCst);
        self.send(outputs);
    }

    pub(super) fn send(&self, outputs: Vec<ProviderOutput>) {
        if self.finished.load(Ordering::SeqCst) {
            return;
        }
        let mut sink = lock(&self.sink);
        for output in outputs {
            sink(output);
        }
    }

    /// Reports the turn's session, once.
    pub(super) fn announce(&self, session_id: &str) {
        let previous = lock(&self.session_id).replace(session_id.to_owned());
        if previous.as_deref() != Some(session_id) {
            self.send(vec![ProviderOutput::SessionStarted {
                session_id: session_id.to_owned(),
            }]);
        }
    }

    /// Whether the turn can still start over in a new process, having lost the one it reused.
    pub(super) fn retryable(&self) -> bool {
        self.reused.load(Ordering::SeqCst)
            && !self.progressed.load(Ordering::SeqCst)
            && !self.cancelled.load(Ordering::SeqCst)
    }

    pub(super) fn process_ended(&self, stderr: String) {
        if self.retryable() {
            return;
        }
        let outcome = if self.cancelled.load(Ordering::SeqCst) {
            TurnOutcome::Interrupted
        } else {
            TurnOutcome::Failed {
                reason: TurnFailure::ProcessExited { detail: stderr },
            }
        };
        self.finish(outcome);
    }

    /// Ends the turn once, after what it still streamed.
    pub(super) fn finish(&self, outcome: TurnOutcome) {
        let mut outputs = Vec::new();
        lock(&self.translator).flush(&mut outputs);
        self.emit(outputs);
        if self.finished.swap(true, Ordering::SeqCst) {
            return;
        }
        lock(&self.sink)(ProviderOutput::TurnCompleted { outcome });
    }

    pub(super) fn connection(&self) -> Option<Connection> {
        lock(&self.connection).clone()
    }

    pub(super) fn receive(&self, incoming: Incoming) {
        match incoming {
            Incoming::Notification { method, params } if method == "session/update" => {
                if self.loading.load(Ordering::SeqCst) {
                    return;
                }
                let update = &params["update"];
                if let Some(outputs) = lock(&self.subagents).update(update) {
                    self.emit(outputs);
                    return;
                }
                if let Some(path) = plan_file(update) {
                    *lock(&self.plan_file) = Some(path);
                }
                let outputs = lock(&self.translator).update(update);
                self.emit(outputs);
            }
            Incoming::Notification { method, params } if method == SESSION_EVENT => {
                self.session_event(&params);
            }
            Incoming::Notification { .. } => {}
            Incoming::Request { id, method, params } if method == "session/request_permission" => {
                self.permission(id, &params);
            }
            Incoming::Request { id, .. } => {
                if let Some(connection) = self.connection() {
                    connection.reply_error(&id, -32601, "not supported by this client");
                }
            }
        }
    }

    pub(super) fn session_event(&self, event: &Value) {
        let data = &event["data"];
        let agent = event["agentId"].as_str();
        match (event["type"].as_str(), agent, data["toolCallId"].as_str()) {
            (Some("subagent.started"), Some(agent), Some(call)) => {
                lock(&self.subagents).started(agent, call);
                if let Some(model) = data["model"].as_str() {
                    self.emit(vec![ProviderOutput::ToolCallDetail {
                        item_id: call.to_owned(),
                        detail: ToolDetail::Subagent {
                            model: model.to_owned(),
                            effort: None,
                            resume_id: None,
                        },
                    }]);
                }
                return;
            }
            (Some("subagent.completed" | "subagent.failed"), Some(agent), _) => {
                let outputs = lock(&self.subagents).ended(agent);
                self.emit(outputs);
                return;
            }
            _ => {}
        }
        if agent.is_some() {
            return;
        }
        if event["type"] == "session.compaction_complete"
            && data["success"] == true
            && (data["outcome"].is_null() || data["outcome"] == "applied")
        {
            self.emit(vec![ProviderOutput::CompactionFinished {
                before_tokens: data["preCompactionTokens"].as_u64().unwrap_or_default(),
                after_tokens: data["postCompactionTokens"].as_u64().unwrap_or_default(),
            }]);
        }
    }

    pub(super) fn permission(&self, id: Value, params: &Value) {
        let options: HashMap<String, String> = params["options"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|option| {
                Some((
                    option["kind"].as_str()?.to_owned(),
                    option["optionId"].as_str()?.to_owned(),
                ))
            })
            .collect();
        let call = &params["toolCall"];
        let kind = call["kind"].as_str().unwrap_or_default();
        let hoshi_tool = call["toolCallId"]
            .as_str()
            .and_then(|id| lock(&self.translator).tool(id).cloned())
            .is_some_and(|tool| tool.name.starts_with(HOSHI_TOOL_PREFIX));
        let allowed = hoshi_tool
            || match self.access {
                AccessMode::FullAccess => true,
                AccessMode::AcceptEdits | AccessMode::Auto => matches!(kind, "read" | "edit"),
                AccessMode::Ask => kind == "read",
            };
        let permission = Permission { id, options };
        if allowed {
            self.answer(&permission, "allow_once");
            return;
        }
        let tool_call_id = call["toolCallId"].as_str().map(str::to_owned);
        let (tool_name, input) = tool_call_id
            .as_deref()
            .and_then(|tool| lock(&self.translator).tool(tool).cloned())
            .map_or_else(|| normalize_tool(call), |tool| (tool.name, tool.input));
        let request_id = format!("permission-{}", permission.id);
        let content = ItemContent::Approval {
            tool_call_id,
            tool_name,
            input: truncate_json(&input, APPROVAL_INPUT_MAX_CHARS),
            reason: None,
            can_accept_for_session: permission.options.contains_key("allow_always"),
            status: RequestStatus::Pending,
            decision: None,
            feedback: None,
        };
        lock(&self.permissions).insert(request_id.clone(), permission);
        self.emit(vec![ProviderOutput::RequestOpened {
            request_id,
            content,
        }]);
    }

    /// Picks the option of `kind`, or cancels when the agent offered none.
    pub(super) fn answer(&self, permission: &Permission, kind: &str) {
        let Some(connection) = self.connection() else {
            return;
        };
        let outcome = match permission.options.get(kind) {
            Some(option) => json!({ "outcome": "selected", "optionId": option }),
            None => json!({ "outcome": "cancelled" }),
        };
        connection.reply(&permission.id, json!({ "outcome": outcome }));
    }

    pub(super) fn respond(&self, request_id: &str, response: &RequestResponse) {
        let mut plan = lock(&self.plan_answer);
        if plan.as_ref().is_some_and(|(id, _)| id == request_id) {
            if let Some((_, answer)) = plan.take() {
                let _ = answer.send(response.clone());
            }
            return;
        }
        drop(plan);
        let Some(permission) = lock(&self.permissions).remove(request_id) else {
            return;
        };
        let decision = match response {
            RequestResponse::Decision { decision, .. } => *decision,
            _ => ApprovalDecision::Decline,
        };
        let kind = match decision {
            ApprovalDecision::Accept => "allow_once",
            ApprovalDecision::AcceptForSession => "allow_always",
            ApprovalDecision::Decline | ApprovalDecision::Cancel => "reject_once",
        };
        self.answer(&permission, kind);
        if decision == ApprovalDecision::Cancel {
            self.cancel();
        }
    }

    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        lock(&self.plan_answer).take();
        let Some(connection) = self.connection() else {
            return;
        };
        for (_, permission) in lock(&self.permissions).drain() {
            connection.reply(
                &permission.id,
                json!({ "outcome": { "outcome": "cancelled" } }),
            );
        }
        if let Some(session_id) = lock(&self.session_id).clone() {
            connection.notify("session/cancel", json!({ "sessionId": session_id }));
        }
    }

    /// Opens or loads the session, unless the process `reused` holds it already, and sends the
    /// prompt.
    pub(super) fn run(
        &self,
        connection: &Connection,
        request: &TurnRequest,
        reused: bool,
    ) -> Attempt {
        let failed = |detail: String| Attempt::Done {
            outcome: TurnOutcome::Failed {
                reason: TurnFailure::ProviderError { detail },
            },
            reusable: false,
        };
        let interrupted = Attempt::Done {
            outcome: TurnOutcome::Interrupted,
            reusable: true,
        };
        let (session_id, fresh) = match request.resume_session_id.clone().filter(|_| reused) {
            Some(session_id) => (session_id, false),
            None => match self.open(connection, request) {
                Ok(opened) => opened,
                Err(detail) => return failed(detail),
            },
        };
        self.announce(&session_id);
        if !reused && let Some(model) = chosen(request.model.as_deref()) {
            let _ = connection.call(
                "session/set_model",
                json!({ "sessionId": session_id, "modelId": model }),
                SETTING_TIMEOUT,
            );
        }
        if request.plan_mode || !fresh {
            let mode = if request.plan_mode {
                PLAN_MODE
            } else {
                AGENT_MODE
            };
            let _ = connection.call(
                "session/set_config_option",
                json!({ "sessionId": session_id, "configId": "mode", "value": mode }),
                SETTING_TIMEOUT,
            );
        }
        if self.cancelled.load(Ordering::SeqCst) {
            return interrupted;
        }
        let instructions = request
            .instructions
            .as_deref()
            .filter(|_| fresh)
            .map(copilot_instructions);
        let mut prompt = prompt_blocks(
            &request.prompt,
            &request.attachments,
            instructions.as_deref(),
        );
        let mut planning = request.plan_mode;
        let mut proposals = 0;
        loop {
            let answer = connection
                .request(
                    "session/prompt",
                    json!({ "sessionId": session_id, "prompt": prompt }),
                )
                .recv()
                .unwrap_or_else(|_| Err(RpcError::gone()));
            if let Err(error) = &answer
                && proposals == 0
                && self.retryable()
                && lost(error)
            {
                return Attempt::Retry;
            }
            if self.cancelled.load(Ordering::SeqCst) {
                return Attempt::Done {
                    outcome: TurnOutcome::Interrupted,
                    reusable: answer.is_ok(),
                };
            }
            match answer {
                Ok(result) => match result["stopReason"].as_str() {
                    Some("cancelled") => return interrupted,
                    Some("refusal") => {
                        return Attempt::Done {
                            outcome: TurnOutcome::Failed {
                                reason: TurnFailure::ProviderError {
                                    detail: "the model refused".into(),
                                },
                            },
                            reusable: true,
                        };
                    }
                    _ => {}
                },
                Err(error) => return failed(error.detail()),
            }
            let completed = Attempt::Done {
                outcome: TurnOutcome::Completed,
                reusable: true,
            };
            let plan = lock(&self.plan_file)
                .take()
                .and_then(|path| std::fs::read_to_string(path).ok())
                .filter(|plan| !plan.trim().is_empty());
            let (true, Some(plan)) = (planning, plan) else {
                return completed;
            };
            proposals += 1;
            let request_id = format!("plan-{session_id}-{proposals}");
            let (sender, answers) = mpsc::channel();
            {
                let mut slot = lock(&self.plan_answer);
                if self.cancelled.load(Ordering::SeqCst) {
                    return interrupted;
                }
                *slot = Some((request_id.clone(), sender));
            }
            {
                let mut outputs = Vec::new();
                lock(&self.translator).flush(&mut outputs);
                outputs.push(ProviderOutput::RequestOpened {
                    request_id,
                    content: ItemContent::ProposedPlan {
                        plan,
                        status: RequestStatus::Pending,
                        decision: None,
                        feedback: None,
                    },
                });
                self.emit(outputs);
            }
            let Ok(RequestResponse::Decision { decision, feedback }) = answers.recv() else {
                return if self.cancelled.load(Ordering::SeqCst) {
                    interrupted
                } else {
                    completed
                };
            };
            let next = match (decision, feedback.filter(|text| !text.trim().is_empty())) {
                (decision, _) if decision.accepts() => {
                    planning = false;
                    let _ = connection.call(
                        "session/set_config_option",
                        json!({ "sessionId": session_id, "configId": "mode", "value": AGENT_MODE }),
                        SETTING_TIMEOUT,
                    );
                    IMPLEMENT_PLAN.to_owned()
                }
                (ApprovalDecision::Decline, Some(feedback)) => feedback,
                _ => return completed,
            };
            prompt = vec![json!({ "type": "text", "text": next })];
        }
    }

    /// Starts the process's session: the thread's earlier one loaded again, else a new one.
    /// The session's id, and whether it is new.
    pub(super) fn open(
        &self,
        connection: &Connection,
        request: &TurnRequest,
    ) -> Result<(String, bool), String> {
        connection
            .call(
                "initialize",
                json!({ "protocolVersion": 1, "clientCapabilities": client_capabilities() }),
                HANDSHAKE_TIMEOUT,
            )
            .map_err(|error| error.detail())?;
        let mcp_servers = request
            .mcp
            .as_ref()
            .map_or_else(Vec::new, |server| vec![mcp(server)]);
        if let Some(resumed) = &request.resume_session_id {
            self.loading.store(true, Ordering::SeqCst);
            let loaded = connection.call(
                "session/load",
                json!({ "sessionId": resumed, "cwd": request.cwd, "mcpServers": mcp_servers }),
                LOAD_TIMEOUT,
            );
            self.loading.store(false, Ordering::SeqCst);
            if loaded.is_ok() {
                return Ok((resumed.clone(), false));
            }
        }
        let session = connection
            .call(
                "session/new",
                json!({ "cwd": request.cwd, "mcpServers": mcp_servers }),
                HANDSHAKE_TIMEOUT,
            )
            .map_err(|error| error.detail())?;
        match session["sessionId"].as_str() {
            Some(id) => Ok((id.to_owned(), true)),
            None => Err("the agent started no session".into()),
        }
    }
}

/// The plan file a plan-mode update wrote: Copilot keeps it as `plan.md` in its session.
fn plan_file(update: &Value) -> Option<std::path::PathBuf> {
    let is_plan = |path: &str| {
        let path = Path::new(path);
        path.file_name().is_some_and(|name| name == "plan.md")
            && path
                .components()
                .any(|part| part.as_os_str() == "session-state")
    };
    let from_diff = update["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| block["path"].as_str())
        .find(|path| is_plan(path));
    let from_input = update["rawInput"]["path"]
        .as_str()
        .filter(|path| is_plan(path));
    from_diff.or(from_input).map(std::path::PathBuf::from)
}

/// hoshi's instructions as Copilot reads them: its tools are named `hoshi-<tool>`, and none
/// are deferred.
fn copilot_instructions(instructions: &str) -> String {
    instructions
        .replace(crate::DEFERRED_TOOLS, "")
        .replace(HOSHI_TOOL_PREFIX, &format!("{}-", crate::MCP_SERVER_NAME))
}

/// The model to ask for; `None` leaves the plan's default.
pub(super) fn chosen(model: Option<&str>) -> Option<&str> {
    model.filter(|model| *model != DEFAULT_MODEL)
}

/// hoshi's MCP server as ACP describes one reached over HTTP.
pub(super) fn mcp(server: &McpServer) -> Value {
    json!({
        "type": "http",
        "name": crate::MCP_SERVER_NAME,
        "url": server.url,
        "headers": [{ "name": "Authorization", "value": server.authorization }],
    })
}

/// The prompt's content blocks: hoshi's instructions on a new session, the text, then its
/// images and files.
fn prompt_blocks(
    prompt: &str,
    attachments: &[TurnAttachment],
    instructions: Option<&str>,
) -> Vec<Value> {
    let text = match instructions {
        Some(instructions) => {
            format!("<hoshi-instructions>\n{instructions}\n</hoshi-instructions>\n\n{prompt}")
        }
        None => prompt.to_owned(),
    };
    let mut blocks = vec![json!({ "type": "text", "text": text })];
    for attachment in attachments {
        let Some(path) = &attachment.path else {
            continue;
        };
        match &attachment.attachment {
            MessageAttachment::Image {
                mime_type, name, ..
            } => match std::fs::read(path) {
                Ok(bytes) => blocks.push(json!({
                    "type": "image",
                    "mimeType": mime_type,
                    "data": base64::engine::general_purpose::STANDARD.encode(bytes),
                })),
                Err(_) => blocks.push(resource_link(path, name)),
            },
            MessageAttachment::File { name, .. } => {
                let path = attachment.extracted_text.as_deref().unwrap_or(path);
                blocks.push(resource_link(path, name));
            }
            _ => {}
        }
    }
    blocks
}

pub(super) fn resource_link(path: &Path, name: &str) -> Value {
    let path = path.to_string_lossy().replace('\\', "/");
    let uri = if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    };
    json!({ "type": "resource_link", "uri": uri, "name": name })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plan_comes_from_the_plan_file_in_copilots_session() {
        let path = |parts: &[&str]| parts.join(std::path::MAIN_SEPARATOR_STR);
        let written = json!({
            "sessionUpdate": "tool_call_update",
            "content": [{
                "type": "diff",
                "path": path(&["", "home", "me", ".copilot", "session-state", "s1", "plan.md"]),
                "newText": "# Plan",
            }],
        });
        let elsewhere = json!({
            "sessionUpdate": "tool_call_update",
            "content": [{ "type": "diff", "path": path(&["", "repo", "plan.md"]), "newText": "# Plan" }],
        });
        assert!(plan_file(&written).is_some());
        assert_eq!(plan_file(&elsewhere), None);
    }

    #[test]
    fn copilot_reads_hoshis_tools_by_its_own_names() {
        let instructions = format!("Use mcp__hoshi__delegate_task{}.", crate::DEFERRED_TOOLS);
        assert_eq!(
            copilot_instructions(&instructions),
            "Use hoshi-delegate_task."
        );
    }

    #[test]
    fn instructions_lead_a_new_sessions_prompt() {
        let blocks = prompt_blocks("hi", &[], Some("Use hoshi's tools."));
        assert_eq!(
            blocks,
            [json!({
                "type": "text",
                "text": "<hoshi-instructions>\nUse hoshi's tools.\n</hoshi-instructions>\n\nhi",
            })]
        );
        assert_eq!(
            prompt_blocks("hi", &[], None),
            [json!({ "type": "text", "text": "hi" })]
        );
    }
}
