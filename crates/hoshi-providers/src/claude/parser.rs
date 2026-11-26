use super::*;

pub(super) struct Unlanded {
    item_id: String,
    reasoning: bool,
    text: String,
}

/// Translates Claude Code's `stream-json` output, one line at a time.
#[derive(Default)]
pub struct ClaudeParser {
    /// Content blocks seen so far per API message, to number assistant items
    /// the same way the stream numbers its deltas.
    pub(super) blocks_seen: HashMap<String, usize>,
    /// Blocks the stream wrote whose assistant message has not come yet, in order.
    pub(super) unlanded: Vec<Unlanded>,
    /// The window of the turn's model, until the turn's result reports the real one.
    pub(super) context_window: Option<u64>,
    /// The model of the agent's latest call, whose window the result reports.
    pub(super) model: Option<String>,
    pub(super) used_tokens: Option<u64>,
    /// How long the provider caches the prompt, as its latest call wrote it.
    pub(super) cache_ttl_secs: Option<u64>,
    /// Calls of the tools that move the session, by id, to read where their results lead.
    pub(super) moves: HashMap<String, WorktreeMove>,
    /// Where the session works, as last reported.
    pub(super) session_cwd: Option<String>,
    pub(super) requests: Requests,
    /// Lines to write back to the agent, such as answers it does not need the user for.
    pub(super) replies: Vec<String>,
    pub(super) session_id: Option<String>,
    /// Calls that change the task list Claude Code keeps on disk, by id.
    pub(super) task_calls: HashSet<String>,
    pub(super) live_output: LiveOutput,
    pub(super) queue: Queue,
    /// What the turn's frames told before its result.
    pub(super) signals: outcome::Signals,
    /// A run goes on: it began and its result has not come yet.
    pub(super) in_run: bool,
    /// The run going on wrote text of the agent's own.
    pub(super) run_text: bool,
    /// What the local command the run ran printed.
    pub(super) local_output: Option<String>,
    /// A message sent to the turn cut its run short; the run that answers it is due.
    pub(super) cut_short: bool,
    /// Subagents running in the background, by task id. A turn that ends while they run is
    /// not over: Claude Code goes on once they finish, in a run of its own.
    pub(super) background_agents: HashSet<String>,
    /// Commands running in the background, as the user sees them. Only those the agent
    /// awaits hold the turn; the others keep the process once the turn completed.
    pub(super) background_commands: Vec<BackgroundTask>,
    /// Commands that left the list before their end was told, which Claude Code answers
    /// once it is.
    pub(super) ending: HashSet<String>,
    /// The call that started each task, by task id.
    pub(super) task_calls_by_task: HashMap<String, String>,
    /// A run ended and the turn only waits on its background work: no run goes on.
    pub(super) idle: bool,
    /// How the latest run ended, which a turn that ends on its own between runs reports.
    pub(super) last_outcome: Option<TurnOutcome>,
    /// Every task of the thread's own agent that ran in the background, by task id.
    pub(super) ever_background: HashSet<String>,
    /// The subagent each task a subagent started in the background belongs to, by task id.
    /// Claude Code tells that subagent, not the thread's agent, when such a task ends.
    pub(super) owners: HashMap<String, Option<String>>,
    /// Subagents Claude Code runs again, as a task of theirs ended after they did.
    pub(super) reviving: HashSet<String>,
    /// Background tasks that ended, by run id in the order they ended, until Claude Code's
    /// turn about each ends: it answers each one in a run of its own, or in the run going on.
    pub(super) pending_wakes: VecDeque<String>,
    /// Calls that started a subagent or a workflow in the background, whose result comes
    /// with its end rather than at once; each with the subagent that made it, if one did.
    pub(super) async_agents: HashMap<String, Option<String>>,
    /// The runs of the turn that ended so far.
    pub(super) runs: u32,
    /// A run ended and no other started yet.
    pub(super) between_runs: bool,
    /// Background tasks that ended while a run went on, by run id, each told as the run that
    /// answers it starts.
    pub(super) ended_tasks: VecDeque<(String, ProviderOutput)>,
    pub(super) subagents: Subagents,
    /// The call that first started each subagent, by task id. One that runs again, as
    /// `SendMessage` resumes it, answers that call again.
    pub(super) launches: HashMap<String, String>,
    /// Tasks that run agents, by task id. Claude Code answers such a task even once stopped.
    pub(super) agent_tasks: HashSet<String>,
    /// `SendMessage` calls, by id, which may resume a subagent of an earlier process.
    pub(super) messages: HashSet<String>,
    /// `ScheduleWakeup` calls not answered yet, by id.
    pub(super) wakeup_calls: HashSet<String>,
    /// When Claude Code starts a run by itself, as `ScheduleWakeup` asked, in Unix
    /// milliseconds. The process stays for it.
    pub(super) wakeup: Option<u64>,
    pub(super) workflows: Workflows,
}

#[derive(Clone, Copy)]
pub(super) enum WorktreeMove {
    Enter,
    Exit,
}

impl ClaudeParser {
    /// `context_window` is the model's window when known before the turn runs.
    pub fn new(context_window: Option<u64>) -> Self {
        Self {
            context_window,
            ..Self::default()
        }
    }

    /// For a turn on `model`, `None` for the default one, at `effort`.
    pub fn for_turn(model: Option<&str>, effort: Option<Effort>) -> Self {
        let (slug, _) = manifest().run(model, None);
        Self {
            context_window: manifest().context_window(&slug),
            subagents: Subagents::for_turn(model, effort),
            ..Self::default()
        }
    }

    /// What to write back to the agent after the last lines, oldest first.
    pub fn take_replies(&mut self) -> Vec<String> {
        std::mem::take(&mut self.replies)
    }

