use super::*;

/// A turn's state, shared by the thread that runs it and the one that reads the server.
pub(super) struct Turn {
    sink: Mutex<OutputSink>,
    /// Set once the thread is open.
    conversation: Mutex<Option<Conversation>>,
    pub(super) connection: Mutex<Option<Connection>>,
    pub(super) finished: AtomicBool,
    pub(super) cancelled: AtomicBool,
    requests: Mutex<HashMap<String, Pending>>,
    /// Hands the thread's turn, as it completed, to the thread that waits for it.
    ended: Mutex<Option<mpsc::Sender<Value>>>,
    /// What Codex said last went wrong, for a turn that fails: its `TurnError`.
    error: Mutex<Option<Value>>,
    /// The model call Codex retries, while no answer came.
    retry: Mutex<Option<Retry>>,
    /// Answers the plan proposal the turn waits on.
    plan_answer: Mutex<Option<(String, mpsc::Sender<RequestResponse>)>>,
    /// The limits reported last, which Codex repeats after every model call.
    limits: Mutex<Option<UsageLimits>>,
    session_approvals: Arc<Mutex<SessionApprovals>>,
    /// The access mode the turn runs in.
    access: Mutex<AccessMode>,
    /// The model the thread runs, as opening it said.
    thread_model: Mutex<Option<String>>,
    /// What came before the thread was open and shows once it is.
    early: Mutex<Vec<(String, Value)>>,
    /// The turn completed and the server stays for the commands it left running.
    pub(super) lingering: AtomicBool,
    /// Until the server is shut down.
    pub(super) server: Mutex<Option<Arc<Server>>>,
}

/// A model call Codex retries.
pub(super) struct Retry {
    attempt: u32,
    /// When Codex last said it retries.
    since: Instant,
    /// Its `TurnError`.
    error: Value,
}

impl Turn {
    pub(super) fn new(sink: OutputSink, session_approvals: Arc<Mutex<SessionApprovals>>) -> Self {
        Self {
            sink: Mutex::new(sink),
            conversation: Mutex::default(),
            connection: Mutex::default(),
            finished: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            requests: Mutex::default(),
            ended: Mutex::default(),
            error: Mutex::default(),
            retry: Mutex::default(),
            plan_answer: Mutex::default(),
            limits: Mutex::default(),
            session_approvals,
            access: Mutex::default(),
            thread_model: Mutex::default(),
            early: Mutex::default(),
            lingering: AtomicBool::new(false),
            server: Mutex::default(),
        }
    }

    pub(super) fn emit(&self, outputs: Vec<ProviderOutput>) {
        if outputs.is_empty() {
            return;
        }
        let mut sink = lock(&self.sink);
        if self.finished.load(Ordering::SeqCst) {
            return;
        }
        for output in outputs {
            sink(output);
        }
    }

    /// Ends the turn once.
    pub(super) fn finish(&self, outcome: TurnOutcome) {
        if self.finished.swap(true, Ordering::SeqCst) {
            return;
        }
        lock(&self.sink)(ProviderOutput::TurnCompleted { outcome });
    }

    /// Ends the turn once it ran. A turn that completed with commands still running leaves
    /// them in the background and keeps the server for them; otherwise the server ends.
    pub(super) fn end(&self, outcome: TurnOutcome) {
        let mut outputs = Vec::new();
        if outcome == TurnOutcome::Completed
            && !self.cancelled.load(Ordering::SeqCst)
            && let Some(conversation) = lock(&self.conversation).as_mut()
        {
            outputs = conversation.settle();
            conversation.take_background_change();
            let tasks = conversation.background();
            if !tasks.is_empty() {
                self.lingering.store(true, Ordering::SeqCst);
                outputs.push(ProviderOutput::BackgroundTasks {
                    tasks,
                    waiting: false,
                });
            }
        }
        self.emit(outputs);
        self.finish(outcome);
        self.shut_down_when_idle();
    }

    /// Ends the server of a completed turn once nothing it kept runs.
    pub(super) fn shut_down_when_idle(&self) {
        let mut slot = lock(&self.server);
        if !self.finished.load(Ordering::SeqCst) {
            return;
        }
        let running = self.lingering.load(Ordering::SeqCst)
            && lock(&self.conversation)
                .as_ref()
                .is_some_and(|conversation| !conversation.background().is_empty());
        if running {
            return;
        }
        let Some(server) = slot.take() else {
            return;
        };
        drop(slot);
        if let Some(connection) = self.connection() {
            connection.close();
        }
        server.kill();
    }

