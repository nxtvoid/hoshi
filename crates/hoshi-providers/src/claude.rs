use crate::claude_account;
use crate::claude_commands;
use crate::claude_install;
use crate::claude_models::{TurnModel, Version, manifest, parse_version};
use crate::claude_output::LiveOutput;
use crate::claude_prompt;
use crate::claude_requests::{self, Access, Grants, Requests};
use crate::claude_titles;
use crate::claude_tools;
use crate::claude_usage;
use crate::claude_writing;
use crate::process::{ProcessTurn, hide_console, spawn_turn};
use crate::{
    CommitMessage, CommitMessageRequest, GeneratedTitle, Installation, MCP_SERVER_NAME, McpServer,
    OutputSink, ProviderAdapter, PullRequestText, PullRequestTextRequest, RunningTurn,
    TitleRequest, TurnAttachment, TurnRequest, truncate,
};
use hoshi_core::{
    AccessMode, BackgroundTask, ContextUsage, ProviderAccountStatus, ProviderModels,
    ProviderOutput, RequestResponse, SlashCommand, TurnFailure, TurnOutcome, UsageLimits,
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

    fn account_status(&self) -> ProviderAccountStatus {
        let status = claude_account::probe(self.binary());
        self.forget_version();
        status
    }

    fn slash_commands(&self, cwd: &Path) -> Option<Vec<SlashCommand>> {
        claude_commands::probe(self.binary(), cwd)
    }

    fn generate_title(&self, request: &TitleRequest) -> Option<GeneratedTitle> {
        claude_titles::generate(self.binary(), request)
    }

    fn write_commit_message(&self, request: &CommitMessageRequest) -> Option<CommitMessage> {
        claude_writing::write_commit(self.binary(), request)
    }

    fn write_pull_request(&self, request: &PullRequestTextRequest) -> Option<PullRequestText> {
        claude_writing::write_pull_request(self.binary(), request)
    }

    fn start_turn(
        &self,
        request: TurnRequest,
        sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
        let turn_model = turn_model(&request, self.model.as_deref());
        let access = access(&request);
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
            if let Some(message) = &request.resume_session_at {
                command.args(["--resume-session-at", message]);
            }
            let (rules, directories) = self.grants.arguments(session_id);
            for rule in rules {
                command.args(["--allowedTools", &rule]);
            }
            for directory in directories {
                command.args(["--add-dir", &directory]);
            }
        }
        let message = user_message(&request.prompt, &request.attachments);
        // Over stdin rather than argv, which keeps the token out of the process list. The CLI
        // connects before it reads the message, so the first turn has the tools.
        let input = match &request.mcp {
            Some(server) => format!("{}\n{message}", mcp_servers(server)),
            None => message,
        };
        let requests = Requests::default();
        let live_output = LiveOutput::default();
        let queue = Queue::default();
        queue.state().access = access;
        let mut parser = ClaudeParser {
            requests: requests.clone(),
            live_output: live_output.clone(),
            queue: queue.clone(),
            ..ClaudeParser::new(manifest().context_window(&turn_model.slug))
        };
        let (ending, in_turn) = (queue.clone(), queue.clone());
        let process = spawn_turn(
            command,
            input,
            sink,
            move |line, stdin| {
                let outputs = parser.parse_line(line);
                for reply in parser.take_replies() {
                    stdin.write_line(&reply);
                }
                if ending.state().closing {
                    stdin.close();
                }
                outputs
            },
            move || in_turn.state().turn_open,
        )?;
        live_output.poll(process.stdin().clone());
        Ok(Box::new(ClaudeTurn {
            process,
            requests,
            grants: self.grants.clone(),
            live_output,
            queue,
            launch: Launch {
                cwd: request.cwd,
                model: turn_model,
                instructions: request.instructions,
                mcp: request.mcp,
                full_access: request.access_mode == AccessMode::FullAccess,
            },
            default_model: self.model.clone(),
        }))
    }
}

fn turn_model(request: &TurnRequest, default_model: Option<&str>) -> TurnModel {
    manifest().resolve(
        request.model.as_deref().or(default_model),
        request.effort,
        request.fast_mode,
        request.ultracode,
    )
}

fn access(request: &TurnRequest) -> Access {
    Access {
        mode: request.access_mode,
        plan_mode: request.plan_mode,
    }
}

fn user_message(prompt: &str, attachments: &[TurnAttachment]) -> String {
    json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": claude_prompt::message_content(prompt, attachments),
        },
    })
    .to_string()
}

const INTERRUPT_REQUEST_ID: &str = "hoshi-interrupt";
const MODE_REQUEST_ID: &str = "hoshi-mode";

/// How long a turn asked to stop may take to wind down before its process is killed.
const INTERRUPT_GRACE: Duration = Duration::from_secs(5);