    pub fn parse_line(&mut self, line: &str) -> Vec<ProviderOutput> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        let outputs = self.parse_value(value);
        self.queue.state().wakeup = self.wakeup.is_some();
        outputs
            .into_iter()
            .map(|output| self.with_subagent_detail(output))
            .collect()
    }

    /// A call that started a subagent completes with what the subagent ran on.
    pub(super) fn with_subagent_detail(&self, output: ProviderOutput) -> ProviderOutput {
        match output {
            ProviderOutput::ToolCallCompleted {
                item_id,
                output,
                is_error,
                detail,
            } => {
                let detail = detail.or_else(|| self.subagents.detail(&item_id));
                ProviderOutput::ToolCallCompleted {
                    item_id,
                    output,
                    is_error,
                    detail,
                }
            }
            ProviderOutput::Subagent {
                tool_use_id,
                output,
            } => ProviderOutput::Subagent {
                tool_use_id,
                output: Box::new(self.with_subagent_detail(*output)),
            },
            other => other,
        }
    }

    pub(super) fn parse_value(&mut self, value: Value) -> Vec<ProviderOutput> {
        let retarget = self.queue.state().retarget.take();
        if let Some((model, effort)) = retarget {
            self.retarget(model.as_deref(), effort);
        }
        if value["type"] == "result" {
            self.in_run = false;
        }
        match value["type"].as_str() {
            Some("control_request") => {
                let access = self.queue.state().access;
                let (outputs, reply) = requests::receive(&value, access, &self.requests);
                self.replies.extend(reply);
                return outputs;
            }
            Some("control_cancel_request") => {
                return requests::withdraw(&value, &self.requests);
            }
            Some("control_response") => {
                let response = &value["response"];
                match response["request_id"].as_str() {
                    Some(INTERRUPT_REQUEST_ID) => return self.interrupt_answered(),
                    Some(MODEL_REQUEST_ID | FLAGS_REQUEST_ID) => {
                        if response["subtype"] == "error" {
                            self.queue.state().model_unsure = true;
                        }
                        return Vec::new();
                    }
                    _ => {}
                }
                return self.live_output.answer(&value).into_iter().collect();
            }
            Some("system") => match value["subtype"].as_str() {
                Some("task_started") => return self.task_started(&value),
                Some("task_notification") => {
                    self.live_output.ended(&value);
                    let task = value["task_id"].as_str().unwrap_or_default();
                    let mut outputs = self.background_agent_ended(&value);
                    self.background_agents.remove(task);
                    self.reviving.remove(task);
                    self.ending.remove(task);
                    outputs.extend(self.background_command_ended(task));
                    let stopped = value["status"] == "stopped";
                    let unanswered = stopped && !self.agent_tasks.contains(task);
                    if let Some(Some(owner)) = self.owners.get(task)
                        && !stopped
                        && !self.background_agents.contains(owner)
                    {
                        self.reviving.insert(owner.clone());
                    }
                    if self.ever_background.contains(task)
                        && !unanswered
                        && let Some(run) = value["run_id"].as_str()
                    {
                        self.pending_wakes.push_back(run.to_owned());
                        let ended = ProviderOutput::BackgroundTaskEnded {
                            task_id: task.to_owned(),
                            tool_use_id: value["tool_use_id"].as_str().map(str::to_owned),
                            failed: value["status"] == "failed",
                        };
                        if std::mem::take(&mut self.between_runs) {
                            outputs.push(ended);
                        } else {
                            self.ended_tasks.push_back((run.to_owned(), ended));
                        }
                    }
                    outputs.extend(self.end_between_runs());
                    self.close_when_idle();
                    return outputs;
                }
                Some("background_tasks_changed") => {
                    let mut outputs: Vec<_> =
                        self.background_tasks_changed(&value).into_iter().collect();
                    outputs.extend(self.end_between_runs());
                    self.close_when_idle();
                    return outputs;
                }
                Some("task_progress") => {
                    return self.subagent_progress(&value).into_iter().collect();
                }
                Some("api_retry") => {
                    self.unlanded.clear();
                    return vec![outcome::retrying(&value)];
                }
                Some("status") if value["status"] == "compacting" => {
                    return vec![ProviderOutput::CompactionStarted];
                }
                Some("status") if value["compact_result"] == "failed" => {
                    let error = value["compact_error"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned();
                    return vec![ProviderOutput::CompactionFailed { error }];
                }
                Some("compact_boundary") => {
                    let metadata = &value["compact_metadata"];
                    let before_tokens = metadata["pre_tokens"].as_u64().unwrap_or_default();
                    let after_tokens = metadata["post_tokens"].as_u64().unwrap_or_default();
                    self.used_tokens = Some(after_tokens);
                    return vec![
                        ProviderOutput::CompactionFinished {
                            before_tokens,
                            after_tokens,
                        },
                        ProviderOutput::ContextUsage(ContextUsage {
                            used_tokens: after_tokens,
                            max_tokens: self.context_window,
                            cache_ttl_secs: self.cache_ttl_secs,
                            cache_expires_at: None,
                        }),
                    ];
                }
                _ => {}
            },
            _ => {}
        }
        if let Some(parent) = value["parent_tool_use_id"].as_str() {
            return self.parse_subagent(parent, &value);
        }
        // A resumed session first tells, in a run of its own, what an earlier process left
        // running, as a stop does; the turn's prompt runs after it.
        if value["type"] == "result"
            && value["origin"]["kind"] == "task-notification"
            && value["origin"]["runId"].is_null()
        {
            self.unlanded.clear();
            return Vec::new();
        }
        self.wakeup_fired(&value);
        if self.another_run_ended(&value) {
            self.unlanded.clear();
            return self.answered(&value);
        }
        let run_going = match value["type"].as_str() {
            Some("stream_event" | "assistant" | "user") => true,
            Some("system") => value["subtype"] == "init",
            Some("command_lifecycle") => value["state"] == "started",
            _ => false,
        };
        if run_going {
            self.in_run = true;
            self.cut_short = false;
        }
        if matches!(
            value["type"].as_str(),
            Some("stream_event" | "assistant" | "result")
        ) {
            let mut queue = self.queue.state();
            if !queue.awaits_begin() {
                queue.answering = true;
            }
        }
        let mut outputs = Vec::new();
        if run_going || value["type"] == "result" {
            outputs.extend(self.wake());
        }
        if run_going && std::mem::take(&mut self.idle) {
            outputs.extend(self.waiting_changed());
        }
        outputs.extend(self.parse_own_line(&value));
        outputs
    }

    /// Notes which of our messages the line is about; true for the result of a run about
    /// something else that the turn's message waits behind.
    pub(super) fn another_run_ended(&self, value: &Value) -> bool {
        let mut queue = self.queue.state();
        let echoed = echoed_messages(value);
        let lifecycle = value["type"] == "command_lifecycle"
            && value["command_uuid"]
                .as_str()
                .is_some_and(|id| queue.sent.contains(id));
        if lifecycle || !echoed.is_empty() {
            queue.echoes = true;
        }
        if echoed.iter().any(|id| queue.sent.contains(*id)) {
            queue.unbegun = false;
        }
        for id in &echoed {
            queue.waiting.remove(*id);
            if queue.prompt.as_deref() == Some(*id) {
                queue.prompt_started = true;
            }
        }
        value["type"] == "result"
            && queue.awaits_begin()
            && !queue.interrupted
            && (!echoed.is_empty()
                || value["origin"]["kind"]
                    .as_str()
                    .is_some_and(|kind| kind != "human"))
    }

    /// A line of the thread's own agent, not of one of its subagents.
    pub(super) fn parse_own_line(&mut self, value: &Value) -> Vec<ProviderOutput> {
        match value["type"].as_str() {
            Some("system") if value["subtype"] == "init" => {
                self.signals.session_started(value);
                self.run_text = false;
                if let Some(session_id) = value["session_id"].as_str() {
                    self.requests.set_session(session_id);
                    self.session_id = Some(session_id.to_owned());
                }
                let session =
                    value["session_id"]
                        .as_str()
                        .map(|session_id| ProviderOutput::SessionStarted {
                            session_id: session_id.to_owned(),
                        });
                // A resumed session reopens the worktree it was in, wherever it was started.
                // Every later run of the process repeats the folder it already reported.
                let cwd = value["cwd"]
                    .as_str()
                    .filter(|path| self.session_cwd.as_deref() != Some(*path))
                    .map(|path| {
                        self.session_cwd = Some(path.to_owned());
                        self.queue.state().cwd = Some(path.to_owned());
                        ProviderOutput::WorkingDirectory {
                            path: path.to_owned(),
                        }
                    });
                session.into_iter().chain(cwd).collect()
            }
            Some("stream_event") => {
                let outputs = self.parse_stream_event(value);
                self.run_text |= outputs
                    .iter()
                    .any(|output| matches!(output, ProviderOutput::TextDelta { .. }));
                outputs
            }
            Some("assistant") => {
                self.signals.assistant(value);
                let mut outputs = self.parse_assistant(None, &value["message"]);
                self.run_text |= outputs
                    .iter()
                    .any(|output| matches!(output, ProviderOutput::AssistantMessage { .. }));
                if let Some(uuid) = value["uuid"].as_str() {
                    outputs.push(ProviderOutput::ConversationCursor {
                        cursor: uuid.to_owned(),
                    });
                }
                outputs
            }
            Some("user") => {
                if let Some(printed) = outcome::local_command_output(value) {
                    self.local_output = Some(printed);
                }
                let interrupted = self.queue.state().interrupted;
                let results = parse_tool_results(value, interrupted);
                let mut outputs = self.answered(value);
                self.wakeup_scheduled(value);
                outputs.extend(self.hold_async_agents(value, results, None));
                outputs.extend(self.worktree_move(value));
                outputs.extend(self.changed_task_list(&value["message"]));
                outputs
            }
            Some("rate_limit_event") => {
                self.signals.rate_limit_event(&value["rate_limit_info"]);
                usage::rate_limit_event(&value["rate_limit_info"])
                    .map(ProviderOutput::UsageLimits)
                    .into_iter()
                    .collect()
            }
            Some("result") => {
                let mut outputs = self.cut_off_blocks();
                outputs.extend(self.reported_window(value));
                outputs.extend(self.answered(value));
                let outcome = self.parse_result(value);
                outputs.extend(self.reply_from_result(value, &outcome));
                let shared = self.queue.clone();
                let mut queue = shared.state();
                // A message the agent cut its run short for runs next.
                self.cut_short = !queue.interrupted
                    && queue.sent.len() > 1
                    && matches!(
                        value["terminal_reason"].as_str(),
                        Some("aborted_streaming" | "aborted_tools")
                    );
                // A message not yet taken in, or background work the turn waits for, gets
                // another run.
                let over = queue.interrupted
                    || (!self.cut_short
                        && !queue.steers_waiting()
                        && !self.holds()
                        && self.pending_wakes.is_empty());
                self.runs += 1;
                if over {
                    let outcome = if queue.interrupted {
                        TurnOutcome::Interrupted
                    } else {
                        outcome
                    };
                    outputs.extend(self.complete(&mut queue, outcome));
                    return outputs;
                }
                self.last_outcome = Some(outcome);
                self.idle = true;
                outputs.push(ProviderOutput::RunEnded { run: self.runs });
                match self.ended_tasks.pop_front() {
                    Some((_, ended)) => outputs.push(ended),
                    None => self.between_runs = true,
                }
                if !self.background_commands.is_empty() {
                    outputs.push(self.roster_with(&queue));
                }
                outputs
            }
            Some("command_lifecycle") => self.command_lifecycle(value),
            _ => Vec::new(),
        }
    }

    pub(super) fn parse_stream_event(&mut self, value: &Value) -> Vec<ProviderOutput> {
        let event = &value["event"];
        match event["type"].as_str() {
            Some("message_start") => {
                self.unlanded.clear();
                let message = &event["message"];
                if let Some(model) = message["model"].as_str() {
                    self.model = Some(model.to_owned());
                }
                let creation = &message["usage"]["cache_creation"];
                let written = |key: &str| creation[key].as_u64().is_some_and(|tokens| tokens > 0);
                if written("ephemeral_1h_input_tokens") {
                    self.cache_ttl_secs = Some(60 * 60);
                } else if written("ephemeral_5m_input_tokens") {
                    self.cache_ttl_secs = Some(5 * 60);
                }
                return self.context_usage(&message["usage"]).into_iter().collect();
            }
            Some("message_delta") => {
                return self.context_usage(&event["usage"]).into_iter().collect();
            }
            _ => {}
        }
        if event["type"] != "content_block_delta" {
            return Vec::new();
        }
        let (Some(message_id), Some(index)) =
            (value["api_message_id"].as_str(), event["index"].as_u64())
        else {
            return Vec::new();
        };
        let item_id = format!("{message_id}:{index}");
        let delta = &event["delta"];
        let (reasoning, text) = match (
            delta["type"].as_str(),
            delta["text"].as_str(),
            delta["thinking"].as_str(),
        ) {
            (Some("text_delta"), Some(text), _) => (false, text),
            (Some("thinking_delta"), _, Some(thinking)) => (true, thinking),
            _ => return Vec::new(),
        };
        match self
            .unlanded
            .iter_mut()
            .find(|block| block.item_id == item_id)
        {
            Some(block) => block.text.push_str(text),
            None => self.unlanded.push(Unlanded {
                item_id: item_id.clone(),
                reasoning,
                text: text.to_owned(),
            }),
        }
        let delta = text.to_owned();
        vec![if reasoning {
            ProviderOutput::ReasoningDelta { item_id, delta }
        } else {
            ProviderOutput::TextDelta { item_id, delta }
        }]
    }

    /// What the stream wrote of blocks a run ended in the middle of, as Claude Code sends no
    /// message for them: a message sent at once, or a stop, cut them.
    pub(super) fn cut_off_blocks(&mut self) -> Vec<ProviderOutput> {
        std::mem::take(&mut self.unlanded)
            .into_iter()
            .filter(|block| !block.text.trim().is_empty())
            .map(|block| {
                if block.reasoning {
                    ProviderOutput::Reasoning {
                        item_id: block.item_id,
                        text: block.text,
                    }
                } else {
                    ProviderOutput::AssistantMessage {
                        item_id: block.item_id,
                        text: block.text,
                    }
                }
            })
            .collect()
    }

    /// A message of the turn's agent, or of the subagent the call `owner` started.
    pub(super) fn parse_assistant(
        &mut self,
        owner: Option<&str>,
        message: &Value,
    ) -> Vec<ProviderOutput> {
        let Some(message_id) = message["id"].as_str() else {
            return Vec::new();
        };
        let Some(blocks) = message["content"].as_array() else {
            return Vec::new();
        };
        let mut outputs = Vec::new();
        for block in blocks {
            let index = self.blocks_seen.entry(message_id.to_owned()).or_default();
            let item_id = format!("{message_id}:{index}");
            *index += 1;
            self.unlanded.retain(|block| block.item_id != item_id);
            match block["type"].as_str() {
                Some("text") => {
                    let text = block["text"].as_str().unwrap_or_default();
                    if !text.trim().is_empty() {
                        outputs.push(ProviderOutput::AssistantMessage {
                            item_id,
                            text: text.to_owned(),
                        });
                    }
                }
                Some("thinking") => {
                    let text = block["thinking"].as_str().unwrap_or_default();
                    if !text.trim().is_empty() {
                        outputs.push(ProviderOutput::Reasoning {
                            item_id,
                            text: text.to_owned(),
                        });
                    }
                }
                Some("tool_use") => {
                    let name = block["name"].as_str().unwrap_or_default();
                    let movement = match name {
                        "EnterWorktree" => Some(WorktreeMove::Enter),
                        "ExitWorktree" => Some(WorktreeMove::Exit),
                        _ => None,
                    };
                    if let (Some(movement), Some(id)) = (movement, block["id"].as_str()) {
                        self.moves.insert(id.to_owned(), movement);
                    }
                    if matches!(name, "TaskCreate" | "TaskUpdate")
                        && let Some(id) = block["id"].as_str()
                    {
                        self.task_calls.insert(id.to_owned());
                    }
                    if let Some(id) = block["id"].as_str() {
                        match name {
                            "Agent" | "Task" | "Workflow" => self.subagents.launched(
                                owner,
                                id,
                                name,
                                truncate_json(&block["input"], TOOL_INPUT_MAX_CHARS),
                            ),
                            "SendMessage" => {
                                self.messages.insert(id.to_owned());
                            }
                            "ScheduleWakeup" => {
                                self.wakeup_calls.insert(id.to_owned());
                            }
                            _ => {}
                        }
                    }
                    let todos = (name == "TodoWrite")
                        .then(|| tools::todo_list(&block["input"]))
                        .flatten();
                    let call_id = block["id"].as_str().unwrap_or(&item_id).to_owned();
                    let subagent = matches!(name, "Agent" | "Task")
                        .then(|| self.subagents.started(owner, &call_id, &block["input"]))
                        .flatten();
                    outputs.push(ProviderOutput::ToolCallStarted {
                        item_id: call_id.clone(),
                        name: name.to_owned(),
                        input: truncate_json(&block["input"], TOOL_INPUT_MAX_CHARS),
                    });
                    outputs.extend(subagent.map(|detail| ProviderOutput::ToolCallDetail {
                        item_id: call_id,
                        detail,
                    }));
                    outputs.extend(todos.map(|tasks| ProviderOutput::TaskList { tasks }));
                }
                _ => {}
            }
        }
        outputs
    }
}