    /// The server ended: the turn running fails, and a server kept between turns is gone.
    pub(super) fn gone(&self, stderr: String) {
        lock(&self.ended).take();
        lock(&self.plan_answer).take();
        if self.finished.load(Ordering::SeqCst) {
            if self.lingering.swap(false, Ordering::SeqCst) {
                lock(&self.sink)(ProviderOutput::ProcessEnded);
            }
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

    /// Runs the thread's next turn in the server a completed turn kept; `false` when it
    /// can't, as when the request asks for a thread opened otherwise.
    pub(super) fn continue_with(
        self: &Arc<Self>,
        request: &TurnRequest,
        launch: &Launch,
        fast_tier: Option<String>,
    ) -> bool {
        let fits = *launch == Launch::of(request)
            && request.resume_session_at.is_none()
            && !request.fork_session
            && request.resume_session_id.is_some()
            && request.resume_session_id == self.thread_id();
        let slot = lock(&self.server);
        let alive = slot
            .as_ref()
            .is_some_and(|server| !server.exited.load(Ordering::SeqCst));
        if !fits || !alive || !self.lingering.load(Ordering::SeqCst) {
            return false;
        }
        let Some(connection) = self.connection() else {
            return false;
        };
        self.cancelled.store(false, Ordering::SeqCst);
        lock(&self.error).take();
        lock(&self.retry).take();
        self.finished.store(false, Ordering::SeqCst);
        self.lingering.store(false, Ordering::SeqCst);
        drop(slot);
        let (turn, request) = (self.clone(), copy(request));
        thread::spawn(move || {
            let outcome = turn.turns(&connection, &request, fast_tier);
            turn.end(outcome);
        });
        true
    }

    /// Ends a command the turn left running in the background.
    pub(super) fn stop_background_task(&self, task_id: &str) {
        let target = lock(&self.conversation).as_ref().and_then(|conversation| {
            Some((
                conversation.thread_id().to_owned(),
                conversation.process_id(task_id)?.to_owned(),
            ))
        });
        let (Some(connection), Some((thread, process))) = (self.connection(), target) else {
            return;
        };
        let _ = connection.request(
            "thread/backgroundTerminals/terminate",
            json!({ "threadId": thread, "processId": process }),
        );
    }

    pub(super) fn connection(&self) -> Option<Connection> {
        lock(&self.connection).clone()
    }

    pub(super) fn thread_id(&self) -> Option<String> {
        lock(&self.conversation)
            .as_ref()
            .map(|conversation| conversation.thread_id().to_owned())
    }

    /// The thread's running turn, as `(thread, turn)`.
    pub(super) fn running(&self) -> Option<(String, String)> {
        let conversation = lock(&self.conversation);
        let conversation = conversation.as_ref()?;
        Some((
            conversation.thread_id().to_owned(),
            conversation.turn_id()?.to_owned(),
        ))
    }

    pub(super) fn receive(&self, incoming: Incoming) {
        match incoming {
            Incoming::Notification { method, params } => self.notification(&method, &params),
            Incoming::Request { id, method, params } => match Kind::of(&method) {
                Some(kind) => self.request(id, kind, params),
                None => {
                    if let Some(connection) = self.connection() {
                        connection.reply_error(&id, -32601, "not supported by this client");
                    }
                }
            },
        }
    }

    pub(super) fn notification(&self, method: &str, params: &Value) {
        match method {
            "account/rateLimits/updated" => {
                let update = catalog::usage_update(params);
                let mut reported = lock(&self.limits);
                if let Some(update) = update.filter(|update| reported.as_ref() != Some(update)) {
                    *reported = Some(update.clone());
                    drop(reported);
                    self.emit(vec![ProviderOutput::UsageLimits(update)]);
                }
                return;
            }
            "serverRequest/resolved" => {
                self.resolved(&params["requestId"]);
                return;
            }
            "error" if self.own(params) => {
                if params["willRetry"] == true {
                    self.retrying(&params["error"]);
                } else {
                    *lock(&self.error) = Some(params["error"].clone());
                }
                return;
            }
            _ if MODEL_OUTPUT.contains(&method) && self.own(params) => {
                lock(&self.retry).take();
            }
            _ => {}
        }
        let (outputs, ended, background) = {
            let mut conversation = lock(&self.conversation);
            let Some(conversation) = conversation.as_mut() else {
                if method == "mcpServer/startupStatus/updated" {
                    lock(&self.early).push((method.to_owned(), params.clone()));
                }
                return;
            };
            let mut outputs = conversation.notification(method, params);
            let ended = method == "turn/completed"
                && params["threadId"].as_str() == Some(conversation.thread_id());
            if ended {
                conversation.set_turn(None);
            }
            let background = conversation.take_background_change();
            if let Some(tasks) = &background
                && !self.finished.load(Ordering::SeqCst)
            {
                outputs.push(ProviderOutput::BackgroundTasks {
                    tasks: tasks.clone(),
                    waiting: false,
                });
            }
            (outputs, ended, background)
        };
        self.emit(outputs);
        if let Some(tasks) = background
            && self.finished.load(Ordering::SeqCst)
            && self.lingering.load(Ordering::SeqCst)
        {
            lock(&self.sink)(ProviderOutput::BackgroundTasks {
                tasks,
                waiting: false,
            });
            self.shut_down_when_idle();
        }
        if ended && let Some(sender) = lock(&self.ended).as_ref() {
            let _ = sender.send(params["turn"].clone());
        }
    }

    /// Whether a notification is of the turn's own thread, not a subagent's.
    pub(super) fn own(&self, params: &Value) -> bool {
        let thread = self.thread_id();
        thread.is_some() && params["threadId"].as_str() == thread.as_deref()
    }

    /// Codex failed to reach the model and tries again.
    pub(super) fn retrying(&self, error: &Value) {
        let progress = retry_progress(error["message"].as_str().unwrap_or_default());
        let mut retry = lock(&self.retry);
        let attempt = progress.map_or_else(
            || retry.as_ref().map_or(0, |retry| retry.attempt) + 1,
            |(attempt, _)| attempt,
        );
        *retry = Some(Retry {
            attempt,
            since: Instant::now(),
            error: error.clone(),
        });
        drop(retry);
        self.emit(vec![ProviderOutput::Retrying {
            attempt,
            max_attempts: progress.map(|(_, max)| max),
            retry_in_ms: None,
            error: Some(error_detail(error)).filter(|detail| !detail.is_empty()),
        }]);
    }

    /// Why the turn fails, once Codex retried for too long with no answer by `now`.
    pub(super) fn stalled_retry(&self, now: Instant) -> Option<TurnFailure> {
        let retry = lock(&self.retry);
        let retry = retry
            .as_ref()
            .filter(|retry| now.saturating_duration_since(retry.since) >= RETRY_STALL_TIMEOUT)?;
        Some(turn_failure(&retry.error, lock(&self.limits).as_ref()))
    }

    /// Opens the thread's conversation, with what came before it was open.
    pub(super) fn opened(&self, thread_id: String) {
        let outputs: Vec<ProviderOutput> = {
            let mut slot = lock(&self.conversation);
            let conversation = slot.insert(Conversation::new(thread_id));
            std::mem::take(&mut *lock(&self.early))
                .into_iter()
                .flat_map(|(method, params)| conversation.notification(&method, &params))
                .collect()
        };
        self.emit(outputs);
    }

    /// Codex stopped waiting for a request hoshi still shows, as when its turn ended.
    pub(super) fn resolved(&self, rpc_id: &Value) {
        let mut requests = lock(&self.requests);
        let Some(request_id) = requests
            .iter()
            .find(|(_, pending)| &pending.rpc_id == rpc_id)
            .map(|(request_id, _)| request_id.clone())
        else {
            return;
        };
        requests.remove(&request_id);
        drop(requests);
        self.emit(vec![ProviderOutput::RequestWithdrawn { request_id }]);
    }

    pub(super) fn request(&self, rpc_id: Value, kind: Kind, params: Value) {
        let Some(connection) = self.connection() else {
            return;
        };
        if self.cancelled.load(Ordering::SeqCst) {
            connection.reply(&rpc_id, requests::withdrawn(kind));
            return;
        }
        let root = self.thread_id().unwrap_or_default();
        let access = *lock(&self.access);
        if lock(&self.session_approvals).allows(&root, access, kind) {
            connection.reply(&rpc_id, requests::accepted_for_session());
            return;
        }
        let tool = lock(&self.conversation).as_ref().and_then(|conversation| {
            conversation.tool(
                params["threadId"].as_str().unwrap_or(&root),
                params["itemId"].as_str()?,
            )
        });
        let request_id = requests::request_id(kind, &params, &rpc_id);
        let content = requests::content(kind, &params, tool);
        lock(&self.requests).insert(
            request_id.clone(),
            Pending {
                rpc_id,
                kind,
                params,
            },
        );
        self.emit(vec![ProviderOutput::RequestOpened {
            request_id,
            content,
        }]);
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
        let Some(pending) = lock(&self.requests).remove(request_id) else {
            return;
        };
        let Some(connection) = self.connection() else {
            return;
        };
        connection.reply(
            &pending.rpc_id,
            requests::answer(pending.kind, &pending.params, response),
        );
        let RequestResponse::Decision { decision, feedback } = response else {
            return;
        };
        match decision {
            ApprovalDecision::AcceptForSession if pending.kind.lasts_the_session() => {
                if let Some(root) = self.thread_id() {
                    let access = *lock(&self.access);
                    lock(&self.session_approvals).grant(&root, access, pending.kind);
                }
            }
            ApprovalDecision::Decline => {
                let feedback = feedback.as_deref().map(str::trim).unwrap_or_default();
                if let (false, Some((thread, turn))) = (feedback.is_empty(), self.running()) {
                    let _ = connection.request(
                        "turn/steer",
                        json!({ "threadId": thread, "expectedTurnId": turn, "input": [text_input(feedback)] }),
                    );
                }
            }
            ApprovalDecision::Cancel => self.cancel(),
            _ => {}
        }
    }

    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        lock(&self.plan_answer).take();
        let Some(connection) = self.connection() else {
            return;
        };
        for (_, pending) in lock(&self.requests).drain() {
            connection.reply(&pending.rpc_id, requests::withdrawn(pending.kind));
        }
        self.interrupt_running(&connection);
    }

    pub(super) fn interrupt_running(&self, connection: &Connection) {
        if let Some((thread, turn)) = self.running() {
            let _ = connection.request(
                "turn/interrupt",
                json!({ "threadId": thread, "turnId": turn }),
            );
        }
    }

    pub(super) fn steer(
        &self,
        message_id: &str,
        prompt: &str,
        attachments: &[TurnAttachment],
    ) -> bool {
        if self.finished.load(Ordering::SeqCst) || self.cancelled.load(Ordering::SeqCst) {
            return false;
        }
        let (Some(connection), Some((thread, turn))) = (self.connection(), self.running()) else {
            return false;
        };
        connection
            .call(
                "turn/steer",
                json!({
                    "threadId": thread,
                    "expectedTurnId": turn,
                    "input": user_input(prompt, attachments, &HashMap::new()),
                    "clientUserMessageId": message_id,
                }),
                STEER_TIMEOUT,
            )
            .is_ok()
    }

    pub(super) fn stop_subagent(&self, tool_use_id: &str) {
        let Some(connection) = self.connection() else {
            return;
        };
        let turns = lock(&self.conversation)
            .as_ref()
            .map(|conversation| conversation.subagent_turns(tool_use_id))
            .unwrap_or_default();
        for (thread, turn) in turns {
            let _ = connection.request(
                "turn/interrupt",
                json!({ "threadId": thread, "turnId": turn }),
            );
        }
    }

    /// Opens the thread and runs its turns: one, or one more for each plan the user answers.
    pub(super) fn run(
        &self,
        connection: &Connection,
        request: &TurnRequest,
        fast_tier: Option<String>,
    ) -> TurnOutcome {
        if let Err(error) = initialize(connection) {
            return provider_failed(error.detail());
        }
        let call = |method: &str, params| connection.call(method, params, THREAD_TIMEOUT);
        let (thread_id, thread_model) = match open_thread(&call, request, CLOSING_WAIT) {
            Ok(thread) => thread,
            Err(reason) => return TurnOutcome::Failed { reason },
        };
        *lock(&self.thread_model) = thread_model;
        self.opened(thread_id.clone());
        self.emit(vec![ProviderOutput::SessionStarted {
            session_id: thread_id,
        }]);
        self.turns(connection, request, fast_tier)
    }

    /// Runs the request's turns in the open thread.
    pub(super) fn turns(
        &self,
        connection: &Connection,
        request: &TurnRequest,
        fast_tier: Option<String>,
    ) -> TurnOutcome {
        let Some(thread_id) = self.thread_id() else {
            return provider_failed(String::new());
        };
        *lock(&self.access) = request.access_mode;
        lock(&self.session_approvals).begin(&thread_id, request.access_mode);
        let compact = request.prompt.trim() == COMPACT_COMMAND && request.attachments.is_empty();
        let skills = skill_paths(connection, request);
        let prompt = match &request.handoff {
            Some(handoff) => {
                let injected = connection
                    .call(
                        "thread/inject_items",
                        json!({ "threadId": thread_id, "items": history_items(handoff) }),
                        THREAD_TIMEOUT,
                    )
                    .is_ok();
                self.emit(vec![ProviderOutput::ContextHandoffDelivered { injected }]);
                if injected {
                    request.prompt.clone()
                } else {
                    handoff.inline_prompt(&request.prompt)
                }
            }
            None => request.prompt.clone(),
        };
        let mut input = user_input(&prompt, &request.attachments, &skills);
        let settings = TurnSettings {
            model: request
                .model
                .clone()
                .or_else(|| lock(&self.thread_model).clone())
                .unwrap_or_default(),
            effort: if request.ultracode {
                Some(ULTRA_EFFORT.to_owned())
            } else {
                request.effort.map(|effort| effort.as_str().to_owned())
            },
            fast_tier,
            access: request.access_mode,
            cwd: request.cwd.clone(),
        };
        let mut planning = request.plan_mode;
        let mut compacting = compact;
        loop {
            if self.cancelled.load(Ordering::SeqCst) {
                return TurnOutcome::Interrupted;
            }
            let (sender, ended) = mpsc::channel();
            *lock(&self.ended) = Some(sender);
            let started = if compacting {
                connection.call(
                    "thread/compact/start",
                    json!({ "threadId": thread_id }),
                    THREAD_TIMEOUT,
                )
            } else {
                connection.call(
                    "turn/start",
                    settings.turn_params(&thread_id, &input, planning),
                    THREAD_TIMEOUT,
                )
            };
            match started {
                Ok(answer) => {
                    if let Some(turn) = answer["turn"]["id"].as_str()
                        && let Some(conversation) = lock(&self.conversation).as_mut()
                    {
                        conversation.set_turn(Some(turn.to_owned()));
                    }
                }
                Err(error) => return provider_failed(error.detail()),
            }
            if self.cancelled.load(Ordering::SeqCst) {
                self.interrupt_running(connection);
            }
            let turn = loop {
                match ended.recv_timeout(STALL_POLL) {
                    Ok(turn) => break turn,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return TurnOutcome::Interrupted,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if let Some(reason) = self.stalled_retry(Instant::now()) {
                            self.interrupt_running(connection);
                            return TurnOutcome::Failed { reason };
                        }
                    }
                }
            };
            lock(&self.retry).take();
            if let Some(cursor) = turn["id"].as_str() {
                self.emit(vec![ProviderOutput::ConversationCursor {
                    cursor: cursor.to_owned(),
                }]);
            }
            match turn["status"].as_str() {
                Some("interrupted") => return TurnOutcome::Interrupted,
                Some("failed") => {
                    let error = latest_error(&turn["error"], lock(&self.error).take());
                    return TurnOutcome::Failed {
                        reason: turn_failure(&error, lock(&self.limits).as_ref()),
                    };
                }
                _ => {}
            }
            if self.cancelled.load(Ordering::SeqCst) {
                return TurnOutcome::Interrupted;
            }
            compacting = false;
            let plan = lock(&self.conversation)
                .as_mut()
                .and_then(Conversation::take_plan);
            let (true, Some(plan)) = (planning, plan) else {
                return TurnOutcome::Completed;
            };
            let request_id = plan_request_id(turn["id"].as_str().unwrap_or_default());
            let (sender, answers) = mpsc::channel();
            *lock(&self.plan_answer) = Some((request_id.clone(), sender));
            self.emit(vec![ProviderOutput::RequestOpened {
                request_id,
                content: ItemContent::ProposedPlan {
                    plan,
                    status: RequestStatus::Pending,
                    decision: None,
                    feedback: None,
                },
            }]);
            let Ok(RequestResponse::Decision { decision, feedback }) = answers.recv() else {
                return if self.cancelled.load(Ordering::SeqCst) {
                    TurnOutcome::Interrupted
                } else {
                    TurnOutcome::Completed
                };
            };
            let next = match (decision, feedback.filter(|text| !text.trim().is_empty())) {
                (decision, _) if decision.accepts() => {
                    planning = false;
                    IMPLEMENT_PLAN.to_owned()
                }
                (ApprovalDecision::Decline, Some(feedback)) => feedback,
                _ => return TurnOutcome::Completed,
            };
            input = vec![text_input(&next)];
        }
    }
}

