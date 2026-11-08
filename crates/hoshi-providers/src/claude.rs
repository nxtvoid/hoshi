use crate::claude_commands;
use crate::claude_install;
use crate::claude_models::{TurnModel, Version, manifest, parse_version};
use crate::claude_output::LiveOutput;
use crate::claude_prompt;
use crate::claude_requests::{self, Access, Grants, Requests};
use crate::claude_titles;
use crate::claude_tools;
use crate::claude_usage;
use crate::process::{ProcessTurn, hide_console, spawn_turn};
use crate::{
    GeneratedTitle, Installation, MCP_SERVER_NAME, McpServer, OutputSink, ProviderAdapter,
    RunningTurn, TitleRequest, TurnAttachment, TurnRequest, truncate,
};
use hoshi_core::{
    AccessMode, ContextUsage, ProviderModels, ProviderOutput, RequestResponse, SlashCommand,
    TurnFailure, TurnOutcome, UsageLimits,
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

const TOOL_INPUT_MAX_CHARS: usize = 2_000;
const TOOL_OUTPUT_MAX_CHARS: usize = 8_000;

pub struct ClaudeCodeAdapter {
    /// Empty means `claude` on the PATH.
    binary_path: Option<String>,
    /// Runs threads without a model of their own; empty means the manifest's default.
    model: Option<String>,
    /// `None` until asked; then `Some(None)` when the CLI can't be run.
    version: Mutex<Option<Option<Version>>>,
    grants: Grants,
}

impl ClaudeCodeAdapter {
    pub fn new(binary_path: Option<String>, model: Option<String>) -> Self {
        Self {
            binary_path,
            model,
            version: Mutex::default(),
            grants: Grants::default(),
        }
    }

    fn binary(&self) -> &str {
        self.binary_path.as_deref().unwrap_or("claude")
    }

    /// Asks the CLI once; `None` when it can't be run.
    fn version(&self) -> Option<Version> {
        let mut version = self
            .version
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *version.get_or_insert_with(|| {
            let mut command = Command::new(self.binary());
            command
                .arg("--version")
                .stdin(Stdio::null())
                .stderr(Stdio::null());
            hide_console(&mut command);
            let output = command.output().ok()?;
            parse_version(&String::from_utf8_lossy(&output.stdout))
        })
    }
}

impl ProviderAdapter for ClaudeCodeAdapter {
    fn models(&self) -> ProviderModels {
        let today = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| (elapsed.as_secs() / 86_400) as i64)
            .unwrap_or_default();
        manifest().catalog(self.version(), today)
    }

    fn usage_limits(&self) -> Option<UsageLimits> {
        claude_usage::probe(self.binary())
    }

    fn installation(&self) -> Option<Installation> {
        let installed = self
            .version()
            .map(|(major, minor, patch)| format!("{major}.{minor}.{patch}"));
        Some(claude_install::installation(self.binary(), installed))
    }

    fn forget_version(&self) {
        *self
            .version
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    fn slash_commands(&self, cwd: &Path) -> Option<Vec<SlashCommand>> {
        claude_commands::probe(self.binary(), cwd)
    }

    fn generate_title(&self, request: &TitleRequest) -> Option<GeneratedTitle> {
        claude_titles::generate(self.binary(), request)
    }

    fn start_turn(
        &self,
        request: TurnRequest,
        sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
        let turn_model = manifest().resolve(
            request.model.as_deref().or(self.model.as_deref()),
            request.effort,
            request.fast_mode,
            request.ultracode,
        );
        let access = Access {
            mode: request.access_mode,
            plan_mode: request.plan_mode,
        };
        let mut command = Command::new(self.binary());
        command
            .current_dir(&request.cwd)
            .args(["-p", "--input-format", "stream-json"])
            .args(["--output-format", "stream-json", "--verbose"])
            .arg("--include-partial-messages")
            .args(["--thinking-display", "summarized"])
            .args(["--permission-prompt-tool", "stdio"])
            // Reports when each message sent mid-turn is taken in, to know when a turn is over.
            .arg("--replay-user-messages")
            // `-p` leaves the task list tools out unless asked for.
            .env("CLAUDE_CODE_ENABLE_TODO_TOOLS", "1")
            .args(["--permission-mode", access.permission_mode()]);
        // Also lets an accepted plan hand the session back to full access.
        if request.access_mode == AccessMode::FullAccess {
            command.arg("--allow-dangerously-skip-permissions");
        }
        command.args(["--model", &turn_model.slug]);
        if let Some(effort) = &turn_model.effort {
            command.args(["--effort", effort]);
        }
        if let Some(settings) = session_settings(&turn_model) {
            command.args(["--settings", &settings.to_string()]);
        }
        if let Some(instructions) = &request.instructions {
            command.args(["--append-system-prompt", instructions]);
        }
        if let Some(session_id) = &request.resume_session_id {
            command.args(["--resume", session_id]);
            let (rules, directories) = self.grants.arguments(session_id);
            for rule in rules {
                command.args(["--allowedTools", &rule]);
            }
            for directory in directories {
                command.args(["--add-dir", &directory]);
            }
        }
        let message = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": claude_prompt::message_content(&request.prompt, &request.attachments),
            },
        });
        // Over stdin rather than argv, which keeps the token out of the process list. The CLI
        // connects before it reads the message, so the first turn has the tools.
        let input = match &request.mcp {
            Some(server) => format!("{}\n{message}", mcp_servers(server)),
            None => message.to_string(),
        };
        let requests = Requests::default();
        let live_output = LiveOutput::default();
        let queue = Queue::default();
        let mut parser = ClaudeParser {
            access,
            requests: requests.clone(),
            live_output: live_output.clone(),
            queue: queue.clone(),
            ..ClaudeParser::new(manifest().context_window(&turn_model.slug))
        };
        let process = spawn_turn(command, input.to_string(), sink, move |line, stdin| {
            let outputs = parser.parse_line(line);
            for reply in parser.take_replies() {
                stdin.write_line(&reply);
            }
            outputs
        })?;
        live_output.poll(process.stdin().clone());
        Ok(Box::new(ClaudeTurn {
            process,
            requests,
            grants: self.grants.clone(),
            access,
            live_output,
            queue,
        }))
    }
}