impl ClaudeParser {
    /// A call that started a subagent or a workflow in the background answers only that it
    /// started; the call stays open until what it started ends. `owner` is the subagent
    /// whose frame it is, whose results come without the CLI's reading of them.
    pub(super) fn hold_async_agents(
        &mut self,
        frame: &Value,
        outputs: Vec<ProviderOutput>,
        owner: Option<&str>,
    ) -> Vec<ProviderOutput> {
        let result = &frame["tool_use_result"];
        let launched = result["isAsync"] == true
            || result["status"] == "async_launched"
            || result["taskType"] == "local_workflow";
        let calls: Vec<String> = outputs
            .iter()
            .filter_map(|output| match output {
                ProviderOutput::ToolCallCompleted { item_id, .. } => Some(item_id.clone()),
                _ => None,
            })
            .collect();
        let identified: Vec<ProviderOutput> = match result["agentId"].as_str() {
            Some(agent_id) if calls.len() == 1 => self
                .subagents
                .identified(&calls[0], agent_id)
                .into_iter()
                .collect(),
            _ => Vec::new(),
        };
        let held = outputs
            .into_iter()
            .filter(|output| match output {
                ProviderOutput::ToolCallCompleted {
                    item_id,
                    output,
                    is_error: false,
                    ..
                } if launched
                    || (output.starts_with(ASYNC_AGENT_LAUNCHED)
                        && self.subagents.owner(item_id).is_some()) =>
                {
                    self.async_agents
                        .insert(item_id.clone(), owner.map(str::to_owned));
                    false
                }
                _ => true,
            })
            .collect::<Vec<_>>();
        identified.into_iter().chain(held).collect()
    }