pub(super) fn provider_failed(detail: String) -> TurnOutcome {
    TurnOutcome::Failed {
        reason: TurnFailure::ProviderError { detail },
    }
}

/// The error a turn failed with: the one it completed with, filled in with what Codex said
/// before it.
pub(super) fn latest_error(completed: &Value, notified: Option<Value>) -> Value {
    let mut error = notified
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    for (key, value) in completed.as_object().into_iter().flatten() {
        if !value.is_null() {
            error[key] = value.clone();
        }
    }
    error
}

/// What a `TurnError` says went wrong. `limits` are the plan's, as reported last.
pub(super) fn turn_failure(error: &Value, limits: Option<&UsageLimits>) -> TurnFailure {
    let detail = error_detail(error);
    let info = &error["codexErrorInfo"];
    let code = match info {
        Value::String(code) => Some(code.as_str()),
        Value::Object(fields) => fields.keys().next().map(String::as_str),
        _ => None,
    };
    let status = info
        .as_object()
        .and_then(|fields| fields.values().next())
        .and_then(|fields| fields["httpStatusCode"].as_u64());
    match (code, status) {
        (Some("usageLimitExceeded"), _) => TurnFailure::UsageLimited {
            detail,
            resets_at: limits.and_then(catalog::resets_at),
        },
        (Some("contextWindowExceeded"), _) => TurnFailure::ContextFull { detail },
        (Some("unauthorized"), _) | (_, Some(401)) => TurnFailure::AuthenticationFailed { detail },
        (Some("rateLimitExceeded" | "serverOverloaded"), _) | (_, Some(429)) => {
            TurnFailure::Overloaded { detail }
        }
        _ => TurnFailure::ProviderError { detail },
    }
}