/// How long a turn asked to stop may take to wind down before its process is killed.
const INTERRUPT_GRACE: Duration = Duration::from_secs(5);

/// Messages sent to a running turn, and whether it may still take them.
#[derive(Debug, Default)]
struct QueueState {
    /// Sent, but not yet taken in by the agent, by message id.
    waiting: HashSet<String>,
    /// The turn reported its end: its stdin closes, so nothing more can join.
    closing: bool,
    /// The user asked the turn to stop.
    interrupted: bool,
    /// Commands a message moved to the background, by task id: the turn waits for them, as
    /// it would have had they stayed in the foreground.
    moved: HashSet<String>,
}

#[derive(Debug, Clone, Default)]
struct Queue(Arc<Mutex<QueueState>>);

impl Queue {
    fn state(&self) -> MutexGuard<'_, QueueState> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

struct ClaudeTurn {
    process: ProcessTurn,
    requests: Requests,
    grants: Grants,
    access: Access,
    live_output: LiveOutput,
    queue: Queue,
}

impl RunningTurn for ClaudeTurn {
    /// Asks Claude Code to stop, which lets it record the stop in its session; kills it when it
    /// takes too long.
    fn interrupt(&self) {
        let request = json!({
            "type": "control_request",
            "request_id": "hoshi-interrupt",
            "request": { "subtype": "interrupt" },
        });
        self.queue.state().interrupted = true;
        let stdin = self.process.stdin().clone();
        if !stdin.write_line(&request.to_string()) {
            self.process.interrupt();
            return;
        }
        let process = self.process.clone();
        thread::spawn(move || {
            thread::sleep(INTERRUPT_GRACE);
            if stdin.is_open() {
                process.interrupt();
            }
        });
    }