    /// The end of a subagent or workflow started in the background closes the call that
    /// started it, in the transcript of the subagent that made the call.
    pub(super) fn background_agent_ended(&mut self, value: &Value) -> Vec<ProviderOutput> {
        let task_id = value["task_id"].as_str().unwrap_or_default();
        let workflow = self.workflows.ended(value);
        let Some(item_id) = self
            .launches
            .get(task_id)
            .map(String::as_str)
            .or(value["tool_use_id"].as_str())
            .map(str::to_owned)
        else {
            return Vec::new();
        };
        let Some(owner) = self.async_agents.remove(&item_id) else {
            return Vec::new();
        };
        let status = value["status"].as_str().unwrap_or_default();
        let summary = value["summary"].as_str().unwrap_or_default();
        let output = match workflow {
            Some(workflow) => format!("{summary}\n\n{workflow}"),
            None => summary.to_owned(),
        };
        let mut outputs = vec![ProviderOutput::ToolCallCompleted {
            item_id: item_id.clone(),
            output: truncate(&output, TOOL_OUTPUT_MAX_CHARS),
            is_error: status == "failed",
            detail: None,
        }];
        if status == "stopped" {
            outputs.push(ProviderOutput::ToolCallStopped { item_id });
        }
        match owner {
            Some(owner) => outputs
                .into_iter()
                .map(|output| ProviderOutput::Subagent {
                    tool_use_id: owner.clone(),
                    output: Box::new(output),
                })
                .collect(),
            None => outputs,
        }
    }