/// A `TurnError`'s own words: its details when it gave some, else its message.
pub(super) fn error_detail(error: &Value) -> String {
    error["additionalDetails"]
        .as_str()
        .map(str::trim)
        .filter(|details| !details.is_empty())
        .or_else(|| error["message"].as_str().map(str::trim))
        .unwrap_or_default()
        .to_owned()
}

/// The attempt and the most Codex makes, as a retry's message counts them: `Reconnecting...
/// 2/5`.
fn retry_progress(message: &str) -> Option<(u32, u32)> {
    let chars: Vec<char> = message.chars().collect();
    let number = |digits: &[char]| -> Option<u32> {
        if digits.is_empty() {
            return None;
        }
        digits.iter().collect::<String>().parse().ok()
    };
    let bounded = |at: Option<&char>| at.is_none_or(|char| !char.is_alphanumeric());
    chars
        .iter()
        .enumerate()
        .filter(|(_, char)| **char == '/')
        .find_map(|(slash, _)| {
            let mut end = slash;
            while end > 0 && chars[end - 1] == ' ' {
                end -= 1;
            }
            let mut start = end;
            while start > 0 && chars[start - 1].is_ascii_digit() {
                start -= 1;
            }
            let mut first = slash + 1;
            while chars.get(first) == Some(&' ') {
                first += 1;
            }
            let mut last = first;
            while chars.get(last).is_some_and(char::is_ascii_digit) {
                last += 1;
            }
            let before = start.checked_sub(1).and_then(|at| chars.get(at));
            if !bounded(before) || !bounded(chars.get(last)) {
                return None;
            }
            let attempt = number(&chars[start..end])?;
            let max = number(&chars[first..last])?;
            (attempt >= 1 && max >= 1).then_some((attempt, max))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_messages_count_their_attempts() {
        assert_eq!(retry_progress("Reconnecting... 2/5"), Some((2, 5)));
        assert_eq!(retry_progress("Reconnecting... (3 / 10)"), Some((3, 10)));
        assert_eq!(retry_progress("Reconnecting..."), None);
        assert_eq!(retry_progress("see https://status.openai.com/v2"), None);
        assert_eq!(retry_progress("0/5"), None);
    }
}