    /// The agent reads the message at once: what runs in the foreground moves to the
    /// background first and keeps running, rather than being cut by the message.
    fn steer(&self, message_id: &str, prompt: &str, attachments: &[TurnAttachment]) -> bool {
        let mut queue = self.queue.state();
        if queue.closing || queue.interrupted {
            return false;
        }
        let stdin = self.process.stdin();
        if self.live_output.anything_running() {
            let move_aside = json!({
                "type": "control_request",
                "request_id": format!("hoshi-background:{message_id}"),
                "request": { "subtype": "background_tasks" },
            });
            if stdin.write_line(&move_aside.to_string()) {
                queue.moved.extend(self.live_output.commands());
            }
        }
        let message = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": claude_prompt::message_content(prompt, attachments),
            },
            "parent_tool_use_id": null,
            "uuid": message_id,
            "priority": "now",
        });
        let sent = stdin.write_line(&message.to_string());
        if sent {
            queue.waiting.insert(message_id.to_owned());
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
        let line = claude_requests::reply(request_id, &pending, response, self.access, session);
        self.process.stdin().write_line(&line);
    }

    fn stop_subagent(&self, tool_use_id: &str) {
        let Some(task_id) = self.live_output.agent_task(tool_use_id) else {
            return;
        };
        let request = json!({
            "type": "control_request",
            "request_id": format!("hoshi-stop:{task_id}"),
            "request": { "subtype": "stop_task", "task_id": task_id },
        });
        self.process.stdin().write_line(&request.to_string());
    }
}

fn mcp_servers(server: &McpServer) -> Value {
    json!({
        "type": "control_request",
        "request_id": "hoshi-mcp",
        "request": {
            "subtype": "mcp_set_servers",
            "servers": {
                MCP_SERVER_NAME: {
                    "type": "http",
                    "url": server.url,
                    "headers": { "Authorization": server.authorization },
                },
            },
        },
    })
}

/// Ultracode runs through the `Workflow` tool, which `-p` would refuse
/// without an allow rule since it never prompts.
fn session_settings(model: &TurnModel) -> Option<Value> {
    if !model.fast_mode && !model.ultracode {
        return None;
    }
    let mut settings = json!({});
    if model.fast_mode {
        settings["fastMode"] = json!(true);
    }
    if model.ultracode {
        settings["ultracode"] = json!(true);
        settings["permissions"] = json!({ "allow": ["Workflow"] });
    }
    Some(settings)
}

/// Translates Claude Code's `stream-json` output, one line at a time.
#[derive(Default)]
pub struct ClaudeParser {
    /// Content blocks seen so far per API message, to number assistant items
    /// the same way the stream numbers its deltas.
    blocks_seen: HashMap<String, usize>,
    /// The window of the turn's model, until the turn's result reports the real one.
    context_window: Option<u64>,
    /// The model of the agent's latest call, whose window the result reports.
    model: Option<String>,
    used_tokens: Option<u64>,
    /// Calls of the tools that move the session, by id, to read where their results lead.
    moves: HashMap<String, WorktreeMove>,
    access: Access,
    requests: Requests,
    /// Lines to write back to the agent, such as answers it does not need the user for.
    replies: Vec<String>,
    session_id: Option<String>,
    /// Calls that change the task list Claude Code keeps on disk, by id.
    task_calls: HashSet<String>,
    live_output: LiveOutput,
    queue: Queue,
    /// Subagents running in the background, by task id. A turn that ends while they run
    /// is not over: Claude Code goes on once they finish, in a turn of its own.
    background_agents: HashSet<String>,
    /// Every task that ran in the background, by task id.
    ever_background: HashSet<String>,
    /// Background tasks that ended, by run id, until Claude Code's turn about each ends:
    /// it answers each one in a run of its own.
    pending_wakes: HashSet<String>,
    /// Calls that started a subagent in the background, whose result comes with the
    /// subagent's end rather than at once.
    async_agents: HashSet<String>,
    /// The runs of the turn that ended so far.
    runs: u32,
    /// A run ended and no other started yet.
    between_runs: bool,
    /// Background tasks that ended while a run went on, each told as the run that answers
    /// it starts.
    ended_tasks: VecDeque<ProviderOutput>,
}

#[derive(Clone, Copy)]
enum WorktreeMove {
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

    /// What to write back to the agent after the last lines, oldest first.
    pub fn take_replies(&mut self) -> Vec<String> {
        std::mem::take(&mut self.replies)
    }