    /// A task started. A subagent that runs again, as `SendMessage` resumed it or a task of
    /// its own ended after it did, answers the call that first started it: that call runs
    /// again until the subagent ends.
    pub(super) fn task_started(&mut self, value: &Value) -> Vec<ProviderOutput> {
        let mut outputs = Vec::new();
        let launch = match (value["task_id"].as_str(), value["tool_use_id"].as_str()) {
            (Some(task_id), Some(call))
                if value["task_type"].as_str().is_some_and(output::runs_agents) =>
            {
                self.agent_tasks.insert(task_id.to_owned());
                let (launch, again) = self.launch_of(task_id, call);
                let running = self.live_output.agent_task(&launch).is_some();
                if again && !running && !self.async_agents.contains_key(&launch) {
                    let owner = self.subagents.owner(&launch).flatten().map(str::to_owned);
                    outputs.extend(self.subagents.reopened(&launch));
                    self.async_agents.insert(launch.clone(), owner);
                }
                outputs.extend(self.subagents.identified(&launch, task_id));
                Some(launch)
            }
            _ => None,
        };
        self.live_output.started(value, launch.as_deref());
        outputs.extend(self.background_task_started(value));
        outputs
    }

    /// The call that first started task `task_id`, which call `call` starts now; and whether
    /// the task ran before.
    pub(super) fn launch_of(&mut self, task_id: &str, call: &str) -> (String, bool) {
        if let Some(launch) = self.launches.get(task_id) {
            return (launch.clone(), true);
        }
        let earlier = match &self.session_id {
            Some(session_id) if self.messages.contains(call) => {
                self.subagents.launched_earlier(session_id, task_id)
            }
            _ => None,
        };
        let (launch, again) = match earlier {
            Some(launch) if launch != call => (launch, true),
            _ => (call.to_owned(), false),
        };
        self.launches.insert(task_id.to_owned(), launch.clone());
        (launch, again)
    }

    pub(super) fn background_task_started(&mut self, value: &Value) -> Option<ProviderOutput> {
        let task_id = value["task_id"].as_str()?;
        self.reviving.remove(task_id);
        self.own_task(value);
        let tool_use_id = value["tool_use_id"].as_str()?;
        self.task_calls_by_task
            .insert(task_id.to_owned(), tool_use_id.to_owned());
        let command = self
            .background_commands
            .iter_mut()
            .find(|command| command.task_id == task_id && command.tool_call_id.is_none())?;
        command.tool_call_id = Some(tool_use_id.to_owned());
        Some(self.background_roster())
    }

    /// Notes the subagent a task belongs to, from fields Claude Code sends without declaring.
    pub(super) fn own_task(&mut self, task: &Value) {
        let Some(task_id) = task["task_id"].as_str() else {
            return;
        };
        let owner = task["parent_task_id"]
            .as_str()
            .filter(|owner| !owner.is_empty())
            .map(str::to_owned);
        let nested = task["spawn_depth"].as_u64().is_some_and(|depth| depth > 1);
        if owner.is_some() || task["owned_by_subagent"] == true || nested {
            self.ever_background.remove(task_id);
            self.owners.insert(task_id.to_owned(), owner);
        }
    }

    /// Claude Code lists what runs in the background whenever that changes. Subagents and
    /// workflows hold the turn; shell commands (`Bash` sent to the background, `Monitor`)
    /// only when the agent awaits them.
    pub(super) fn background_tasks_changed(&mut self, value: &Value) -> Option<ProviderOutput> {
        let tasks: Vec<&Value> = value["tasks"].as_array().into_iter().flatten().collect();
        for task in &tasks {
            self.own_task(task);
        }
        let runs_agents =
            |task: &Value| task["task_type"].as_str().is_some_and(output::runs_agents);
        self.background_agents = tasks
            .iter()
            .filter(|task| runs_agents(task))
            .filter_map(|task| task["task_id"].as_str().map(str::to_owned))
            .collect();
        self.agent_tasks
            .extend(self.background_agents.iter().cloned());
        self.reviving
            .retain(|owner| !self.background_agents.contains(owner));
        let now = unix_millis();
        let commands: Vec<BackgroundTask> = tasks
            .iter()
            .filter(|task| task["task_type"] == "local_bash")
            .filter_map(|task| {
                let task_id = task["task_id"].as_str()?;
                let known = self
                    .background_commands
                    .iter()
                    .find(|command| command.task_id == task_id);
                Some(known.cloned().unwrap_or_else(|| BackgroundTask {
                    task_id: task_id.to_owned(),
                    tool_call_id: self.task_calls_by_task.get(task_id).cloned(),
                    description: task["description"].as_str().unwrap_or_default().to_owned(),
                    started_at: now,
                }))
            })
            .collect();
        let own = tasks
            .iter()
            .filter(|task| runs_agents(task) || task["task_type"] == "local_bash")
            .filter_map(|task| task["task_id"].as_str())
            .filter(|task_id| !self.owners.contains_key(*task_id))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        self.ever_background.extend(own);
        for gone in &self.background_commands {
            if !commands
                .iter()
                .any(|command| command.task_id == gone.task_id)
            {
                self.ending.insert(gone.task_id.clone());
            }
        }
        if commands == self.background_commands {
            return None;
        }
        self.set_commands(commands);
        Some(self.background_roster())
    }

    pub(super) fn background_command_ended(&mut self, task_id: &str) -> Option<ProviderOutput> {
        if !self
            .background_commands
            .iter()
            .any(|command| command.task_id == task_id)
        {
            return None;
        }
        let left = self
            .background_commands
            .iter()
            .filter(|command| command.task_id != task_id)
            .cloned()
            .collect();
        self.set_commands(left);
        Some(self.background_roster())
    }

    /// Also forgets that the agent awaited commands no longer running.
    pub(super) fn set_commands(&mut self, commands: Vec<BackgroundTask>) {
        self.background_commands = commands;
        let running: HashSet<String> = self
            .background_commands
            .iter()
            .map(|command| command.task_id.clone())
            .collect();
        let mut queue = self.queue.state();
        queue.awaited.retain(|task_id| running.contains(task_id));
        queue.commands = running;
    }

    /// Whether the turn waits for background work: its subagents. Commands the agent awaits
    /// leave the turn to end, and wake the agent once they do.
    pub(super) fn holds(&self) -> bool {
        !self.background_agents.is_empty() || !self.reviving.is_empty()
    }