/// The process's turn, the messages sent to it, and the background commands it may wait for.
#[derive(Debug)]
struct QueueState {
    /// Sent, but not yet taken in by the agent, by message id.
    waiting: HashSet<String>,
    /// The process is to end: its stdin closes, so nothing more can join.
    closing: bool,
    /// The user asked the turn to stop.
    interrupted: bool,
    /// A turn runs. Between turns, the process only keeps the commands the last one left
    /// running in the background.
    turn_open: bool,
    /// The background commands running, by task id.
    commands: HashSet<String>,
    /// The commands among them the agent said its turn waits for.
    awaited: HashSet<String>,
    /// What the agent may do, as the latest turn asked.
    access: Access,
    /// Where the session works, as last reported.
    cwd: Option<String>,
}

impl Default for QueueState {
    fn default() -> Self {
        Self {
            waiting: HashSet::new(),
            closing: false,
            interrupted: false,
            turn_open: true,
            commands: HashSet::new(),
            awaited: HashSet::new(),
            access: Access::default(),
            cwd: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Queue(Arc<Mutex<QueueState>>);

impl Queue {
    fn state(&self) -> MutexGuard<'_, QueueState> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Marks the running commands among `task_ids` as awaited, or no longer; returns them.
    fn await_tasks(&self, task_ids: &[String], awaited: bool) -> Vec<String> {
        let mut state = self.state();
        let running: Vec<String> = task_ids
            .iter()
            .filter(|task_id| state.commands.contains(*task_id))
            .cloned()
            .collect();
        for task_id in &running {
            if awaited {
                state.awaited.insert(task_id.clone());
            } else {
                state.awaited.remove(task_id);
            }
        }
        running
    }
}

/// What a process was started with, which a later turn must share to run in it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Launch {
    cwd: String,
    model: TurnModel,
    instructions: Option<String>,
    mcp: Option<McpServer>,
    /// Allowed to bypass permissions, which a later turn can only switch to if so.
    full_access: bool,
}

/// The lines that start `request` as the next turn of a process started as `launch`, whose
/// session is `session`, and open that turn. `None` when the request asks for what the
/// process can't change: another model, folder or point of the conversation.
fn next_turn(
    launch: &Launch,
    default_model: Option<&str>,
    session: Option<&str>,
    queue: &mut QueueState,
    request: &TurnRequest,
) -> Option<Vec<String>> {
    let access = access(request);
    let here = same_folder(&request.cwd, &launch.cwd)
        || queue
            .cwd
            .as_deref()
            .is_some_and(|cwd| same_folder(&request.cwd, cwd));
    let fits = here
        && session.is_some()
        && request.resume_session_id.as_deref() == session
        && request.resume_session_at.is_none()
        && turn_model(request, default_model) == launch.model
        && request.instructions == launch.instructions
        && request.mcp == launch.mcp
        && (access.mode != AccessMode::FullAccess || launch.full_access);
    if !fits || queue.closing || queue.turn_open {
        return None;
    }
    let mut lines = Vec::with_capacity(2);
    if access != queue.access {
        let mode = json!({
            "type": "control_request",
            "request_id": MODE_REQUEST_ID,
            "request": { "subtype": "set_permission_mode", "mode": access.permission_mode() },
        });
        lines.push(mode.to_string());
        queue.access = access;
    }
    lines.push(user_message(&request.prompt, &request.attachments));
    queue.turn_open = true;
    queue.interrupted = false;
    Some(lines)
}

fn same_folder(one: &str, other: &str) -> bool {
    let normal = |path: &str| {
        let path = path.replace('\\', "/");
        let path = path.trim_end_matches('/');
        if cfg!(windows) {
            path.to_lowercase()
        } else {
            path.to_owned()
        }
    };
    normal(one) == normal(other)
}

struct ClaudeTurn {
    process: ProcessTurn,
    requests: Requests,
    grants: Grants,
    live_output: LiveOutput,
    queue: Queue,
    launch: Launch,
    /// The adapter's model, for requests that name none.
    default_model: Option<String>,
}

impl RunningTurn for ClaudeTurn {
    /// Asks Claude Code to stop, which lets it record the stop in its session; kills it when it
    /// takes too long.
    fn interrupt(&self) {
        let request = json!({
            "type": "control_request",
            "request_id": INTERRUPT_REQUEST_ID,
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
        if queue.closing || queue.interrupted || !queue.turn_open {
            return false;
        }
        let stdin = self.process.stdin();
        if self.live_output.anything_running() {
            let move_aside = json!({
                "type": "control_request",
                "request_id": format!("hoshi-background:{message_id}"),
                "request": { "subtype": "background_tasks" },
            });
            stdin.write_line(&move_aside.to_string());
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
        let access = self.queue.state().access;
        let line = claude_requests::reply(request_id, &pending, response, access, session);
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

    fn continue_with(&self, request: &TurnRequest) -> bool {
        let session = self.requests.session();
        let mut queue = self.queue.state();
        let Some(lines) = next_turn(
            &self.launch,
            self.default_model.as_deref(),
            session.as_deref(),
            &mut queue,
            request,
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

    fn close(&self) {
        self.queue.state().closing = true;
        self.process.interrupt();
    }
}

impl ClaudeTurn {
    fn stop_task(&self, task_id: &str) {
        let request = json!({
            "type": "control_request",
            "request_id": format!("hoshi-stop:{task_id}"),
            "request": { "subtype": "stop_task", "task_id": task_id },
        });
        self.process.stdin().write_line(&request.to_string());
    }
}

/// An hour for the user to answer a card, and some slack, as t3code gives its tools.
const MCP_TOOL_TIMEOUT_MS: u64 = 65 * 60 * 1000;

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
                    // A tool can wait on the user, as `request_secret` does.
                    "timeout": MCP_TOOL_TIMEOUT_MS,
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
    /// How long the provider caches the prompt, as its latest call wrote it.
    cache_ttl_secs: Option<u64>,
    /// Calls of the tools that move the session, by id, to read where their results lead.
    moves: HashMap<String, WorktreeMove>,
    /// Where the session works, as last reported.
    session_cwd: Option<String>,
    requests: Requests,
    /// Lines to write back to the agent, such as answers it does not need the user for.
    replies: Vec<String>,
    session_id: Option<String>,
    /// Calls that change the task list Claude Code keeps on disk, by id.
    task_calls: HashSet<String>,
    live_output: LiveOutput,
    queue: Queue,
    /// Subagents running in the background, by task id. A turn that ends while they run is
    /// not over: Claude Code goes on once they finish, in a run of its own.
    background_agents: HashSet<String>,
    /// Commands running in the background, as the user sees them. Only those the agent
    /// awaits hold the turn; the others keep the process once the turn completed.
    background_commands: Vec<BackgroundTask>,
    /// Commands that left the list before their end was told, which Claude Code answers
    /// once it is.
    ending: HashSet<String>,
    /// The call that started each task, by task id.
    task_calls_by_task: HashMap<String, String>,
    /// A run ended and the turn only waits on its background work: no run goes on.
    idle: bool,
    /// How the latest run ended, which a turn that ends on its own between runs reports.
    last_outcome: Option<TurnOutcome>,
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
                let access = self.queue.state().access;
                let (outputs, reply) = claude_requests::receive(&value, access, &self.requests);
                self.replies.extend(reply);
                return outputs;
            }
            Some("control_cancel_request") => {
                return claude_requests::withdraw(&value, &self.requests);
            }
            Some("control_response") => {
                if value["response"]["request_id"] == INTERRUPT_REQUEST_ID {
                    return self.end_between_runs();
                }
                return self.live_output.answer(&value).into_iter().collect();
            }
            Some("system") => match value["subtype"].as_str() {
                Some("task_started") => {
                    self.live_output.started(&value);
                    return self.background_task_started(&value).into_iter().collect();
                }
                Some("task_notification") => {
                    self.live_output.ended(&value);
                    let task = value["task_id"].as_str().unwrap_or_default();
                    let mut outputs = self.background_agent_ended(&value);
                    self.background_agents.remove(task);
                    self.ending.remove(task);
                    outputs.extend(self.background_command_ended(task));
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
        let run_going = match value["type"].as_str() {
            Some("stream_event" | "assistant" | "user") => true,
            Some("system") => value["subtype"] == "init",
            Some("command_lifecycle") => value["state"] == "started",
            _ => false,
        };
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

    /// A line of the thread's own agent, not of one of its subagents.
    fn parse_own_line(&mut self, value: &Value) -> Vec<ProviderOutput> {
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
            Some("stream_event") => self.parse_stream_event(value),
            Some("assistant") => {
                let mut outputs = self.parse_assistant(&value["message"]);
                if let Some(uuid) = value["uuid"].as_str() {
                    outputs.push(ProviderOutput::ConversationCursor {
                        cursor: uuid.to_owned(),
                    });
                }
                outputs
            }
            Some("user") => {
                let interrupted = self.queue.state().interrupted;
                let results = parse_tool_results(value, interrupted);
                let mut outputs = self.hold_async_agents(value, results);
                outputs.extend(self.worktree_move(value));
                outputs.extend(self.changed_task_list(&value["message"]));
                outputs
            }
            Some("rate_limit_event") => claude_usage::rate_limit_event(&value["rate_limit_info"])
                .map(ProviderOutput::UsageLimits)
                .into_iter()
                .collect(),
            Some("result") => {
                let mut outputs: Vec<_> = self.reported_window(value).into_iter().collect();
                if value["origin"]["kind"] == "task-notification"
                    && let Some(run) = value["origin"]["runId"].as_str()
                {
                    self.pending_wakes.remove(run);
                }
                let shared = self.queue.clone();
                let mut queue = shared.state();
                // A message not yet taken in, or background work the turn waits for, gets
                // another run.
                let over = queue.interrupted
                    || (queue.waiting.is_empty()
                        && !self.holds(&queue)
                        && self.pending_wakes.is_empty());
                self.runs += 1;
                if over {
                    let outcome = if queue.interrupted {
                        TurnOutcome::Interrupted
                    } else {
                        parse_result(value)
                    };
                    outputs.extend(self.complete(&mut queue, outcome));
                    return outputs;
                }
                self.last_outcome = Some(parse_result(value));
                self.idle = true;
                outputs.push(ProviderOutput::RunEnded { run: self.runs });
                match self.ended_tasks.pop_front() {
                    Some(ended) => outputs.push(ended),
                    None => self.between_runs = true,
                }
                outputs.extend(self.waiting_changed());
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

    fn background_task_started(&mut self, value: &Value) -> Option<ProviderOutput> {
        let task_id = value["task_id"].as_str()?;
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

    /// Claude Code lists what runs in the background whenever that changes. Subagents hold
    /// the turn; shell commands (`Bash` sent to the background, `Monitor`) only when the agent
    /// awaits them.
    fn background_tasks_changed(&mut self, value: &Value) -> Option<ProviderOutput> {
        let tasks: Vec<&Value> = value["tasks"].as_array().into_iter().flatten().collect();
        self.background_agents = tasks
            .iter()
            .filter(|task| task["task_type"] == "local_agent")
            .filter_map(|task| task["task_id"].as_str().map(str::to_owned))
            .collect();
        self.ever_background
            .extend(self.background_agents.iter().cloned());
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
        self.ever_background
            .extend(commands.iter().map(|command| command.task_id.clone()));
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

    fn background_command_ended(&mut self, task_id: &str) -> Option<ProviderOutput> {
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
    fn set_commands(&mut self, commands: Vec<BackgroundTask>) {
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

    /// Whether the turn waits for background work: its subagents, and the commands the
    /// agent awaits.
    fn holds(&self, queue: &QueueState) -> bool {
        !self.background_agents.is_empty() || !queue.awaited.is_empty()
    }

    /// Ends the turn. Its process stays for the commands still running in the background,
    /// unless the user stopped the turn.
    fn complete(&mut self, queue: &mut QueueState, outcome: TurnOutcome) -> Vec<ProviderOutput> {
        let lingers =
            !queue.interrupted && (!self.background_commands.is_empty() || !self.ending.is_empty());
        queue.turn_open = false;
        queue.closing = !lingers;
        self.runs = 0;
        self.idle = false;
        self.between_runs = false;
        self.last_outcome = None;
        let mut outputs = Vec::new();
        if lingers && !self.background_commands.is_empty() {
            outputs.push(self.background_roster());
        }
        outputs.push(ProviderOutput::TurnCompleted { outcome });
        outputs
    }

    /// A run that starts between turns is the agent going on by itself, as a command left
    /// running ended: a turn of its own, which first tells what ended.
    fn wake(&mut self) -> Vec<ProviderOutput> {
        {
            let mut queue = self.queue.state();
            if queue.turn_open || queue.closing {
                return Vec::new();
            }
            queue.turn_open = true;
        }
        let mut outputs = vec![ProviderOutput::Woke];
        outputs.extend(self.ended_tasks.pop_front());
        outputs
    }

    /// Between turns, the process ends once nothing it kept runs or waits for an answer.
    fn close_when_idle(&self) {
        let mut queue = self.queue.state();
        if !queue.turn_open
            && self.background_commands.is_empty()
            && self.ending.is_empty()
            && self.background_agents.is_empty()
            && self.pending_wakes.is_empty()
        {
            queue.closing = true;
        }
    }

    fn background_roster(&self) -> ProviderOutput {
        ProviderOutput::BackgroundTasks {
            tasks: self.background_commands.clone(),
            waiting: self.idle,
        }
    }

    /// A run started or ended while commands run in the background.
    fn waiting_changed(&self) -> Option<ProviderOutput> {
        (!self.background_commands.is_empty()).then(|| self.background_roster())
    }

    /// A turn that waits between runs ends on its own once nothing is left to wait for, or
    /// once the user asked it to stop: Claude Code answers no stopped task, and an interrupt
    /// with no run going ends none.
    fn end_between_runs(&mut self) -> Vec<ProviderOutput> {
        if !self.idle {
            return Vec::new();
        }
        let shared = self.queue.clone();
        let mut queue = shared.state();
        let waits =
            self.holds(&queue) || !self.pending_wakes.is_empty() || !queue.waiting.is_empty();
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
        self.session_cwd = Some(path.to_owned());
        self.queue.state().cwd = Some(path.to_owned());
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
            cache_ttl_secs: self.cache_ttl_secs,
            cache_expires_at: None,
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
            cache_ttl_secs: self.cache_ttl_secs,
            cache_expires_at: None,
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

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
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
    fn a_compaction_reports_its_start_its_sizes_and_its_failure() {
        let mut parser = ClaudeParser::new(Some(200_000));
        let started = json!({ "type": "system", "subtype": "status", "status": "compacting" });
        let boundary = json!({ "type": "system", "subtype": "compact_boundary",
            "compact_metadata": { "trigger": "manual", "pre_tokens": 45_831, "post_tokens": 6_534 } });
        let failed = json!({ "type": "system", "subtype": "status", "status": null,
            "compact_result": "failed", "compact_error": "Not enough messages to compact." });
        let cached = json!({ "type": "stream_event", "event": { "type": "message_start", "message": {
            "model": "claude-haiku-5-5",
            "usage": { "input_tokens": 2, "cache_creation_input_tokens": 100, "cache_read_input_tokens": 0,
                       "cache_creation": { "ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 100 },
                       "output_tokens": 1 } } } });

        assert_eq!(
            parser.parse_line(&started.to_string()),
            [ProviderOutput::CompactionStarted]
        );
        let usage = parser.parse_line(&cached.to_string());
        assert!(matches!(
            usage.as_slice(),
            [ProviderOutput::ContextUsage(ContextUsage {
                cache_ttl_secs: Some(3600),
                ..
            })]
        ));
        assert_eq!(
            parser.parse_line(&boundary.to_string()),
            [
                ProviderOutput::CompactionFinished {
                    before_tokens: 45_831,
                    after_tokens: 6_534
                },
                ProviderOutput::ContextUsage(ContextUsage {
                    used_tokens: 6_534,
                    max_tokens: Some(200_000),
                    cache_ttl_secs: Some(3600),
                    cache_expires_at: None,
                }),
            ]
        );
        assert_eq!(
            parser.parse_line(&failed.to_string()),
            [ProviderOutput::CompactionFailed {
                error: "Not enough messages to compact.".into()
            }]
        );
    }

    #[test]
    fn the_conversation_s_own_messages_are_where_a_rewind_can_resume() {
        let mut parser = ClaudeParser::new(None);
        let own = json!({ "type": "assistant", "parent_tool_use_id": null, "uuid": "u1",
            "message": { "id": "msg_1", "content": [{ "type": "text", "text": "Done." }] } });
        let subagent = json!({ "type": "assistant", "parent_tool_use_id": "toolu_agent",
            "uuid": "u2", "message": { "id": "msg_2", "content": [{ "type": "text", "text": "x" }] } });

        let outputs = parser.parse_line(&own.to_string());
        let inside = parser.parse_line(&subagent.to_string());

        assert!(matches!(
            outputs.last(),
            Some(ProviderOutput::ConversationCursor { cursor }) if cursor == "u1"
        ));
        assert!(
            !inside
                .iter()
                .any(|output| matches!(output, ProviderOutput::ConversationCursor { .. }))
        );
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

    fn feed(parser: &mut ClaudeParser, lines: &[Value]) -> Vec<ProviderOutput> {
        lines
            .iter()
            .flat_map(|line| parser.parse_line(&line.to_string()))
            .collect()
    }

    fn roster(tasks: Value) -> Value {
        json!({ "type": "system", "subtype": "background_tasks_changed", "tasks": tasks })
    }

    /// The agent starts `sleep` in the background; its run has not ended yet.
    fn sleep_in_background() -> Vec<Value> {
        vec![
            json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "id": "msg_1",
                "content": [{ "type": "tool_use", "id": "toolu_bg", "name": "Bash", "input": {
                    "command": "sleep 3 && echo done", "description": "Sleep then echo done",
                    "run_in_background": true } }] } }),
            roster(
                json!([{ "task_id": "b1", "run_id": "run-b1", "task_type": "local_bash",
                "description": "Sleep then echo done" }]),
            ),
            json!({ "type": "system", "subtype": "task_started", "task_id": "b1",
                "run_id": "run-b1", "tool_use_id": "toolu_bg",
                "description": "Sleep then echo done", "is_backgrounded": true,
                "task_type": "local_bash" }),
            json!({ "type": "user", "parent_tool_use_id": null,
                "tool_use_result": { "stdout": "", "stderr": "", "backgroundTaskId": "b1" },
                "message": { "role": "user", "content": [{ "type": "tool_result",
                    "tool_use_id": "toolu_bg",
                    "content": "Command running in background with ID: b1." }] } }),
        ]
    }

    fn run_result() -> Value {
        json!({ "type": "result", "subtype": "success", "is_error": false, "result": "started" })
    }

    /// The agent starts `sleep` in the background, says its turn waits for it, and ends its
    /// run.
    fn await_sleep(parser: &mut ClaudeParser) -> Vec<ProviderOutput> {
        let mut outputs = feed(parser, &sleep_in_background());
        assert_eq!(parser.queue.await_tasks(&["b1".into()], true), ["b1"]);
        outputs.extend(feed(parser, &[run_result()]));
        outputs
    }

    /// The agent starts `sleep` in the background and ends its turn, leaving it running.
    fn leave_sleep_running(parser: &mut ClaudeParser) -> Vec<ProviderOutput> {
        let mut lines = sleep_in_background();
        lines.push(run_result());
        feed(parser, &lines)
    }

    fn sleep_ended() -> Vec<Value> {
        vec![
            json!({ "type": "system", "subtype": "task_updated", "task_id": "b1",
                "patch": { "status": "completed" } }),
            json!({ "type": "system", "subtype": "task_notification", "task_id": "b1",
                "run_id": "run-b1", "tool_use_id": "toolu_bg", "status": "completed",
                "summary": "Background command \"Sleep then echo done\" completed (exit code 0)" }),
            roster(json!([])),
        ]
    }

    fn answer_to_sleep() -> Vec<Value> {
        vec![
            json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
            json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                "id": "msg_2", "content": [{ "type": "text", "text": "It printed done." }] } }),
            json!({ "type": "result", "subtype": "success", "is_error": false,
                "origin": { "kind": "task-notification", "runId": "run-b1" } }),
        ]
    }

    type Roster = (Vec<(String, Option<String>)>, bool);

    fn rosters(outputs: &[ProviderOutput]) -> Vec<Roster> {
        outputs
            .iter()
            .filter_map(|output| match output {
                ProviderOutput::BackgroundTasks { tasks, waiting } => Some((
                    tasks
                        .iter()
                        .map(|task| (task.description.clone(), task.tool_call_id.clone()))
                        .collect(),
                    *waiting,
                )),
                _ => None,
            })
            .collect()
    }

    fn completions(outputs: &[ProviderOutput]) -> Vec<TurnOutcome> {
        outputs
            .iter()
            .filter_map(|output| match output {
                ProviderOutput::TurnCompleted { outcome } => Some(outcome.clone()),
                _ => None,
            })
            .collect()
    }

    fn sleep_ended_output() -> ProviderOutput {
        ProviderOutput::BackgroundTaskEnded {
            task_id: "b1".into(),
            tool_use_id: Some("toolu_bg".into()),
            failed: false,
        }
    }

    #[test]
    fn an_awaited_background_command_holds_the_turn_until_claude_answers_it() {
        let mut parser = ClaudeParser::new(None);
        let started = await_sleep(&mut parser);

        let description = "Sleep then echo done".to_owned();
        let called = (description.clone(), Some("toolu_bg".to_owned()));
        assert_eq!(
            rosters(&started),
            [
                (vec![(description, None)], false),
                (vec![called.clone()], false),
                (vec![called], true),
            ]
        );
        assert!(started.contains(&ProviderOutput::RunEnded { run: 1 }));
        assert!(completions(&started).is_empty(), "{started:#?}");

        let ended = feed(&mut parser, &sleep_ended());
        assert_eq!(
            ended,
            [
                ProviderOutput::BackgroundTasks {
                    tasks: Vec::new(),
                    waiting: true
                },
                sleep_ended_output(),
            ]
        );

        let answered = feed(&mut parser, &answer_to_sleep());
        assert!(rosters(&answered).is_empty(), "{answered:#?}");
        assert!(!answered.contains(&ProviderOutput::Woke));
        assert_eq!(completions(&answered), [TurnOutcome::Completed]);
        assert!(parser.queue.state().closing);
    }

    #[test]
    fn a_command_left_running_ends_the_turn_and_keeps_the_process() {
        let mut parser = ClaudeParser::new(None);
        let outputs = leave_sleep_running(&mut parser);

        assert!(!outputs.contains(&ProviderOutput::RunEnded { run: 1 }));
        assert_eq!(completions(&outputs), [TurnOutcome::Completed]);
        let called = vec![(
            "Sleep then echo done".to_owned(),
            Some("toolu_bg".to_owned()),
        )];
        assert_eq!(rosters(&outputs).last(), Some(&(called, false)));
        let queue = parser.queue.state();
        assert!(!queue.closing && !queue.turn_open);
    }

    #[test]
    fn a_command_left_running_that_ends_wakes_the_agent_in_a_turn_of_its_own() {
        let mut parser = ClaudeParser::new(None);
        leave_sleep_running(&mut parser);

        let ended = feed(&mut parser, &sleep_ended());
        assert_eq!(
            ended,
            [ProviderOutput::BackgroundTasks {
                tasks: Vec::new(),
                waiting: false
            }]
        );
        assert!(!parser.queue.state().closing, "Claude still answers it");

        let woke = feed(&mut parser, &answer_to_sleep());
        assert_eq!(woke[..2], [ProviderOutput::Woke, sleep_ended_output()]);
        assert!(woke.contains(&ProviderOutput::AssistantMessage {
            item_id: "msg_2:0".into(),
            text: "It printed done.".into()
        }));
        assert_eq!(completions(&woke), [TurnOutcome::Completed]);
        assert!(parser.queue.state().closing, "nothing is left running");
    }

    #[test]
    fn a_wake_that_ends_with_commands_still_running_keeps_the_process() {
        let mut parser = ClaudeParser::new(None);
        leave_sleep_running(&mut parser);
        let server = json!({ "task_id": "s1", "task_type": "local_bash",
            "description": "Start the dev server" });
        feed(
            &mut parser,
            &[roster(json!([
                { "task_id": "b1", "task_type": "local_bash", "description": "Sleep then echo done" },
                server.clone(),
            ]))],
        );

        let mut lines = sleep_ended();
        lines.pop();
        lines.push(roster(json!([server])));
        lines.extend(answer_to_sleep());
        let woke = feed(&mut parser, &lines);

        assert_eq!(
            woke.iter().filter(|o| **o == ProviderOutput::Woke).count(),
            1
        );
        assert_eq!(completions(&woke), [TurnOutcome::Completed]);
        let queue = parser.queue.state();
        assert!(!queue.closing && !queue.turn_open);
    }

    #[test]
    fn the_next_message_runs_in_the_process_left_running() {
        let mut parser = ClaudeParser::new(None);
        feed(
            &mut parser,
            &[json!({ "type": "system", "subtype": "init", "session_id": "s1", "cwd": "/repo" })],
        );
        leave_sleep_running(&mut parser);
        let launch = launch("/repo");

        let lines = next_turn(
            &launch,
            None,
            Some("s1"),
            &mut parser.queue.state(),
            &request("/repo", "and now?"),
        )
        .unwrap();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("and now?"), "{lines:?}");

        let answered = feed(
            &mut parser,
            &[
                json!({ "type": "system", "subtype": "init", "session_id": "s1", "cwd": "/repo" }),
                json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                    "id": "msg_3", "content": [{ "type": "text", "text": "Still sleeping." }] } }),
                run_result(),
            ],
        );
        assert!(!answered.contains(&ProviderOutput::Woke), "{answered:#?}");
        assert_eq!(completions(&answered), [TurnOutcome::Completed]);
        assert!(!parser.queue.state().closing, "the command still runs");
    }

    fn launch(cwd: &str) -> Launch {
        Launch {
            cwd: cwd.into(),
            model: turn_model(&request(cwd, ""), None),
            instructions: None,
            mcp: None,
            full_access: false,
        }
    }

    fn request(cwd: &str, prompt: &str) -> TurnRequest {
        TurnRequest {
            cwd: cwd.into(),
            prompt: prompt.into(),
            attachments: Vec::new(),
            resume_session_id: Some("s1".into()),
            resume_session_at: None,
            model: Some("claude-opus-5-5".into()),
            effort: None,
            fast_mode: false,
            ultracode: false,
            access_mode: AccessMode::AcceptEdits,
            plan_mode: false,
            mcp: None,
            instructions: None,
        }
    }

    #[test]
    fn a_turn_that_changes_how_the_process_started_needs_a_process_of_its_own() {
        let launch = launch("/repo");
        let next = |request: TurnRequest, cwd: Option<&str>| {
            let mut queue = QueueState {
                turn_open: false,
                access: Access {
                    mode: AccessMode::AcceptEdits,
                    plan_mode: false,
                },
                cwd: cwd.map(str::to_owned),
                ..QueueState::default()
            };
            next_turn(&launch, None, Some("s1"), &mut queue, &request)
        };

        assert!(next(request("/repo/", "hi"), None).is_some());
        assert!(next(request("/repo/.claude/worktrees/w", "hi"), None).is_none());
        assert!(
            next(
                request("/repo/.claude/worktrees/w", "hi"),
                Some("/repo/.claude/worktrees/w")
            )
            .is_some()
        );
        let rewound = TurnRequest {
            resume_session_at: Some("u1".into()),
            ..request("/repo", "hi")
        };
        assert!(next(rewound, None).is_none());
        let other_model = TurnRequest {
            model: Some("claude-haiku-5-5".into()),
            ..request("/repo", "hi")
        };
        assert!(next(other_model, None).is_none());
        let full_access = TurnRequest {
            access_mode: AccessMode::FullAccess,
            ..request("/repo", "hi")
        };
        assert!(next(full_access, None).is_none());

        let planning = TurnRequest {
            plan_mode: true,
            ..request("/repo", "plan it")
        };
        let lines = next(planning, None).unwrap();
        assert_eq!(lines.len(), 2);
        let mode: Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(mode["request"]["subtype"], "set_permission_mode");
        assert_eq!(mode["request"]["mode"], "plan");
    }

    #[test]
    fn a_process_with_a_turn_or_closing_takes_no_other_turn() {
        let launch = launch("/repo");
        let mut queue = QueueState::default();
        assert!(
            next_turn(
                &launch,
                None,
                Some("s1"),
                &mut queue,
                &request("/repo", "a")
            )
            .is_none()
        );
        queue.turn_open = false;
        queue.closing = true;
        assert!(
            next_turn(
                &launch,
                None,
                Some("s1"),
                &mut queue,
                &request("/repo", "a")
            )
            .is_none()
        );
    }

    #[test]
    fn only_running_commands_can_be_awaited() {
        let mut parser = ClaudeParser::new(None);
        feed(&mut parser, &sleep_in_background());

        assert_eq!(
            parser
                .queue
                .await_tasks(&["b1".into(), "nope".into()], true),
            ["b1"]
        );
        assert_eq!(parser.queue.await_tasks(&["b1".into()], false), ["b1"]);
        assert!(parser.queue.state().awaited.is_empty());
    }

    #[test]
    fn a_process_left_running_ends_once_its_commands_were_stopped() {
        let mut parser = ClaudeParser::new(None);
        leave_sleep_running(&mut parser);

        let stopped = feed(
            &mut parser,
            &[
                json!({ "type": "system", "subtype": "task_notification", "task_id": "b1",
                    "run_id": "run-b1", "tool_use_id": "toolu_bg", "status": "stopped" }),
                roster(json!([])),
            ],
        );

        assert!(completions(&stopped).is_empty());
        assert!(!stopped.contains(&ProviderOutput::Woke));
        assert!(parser.queue.state().closing);
    }

    #[test]
    fn a_process_waits_for_the_end_of_a_command_that_left_the_list_first() {
        let mut parser = ClaudeParser::new(None);
        leave_sleep_running(&mut parser);

        feed(&mut parser, &[roster(json!([]))]);
        assert!(!parser.queue.state().closing);

        let mut lines = sleep_ended();
        lines.pop();
        lines.extend(answer_to_sleep());
        let woke = feed(&mut parser, &lines);
        assert_eq!(woke[..2], [ProviderOutput::Woke, sleep_ended_output()]);
        assert!(parser.queue.state().closing);
    }

    #[test]
    fn an_interrupted_turn_ends_its_process_with_the_commands_it_left() {
        let mut parser = ClaudeParser::new(None);
        feed(&mut parser, &sleep_in_background());
        parser.queue.state().interrupted = true;

        let stopped = feed(
            &mut parser,
            &[json!({ "type": "result", "subtype": "error_during_execution", "is_error": true })],
        );

        assert_eq!(completions(&stopped), [TurnOutcome::Interrupted]);
        assert!(parser.queue.state().closing);
    }

    #[test]
    fn a_run_beside_an_awaited_background_command_is_not_waiting() {
        let mut parser = ClaudeParser::new(None);
        await_sleep(&mut parser);
        parser.queue.state().waiting.insert("m2".into());

        let steered = feed(
            &mut parser,
            &[
                json!({ "type": "command_lifecycle", "command_uuid": "m2", "state": "started" }),
                json!({ "type": "result", "subtype": "success", "is_error": false }),
            ],
        );

        let waiting: Vec<bool> = rosters(&steered).into_iter().map(|(_, w)| w).collect();
        assert_eq!(waiting, [false, true]);
    }

    #[test]
    fn a_stopped_awaited_command_ends_a_turn_with_nothing_left_to_wait_for() {
        let mut parser = ClaudeParser::new(None);
        await_sleep(&mut parser);

        let stopped = feed(
            &mut parser,
            &[
                json!({ "type": "system", "subtype": "task_notification", "task_id": "b1",
                    "run_id": "run-b1", "tool_use_id": "toolu_bg", "status": "stopped",
                    "summary": "Sleep then echo done" }),
                roster(json!([])),
            ],
        );

        assert_eq!(
            stopped,
            [
                ProviderOutput::BackgroundTasks {
                    tasks: Vec::new(),
                    waiting: true
                },
                ProviderOutput::TurnCompleted {
                    outcome: TurnOutcome::Completed
                },
            ]
        );
        assert!(parser.queue.state().closing);
    }

    #[test]
    fn a_turn_asked_to_stop_while_it_waits_ends_once_the_agent_agrees() {
        let mut parser = ClaudeParser::new(None);
        await_sleep(&mut parser);
        parser.queue.state().interrupted = true;

        let answer = json!({ "type": "control_response", "response": {
            "subtype": "success", "request_id": "hoshi-interrupt" } });

        assert_eq!(
            completions(&feed(&mut parser, &[answer])),
            [TurnOutcome::Interrupted]
        );
        assert!(parser.queue.state().closing);
    }

    #[test]
    fn a_stop_while_a_message_runs_waits_for_that_run_to_end() {
        let mut parser = ClaudeParser::new(None);
        await_sleep(&mut parser);
        parser.queue.state().waiting.insert("m2".into());

        let during = feed(
            &mut parser,
            &[
                json!({ "type": "command_lifecycle", "command_uuid": "m2", "state": "started" }),
                json!({ "type": "system", "subtype": "task_notification", "task_id": "b1",
                    "run_id": "run-b1", "tool_use_id": "toolu_bg", "status": "stopped" }),
                roster(json!([])),
            ],
        );
        assert!(completions(&during).is_empty(), "{during:#?}");

        let result = json!({ "type": "result", "subtype": "success", "is_error": false });
        assert_eq!(
            completions(&feed(&mut parser, &[result])),
            [TurnOutcome::Completed]
        );
    }

    #[test]
    fn foreground_commands_never_hold_the_turn() {
        let mut parser = ClaudeParser::new(None);
        let outputs = feed(
            &mut parser,
            &[
                json!({ "type": "system", "subtype": "task_started", "task_id": "f1",
                    "tool_use_id": "toolu_fg", "is_backgrounded": false,
                    "task_type": "local_bash" }),
                json!({ "type": "system", "subtype": "task_notification", "task_id": "f1",
                    "tool_use_id": "toolu_fg", "status": "completed" }),
                json!({ "type": "result", "subtype": "success", "is_error": false }),
            ],
        );

        assert!(rosters(&outputs).is_empty());
        assert_eq!(completions(&outputs), [TurnOutcome::Completed]);
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