    pub fn parse_line(&mut self, line: &str) -> Vec<ProviderOutput> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        match value["type"].as_str() {
            Some("control_request") => {
                let (outputs, reply) =
                    claude_requests::receive(&value, self.access, &self.requests);
                self.replies.extend(reply);
                return outputs;
            }
            Some("control_cancel_request") => {
                return claude_requests::withdraw(&value, &self.requests);
            }
            Some("control_response") => {
                return self.live_output.answer(&value).into_iter().collect();
            }
            Some("system") => match value["subtype"].as_str() {
                Some("task_started") => self.live_output.started(&value),
                Some("task_notification") => {
                    self.live_output.ended(&value);
                    let task = value["task_id"].as_str().unwrap_or_default();
                    let mut outputs = self.background_agent_ended(&value);
                    if self.ever_background.contains(task)
                        && value["status"] != "stopped"
                        && let Some(run) = value["run_id"].as_str()
                    {
                        self.pending_wakes.insert(run.to_owned());
                        let ended = ProviderOutput::BackgroundTaskEnded {
                            task_id: task.to_owned(),
                            tool_use_id: value["tool_use_id"].as_str().map(str::to_owned),
                            failed: value["status"] == "failed",
                        };
                        if std::mem::take(&mut self.between_runs) {
                            outputs.push(ended);
                        } else {
                            self.ended_tasks.push_back(ended);
                        }
                    }
                    return outputs;
                }
                Some("background_tasks_changed") => {
                    let moved = self.queue.state().moved.clone();
                    self.background_agents = value["tasks"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|task| {
                            let id = task["task_id"].as_str()?;
                            (task["task_type"] == "local_agent" || moved.contains(id))
                                .then(|| id.to_owned())
                        })
                        .collect();
                    self.ever_background
                        .extend(self.background_agents.iter().cloned());
                    return Vec::new();
                }
                Some("task_progress") => {
                    return self.subagent_progress(&value).into_iter().collect();
                }
                _ => {}
            },
            _ => {}
        }
        if let Some(parent) = value["parent_tool_use_id"].as_str() {
            return self.parse_subagent(parent, &value);
        }
        match value["type"].as_str() {
            Some("system") if value["subtype"] == "init" => {
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
                let cwd = value["cwd"]
                    .as_str()
                    .map(|path| ProviderOutput::WorkingDirectory {
                        path: path.to_owned(),
                    });
                session.into_iter().chain(cwd).collect()
            }
            Some("stream_event") => self.parse_stream_event(&value),
            Some("assistant") => self.parse_assistant(&value["message"]),
            Some("user") => {
                let interrupted = self.queue.state().interrupted;
                let results = parse_tool_results(&value, interrupted);
                let mut outputs = self.hold_async_agents(&value, results);
                outputs.extend(self.worktree_move(&value));
                outputs.extend(self.changed_task_list(&value["message"]));
                outputs
            }
            Some("rate_limit_event") => claude_usage::rate_limit_event(&value["rate_limit_info"])
                .map(ProviderOutput::UsageLimits)
                .into_iter()
                .collect(),
            Some("result") => {
                let mut outputs: Vec<_> = self.reported_window(&value).into_iter().collect();
                if value["origin"]["kind"] == "task-notification"
                    && let Some(run) = value["origin"]["runId"].as_str()
                {
                    self.pending_wakes.remove(run);
                }
                let mut queue = self.queue.state();
                // A message not yet taken in, or a subagent still working, gets another run.
                let over = queue.interrupted
                    || (queue.waiting.is_empty()
                        && self.background_agents.is_empty()
                        && self.pending_wakes.is_empty());
                self.runs += 1;
                if !over {
                    outputs.push(ProviderOutput::RunEnded { run: self.runs });
                    match self.ended_tasks.pop_front() {
                        Some(ended) => outputs.push(ended),
                        None => self.between_runs = true,
                    }
                }
                if over {
                    queue.closing = true;
                    outputs.push(ProviderOutput::TurnCompleted {
                        outcome: if queue.interrupted {
                            TurnOutcome::Interrupted
                        } else {
                            parse_result(&value)
                        },
                    });
                }
                outputs
            }
            Some("command_lifecycle") => {
                if matches!(
                    value["state"].as_str(),
                    Some("started" | "completed" | "cancelled")
                ) && let Some(message_id) = value["command_uuid"].as_str()
                {
                    self.queue.state().waiting.remove(message_id);
                    if value["state"] == "started" {
                        self.between_runs = false;
                    }
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn parse_stream_event(&mut self, value: &Value) -> Vec<ProviderOutput> {
        let event = &value["event"];
        match event["type"].as_str() {
            Some("message_start") => {
                let message = &event["message"];
                if let Some(model) = message["model"].as_str() {
                    self.model = Some(model.to_owned());
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
        match (
            delta["type"].as_str(),
            delta["text"].as_str(),
            delta["thinking"].as_str(),
        ) {
            (Some("text_delta"), Some(text), _) => vec![ProviderOutput::TextDelta {
                item_id,
                delta: text.to_owned(),
            }],
            (Some("thinking_delta"), _, Some(thinking)) => vec![ProviderOutput::ReasoningDelta {
                item_id,
                delta: thinking.to_owned(),
            }],
            _ => Vec::new(),
        }
    }

    fn parse_assistant(&mut self, message: &Value) -> Vec<ProviderOutput> {
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
                    let todos = (name == "TodoWrite")
                        .then(|| claude_tools::todo_list(&block["input"]))
                        .flatten();
                    outputs.push(ProviderOutput::ToolCallStarted {
                        item_id: block["id"].as_str().unwrap_or(&item_id).to_owned(),
                        name: name.to_owned(),
                        input: truncate(&block["input"].to_string(), TOOL_INPUT_MAX_CHARS),
                    });
                    outputs.extend(todos.map(|tasks| ProviderOutput::TaskList { tasks }));
                }
                _ => {}
            }
        }
        outputs
    }
}

impl ClaudeParser {
    /// A call that started a subagent in the background answers only that it started; the
    /// call stays open until the subagent ends.
    fn hold_async_agents(
        &mut self,
        frame: &Value,
        outputs: Vec<ProviderOutput>,
    ) -> Vec<ProviderOutput> {
        let result = &frame["tool_use_result"];
        if result["isAsync"] != true && result["status"] != "async_launched" {
            return outputs;
        }
        outputs
            .into_iter()
            .filter(|output| match output {
                ProviderOutput::ToolCallCompleted { item_id, .. } => {
                    self.async_agents.insert(item_id.clone());
                    false
                }
                _ => true,
            })
            .collect()
    }

    /// The end of a subagent started in the background closes the call that started it.
    fn background_agent_ended(&mut self, value: &Value) -> Vec<ProviderOutput> {
        let Some(item_id) = value["tool_use_id"].as_str() else {
            return Vec::new();
        };
        if !self.async_agents.remove(item_id) {
            return Vec::new();
        }
        let status = value["status"].as_str().unwrap_or_default();
        let mut outputs = vec![ProviderOutput::ToolCallCompleted {
            item_id: item_id.to_owned(),
            output: truncate(
                value["summary"].as_str().unwrap_or_default(),
                TOOL_OUTPUT_MAX_CHARS,
            ),
            is_error: status == "failed",
            detail: None,
        }];
        if status == "stopped" {
            outputs.push(ProviderOutput::ToolCallStopped {
                item_id: item_id.to_owned(),
            });
        }
        outputs
    }

    /// A subagent's own messages and tool calls, which go to its transcript. Its prompt, sent
    /// as a user message, is already the input of the call that started it.
    fn parse_subagent(&mut self, parent: &str, value: &Value) -> Vec<ProviderOutput> {
        let outputs = match value["type"].as_str() {
            Some("assistant") => self.parse_assistant(&value["message"]),
            Some("user") => parse_tool_results(value, self.queue.state().interrupted),
            _ => Vec::new(),
        };
        outputs
            .into_iter()
            .filter(|output| {
                matches!(
                    output,
                    ProviderOutput::AssistantMessage { .. }
                        | ProviderOutput::Reasoning { .. }
                        | ProviderOutput::ToolCallStarted { .. }
                        | ProviderOutput::ToolCallCompleted { .. }
                        | ProviderOutput::ToolCallStopped { .. }
                )
            })
            .map(|output| ProviderOutput::Subagent {
                tool_use_id: parent.to_owned(),
                output: Box::new(output),
            })
            .collect()
    }

    /// What a running subagent is doing, as a line that replaces the previous one.
    fn subagent_progress(&self, value: &Value) -> Option<ProviderOutput> {
        let task_id = value["task_id"].as_str()?;
        if self.live_output.follows(task_id) {
            return None;
        }
        Some(ProviderOutput::ToolOutput {
            item_id: value["tool_use_id"].as_str()?.to_owned(),
            output: value["description"].as_str()?.to_owned(),
        })
    }

    /// The task list as Claude Code saved it, once a call that changes it succeeded.
    fn changed_task_list(&mut self, message: &Value) -> Option<ProviderOutput> {
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
        let dir = claude_install::config_dir()?.join("tasks").join(
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
        let tasks = claude_tools::saved_task_list(&dir)?;
        Some(ProviderOutput::TaskList { tasks })
    }

    /// Where a successful `EnterWorktree` or `ExitWorktree` left the session, from the
    /// structured result the CLI sends along with the tool's text.
    fn worktree_move(&mut self, value: &Value) -> Option<ProviderOutput> {
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
        Some(ProviderOutput::WorkingDirectory {
            path: path.to_owned(),
        })
    }

    /// Everything the call sent and wrote stays in the context, cached or not.
    fn context_usage(&mut self, usage: &Value) -> Option<ProviderOutput> {
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
        }))
    }

    /// The result names the window Claude Code measured the model against.
    fn reported_window(&mut self, result: &Value) -> Option<ProviderOutput> {
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
        }))
    }
}