    /// Ends the turn. Its process stays for the commands still running in the background,
    /// unless the user stopped the turn.
    pub(super) fn complete(
        &mut self,
        queue: &mut QueueState,
        outcome: TurnOutcome,
    ) -> Vec<ProviderOutput> {
        let lingers = !queue.interrupted
            && (!self.background_commands.is_empty()
                || !self.ending.is_empty()
                || self.wakeup.is_some());
        queue.turn_open = false;
        queue.closing = !lingers;
        queue.sent.clear();
        queue.unbegun = false;
        queue.prompt = None;
        queue.prompt_started = false;
        let dropped = queue.dropped(outcome != TurnOutcome::Completed);
        self.runs = 0;
        self.idle = false;
        self.between_runs = false;
        self.last_outcome = None;
        self.in_run = false;
        self.run_text = false;
        self.local_output = None;
        self.cut_short = false;
        self.unlanded.clear();
        self.signals.turn_ended();
        let mut outputs = Vec::new();
        if lingers && !self.background_commands.is_empty() {
            outputs.push(self.roster_with(queue));
        }
        if !dropped.is_empty() {
            outputs.push(ProviderOutput::MessagesDropped {
                message_ids: dropped,
            });
        }
        outputs.push(ProviderOutput::TurnCompleted { outcome });
        outputs
    }

    /// A run that starts between turns is the agent going on by itself, as a command left
    /// running ended: a turn of its own, which first tells what ended.
    pub(super) fn wake(&mut self) -> Vec<ProviderOutput> {
        {
            let mut queue = self.queue.state();
            if queue.turn_open || queue.closing {
                return Vec::new();
            }
            queue.turn_open = true;
            queue.answering = true;
        }
        let mut outputs = vec![ProviderOutput::Woke];
        outputs.extend(self.ended_tasks.pop_front().map(|(_, ended)| ended));
        outputs
    }

    /// A line Claude Code sent about a background task that ended answers it, as a run of
    /// its own ending or as the run going on taking it in. It answers the tasks that ended
    /// before it too, which Claude Code answered first or took in along with it; those not
    /// told yet are told now.
    pub(super) fn answered(&mut self, value: &Value) -> Vec<ProviderOutput> {
        if value["origin"]["kind"] != "task-notification" {
            return Vec::new();
        }
        let Some(run) = value["origin"]["runId"].as_str() else {
            return Vec::new();
        };
        let Some(position) = self.pending_wakes.iter().position(|wake| wake == run) else {
            return Vec::new();
        };
        let answered: HashSet<String> = self.pending_wakes.drain(..=position).collect();
        let (told, untold) = std::mem::take(&mut self.ended_tasks)
            .into_iter()
            .partition(|(run, _)| answered.contains(run));
        self.ended_tasks = untold;
        told.into_iter().map(|(_, ended)| ended).collect()
    }

    /// Between turns, the process ends once nothing it kept runs or waits for an answer.
    pub(super) fn close_when_idle(&self) {
        let mut queue = self.queue.state();
        if !queue.turn_open
            && self.background_commands.is_empty()
            && self.ending.is_empty()
            && self.background_agents.is_empty()
            && self.reviving.is_empty()
            && self.pending_wakes.is_empty()
            && self.wakeup.is_none()
        {
            queue.closing = true;
        }
    }

    pub(super) fn background_roster(&self) -> ProviderOutput {
        let queue = self.queue.state();
        self.roster_with(&queue)
    }

    /// Waiting while the turn only waits between runs, or, once it ended, while the agent
    /// awaits commands still running.
    fn roster_with(&self, queue: &QueueState) -> ProviderOutput {
        ProviderOutput::BackgroundTasks {
            tasks: self.background_commands.clone(),
            waiting: self.idle || (!queue.turn_open && !queue.awaited.is_empty()),
        }
    }

    /// A run started or ended while commands run in the background.
    pub(super) fn waiting_changed(&self) -> Option<ProviderOutput> {
        (!self.background_commands.is_empty()).then(|| self.background_roster())
    }

    /// A turn that waits between runs ends on its own once nothing is left to wait for, or
    /// once the user asked it to stop: Claude Code answers no stopped task, and an interrupt
    /// with no run going ends none.
    pub(super) fn end_between_runs(&mut self) -> Vec<ProviderOutput> {
        if !self.idle {
            return Vec::new();
        }
        let shared = self.queue.clone();
        let mut queue = shared.state();
        let waits = self.holds()
            || !self.pending_wakes.is_empty()
            || queue.steers_waiting()
            || self.cut_short;
        if queue.closing || !queue.turn_open || (waits && !queue.interrupted) {
            return Vec::new();
        }
        let outcome = if queue.interrupted {
            TurnOutcome::Interrupted
        } else {
            self.last_outcome.clone().unwrap_or(TurnOutcome::Completed)
        };
        self.complete(&mut queue, outcome)
    }

    /// A subagent's own messages and tool calls, which go to its transcript. Its prompt, sent
    /// as a user message, is already the input of the call that started it.
    pub(super) fn parse_subagent(&mut self, parent: &str, value: &Value) -> Vec<ProviderOutput> {
        let mut seen = Vec::new();
        let outputs = match value["type"].as_str() {
            Some("assistant") => {
                let model = value["message"]["model"].as_str();
                seen.extend(self.subagents.model_seen(parent, model));
                self.parse_assistant(Some(parent), &value["message"])
            }
            Some("user") => {
                let results = parse_tool_results(value, self.queue.state().interrupted);
                self.hold_async_agents(value, results, Some(parent))
            }
            _ => Vec::new(),
        };
        let own = outputs
            .into_iter()
            .filter(|output| {
                matches!(
                    output,
                    ProviderOutput::AssistantMessage { .. }
                        | ProviderOutput::Reasoning { .. }
                        | ProviderOutput::ToolCallStarted { .. }
                        | ProviderOutput::ToolCallDetail { .. }
                        | ProviderOutput::ToolCallCompleted { .. }
                        | ProviderOutput::ToolCallStopped { .. }
                )
            })
            .map(|output| ProviderOutput::Subagent {
                tool_use_id: parent.to_owned(),
                output: Box::new(output),
            });
        seen.into_iter().chain(own).collect()
    }

    /// What a running subagent is doing, as a line that replaces the previous one; what each
    /// agent of a running workflow is doing.
    pub(super) fn subagent_progress(&mut self, value: &Value) -> Option<ProviderOutput> {
        let task_id = value["task_id"].as_str()?;
        if self.live_output.follows(task_id) {
            return None;
        }
        let item_id = self
            .launches
            .get(task_id)
            .map(String::as_str)
            .or(value["tool_use_id"].as_str())?
            .to_owned();
        let output = match self.workflows.update(value) {
            Some(workflow) => workflow,
            None => value["description"].as_str()?.to_owned(),
        };
        Some(ProviderOutput::ToolOutput { item_id, output })
    }

    /// Notes when `ScheduleWakeup` has Claude Code start a run by itself, from the result of
    /// the call: `scheduledFor` is 0 once the agent called it to stop.
    pub(super) fn wakeup_scheduled(&mut self, frame: &Value) {
        let blocks = frame["message"]["content"].as_array().into_iter().flatten();
        let answered: Vec<&Value> = blocks
            .filter(|block| {
                block["tool_use_id"]
                    .as_str()
                    .is_some_and(|id| self.wakeup_calls.remove(id))
            })
            .collect();
        if answered.is_empty() {
            return;
        }
        let at = frame["tool_use_result"]["scheduledFor"].as_u64();
        self.wakeup =
            at.filter(|at| *at > 0 && answered.len() == 1 && answered[0]["is_error"] != true);
    }

    /// The run `ScheduleWakeup` asked for starts, or one starts once it is due.
    pub(super) fn wakeup_fired(&mut self, value: &Value) {
        let Some(at) = self.wakeup else {
            return;
        };
        let woke = value["type"] == "command_lifecycle"
            && value["state"] == "started"
            && !value["wakeup_source"].is_null();
        let due = value["type"] == "system" && value["subtype"] == "init" && unix_millis() >= at;
        if woke || due {
            self.wakeup = None;
        }
    }

    /// The task list as Claude Code saved it, once a call that changes it succeeded.
    pub(super) fn changed_task_list(&mut self, message: &Value) -> Option<ProviderOutput> {
        let blocks = message["content"].as_array()?;
        let changed = blocks.iter().fold(false, |changed, block| {
            let call = block["tool_use_id"].as_str();
            let ours = call.is_some_and(|id| self.task_calls.remove(id));
            changed || (ours && block["is_error"] != true)
        });
        if !changed {
            return None;
        }
        let list_id = std::env::var("CLAUDE_CODE_TASK_LIST_ID")
            .ok()
            .filter(|id| !id.trim().is_empty())
            .or_else(|| self.session_id.clone())?;
        let dir = install::config_dir()?.join("tasks").join(
            list_id
                .chars()
                .map(|char| {
                    if char.is_ascii_alphanumeric() || char == '_' || char == '-' {
                        char
                    } else {
                        '-'
                    }
                })
                .collect::<String>(),
        );
        let tasks = tools::saved_task_list(&dir)?;
        Some(ProviderOutput::TaskList { tasks })
    }

    /// Where a successful `EnterWorktree` or `ExitWorktree` left the session, from the
    /// structured result the CLI sends along with the tool's text.
    pub(super) fn worktree_move(&mut self, value: &Value) -> Option<ProviderOutput> {
        let blocks = value["message"]["content"].as_array()?;
        let movement = blocks.iter().find_map(|block| {
            let id = block["tool_use_id"].as_str()?;
            let movement = self.moves.remove(id)?;
            (block["is_error"] != true).then_some(movement)
        })?;
        let result = &value["tool_use_result"];
        let path = match movement {
            WorktreeMove::Enter => result["worktreePath"].as_str(),
            WorktreeMove::Exit => result["restoredCwd"].as_str(),
        }?;
        self.session_cwd = Some(path.to_owned());
        self.queue.state().cwd = Some(path.to_owned());
        Some(ProviderOutput::WorkingDirectory {
            path: path.to_owned(),
        })
    }

    /// Everything the call sent and wrote stays in the context, cached or not.
    pub(super) fn context_usage(&mut self, usage: &Value) -> Option<ProviderOutput> {
        let input = usage["input_tokens"].as_u64()?;
        let used_tokens = input
            + usage["cache_creation_input_tokens"].as_u64().unwrap_or(0)
            + usage["cache_read_input_tokens"].as_u64().unwrap_or(0)
            + usage["output_tokens"].as_u64().unwrap_or(0);
        if used_tokens == 0 {
            return None;
        }
        self.used_tokens = Some(used_tokens);
        Some(ProviderOutput::ContextUsage(ContextUsage {
            used_tokens,
            max_tokens: self.context_window,
            cache_ttl_secs: self.cache_ttl_secs,
            cache_expires_at: None,
        }))
    }

    /// The result names the window Claude Code measured the model against.
    pub(super) fn reported_window(&mut self, result: &Value) -> Option<ProviderOutput> {
        let window = result["modelUsage"][self.model.as_deref()?]["contextWindow"]
            .as_u64()
            .filter(|window| *window > 0)?;
        if self.context_window == Some(window) {
            return None;
        }
        self.context_window = Some(window);
        Some(ProviderOutput::ContextUsage(ContextUsage {
            used_tokens: self.used_tokens?,
            max_tokens: Some(window),
            cache_ttl_secs: self.cache_ttl_secs,
            cache_expires_at: None,
        }))
    }
}

/// `structured` is the CLI's own reading of the result, sent along when the message holds a
/// single one.
/// What Claude Code answers in place of a call the user stopped.
pub(super) const STOPPED_MARKER: &str = "[Request interrupted by user";

/// How a call that started a subagent in the background answers, which a subagent's frames
/// tell without the CLI's reading of the result.
pub(super) const ASYNC_AGENT_LAUNCHED: &str = "Async agent launched successfully.";

/// The results a `user` frame carries. A frame with a single result also carries the CLI's
/// own reading of it, in `tool_use_result`. Once the user asked the turn to stop, a call that
/// fails was stopped.
pub(super) fn parse_tool_results(frame: &Value, interrupted: bool) -> Vec<ProviderOutput> {
    let structured = &frame["tool_use_result"];
    let Some(blocks) = frame["message"]["content"].as_array() else {
        return Vec::new();
    };
    let cancelled = |id: &str| {
        frame["tool_result_meta"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|meta| meta["id"] == id && meta["non_execution_kind"] == "cancelled")
    };
    let results = blocks
        .iter()
        .filter(|block| block["type"] == "tool_result")
        .count();
    let detail = if results == 1 {
        tools::detail(structured)
    } else {
        None
    };
    blocks
        .iter()
        .filter(|block| block["type"] == "tool_result")
        .flat_map(|block| {
            let Some(item_id) = block["tool_use_id"].as_str().map(str::to_owned) else {
                return Vec::new();
            };
            let output = match &block["content"] {
                Value::String(text) => text.clone(),
                Value::Array(parts) => parts
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            };
            let (output, cut_short) = subagent_result(&output);
            let is_error = cut_short || block["is_error"].as_bool().unwrap_or(false);
            let stopped = cancelled(&item_id)
                || (is_error && (interrupted || output.trim_start().starts_with(STOPPED_MARKER)));
            let mut outputs = vec![ProviderOutput::ToolCallCompleted {
                item_id: item_id.clone(),
                output: truncate(&output, TOOL_OUTPUT_MAX_CHARS),
                is_error,
                detail: detail.clone().filter(|_| !is_error),
            }];
            if stopped {
                outputs.push(ProviderOutput::ToolCallStopped { item_id });
            }
            outputs
        })
        .collect()
}