/// `structured` is the CLI's own reading of the result, sent along when the message holds a
/// single one.
/// What Claude Code answers in place of a call the user stopped.
const STOPPED_MARKER: &str = "[Request interrupted by user";

/// The results a `user` frame carries. A frame with a single result also carries the CLI's
/// own reading of it, in `tool_use_result`. Once the user asked the turn to stop, a call that
/// fails was stopped.
fn parse_tool_results(frame: &Value, interrupted: bool) -> Vec<ProviderOutput> {
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
        claude_tools::detail(structured)
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
            let is_error = block["is_error"].as_bool().unwrap_or(false);
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

fn parse_result(value: &Value) -> TurnOutcome {
    if value["is_error"].as_bool().unwrap_or(false) {
        let subtype = value["subtype"].as_str().unwrap_or("error");
        let result = value["result"].as_str().unwrap_or_default();
        TurnOutcome::Failed {
            reason: TurnFailure::ProviderError {
                detail: format!("{subtype}: {result}"),
            },
        }
    } else {
        TurnOutcome::Completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(fast_mode: bool, ultracode: bool) -> TurnModel {
        TurnModel {
            slug: "claude-opus-5-5".into(),
            effort: None,
            fast_mode,
            ultracode,
        }
    }

    #[test]
    fn plan_mode_wins_over_the_access_mode() {
        let mut access = Access {
            mode: AccessMode::AcceptEdits,
            plan_mode: false,
        };
        assert_eq!(access.permission_mode(), "acceptEdits");
        access.mode = AccessMode::Ask;
        assert_eq!(access.permission_mode(), "default");
        access.mode = AccessMode::FullAccess;
        assert_eq!(access.permission_mode(), "bypassPermissions");
        access.plan_mode = true;
        assert_eq!(access.permission_mode(), "plan");
    }

    #[test]
    fn a_stopped_call_completes_and_then_stops() {
        let mut parser = ClaudeParser::new(None);
        let frame = json!({ "type": "user", "parent_tool_use_id": null, "message": { "role": "user",
            "content": [{ "type": "tool_result", "tool_use_id": "toolu_1", "is_error": true,
                          "content": "[Request interrupted by user for tool use]" }] } });

        let outputs = parser.parse_line(&frame.to_string());

        assert!(matches!(
            outputs.as_slice(),
            [
                ProviderOutput::ToolCallCompleted { is_error: true, .. },
                ProviderOutput::ToolCallStopped { item_id }
            ] if item_id == "toolu_1"
        ));
    }

    #[test]
    fn a_call_that_fails_once_the_turn_was_asked_to_stop_was_stopped() {
        let mut parser = ClaudeParser::new(None);
        parser.queue.state().interrupted = true;
        let frame = json!({ "type": "user", "parent_tool_use_id": null, "message": { "role": "user",
            "content": [{ "type": "tool_result", "tool_use_id": "toolu_1", "is_error": true,
                          "content": "The user doesn't want to proceed with this tool use." }] } });

        assert!(matches!(
            parser.parse_line(&frame.to_string()).last(),
            Some(ProviderOutput::ToolCallStopped { .. })
        ));
    }

    #[test]
    fn a_call_the_cli_cancelled_stops_too() {
        let mut parser = ClaudeParser::new(None);
        let frame = json!({ "type": "user", "parent_tool_use_id": "toolu_agent",
            "tool_result_meta": [{ "id": "toolu_1", "non_execution_kind": "cancelled" }],
            "message": { "role": "user",
            "content": [{ "type": "tool_result", "tool_use_id": "toolu_1", "content": "" }] } });

        let outputs = parser.parse_line(&frame.to_string());

        assert!(matches!(
            outputs.last(),
            Some(ProviderOutput::Subagent { output, .. })
                if matches!(**output, ProviderOutput::ToolCallStopped { .. })
        ));
    }

    #[test]
    fn a_turn_waits_for_its_background_subagents() {
        let mut parser = ClaudeParser::new(None);
        let lines = [
            json!({ "type": "system", "subtype": "background_tasks_changed",
                "tasks": [{ "task_id": "a1", "task_type": "local_agent" }] }),
            json!({ "type": "user", "parent_tool_use_id": null,
                "tool_use_result": { "isAsync": true, "status": "async_launched" },
                "message": { "role": "user", "content": [{ "type": "tool_result",
                    "tool_use_id": "toolu_agent", "content": "Async agent launched successfully." }] } }),
            json!({ "type": "result", "subtype": "success", "is_error": false, "result": "launched" }),
        ];
        let early: Vec<_> = lines
            .iter()
            .flat_map(|line| parser.parse_line(&line.to_string()))
            .collect();
        assert_eq!(early, [ProviderOutput::RunEnded { run: 1 }]);

        let ended = parser.parse_line(
            &json!({ "type": "system", "subtype": "task_notification", "task_id": "a1",
                "tool_use_id": "toolu_agent", "run_id": "run-a1", "status": "completed",
                "summary": "tick 6" })
            .to_string(),
        );
        assert_eq!(
            ended,
            vec![
                ProviderOutput::ToolCallCompleted {
                    item_id: "toolu_agent".into(),
                    output: "tick 6".into(),
                    is_error: false,
                    detail: None,
                },
                ProviderOutput::BackgroundTaskEnded {
                    task_id: "a1".into(),
                    tool_use_id: Some("toolu_agent".into()),
                    failed: false,
                }
            ]
        );
        parser.parse_line(
            &json!({ "type": "system", "subtype": "background_tasks_changed", "tasks": [] })
                .to_string(),
        );
        let continued = parser.parse_line(
            &json!({ "type": "result", "subtype": "success", "is_error": false, "result": "done",
                "origin": { "kind": "task-notification", "runId": "run-a1" } })
            .to_string(),
        );
        assert!(matches!(
            continued.as_slice(),
            [ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Completed
            }]
        ));
    }

    #[test]
    fn a_turn_goes_on_while_a_message_sent_to_it_waits() {
        let mut parser = ClaudeParser::new(None);
        parser.queue.state().waiting.insert("m2".into());
        let result =
            json!({ "type": "result", "subtype": "success", "is_error": false }).to_string();

        assert!(matches!(
            parser.parse_line(&result).as_slice(),
            [ProviderOutput::RunEnded { .. }]
        ));
        parser.parse_line(
            &json!({ "type": "command_lifecycle", "command_uuid": "m2", "state": "started" })
                .to_string(),
        );
        assert!(matches!(
            parser.parse_line(&result).as_slice(),
            [ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Completed
            }]
        ));
        assert!(parser.queue.state().closing);
    }

    #[test]
    fn a_command_moved_aside_by_a_message_keeps_the_turn_going() {
        let mut parser = ClaudeParser::new(None);
        parser.queue.state().moved.insert("b1".into());
        parser.parse_line(
            &json!({ "type": "system", "subtype": "background_tasks_changed",
                "tasks": [{ "task_id": "b1", "task_type": "local_bash" },
                          { "task_id": "b2", "task_type": "local_bash" }] })
            .to_string(),
        );
        let result =
            json!({ "type": "result", "subtype": "success", "is_error": false }).to_string();
        assert!(matches!(
            parser.parse_line(&result).as_slice(),
            [ProviderOutput::RunEnded { .. }]
        ));

        parser.parse_line(
            &json!({ "type": "system", "subtype": "background_tasks_changed",
                "tasks": [{ "task_id": "b2", "task_type": "local_bash" }] })
            .to_string(),
        );
        assert!(matches!(
            parser.parse_line(&result).as_slice(),
            [ProviderOutput::TurnCompleted { .. }]
        ));
    }

    #[test]
    fn a_turn_waits_for_claude_to_answer_each_background_task_that_ended() {
        fn parser_line(parser: &mut ClaudeParser, value: Value) -> Vec<ProviderOutput> {
            parser.parse_line(&value.to_string())
        }
        let mut parser = ClaudeParser::new(None);
        for value in [
            json!({ "type": "system", "subtype": "background_tasks_changed", "tasks": [
                { "task_id": "a", "task_type": "local_agent" },
                { "task_id": "b", "task_type": "local_agent" }] }),
            json!({ "type": "system", "subtype": "task_notification", "task_id": "a",
                "run_id": "run-a", "status": "completed" }),
            json!({ "type": "system", "subtype": "task_notification", "task_id": "b",
                "run_id": "run-b", "status": "completed" }),
            json!({ "type": "system", "subtype": "background_tasks_changed", "tasks": [] }),
        ] {
            parser_line(&mut parser, value);
        }
        let result = |origin: Value| json!({ "type": "result", "subtype": "success", "is_error": false, "origin": origin });

        let ended = |task: &str| ProviderOutput::BackgroundTaskEnded {
            task_id: task.to_owned(),
            tool_use_id: None,
            failed: false,
        };

        assert_eq!(
            parser_line(&mut parser, result(Value::Null)),
            [ProviderOutput::RunEnded { run: 1 }, ended("a")]
        );
        let first = result(json!({ "kind": "task-notification", "runId": "run-a" }));
        assert_eq!(
            parser_line(&mut parser, first),
            [ProviderOutput::RunEnded { run: 2 }, ended("b")]
        );
        let second = result(json!({ "kind": "task-notification", "runId": "run-b" }));
        assert!(matches!(
            parser_line(&mut parser, second).as_slice(),
            [ProviderOutput::TurnCompleted { .. }]
        ));
    }

    #[test]
    fn an_interrupted_turn_ends_at_its_result_as_interrupted() {
        let mut parser = ClaudeParser::new(None);
        parser.queue.state().interrupted = true;
        parser.queue.state().waiting.insert("m2".into());
        let result = json!({ "type": "result", "subtype": "error_during_execution",
            "is_error": true })
        .to_string();

        assert!(matches!(
            parser.parse_line(&result).as_slice(),
            [ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Interrupted
            }]
        ));
    }

    #[test]
    fn approvals_are_answered_on_the_same_turn() {
        let mut parser = ClaudeParser::new(None);
        let request = json!({ "type": "control_request", "request_id": "r1", "request": {
            "subtype": "can_use_tool", "tool_name": "Bash", "input": { "command": "ls" },
            "tool_use_id": "toolu_1" } });
        let outputs = parser.parse_line(&request.to_string());

        assert!(matches!(
            outputs.as_slice(),
            [ProviderOutput::RequestOpened { request_id, .. }] if request_id == "r1"
        ));
        assert!(parser.take_replies().is_empty());
        assert!(parser.requests.take("r1").is_some());
    }

    #[test]
    fn no_settings_are_passed_when_both_modes_are_off() {
        assert_eq!(session_settings(&model(false, false)), None);
    }

    #[test]
    fn fast_mode_and_ultracode_go_through_settings() {
        assert_eq!(
            session_settings(&model(true, false)),
            Some(json!({ "fastMode": true }))
        );
        assert_eq!(
            session_settings(&model(false, true)),
            Some(json!({ "ultracode": true, "permissions": { "allow": ["Workflow"] } }))
        );
    }
}