pub(super) const TOOLS_USED_OPEN: &str = "<tools-used-before-summary>";

pub(super) const TOOLS_USED_CLOSE: &str = "</tools-used-before-summary>";

/// A subagent's result without the list of tools Claude Code puts before its summary.
pub(super) fn without_tools_used(output: &str) -> &str {
    let trimmed = output.trim_start();
    match trimmed
        .strip_prefix(TOOLS_USED_OPEN)
        .and_then(|rest| rest.split_once(TOOLS_USED_CLOSE))
    {
        Some((_, summary)) => summary.trim_start(),
        None => output,
    }
}

pub(super) const HAND_BACK: &str = "[Subagent hand-back]";

pub(super) const REPORT_FOLLOWS: &str = "The report follows:\n";

pub(super) const REPORT_INDENT: &str = "  ";

/// How Claude Code opens the result of a subagent an API error cut short, such as a usage
/// limit, ahead of what it recovered of the subagent's report.
pub(super) const CUT_SHORT_MARKER: &str = "Agent terminated early due to an API error: ";

/// What Claude Code appends to an API error between parentheses.
const ERROR_DIAGNOSTICS: [&str; 4] = [
    "error type ",
    "HTTP ",
    "request id ",
    "model sent to the API",
];

/// A subagent's result, and whether an API error cut it short. Claude Code reports such a
/// subagent as completed; its error then comes first, above what it reported until then.
pub(super) fn subagent_result(output: &str) -> (String, bool) {
    let report = subagent_report(output);
    let Some(error) = output.trim_start().strip_prefix(CUT_SHORT_MARKER) else {
        return (report, false);
    };
    let error = without_diagnostics(error.lines().next().unwrap_or_default().trim_end());
    let partial = report.trim();
    let result = if partial.is_empty() || partial.starts_with(CUT_SHORT_MARKER) {
        error.to_owned()
    } else {
        format!("{error}\n\n{partial}")
    };
    (result, true)
}

fn without_diagnostics(error: &str) -> &str {
    error
        .rfind(" (")
        .filter(|at| {
            let note = &error[at + 2..];
            note.ends_with(')')
                && ERROR_DIAGNOSTICS
                    .iter()
                    .any(|prefix| note.starts_with(prefix))
        })
        .map_or(error, |at| &error[..at])
}

/// What a subagent reported, without what Claude Code wraps it in for the agent that
/// started it: notes of its own such as the tools the subagent used, a line saying the
/// report is no message of the user's, and each line of the report indented under it.
pub(super) fn subagent_report(output: &str) -> String {
    let framed = output
        .match_indices(HAND_BACK)
        .find(|(at, _)| *at == 0 || output[..*at].ends_with('\n'))
        .and_then(|(at, _)| output[at..].split_once(REPORT_FOLLOWS));
    let Some((_, framed)) = framed else {
        return without_tools_used(output).to_owned();
    };
    let report = framed
        .lines()
        .take_while(|line| line.is_empty() || line.starts_with(REPORT_INDENT))
        .map(|line| line.strip_prefix(REPORT_INDENT).unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    without_tools_used(report.trim_end()).to_owned()
}

impl ClaudeParser {
    pub(super) fn parse_result(&self, value: &Value) -> TurnOutcome {
        let now = (unix_millis() / 1000) as i64;
        outcome::outcome(value, &self.signals, now)
    }

    /// The result's text, for a completed run that wrote none of its own, as a local command
    /// such as `/cost` does.
    pub(super) fn reply_from_result(
        &mut self,
        value: &Value,
        outcome: &TurnOutcome,
    ) -> Option<ProviderOutput> {
        let wrote = std::mem::take(&mut self.run_text);
        let printed = self.local_output.take();
        if wrote || *outcome != TurnOutcome::Completed {
            return None;
        }
        let text = outcome::reply_text(value, printed.as_deref())?;
        let item_id = match value["uuid"].as_str() {
            Some(uuid) => format!("{uuid}:result"),
            None => format!("result:{}", self.runs),
        };
        Some(ProviderOutput::AssistantMessage { item_id, text })
    }

    /// Where Claude Code is with a message of ours: queued, begun, or let go of.
    pub(super) fn command_lifecycle(&mut self, value: &Value) -> Vec<ProviderOutput> {
        let Some(message_id) = value["command_uuid"].as_str() else {
            return Vec::new();
        };
        let shared = self.queue.clone();
        let mut queue = shared.state();
        queue.lifecycles = true;
        let prompt = queue.prompt.as_deref() == Some(message_id);
        match value["state"].as_str() {
            Some("started") => {
                queue.waiting.remove(message_id);
                queue.prompt_started |= prompt;
                self.between_runs = false;
                self.local_output = None;
                Vec::new()
            }
            Some(state @ ("completed" | "cancelled")) => {
                if queue.waiting.remove(message_id) {
                    queue.unread.push(message_id.to_owned());
                }
                self.cut_short &= prompt;
                if prompt && !queue.prompt_started && queue.turn_open && !queue.closing {
                    let outcome = if queue.interrupted {
                        TurnOutcome::Interrupted
                    } else {
                        TurnOutcome::Failed {
                            reason: TurnFailure::ProviderError {
                                detail: format!("the message was {state} without a run of its own"),
                            },
                        }
                    };
                    return self.complete(&mut queue, outcome);
                }
                drop(queue);
                self.end_between_runs()
            }
            _ => Vec::new(),
        }
    }

    /// Claude Code agreed to stop. A run going on ends with its result; with none, the turn
    /// ends now.
    pub(super) fn interrupt_answered(&mut self) -> Vec<ProviderOutput> {
        if self.in_run {
            return Vec::new();
        }
        let shared = self.queue.clone();
        let mut queue = shared.state();
        if !queue.interrupted || !queue.turn_open || queue.closing {
            return Vec::new();
        }
        self.complete(&mut queue, TurnOutcome::Interrupted)
    }

    /// The turn switched the process to `model`, `None` for the default one, at `effort`.
    pub(super) fn retarget(&mut self, model: Option<&str>, effort: Option<Effort>) {
        self.context_window = Self::for_turn(model, effort).context_window;
        self.subagents.retarget(model, effort);
    }
}
