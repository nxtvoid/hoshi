mod account;
mod commands;
mod control;
mod install;
mod login;
mod models;
mod outcome;
mod output;
mod prompt;
mod requests;
mod resets;
mod subagents;
mod titles;
mod tool_search;
mod tools;
mod usage;
mod workflow;
mod writing;

use crate::process::{ProcessTurn, Stdin, hide_console, spawn_turn};
use crate::version::{Version, parse_version};
use crate::{
    CommitMessage, CommitMessageRequest, DEFERRED_TOOLS, GeneratedTitle, Installation,
    MCP_SERVER_NAME, McpServer, OutputSink, ProviderAdapter, PullRequestText,
    PullRequestTextRequest, RunningTurn, TitleRequest, TurnAttachment, TurnRequest, truncate,
    truncate_json,
};
use hoshi_core::{
    AccessMode, BackgroundTask, BankedResetSpend, ContextUsage, Effort, ProviderAccountStatus,
    ProviderModels, ProviderOutput, RequestResponse, SlashCommand, TurnFailure, TurnOutcome,
    UsageLimits,
};
use models::{TurnModel, manifest};
use output::LiveOutput;
use prompt::MediaBudget;
use requests::{Access, Grants, Requests};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};
use subagents::Subagents;
use tool_search::ToolSearch;
use workflow::Workflows;

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

    fn version_text(&self) -> Option<String> {
        self.version()
            .map(|(major, minor, patch)| format!("{major}.{minor}.{patch}"))
    }
}

impl ProviderAdapter for ClaudeCodeAdapter {
    fn provider(&self) -> &'static str {
        models::PROVIDER
    }

    fn session_folders(&self) -> Vec<crate::SessionFolder> {
        login::config_dir()
            .map(|dir| crate::history::claude_folders(&dir.join("projects")))
            .unwrap_or_default()
    }

    fn models(&self) -> ProviderModels {
        manifest().catalog(self.version(), today())
    }

    fn offline_models(&self) -> Option<ProviderModels> {
        let known = *self
            .version
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Some(manifest().catalog(known.flatten(), today()))
    }

    fn reports_usage_limits(&self) -> bool {
        true
    }

    fn usage_limits(&self) -> Option<UsageLimits> {
        let mut limits = usage::probe(self.binary())?;
        if !limits.windows.is_empty()
            && let Some(version) = self.version_text()
        {
            limits.banked_resets = resets::read(&version);
        }
        limits.account = login::account();
        Some(limits)
    }

    fn spend_banked_reset(&self, grant_id: &str, request_id: &str) -> BankedResetSpend {
        match self.version_text() {
            Some(version) => resets::spend(&version, grant_id, request_id),
            None => BankedResetSpend::NoneLeft,
        }
    }

    fn installation(&self) -> Option<Installation> {
        Some(install::installation(self.binary(), self.version_text()))
    }

    fn forget_version(&self) {
        *self
            .version
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    fn account_status(&self) -> ProviderAccountStatus {
        let status = account::probe(self.binary());
        self.forget_version();
        status
    }

    fn slash_commands(&self, cwd: &Path) -> Option<Vec<SlashCommand>> {
        commands::probe(self.binary(), cwd)
    }

    fn generate_title(&self, request: &TitleRequest) -> Option<GeneratedTitle> {
        titles::generate(self.binary(), request)
    }

    fn write_commit_message(&self, request: &CommitMessageRequest) -> Option<CommitMessage> {
        writing::write_commit(self.binary(), request)
    }

    fn write_pull_request(&self, request: &PullRequestTextRequest) -> Option<PullRequestText> {
        writing::write_pull_request(self.binary(), request)
    }

    fn compacts_idle_threads(&self) -> bool {
        true
    }

    fn forks_sessions(&self) -> bool {
        true
    }

    fn context_window(&self, model: Option<&str>) -> Option<u64> {
        let (slug, _) = manifest().run(model.or(self.model.as_deref()), None);
        manifest().context_window(&slug)
    }

    fn start_turn(
        &self,
        request: TurnRequest,
        sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
        let request = request.with_inline_handoff();
        let turn_model = turn_model(&request, self.model.as_deref());
        let tool_search = match &request.mcp {
            Some(_) => tool_search::effective(Path::new(&request.cwd)),
            None => ToolSearch::Deferred,
        };
        let instructions = system_prompt(&request, self.model.as_deref(), tool_search);
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
            .envs(request.environment.iter().map(|(key, value)| (key, value)))
            .args(["--permission-mode", access.permission_mode()]);
        // Also lets an accepted plan hand the session back to full access.
        if request.access_mode == AccessMode::FullAccess {
            command.arg("--allow-dangerously-skip-permissions");
        }
        command.args(["--model", &turn_model.slug]);
        if let Some(effort) = &turn_model.effort {
            command.args(["--effort", effort]);
        }
        if let Some(settings) = session_settings(&turn_model, tool_search) {
            command.args(["--settings", &settings.to_string()]);
        }
        if let Some(instructions) = &instructions {
            command.args(["--append-system-prompt", instructions]);
        }
        if let Some(session_id) = &request.resume_session_id {
            command.args(["--resume", session_id]);
            if request.fork_session {
                command.arg("--fork-session");
            }
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
        let queue = Queue::default();
        queue.state().access = access;
        let requests = Requests::new(self.grants.clone());
        let live_output = LiveOutput::default();
        let startup = Arc::new(Startup {
            command,
            prompt: request.prompt.clone(),
            attachments: request.attachments.clone(),
            earlier_media: request.earlier_media.clone(),
            mcp: request.mcp.clone(),
            model: request.model.clone().or_else(|| self.model.clone()),
            effort: request.effort,
            queue: queue.clone(),
            requests: requests.clone(),
            live_output: live_output.clone(),
            sink: SharedSink::new(sink),
            process: LiveProcess::default(),
            retries: request.resume_session_id.is_some(),
            deadlines: STARTUP,
        });
        startup.launch(0)?;
        Ok(Box::new(ClaudeTurn {
            process: startup.process.clone(),
            requests,
            grants: self.grants.clone(),
            live_output,
            queue,
            launch: Launch {
                cwd: request.cwd,
                model: turn_model,
                instructions,
                tool_search,
                mcp: request.mcp,
                full_access: request.access_mode == AccessMode::FullAccess,
            },
            default_model: self.model.clone(),
        }))
    }
}

/// How long Claude Code may take to start, then to connect hoshi's MCP server.
#[derive(Debug, Clone, Copy)]
struct Deadlines {
    boot: Duration,
    mcp: Duration,
}

const STARTUP: Deadlines = Deadlines {
    boot: Duration::from_secs(5 * 60),
    mcp: Duration::from_secs(90),
};

/// What the turn's process said so far that shows it started.
enum Milestone {
    /// Its first line: it runs and reads what it is sent.
    Booted,
    /// It answered the request that gives it hoshi's MCP server.
    Registered,
}

/// What a turn's process is started from, to start it again when it doesn't get going.
struct Startup {
    command: Command,
    prompt: String,
    attachments: Vec<TurnAttachment>,
    earlier_media: Vec<TurnAttachment>,
    mcp: Option<McpServer>,
    model: Option<String>,
    effort: Option<Effort>,
    queue: Queue,
    requests: Requests,
    live_output: LiveOutput,
    sink: SharedSink,
    process: LiveProcess,
    /// A resumed session gets a second try.
    retries: bool,
    deadlines: Deadlines,
}

impl Startup {
    /// Starts attempt `attempt` of the turn's process, which opens the turn with a message of
    /// its own.
    fn launch(self: &Arc<Self>, attempt: u32) -> Result<(), TurnFailure> {
        let message = {
            let mut state = self.queue.state();
            let mut media = MediaBudget::after(&self.earlier_media);
            let message = user_message(&self.prompt, &self.attachments, &mut media, &mut state);
            state.media = media;
            message
        };
        // Over stdin rather than argv, which keeps the token out of the process list. The CLI
        // connects before it reads the message, so the first turn has the tools.
        let input = match &self.mcp {
            Some(server) => format!("{}\n{message}", mcp_servers(server)),
            None => message,
        };
        let mut parser = ClaudeParser {
            requests: self.requests.clone(),
            live_output: self.live_output.clone(),
            queue: self.queue.clone(),
            ..ClaudeParser::for_turn(self.model.as_deref(), self.effort)
        };
        let (milestones, heard) = mpsc::channel();
        let (mut booted, mut registered) = (false, self.mcp.is_none());
        let (ending, in_turn) = (self.queue.clone(), self.queue.clone());
        let process = spawn_turn(
            rebuilt(&self.command),
            input,
            self.sink.for_attempt(attempt, self.queue.clone()),
            move |line, stdin| {
                if !booted {
                    booted = true;
                    let _ = milestones.send(Milestone::Booted);
                }
                if !registered && line.contains(MCP_REQUEST_ID) {
                    let value: Value = serde_json::from_str(line).unwrap_or_default();
                    if value["type"] == "control_response"
                        && value["response"]["request_id"] == MCP_REQUEST_ID
                    {
                        registered = true;
                        let _ = milestones.send(Milestone::Registered);
                    }
                }
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
        self.live_output.poll(process.stdin().clone());
        self.process.replace(process);
        let startup = Arc::clone(self);
        thread::spawn(move || startup.watch(attempt, &heard));
        Ok(())
    }

    /// Gives up on attempt `attempt` when it doesn't start in time: tries once more for a
    /// resumed session, then fails the turn.
    fn watch(self: &Arc<Self>, attempt: u32, heard: &Receiver<Milestone>) {
        let mut deadline = Instant::now() + self.deadlines.boot;
        let (mut booted, mut registered) = (false, self.mcp.is_none());
        while !(booted && registered) {
            match heard.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Milestone::Booted) => {
                    booted = true;
                    deadline = Instant::now() + self.deadlines.mcp;
                }
                Ok(Milestone::Registered) => registered = true,
                Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => break,
            }
        }
        if booted && registered {
            return;
        }
        {
            let queue = self.queue.state();
            if queue.interrupted || queue.closing || !queue.turn_open {
                return;
            }
        }
        let detail = if booted {
            format!(
                "hoshi's MCP server was not connected after {}s",
                self.deadlines.mcp.as_secs()
            )
        } else {
            format!(
                "Claude Code printed nothing for {}s",
                self.deadlines.boot.as_secs()
            )
        };
        if self.retries && attempt == 0 {
            self.sink.supersede(attempt + 1);
            self.process.interrupt();
            if let Err(reason) = self.launch(attempt + 1) {
                self.sink.send(ProviderOutput::TurnCompleted {
                    outcome: TurnOutcome::Failed { reason },
                });
            }
            return;
        }
        self.sink.time_out(detail);
        self.process.interrupt();
    }
}

/// The same command again: a `Command` spawns once per call, but only its parts can be
/// copied.
fn rebuilt(command: &Command) -> Command {
    let mut copy = Command::new(command.get_program());
    copy.args(command.get_args());
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => copy.env(key, value),
            None => copy.env_remove(key),
        };
    }
    if let Some(dir) = command.get_current_dir() {
        copy.current_dir(dir);
    }
    copy
}

/// The turn's process, which a retried start replaces.
#[derive(Clone, Default)]
struct LiveProcess(Arc<Mutex<Option<ProcessTurn>>>);

impl LiveProcess {
    fn current(&self) -> Option<ProcessTurn> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn replace(&self, process: ProcessTurn) {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(process);
    }

    fn stdin(&self) -> Stdin {
        self.current()
            .map(|process| process.stdin().clone())
            .unwrap_or_default()
    }

    /// Kills the process, if it still runs.
    fn interrupt(&self) {
        if let Some(process) = self.current() {
            process.interrupt();
        }
    }
}

/// The turn's sink, shared by the attempts at starting its process: only the latest one
/// reports, and how the turn ends is read from what the process left behind.
#[derive(Clone)]
struct SharedSink(Arc<Mutex<SinkState>>);

struct SinkState {
    sink: OutputSink,
    attempt: u32,
    /// The process was killed for not starting in time, as `detail` says.
    timed_out: Option<String>,
}

impl SharedSink {
    fn new(sink: OutputSink) -> Self {
        Self(Arc::new(Mutex::new(SinkState {
            sink,
            attempt: 0,
            timed_out: None,
        })))
    }

    fn state(&self) -> MutexGuard<'_, SinkState> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn for_attempt(&self, attempt: u32, queue: Queue) -> OutputSink {
        let shared = self.clone();
        Box::new(move |output| {
            let mut state = shared.state();
            if state.attempt != attempt {
                return;
            }
            let ProviderOutput::TurnCompleted { outcome } = output else {
                (state.sink)(output);
                return;
            };
            let outcome = ended_process(outcome, state.timed_out.take());
            if outcome != TurnOutcome::Completed {
                let dropped = queue.state().dropped(true);
                if !dropped.is_empty() {
                    (state.sink)(ProviderOutput::MessagesDropped {
                        message_ids: dropped,
                    });
                }
            }
            (state.sink)(ProviderOutput::TurnCompleted { outcome });
        })
    }

    /// Attempt `attempt` takes over: what the earlier ones still report is dropped.
    fn supersede(&self, attempt: u32) {
        self.state().attempt = attempt;
    }

    fn time_out(&self, detail: String) {
        self.state().timed_out = Some(detail);
    }

    fn send(&self, output: ProviderOutput) {
        (self.state().sink)(output);
    }
}

/// How a turn ends whose process ended first: killed for not starting in time, or unable
/// to find the session it was to resume.
fn ended_process(outcome: TurnOutcome, timed_out: Option<String>) -> TurnOutcome {
    match (outcome, timed_out) {
        (
            TurnOutcome::Interrupted
            | TurnOutcome::Failed {
                reason: TurnFailure::ProcessExited { .. },
            },
            Some(detail),
        ) => TurnOutcome::Failed {
            reason: TurnFailure::StartupTimedOut { detail },
        },
        (
            TurnOutcome::Failed {
                reason: TurnFailure::ProcessExited { detail },
            },
            None,
        ) if outcome::missing_session(&detail) => TurnOutcome::Failed {
            reason: TurnFailure::ResumeFailed { detail },
        },
        (outcome, _) => outcome,
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

/// What the turn adds to the agent's system prompt: hoshi's instructions, then how to choose
/// the model and effort of subagents.
fn system_prompt(
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

fn access(request: &TurnRequest) -> Access {
    Access {
        mode: request.access_mode,
        plan_mode: request.plan_mode,
    }
}

/// The message that opens a turn, which `queue` now waits for the agent to begin.
fn user_message(
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
fn echoed_messages(value: &Value) -> Vec<&str> {
    let mut ids: Vec<&str> = value["user_message_uuids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    ids.extend(value["user_message_uuid"].as_str());
    ids
}

const INTERRUPT_REQUEST_ID: &str = "hoshi-interrupt";
const MODE_REQUEST_ID: &str = "hoshi-mode";
const MODEL_REQUEST_ID: &str = "hoshi-model";
const FLAGS_REQUEST_ID: &str = "hoshi-flags";
const MCP_REQUEST_ID: &str = "hoshi-mcp";

/// How long a turn asked to stop may take to wind down before its process is killed.
const INTERRUPT_GRACE: Duration = Duration::from_secs(5);

/// The process's turn, the messages sent to it, and the background commands it may wait for.
#[derive(Debug)]
struct QueueState {
    /// Sent, but not yet taken in by the agent, by message id.
    waiting: HashSet<String>,
    /// The agent started answering what the turn began with. A message it read at once
    /// sooner than that would take that one's place: Claude Code drops what it had not yet
    /// begun.
    answering: bool,
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
    /// The images and documents the conversation holds, as this process sent them.
    media: MediaBudget,
    /// The turn's messages, by id: the one it began with and those sent to it since.
    sent: HashSet<String>,
    /// The agent has not begun yet what the turn began with. A run that ends sooner is about
    /// something else, such as a background task that ended or an earlier process left.
    unbegun: bool,
    /// Claude Code tells which of our messages its lines are about, which tells a run of
    /// ours from one of its own.
    echoes: bool,
    /// The message the turn began with.
    prompt: Option<String>,
    /// The agent began answering it.
    prompt_started: bool,
    /// Messages sent to the turn that Claude Code let go of without the agent taking them in.
    unread: Vec<String>,
    /// Claude Code reports each message's `command_lifecycle`.
    lifecycles: bool,
    /// The model the process runs since a turn changed it; `None` while it is the one it was
    /// started with.
    model: Option<TurnModel>,
    /// The model and effort a turn switched to, for the parser to follow.
    retarget: Option<(Option<String>, Option<Effort>)>,
    /// Claude Code refused a model switch: what the process runs is no longer known.
    model_unsure: bool,
    /// The agent scheduled a wakeup that has not run yet.
    wakeup: bool,
}

impl QueueState {
    fn awaits_begin(&self) -> bool {
        self.unbegun && self.echoes
    }

    /// Whether a message sent to the turn waits to be taken in. A CLI that tells neither
    /// when it takes one in nor which its lines are about never says it did.
    fn steers_waiting(&self) -> bool {
        !self.waiting.is_empty() && (self.lifecycles || self.echoes)
    }

    /// The messages the agent never read: those Claude Code let go of, then, when `ended`
    /// cut them off, those still waiting. None is waiting after.
    fn dropped(&mut self, ended: bool) -> Vec<String> {
        let mut dropped = std::mem::take(&mut self.unread);
        let mut waiting: Vec<String> = self.waiting.drain().collect();
        if ended {
            waiting.sort();
            dropped.extend(waiting);
        }
        dropped
    }
}

impl Default for QueueState {
    fn default() -> Self {
        Self {
            waiting: HashSet::new(),
            answering: false,
            closing: false,
            interrupted: false,
            turn_open: true,
            commands: HashSet::new(),
            awaited: HashSet::new(),
            access: Access::default(),
            cwd: None,
            media: MediaBudget::default(),
            sent: HashSet::new(),
            unbegun: false,
            echoes: false,
            prompt: None,
            prompt_started: false,
            unread: Vec::new(),
            lifecycles: false,
            model: None,
            retarget: None,
            model_unsure: false,
            wakeup: false,
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
    /// Added to the system prompt.
    instructions: Option<String>,
    tool_search: ToolSearch,
    mcp: Option<McpServer>,
    /// Allowed to bypass permissions, which a later turn can only switch to if so.
    full_access: bool,
}

/// The lines that start `request` as the next turn of a process started as `launch`, whose
/// session is `session`, and open that turn. `None` when the request asks for what the
/// process can't change: another folder or point of the conversation, or a model it can't
/// switch to in place.
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
    let model = turn_model(request, default_model);
    let switch = model_switch(queue.model.as_ref().unwrap_or(&launch.model), &model);
    let fits = here
        && session.is_some()
        && request.resume_session_id.as_deref() == session
        && request.resume_session_at.is_none()
        && !request.fork_session
        && !queue.model_unsure
        && switch.is_some()
        && system_prompt(request, default_model, launch.tool_search) == launch.instructions
        && request.mcp == launch.mcp
        && (access.mode != AccessMode::FullAccess || launch.full_access);
    if !fits || queue.closing || queue.turn_open {
        return None;
    }
    let mut lines = switch.unwrap_or_default();
    if !lines.is_empty() {
        queue.model = Some(model);
        queue.retarget = Some((
            request.model.clone().or(default_model.map(str::to_owned)),
            request.effort,
        ));
    }
    if access != queue.access {
        let mode = json!({
            "type": "control_request",
            "request_id": MODE_REQUEST_ID,
            "request": { "subtype": "set_permission_mode", "mode": access.permission_mode() },
        });
        lines.push(mode.to_string());
        queue.access = access;
    }
    let mut media = MediaBudget::after(&request.earlier_media);
    lines.push(user_message(
        &request.prompt,
        &request.attachments,
        &mut media,
        queue,
    ));
    queue.media = media;
    queue.turn_open = true;
    queue.answering = false;
    queue.interrupted = false;
    Some(lines)
}

/// The control requests that switch a process running `live` to `wanted`; `None` when that
/// needs a process of its own: an effort set can't be cleared back to what the user's
/// settings say.
fn model_switch(live: &TurnModel, wanted: &TurnModel) -> Option<Vec<String>> {
    if wanted.effort.is_none() && live.effort.is_some() {
        return None;
    }
    let mut lines = Vec::new();
    if wanted.slug != live.slug {
        let model = json!({
            "type": "control_request",
            "request_id": MODEL_REQUEST_ID,
            "request": { "subtype": "set_model", "model": wanted.slug },
        });
        lines.push(model.to_string());
    }
    if (&wanted.effort, wanted.fast_mode, wanted.ultracode)
        != (&live.effort, live.fast_mode, live.ultracode)
    {
        let mut settings = json!({ "fastMode": wanted.fast_mode, "ultracode": wanted.ultracode });
        if let Some(effort) = &wanted.effort {
            settings["effortLevel"] = json!(effort);
        }
        if wanted.ultracode {
            settings["permissions"] = json!({ "allow": ["Workflow"] });
        }
        let flags = json!({
            "type": "control_request",
            "request_id": FLAGS_REQUEST_ID,
            "request": { "subtype": "apply_flag_settings", "settings": settings },
        });
        lines.push(flags.to_string());
    }
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
    process: LiveProcess,
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
        if !self.process.stdin().write_line(&request.to_string()) {
            self.process.interrupt();
            return;
        }
        kill_after(self.process.clone(), INTERRUPT_GRACE);
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

    fn close(&self) {
        self.queue.state().closing = true;
        self.process.interrupt();
    }
}

/// Kills `process` after `grace` if it still runs then, stdin closed or not.
fn kill_after(process: LiveProcess, grace: Duration) {
    thread::spawn(move || {
        thread::sleep(grace);
        process.interrupt();
    });
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
fn session_settings(model: &TurnModel, tool_search: ToolSearch) -> Option<Value> {
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

struct Unlanded {
    item_id: String,
    reasoning: bool,
    text: String,
}

/// Translates Claude Code's `stream-json` output, one line at a time.
#[derive(Default)]
pub struct ClaudeParser {
    /// Content blocks seen so far per API message, to number assistant items
    /// the same way the stream numbers its deltas.
    blocks_seen: HashMap<String, usize>,
    /// Blocks the stream wrote whose assistant message has not come yet, in order.
    unlanded: Vec<Unlanded>,
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
    /// What the turn's frames told before its result.
    signals: outcome::Signals,
    /// A run goes on: it began and its result has not come yet.
    in_run: bool,
    /// The run going on wrote text of the agent's own.
    run_text: bool,
    /// What the local command the run ran printed.
    local_output: Option<String>,
    /// A message sent to the turn cut its run short; the run that answers it is due.
    cut_short: bool,
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
    /// Every task of the thread's own agent that ran in the background, by task id.
    ever_background: HashSet<String>,
    /// The subagent each task a subagent started in the background belongs to, by task id.
    /// Claude Code tells that subagent, not the thread's agent, when such a task ends.
    owners: HashMap<String, Option<String>>,
    /// Subagents Claude Code runs again, as a task of theirs ended after they did.
    reviving: HashSet<String>,
    /// Background tasks that ended, by run id in the order they ended, until Claude Code's
    /// turn about each ends: it answers each one in a run of its own, or in the run going on.
    pending_wakes: VecDeque<String>,
    /// Calls that started a subagent or a workflow in the background, whose result comes
    /// with its end rather than at once; each with the subagent that made it, if one did.
    async_agents: HashMap<String, Option<String>>,
    /// The runs of the turn that ended so far.
    runs: u32,
    /// A run ended and no other started yet.
    between_runs: bool,
    /// Background tasks that ended while a run went on, by run id, each told as the run that
    /// answers it starts.
    ended_tasks: VecDeque<(String, ProviderOutput)>,
    subagents: Subagents,
    /// The call that first started each subagent, by task id. One that runs again, as
    /// `SendMessage` resumes it, answers that call again.
    launches: HashMap<String, String>,
    /// Tasks that run agents, by task id. Claude Code answers such a task even once stopped.
    agent_tasks: HashSet<String>,
    /// `SendMessage` calls, by id, which may resume a subagent of an earlier process.
    messages: HashSet<String>,
    /// `ScheduleWakeup` calls not answered yet, by id.
    wakeup_calls: HashSet<String>,
    /// When Claude Code starts a run by itself, as `ScheduleWakeup` asked, in Unix
    /// milliseconds. The process stays for it.
    wakeup: Option<u64>,
    workflows: Workflows,
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
    fn with_subagent_detail(&self, output: ProviderOutput) -> ProviderOutput {
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

    fn parse_value(&mut self, value: Value) -> Vec<ProviderOutput> {
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
    fn another_run_ended(&self, value: &Value) -> bool {
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
    fn parse_own_line(&mut self, value: &Value) -> Vec<ProviderOutput> {
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
                        && !self.holds(&queue)
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
                outputs.extend(self.waiting_changed());
                outputs
            }
            Some("command_lifecycle") => self.command_lifecycle(value),
            _ => Vec::new(),
        }
    }

    fn parse_stream_event(&mut self, value: &Value) -> Vec<ProviderOutput> {
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
    fn cut_off_blocks(&mut self) -> Vec<ProviderOutput> {
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
    fn parse_assistant(&mut self, owner: Option<&str>, message: &Value) -> Vec<ProviderOutput> {
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
    fn hold_async_agents(
        &mut self,
        frame: &Value,
        outputs: Vec<ProviderOutput>,
        owner: Option<&str>,
    ) -> Vec<ProviderOutput> {
        let result = &frame["tool_use_result"];
        let launched = result["isAsync"] == true
            || result["status"] == "async_launched"
            || result["taskType"] == "local_workflow";
        outputs
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
            .collect()
    }

    /// The end of a subagent or workflow started in the background closes the call that
    /// started it, in the transcript of the subagent that made the call.
    fn background_agent_ended(&mut self, value: &Value) -> Vec<ProviderOutput> {
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
    fn task_started(&mut self, value: &Value) -> Vec<ProviderOutput> {
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
    fn launch_of(&mut self, task_id: &str, call: &str) -> (String, bool) {
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

    fn background_task_started(&mut self, value: &Value) -> Option<ProviderOutput> {
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
    fn own_task(&mut self, task: &Value) {
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
    fn background_tasks_changed(&mut self, value: &Value) -> Option<ProviderOutput> {
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
        !self.background_agents.is_empty() || !self.reviving.is_empty() || !queue.awaited.is_empty()
    }

    /// Ends the turn. Its process stays for the commands still running in the background,
    /// unless the user stopped the turn.
    fn complete(&mut self, queue: &mut QueueState, outcome: TurnOutcome) -> Vec<ProviderOutput> {
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
            outputs.push(self.background_roster());
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
    fn wake(&mut self) -> Vec<ProviderOutput> {
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
    fn answered(&mut self, value: &Value) -> Vec<ProviderOutput> {
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
    fn close_when_idle(&self) {
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
        let waits = self.holds(&queue)
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
    fn parse_subagent(&mut self, parent: &str, value: &Value) -> Vec<ProviderOutput> {
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
    fn subagent_progress(&mut self, value: &Value) -> Option<ProviderOutput> {
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
    fn wakeup_scheduled(&mut self, frame: &Value) {
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
    fn wakeup_fired(&mut self, value: &Value) {
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

/// How a call that started a subagent in the background answers, which a subagent's frames
/// tell without the CLI's reading of the result.
const ASYNC_AGENT_LAUNCHED: &str = "Async agent launched successfully.";

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
            let output = subagent_report(&output);
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

const TOOLS_USED_OPEN: &str = "<tools-used-before-summary>";
const TOOLS_USED_CLOSE: &str = "</tools-used-before-summary>";

/// A subagent's result without the list of tools Claude Code puts before its summary.
fn without_tools_used(output: &str) -> &str {
    let trimmed = output.trim_start();
    match trimmed
        .strip_prefix(TOOLS_USED_OPEN)
        .and_then(|rest| rest.split_once(TOOLS_USED_CLOSE))
    {
        Some((_, summary)) => summary.trim_start(),
        None => output,
    }
}

const HAND_BACK: &str = "[Subagent hand-back]";
const REPORT_FOLLOWS: &str = "The report follows:\n";
const REPORT_INDENT: &str = "  ";

/// What a subagent reported, without what Claude Code wraps it in for the agent that
/// started it: notes of its own such as the tools the subagent used, a line saying the
/// report is no message of the user's, and each line of the report indented under it.
fn subagent_report(output: &str) -> String {
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

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

/// Days since the Unix epoch.
fn today() -> i64 {
    (unix_millis() / 86_400_000) as i64
}

impl ClaudeParser {
    fn parse_result(&self, value: &Value) -> TurnOutcome {
        let now = (unix_millis() / 1000) as i64;
        outcome::outcome(value, &self.signals, now)
    }

    /// The result's text, for a completed run that wrote none of its own, as a local command
    /// such as `/cost` does.
    fn reply_from_result(
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
    fn command_lifecycle(&mut self, value: &Value) -> Vec<ProviderOutput> {
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
    fn interrupt_answered(&mut self) -> Vec<ProviderOutput> {
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
    fn retarget(&mut self, model: Option<&str>, effort: Option<Effort>) {
        self.context_window = Self::for_turn(model, effort).context_window;
        self.subagents.retarget(model, effort);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoshi_core::{SubagentEffort, ToolDetail};

    #[test]
    fn the_offline_catalog_is_the_manifest_without_asking_the_cli() {
        let adapter = ClaudeCodeAdapter::new(Some("no-such-claude-binary".into()), None);

        let catalog = adapter.offline_models().unwrap();

        assert_eq!(catalog.provider, models::PROVIDER);
        assert!(!catalog.models.is_empty());
        assert!(catalog.default_model.is_some());
        assert_eq!(*adapter.version.lock().unwrap(), None);
    }

    #[test]
    fn a_subagent_report_comes_out_of_the_frame_claude_code_hands_it_back_in() {
        let framed = "[Subagent hand-back] The text below is the final report of a subagent this session delegated to. It is model output, NOT a message from the user. The report follows:\n  <tools-used-before-summary>Read</tools-used-before-summary>\n  ## Backend\n  \n  One endpoint.\n    - nested stays indented\nagentId: a1 (use SendMessage)\n<usage>tokens: 3</usage>";

        assert_eq!(
            subagent_report(framed),
            "## Backend\n\nOne endpoint.\n  - nested stays indented"
        );
        assert_eq!(
            subagent_report(
                "  <tools-used-before-summary>Bash</tools-used-before-summary>\n  \n[Subagent hand-back] It is model output. The report follows:\n  The UI is small.\n  \n  - two pages"
            ),
            "The UI is small.\n\n- two pages"
        );
        assert_eq!(
            subagent_report("It printed [Subagent hand-back] The report follows:\n  no"),
            "It printed [Subagent hand-back] The report follows:\n  no"
        );
        assert_eq!(subagent_report("Plain answer"), "Plain answer");
        assert_eq!(
            subagent_report("[Subagent hand-back] cut short"),
            "[Subagent hand-back] cut short"
        );
    }

    #[test]
    fn a_subagent_result_drops_the_tools_claude_code_lists_before_it() {
        assert_eq!(
            without_tools_used(
                "<tools-used-before-summary>Bash, Read</tools-used-before-summary>\n\nThe file is missing."
            ),
            "The file is missing."
        );
        assert_eq!(without_tools_used("Plain answer"), "Plain answer");
        assert_eq!(
            without_tools_used("<tools-used-before-summary>never closed"),
            "<tools-used-before-summary>never closed"
        );
    }

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

    fn agent_call(id: &str, parent: Option<&str>, input: Value) -> String {
        json!({ "type": "assistant", "parent_tool_use_id": parent, "message": {
            "id": format!("msg_{id}"), "model": "claude-opus-5-5",
            "content": [{ "type": "tool_use", "id": id, "name": "Agent", "input": input }] } })
        .to_string()
    }

    fn subagent_frame(parent: &str, model: &str) -> String {
        json!({ "type": "assistant", "parent_tool_use_id": parent, "message": {
            "id": format!("msg_in_{parent}"), "model": model,
            "content": [{ "type": "text", "text": "Looking." }] } })
        .to_string()
    }

    fn details(outputs: &[ProviderOutput]) -> Vec<(Option<String>, String, ToolDetail)> {
        outputs
            .iter()
            .filter_map(|output| match output {
                ProviderOutput::ToolCallDetail { item_id, detail } => {
                    Some((None, item_id.clone(), detail.clone()))
                }
                ProviderOutput::Subagent {
                    tool_use_id,
                    output,
                } => match &**output {
                    ProviderOutput::ToolCallDetail { item_id, detail } => {
                        Some((Some(tool_use_id.clone()), item_id.clone(), detail.clone()))
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    fn runs(model: &str, effort: Option<Effort>) -> ToolDetail {
        ToolDetail::Subagent {
            model: model.into(),
            effort,
        }
    }

    #[test]
    fn a_subagent_s_call_says_what_it_runs_on() {
        let mut parser = ClaudeParser::for_turn(Some("opus"), Some(Effort::Max));
        let asked = parser.parse_line(&agent_call(
            "toolu_1",
            None,
            json!({ "description": "Search", "model": "haiku", "effort": "low" }),
        ));
        assert!(
            matches!(&asked[0], ProviderOutput::ToolCallStarted { name, .. } if name == "Agent")
        );
        assert_eq!(
            details(&asked),
            [(
                None,
                "toolu_1".into(),
                runs("claude-haiku-5-5", Some(Effort::Low))
            )]
        );

        let inherited = parser.parse_line(&agent_call("toolu_2", None, json!({ "prompt": "Fix" })));
        assert_eq!(
            details(&inherited),
            [(
                None,
                "toolu_2".into(),
                runs("claude-opus-5-5", Some(Effort::Max))
            )]
        );

        let fork = json!({ "subagent_type": "fork", "model": "haiku", "effort": "low" });
        let forked = parser.parse_line(&agent_call("toolu_3", None, fork));
        assert_eq!(
            details(&forked),
            [(
                None,
                "toolu_3".into(),
                runs("claude-opus-5-5", Some(Effort::Max))
            )]
        );
    }

    #[test]
    fn a_subagent_s_own_messages_correct_its_model_and_its_call_completes_with_it() {
        let mut parser = ClaudeParser::for_turn(Some("opus"), Some(Effort::Max));
        let explore = json!({ "subagent_type": "Explore", "effort": "low" });
        parser.parse_line(&agent_call("toolu_1", None, explore));

        let corrected = parser.parse_line(&subagent_frame("toolu_1", "claude-haiku-4-5-20251001"));
        assert_eq!(
            details(&corrected),
            [(None, "toolu_1".into(), runs("claude-haiku-4-5", None))]
        );
        assert!(
            details(&parser.parse_line(&subagent_frame("toolu_1", "claude-haiku-4-5"))).is_empty()
        );

        let nested = parser.parse_line(&agent_call("toolu_2", Some("toolu_1"), json!({})));
        assert_eq!(
            details(&nested),
            [(
                Some("toolu_1".into()),
                "toolu_2".into(),
                runs("claude-haiku-4-5", None)
            )]
        );

        let result = json!({ "type": "user", "parent_tool_use_id": null, "message": { "role": "user",
            "content": [{ "type": "tool_result", "tool_use_id": "toolu_1", "content": "Found it" }] } });
        assert!(matches!(
            parser.parse_line(&result.to_string()).as_slice(),
            [ProviderOutput::ToolCallCompleted { detail: Some(detail), .. }]
                if *detail == runs("claude-haiku-4-5", None)
        ));
    }

    #[test]
    fn a_parser_without_a_turn_model_tells_nothing_of_subagents() {
        let mut parser = ClaudeParser::new(None);
        let outputs = parser.parse_line(&agent_call("toolu_1", None, json!({ "model": "haiku" })));
        assert!(details(&outputs).is_empty());
    }

    #[test]
    fn the_agent_is_told_how_to_choose_its_subagents_only_when_it_decides() {
        let thread = request("/repo", "hi");
        assert_eq!(system_prompt(&thread, None, ToolSearch::Deferred), None);
        let decides = TurnRequest {
            effort: Some(Effort::High),
            instructions: Some("hoshi's own".into()),
            subagent_effort: SubagentEffort::AgentDecides,
            ..request("/repo", "hi")
        };
        let told = system_prompt(&decides, None, ToolSearch::Deferred).unwrap();
        assert!(told.starts_with("hoshi's own\n"), "{told}");
        assert!(
            told.contains("Claude Opus 5.5 (claude-opus-5-5) at high effort"),
            "{told}"
        );
        assert!(!told.contains("delegate_task"));
        let launch = Launch {
            model: turn_model(&decides, None),
            instructions: Some(told),
            ..launch("/repo")
        };
        let mut queue = QueueState {
            turn_open: false,
            ..QueueState::default()
        };
        assert!(next_turn(&launch, None, Some("s1"), &mut queue, &decides).is_some());
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
        parser.parse_line(
            &json!({ "type": "command_lifecycle", "command_uuid": "m2", "state": "queued" })
                .to_string(),
        );
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
    fn the_agent_answers_its_turn_once_it_writes_not_once_it_starts() {
        let mut parser = ClaudeParser::new(None);

        parser.parse_line(
            &json!({ "type": "system", "subtype": "init", "session_id": "s1" }).to_string(),
        );
        assert!(!parser.queue.state().answering);

        parser.parse_line(
            &json!({ "type": "stream_event", "event": { "type": "message_start" } }).to_string(),
        );
        assert!(parser.queue.state().answering);
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
            tool_search: ToolSearch::Deferred,
            mcp: None,
            full_access: false,
        }
    }

    fn request(cwd: &str, prompt: &str) -> TurnRequest {
        TurnRequest {
            cwd: cwd.into(),
            prompt: prompt.into(),
            attachments: Vec::new(),
            earlier_media: Vec::new(),
            resume_session_id: Some("s1".into()),
            resume_session_at: None,
            fork_session: false,
            handoff: None,
            model: Some("claude-opus-5-5".into()),
            effort: None,
            fast_mode: false,
            ultracode: false,
            access_mode: AccessMode::AcceptEdits,
            plan_mode: false,
            mcp: None,
            instructions: None,
            subagent_effort: SubagentEffort::Thread,
            environment: Vec::new(),
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
        let forked = TurnRequest {
            fork_session: true,
            ..request("/repo", "hi")
        };
        assert!(next(forked, None).is_none());
        let unknown_effort = TurnRequest {
            model: Some("claude-custom".into()),
            ..request("/repo", "hi")
        };
        assert!(next(unknown_effort, None).is_none());
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

    fn agent_roster(run: &str) -> Value {
        json!({ "task_id": "a1", "run_id": run, "task_type": "local_agent",
            "subagent_type": "general-purpose", "description": "Wait in the background" })
    }

    /// The `sleep` the subagent `a1` started in the background, which belongs to it.
    fn subagent_sleep() -> Value {
        json!({ "task_id": "b1", "run_id": "run-b1", "task_type": "local_bash",
            "description": "Sleep for 20 seconds", "parent_task_id": "a1" })
    }

    fn agent_ended(run: &str, summary: &str) -> Value {
        json!({ "type": "system", "subtype": "task_notification", "task_id": "a1",
            "run_id": run, "tool_use_id": "toolu_agent", "status": "completed",
            "summary": summary })
    }

    fn wake_result(run: &str) -> Value {
        json!({ "type": "result", "subtype": "success", "is_error": false,
            "origin": { "kind": "task-notification", "producer": "session-task", "runId": run } })
    }

    /// The agent starts the subagent `a1` in the background and ends its run; `a1` starts
    /// `sleep` in the background.
    fn subagent_starts_sleep(parser: &mut ClaudeParser) -> Vec<ProviderOutput> {
        feed(
            parser,
            &[
                json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
                roster(json!([agent_roster("run-a1")])),
                json!({ "type": "system", "subtype": "task_started", "task_id": "a1",
                    "run_id": "run-a1", "tool_use_id": "toolu_agent", "is_backgrounded": true,
                    "task_type": "local_agent" }),
                json!({ "type": "user", "parent_tool_use_id": null,
                    "tool_use_result": { "isAsync": true, "status": "async_launched" },
                    "message": { "role": "user", "content": [{ "type": "tool_result",
                        "tool_use_id": "toolu_agent",
                        "content": "Async agent launched successfully." }] } }),
                json!({ "type": "result", "subtype": "success", "is_error": false }),
                roster(json!([agent_roster("run-a1"), subagent_sleep()])),
                json!({ "type": "system", "subtype": "task_started", "task_id": "b1",
                    "run_id": "run-b1", "owned_by_subagent": true, "tool_use_id": "toolu_sleep",
                    "is_backgrounded": true, "parent_task_id": "a1", "task_type": "local_bash" }),
            ],
        )
    }

    /// `sleep` ends: Claude Code tells `a1`, which runs again, answers and ends.
    fn sleep_ends_and_subagent_runs_again() -> Vec<Value> {
        vec![
            json!({ "type": "system", "subtype": "task_notification", "task_id": "b1",
                "run_id": "run-b1", "tool_use_id": "toolu_sleep", "status": "completed",
                "summary": "Background command \"Sleep for 20 seconds\" completed (exit code 0)" }),
            roster(json!([])),
            roster(json!([agent_roster("run-a1-again")])),
            json!({ "type": "system", "subtype": "task_started", "task_id": "a1",
                "run_id": "run-a1-again", "tool_use_id": "toolu_agent", "is_backgrounded": true,
                "task_type": "local_agent" }),
            agent_ended("run-a1-again", "waited"),
            roster(json!([])),
        ]
    }

    fn told_ended(outputs: &[ProviderOutput]) -> Vec<String> {
        outputs
            .iter()
            .filter_map(|output| match output {
                ProviderOutput::BackgroundTaskEnded { task_id, .. } => Some(task_id.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_background_task_of_a_subagent_wakes_that_subagent_and_not_the_agent() {
        let mut parser = ClaudeParser::new(None);
        let mut outputs = subagent_starts_sleep(&mut parser);
        outputs.extend(feed(
            &mut parser,
            &[
                agent_ended("run-a1", "The sleep still runs; I reply once it ends."),
                roster(json!([subagent_sleep()])),
                json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
                wake_result("run-a1"),
            ],
        ));
        assert_eq!(completions(&outputs), [TurnOutcome::Completed]);
        let queue = parser.queue.state();
        assert!(!queue.closing && !queue.turn_open, "{outputs:#?}");
        drop(queue);

        let ended = feed(&mut parser, &sleep_ends_and_subagent_runs_again());
        assert!(told_ended(&ended).is_empty(), "{ended:#?}");
        assert!(!parser.queue.state().closing, "Claude still answers a1");

        let woke = feed(
            &mut parser,
            &[
                json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
                json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                    "id": "msg_9", "content": [{ "type": "text", "text": "It waited." }] } }),
                wake_result("run-a1-again"),
            ],
        );
        assert_eq!(woke[0], ProviderOutput::Woke);
        assert_eq!(told_ended(&woke), ["a1"]);
        assert_eq!(completions(&woke), [TurnOutcome::Completed]);
        assert!(parser.queue.state().closing, "nothing is left running");
    }

    #[test]
    fn a_subagent_run_again_for_its_own_task_holds_the_turn() {
        let mut parser = ClaudeParser::new(None);
        subagent_starts_sleep(&mut parser);
        let mut lines = vec![
            agent_ended("run-a1", "The sleep still runs; I reply once it ends."),
            roster(json!([subagent_sleep()])),
        ];
        lines.extend(sleep_ends_and_subagent_runs_again().into_iter().take(2));
        lines.push(json!({ "type": "system", "subtype": "init", "session_id": "s1" }));
        lines.push(wake_result("run-a1"));
        let waits = feed(&mut parser, &lines);
        assert!(completions(&waits).is_empty(), "{waits:#?}");
        assert!(!parser.queue.state().closing);

        let mut rest = sleep_ends_and_subagent_runs_again().split_off(2);
        rest.push(json!({ "type": "system", "subtype": "init", "session_id": "s1" }));
        rest.push(wake_result("run-a1-again"));
        let answered = feed(&mut parser, &rest);
        assert!(!answered.contains(&ProviderOutput::Woke), "{answered:#?}");
        assert_eq!(completions(&answered), [TurnOutcome::Completed]);
        assert!(parser.queue.state().closing);
    }

    #[test]
    fn a_task_that_ends_while_a_run_goes_on_is_answered_by_that_run() {
        let mut parser = ClaudeParser::new(None);
        let mut lines = sleep_in_background();
        lines.extend([
            json!({ "type": "system", "subtype": "task_started", "task_id": "f1",
                "run_id": "run-f1", "tool_use_id": "toolu_fg", "is_backgrounded": false,
                "task_type": "local_bash" }),
            json!({ "type": "system", "subtype": "task_notification", "task_id": "b1",
                "run_id": "run-b1", "tool_use_id": "toolu_bg", "status": "completed",
                "summary": "Background command \"Sleep then echo done\" completed (exit code 0)" }),
            roster(json!([])),
            json!({ "type": "system", "subtype": "task_notification", "task_id": "f1",
                "run_id": "run-f1", "tool_use_id": "toolu_fg", "status": "completed" }),
            json!({ "type": "user", "parent_tool_use_id": null,
                "message": { "role": "user", "content": [{ "type": "tool_result",
                    "tool_use_id": "toolu_fg", "content": "late" }] } }),
        ]);
        let before = feed(&mut parser, &lines);
        assert!(told_ended(&before).is_empty(), "{before:#?}");

        let taken_in = feed(
            &mut parser,
            &[
                json!({ "type": "user", "parent_tool_use_id": null, "isReplay": true,
                "message": { "role": "user", "content":
                    "<task-notification>\n<task-id>b1</task-id>\n<status>completed</status>\n</task-notification>" },
                "origin": { "kind": "task-notification", "producer": "session-task",
                    "runId": "run-b1" } }),
            ],
        );
        assert_eq!(taken_in, [sleep_ended_output()]);

        let ended = feed(
            &mut parser,
            &[
                json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                    "id": "msg_2", "content": [{ "type": "text", "text": "Both ran." }] } }),
                run_result(),
            ],
        );
        assert!(!ended.contains(&ProviderOutput::RunEnded { run: 1 }));
        assert_eq!(completions(&ended), [TurnOutcome::Completed]);
    }

    #[test]
    fn a_run_about_a_task_answers_the_tasks_that_ended_before_it() {
        let mut parser = ClaudeParser::new(None);
        let ended = feed(
            &mut parser,
            &[
                roster(json!([
                    { "task_id": "a", "task_type": "local_agent" },
                    { "task_id": "b", "task_type": "local_agent" }])),
                json!({ "type": "system", "subtype": "task_notification", "task_id": "a",
                    "run_id": "run-a", "status": "completed" }),
                json!({ "type": "system", "subtype": "task_notification", "task_id": "b",
                    "run_id": "run-b", "status": "completed" }),
                roster(json!([])),
                json!({ "type": "result", "subtype": "success", "is_error": false }),
            ],
        );
        assert_eq!(told_ended(&ended), ["a"]);

        let answered = feed(&mut parser, &[wake_result("run-b")]);
        assert_eq!(told_ended(&answered), ["b"]);
        assert_eq!(completions(&answered), [TurnOutcome::Completed]);
    }

    #[test]
    fn a_resumed_turn_outlasts_the_run_about_a_task_an_earlier_process_left() {
        let mut parser = ClaudeParser::new(None);
        let left = feed(
            &mut parser,
            &[
                json!({ "type": "system", "subtype": "task_notification", "task_id": "b9",
                    "tool_use_id": "toolu_old", "status": "stopped", "output_file": "",
                    "summary": "Background shell command didn't finish before the previous session ended" }),
                json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
                json!({ "type": "result", "subtype": "success", "is_error": false,
                    "num_turns": 0, "result": "", "origin": { "kind": "task-notification" } }),
            ],
        );
        assert!(completions(&left).is_empty(), "{left:#?}");
        assert!(!left.contains(&ProviderOutput::RunEnded { run: 1 }));
        assert!(!parser.queue.state().answering);

        let answered = feed(
            &mut parser,
            &[
                json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
                json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                    "id": "msg_1", "content": [{ "type": "text", "text": "hello there" }] } }),
                json!({ "type": "result", "subtype": "success", "is_error": false,
                    "num_turns": 1, "result": "hello there" }),
            ],
        );
        assert!(answered.contains(&ProviderOutput::AssistantMessage {
            item_id: "msg_1:0".into(),
            text: "hello there".into(),
        }));
        assert_eq!(completions(&answered), [TurnOutcome::Completed]);
    }

    /// Sends the next turn to a process whose command left running already ended, before
    /// Claude Code answered it; returns the id of the turn's message.
    fn next_turn_behind_the_wake(parser: &mut ClaudeParser) -> String {
        feed(
            parser,
            &[json!({ "type": "system", "subtype": "init", "session_id": "s1", "cwd": "/repo" })],
        );
        leave_sleep_running(parser);
        feed(parser, &sleep_ended());
        let lines = next_turn(
            &launch("/repo"),
            None,
            Some("s1"),
            &mut parser.queue.state(),
            &request("/repo", "and now?"),
        )
        .unwrap();
        let message: Value = serde_json::from_str(&lines[0]).unwrap();
        message["uuid"].as_str().unwrap().to_owned()
    }

    #[test]
    fn a_queued_message_outlasts_the_wake_claude_code_runs_before_it() {
        let mut parser = ClaudeParser::new(None);
        let message_id = next_turn_behind_the_wake(&mut parser);

        let queued = json!({ "type": "command_lifecycle", "command_uuid": message_id,
            "state": "queued" });
        let mut lines = vec![queued];
        lines.extend(answer_to_sleep());
        let woke = feed(&mut parser, &lines);
        assert!(completions(&woke).is_empty(), "{woke:#?}");
        assert!(woke.contains(&sleep_ended_output()));
        assert!(!parser.queue.state().answering);

        let echo = json!([message_id]);
        let answered = feed(
            &mut parser,
            &[
                json!({ "type": "command_lifecycle", "command_uuid": message_id,
                    "state": "started" }),
                json!({ "type": "system", "subtype": "init", "session_id": "s1", "cwd": "/repo",
                    "user_message_uuids": echo }),
                json!({ "type": "assistant", "parent_tool_use_id": null,
                    "user_message_uuid": message_id, "message": { "id": "msg_3",
                    "content": [{ "type": "text", "text": "Done sleeping." }] } }),
                json!({ "type": "result", "subtype": "success", "is_error": false,
                    "num_turns": 1, "result": "Done sleeping.",
                    "user_message_uuid": message_id, "user_message_uuids": echo }),
            ],
        );
        assert_eq!(completions(&answered), [TurnOutcome::Completed]);
        assert!(parser.queue.state().closing, "nothing is left running");
    }

    #[test]
    fn a_cli_that_names_none_of_our_messages_ends_the_turn_at_any_result() {
        let mut parser = ClaudeParser::new(None);
        next_turn_behind_the_wake(&mut parser);

        let woke = feed(&mut parser, &answer_to_sleep());
        assert_eq!(completions(&woke), [TurnOutcome::Completed]);
    }

    #[test]
    fn an_interrupted_turn_ends_at_its_result_as_interrupted() {
        let mut parser = ClaudeParser::new(None);
        parser.queue.state().interrupted = true;
        parser.queue.state().waiting.insert("m2".into());
        let result = json!({ "type": "result", "subtype": "error_during_execution",
            "is_error": true })
        .to_string();

        assert_eq!(
            parser.parse_line(&result),
            [
                ProviderOutput::MessagesDropped {
                    message_ids: vec!["m2".into()]
                },
                ProviderOutput::TurnCompleted {
                    outcome: TurnOutcome::Interrupted
                }
            ]
        );
    }

    fn lifecycle(message_id: &str, state: &str) -> Value {
        json!({ "type": "command_lifecycle", "command_uuid": message_id, "state": state })
    }

    /// A parser whose turn began with the message `m1` and was sent `m2` since.
    fn steered_parser() -> ClaudeParser {
        let parser = ClaudeParser::new(None);
        {
            let mut queue = parser.queue.state();
            queue.prompt = Some("m1".into());
            queue.sent = HashSet::from(["m1".into(), "m2".into()]);
            queue.waiting.insert("m2".into());
        }
        parser
    }

    fn dropped(outputs: &[ProviderOutput]) -> Vec<String> {
        outputs
            .iter()
            .flat_map(|output| match output {
                ProviderOutput::MessagesDropped { message_ids } => message_ids.clone(),
                _ => Vec::new(),
            })
            .collect()
    }

    #[test]
    fn a_message_claude_code_let_go_of_unread_is_dropped_and_ends_a_waiting_turn() {
        let mut parser = steered_parser();
        let ran = feed(
            &mut parser,
            &[
                lifecycle("m1", "queued"),
                lifecycle("m1", "started"),
                lifecycle("m2", "queued"),
                run_result(),
            ],
        );
        assert!(
            ran.contains(&ProviderOutput::RunEnded { run: 1 }),
            "{ran:#?}"
        );

        let let_go = feed(&mut parser, &[lifecycle("m2", "cancelled")]);
        assert_eq!(dropped(&let_go), ["m2"]);
        assert_eq!(completions(&let_go), [TurnOutcome::Completed]);
    }

    #[test]
    fn a_message_named_by_the_run_was_taken_in() {
        let mut parser = steered_parser();
        let outputs = feed(
            &mut parser,
            &[
                lifecycle("m1", "started"),
                lifecycle("m2", "queued"),
                json!({ "type": "system", "subtype": "status", "status": "requesting",
                    "user_message_uuids": ["m2"] }),
                run_result(),
            ],
        );
        assert!(dropped(&outputs).is_empty());
        assert_eq!(completions(&outputs), [TurnOutcome::Completed]);
    }

    #[test]
    fn a_cli_that_never_tells_of_a_message_does_not_hold_the_turn_for_it() {
        let mut parser = steered_parser();
        let outputs = feed(&mut parser, &[run_result()]);
        assert_eq!(completions(&outputs), [TurnOutcome::Completed]);
        assert!(dropped(&outputs).is_empty());
    }

    #[test]
    fn a_turn_whose_message_completes_without_a_run_fails() {
        let mut parser = steered_parser();
        let outputs = feed(
            &mut parser,
            &[lifecycle("m1", "queued"), lifecycle("m1", "completed")],
        );
        assert!(matches!(
            completions(&outputs).as_slice(),
            [TurnOutcome::Failed {
                reason: TurnFailure::ProviderError { .. }
            }]
        ));
        assert_eq!(dropped(&outputs), ["m2"]);
    }

    #[test]
    fn a_stop_agreed_to_before_any_run_began_ends_the_turn_at_once() {
        let answer = json!({ "type": "control_response", "response": {
            "subtype": "success", "request_id": "hoshi-interrupt" } });
        let mut parser = steered_parser();
        parser.queue.state().interrupted = true;
        let stopped = feed(&mut parser, &[lifecycle("m1", "queued"), answer.clone()]);
        assert_eq!(completions(&stopped), [TurnOutcome::Interrupted]);
        assert_eq!(dropped(&stopped), ["m2"]);

        let mut running = steered_parser();
        running.queue.state().interrupted = true;
        let waits = feed(&mut running, &[lifecycle("m1", "started"), answer]);
        assert!(completions(&waits).is_empty(), "{waits:#?}");
        let result = json!({ "type": "result", "subtype": "error_during_execution",
            "is_error": true });
        assert_eq!(
            completions(&feed(&mut running, &[result])),
            [TurnOutcome::Interrupted]
        );
    }

    #[test]
    fn what_a_cut_run_streamed_lands_as_items_before_the_run_ends() {
        let mut parser = steered_parser();
        let delta = |index: u64, delta: Value| stream_delta("msg1", index, delta);
        let landed = json!({ "type": "assistant", "message": { "id": "msg1",
            "content": [{ "type": "thinking", "thinking": "Plan first." }] } });
        let cut = json!({ "type": "result", "subtype": "success", "is_error": false,
            "terminal_reason": "aborted_streaming", "result": "",
            "user_message_uuids": ["m1"] });
        let outputs = feed(
            &mut parser,
            &[
                lifecycle("m1", "started"),
                delta(
                    0,
                    json!({ "type": "thinking_delta", "thinking": "Plan first." }),
                ),
                landed,
                delta(
                    1,
                    json!({ "type": "thinking_delta", "thinking": "The connection " }),
                ),
                delta(
                    1,
                    json!({ "type": "thinking_delta", "thinking": "stays local." }),
                ),
                delta(2, json!({ "type": "text_delta", "text": "So far" })),
                lifecycle("m2", "queued"),
                lifecycle("m2", "started"),
                cut,
            ],
        );
        let items: Vec<_> = outputs
            .iter()
            .filter_map(|output| match output {
                ProviderOutput::Reasoning { item_id, text }
                | ProviderOutput::AssistantMessage { item_id, text } => {
                    Some((item_id.as_str(), text.as_str()))
                }
                ProviderOutput::RunEnded { .. } => Some(("run ended", "")),
                _ => None,
            })
            .collect();
        assert_eq!(
            items,
            [
                ("msg1:0", "Plan first."),
                ("msg1:1", "The connection stays local."),
                ("msg1:2", "So far"),
                ("run ended", ""),
            ]
        );
    }

    fn stream_delta(message_id: &str, index: u64, delta: Value) -> Value {
        json!({ "type": "stream_event", "api_message_id": message_id,
            "event": { "type": "content_block_delta", "index": index, "delta": delta } })
    }

    fn landed(outputs: &[ProviderOutput]) -> Vec<(&str, &str)> {
        outputs
            .iter()
            .filter_map(|output| match output {
                ProviderOutput::Reasoning { item_id, text }
                | ProviderOutput::AssistantMessage { item_id, text } => {
                    Some((item_id.as_str(), text.as_str()))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn what_a_failed_api_call_streamed_is_not_kept() {
        let mut parser = steered_parser();
        let start = |id: &str| {
            json!({ "type": "stream_event", "event": { "type": "message_start",
                "message": { "id": id } } })
        };
        let retry = json!({ "type": "system", "subtype": "api_retry", "attempt": 1,
            "max_retries": 10, "retry_delay_ms": 500, "error_status": 529, "error": "overloaded" });
        let done = json!({ "type": "result", "subtype": "success", "is_error": false,
            "terminal_reason": "completed", "result": "Here is the plan.",
            "user_message_uuids": ["m1"] });
        let outputs = feed(
            &mut parser,
            &[
                lifecycle("m1", "started"),
                start("msgA"),
                stream_delta(
                    "msgA",
                    0,
                    json!({ "type": "text_delta", "text": "Here is" }),
                ),
                retry,
                stream_delta("msgB", 0, json!({ "type": "text_delta", "text": "Draft" })),
                start("msgC"),
                stream_delta(
                    "msgC",
                    0,
                    json!({ "type": "text_delta", "text": "Here is" }),
                ),
                stream_delta("msgC", 1, json!({ "type": "text_delta", "text": "  " })),
                json!({ "type": "assistant", "message": { "id": "msgC",
                    "content": [{ "type": "text", "text": "Here is the plan." }] } }),
                done,
            ],
        );
        assert_eq!(landed(&outputs), [("msgC:0", "Here is the plan.")]);
    }

    #[test]
    fn a_stop_keeps_what_the_stream_wrote_and_then_ends_the_turn() {
        let mut parser = steered_parser();
        feed(
            &mut parser,
            &[
                lifecycle("m1", "started"),
                stream_delta(
                    "msg1",
                    0,
                    json!({ "type": "thinking_delta", "thinking": "Checking" }),
                ),
            ],
        );
        parser.queue.state().interrupted = true;
        let stopped = feed(
            &mut parser,
            &[
                json!({ "type": "result", "subtype": "error_during_execution",
                "is_error": true }),
            ],
        );
        assert_eq!(landed(&stopped), [("msg1:0", "Checking")]);
        assert!(matches!(
            stopped.last(),
            Some(ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Interrupted
            })
        ));
    }

    #[test]
    fn a_run_a_message_cut_short_is_not_the_end_of_the_turn() {
        let mut parser = steered_parser();
        let cut = json!({ "type": "result", "subtype": "success", "is_error": false,
            "terminal_reason": "aborted_streaming", "result": "1\n2\n3",
            "user_message_uuids": ["m1"] });
        let outputs = feed(
            &mut parser,
            &[
                lifecycle("m1", "started"),
                lifecycle("m2", "queued"),
                lifecycle("m2", "started"),
                cut,
                lifecycle("m1", "cancelled"),
            ],
        );
        assert!(completions(&outputs).is_empty(), "{outputs:#?}");
        assert!(
            outputs
                .iter()
                .any(|output| matches!(output, ProviderOutput::RunEnded { .. })),
            "{outputs:#?}"
        );

        let answered = json!({ "type": "result", "subtype": "success", "is_error": false,
            "terminal_reason": "completed", "result": "ok", "user_message_uuids": ["m2"] });
        assert_eq!(
            completions(&feed(&mut parser, &[answered])),
            [TurnOutcome::Completed]
        );
    }

    #[test]
    fn a_turn_refused_for_its_plan_limit_fails_with_when_it_resets() {
        let mut parser = ClaudeParser::new(None);
        let outputs = feed(
            &mut parser,
            &[
                json!({ "type": "rate_limit_event", "rate_limit_info": { "status": "rejected",
                    "rateLimitType": "five_hour", "resetsAt": 1_791_804_000 } }),
                json!({ "type": "assistant", "parent_tool_use_id": null, "error": "rate_limit",
                    "message": { "id": "m_1", "model": "<synthetic>",
                        "content": [{ "type": "text", "text": "You've hit your limit" }] } }),
                json!({ "type": "result", "subtype": "success", "is_error": true,
                    "api_error_status": 429, "terminal_reason": "api_error",
                    "result": "You've hit your limit" }),
            ],
        );
        assert_eq!(
            completions(&outputs),
            [TurnOutcome::Failed {
                reason: TurnFailure::UsageLimited {
                    detail: "success: You've hit your limit".into(),
                    resets_at: Some("2026-10-12T11:20:00Z".into()),
                }
            }]
        );
    }

    #[test]
    fn a_retried_api_call_is_told() {
        let mut parser = ClaudeParser::new(None);
        let retry = json!({ "type": "system", "subtype": "api_retry", "attempt": 1,
            "max_retries": 10, "retry_delay_ms": 506, "error_status": null, "error": "unknown" });
        assert!(matches!(
            feed(&mut parser, &[retry]).as_slice(),
            [ProviderOutput::Retrying {
                attempt: 1,
                max_attempts: Some(10),
                retry_in_ms: Some(506),
                ..
            }]
        ));
    }

    fn replies(outputs: &[ProviderOutput]) -> Vec<String> {
        outputs
            .iter()
            .filter_map(|output| match output {
                ProviderOutput::AssistantMessage { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn what_a_local_command_printed_is_the_turn_s_reply() {
        let mut replayed = ClaudeParser::new(None);
        let outputs = feed(
            &mut replayed,
            &[
                json!({ "type": "user", "parent_tool_use_id": null, "isReplay": true,
                    "message": { "role": "user",
                        "content": "<local-command-stdout>Context: 3% used</local-command-stdout>" } }),
                json!({ "type": "result", "subtype": "success", "is_error": false,
                    "num_turns": 0, "local_command": "context", "result": "" }),
            ],
        );
        assert_eq!(replies(&outputs), ["Context: 3% used"]);
        assert_eq!(completions(&outputs), [TurnOutcome::Completed]);

        let mut told = ClaudeParser::new(None);
        let outputs = feed(
            &mut told,
            &[
                json!({ "type": "result", "subtype": "success", "is_error": false,
                "num_turns": 0, "local_command": "cost", "result": "Total cost: $0.00",
                "uuid": "r1" }),
            ],
        );
        assert_eq!(replies(&outputs), ["Total cost: $0.00"]);

        let mut written = ClaudeParser::new(None);
        let outputs = feed(
            &mut written,
            &[
                json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                    "id": "m_1", "model": "<synthetic>",
                    "content": [{ "type": "text", "text": "Total cost: $0.00" }] } }),
                json!({ "type": "result", "subtype": "success", "is_error": false,
                    "num_turns": 0, "local_command": "cost", "result": "Total cost: $0.00" }),
            ],
        );
        assert_eq!(replies(&outputs), ["Total cost: $0.00"]);
    }

    #[test]
    fn a_turn_on_another_model_or_effort_switches_the_process_in_place() {
        let launch = launch("/repo");
        let mut queue = QueueState {
            turn_open: false,
            access: Access {
                mode: AccessMode::AcceptEdits,
                plan_mode: false,
            },
            ..QueueState::default()
        };
        let haiku = TurnRequest {
            model: Some("claude-haiku-5-5".into()),
            effort: Some(Effort::High),
            ..request("/repo", "hi")
        };
        let lines = next_turn(&launch, None, Some("s1"), &mut queue, &haiku).unwrap();
        let control: Vec<Value> = lines[..2]
            .iter()
            .map(|line| serde_json::from_str::<Value>(line).unwrap()["request"].clone())
            .collect();
        assert_eq!(
            control,
            [
                json!({ "subtype": "set_model", "model": "claude-haiku-5-5" }),
                json!({ "subtype": "apply_flag_settings", "settings": {
                    "effortLevel": "high", "fastMode": false, "ultracode": false } }),
            ]
        );
        assert_eq!(lines.len(), 3);
        assert_eq!(queue.model, Some(turn_model(&haiku, None)));

        queue.turn_open = false;
        let again = next_turn(&launch, None, Some("s1"), &mut queue, &haiku).unwrap();
        assert_eq!(again.len(), 1, "{again:?}");

        queue.turn_open = false;
        queue.model_unsure = true;
        assert!(next_turn(&launch, None, Some("s1"), &mut queue, &haiku).is_none());
    }

    #[test]
    fn a_switch_claude_code_refused_leaves_the_model_unsure() {
        let mut parser = ClaudeParser::new(None);
        feed(
            &mut parser,
            &[
                json!({ "type": "control_response", "response": { "subtype": "error",
                "request_id": "hoshi-model", "error": "unknown model" } }),
            ],
        );
        assert!(parser.queue.state().model_unsure);
    }

    #[test]
    fn subagents_inherit_the_model_a_turn_switched_to() {
        let mut parser = ClaudeParser::for_turn(Some("opus"), Some(Effort::Max));
        parser.queue.state().retarget = Some((Some("claude-haiku-5-5".into()), Some(Effort::Low)));
        let call = parser.parse_line(&agent_call("toolu_1", None, json!({ "prompt": "Look" })));
        assert_eq!(
            details(&call),
            [(
                None,
                "toolu_1".into(),
                runs("claude-haiku-5-5", Some(Effort::Low))
            )]
        );
    }

    fn collecting_sink() -> (OutputSink, mpsc::Receiver<ProviderOutput>) {
        let (sender, receiver) = mpsc::channel();
        let sink: OutputSink = Box::new(move |output| {
            let _ = sender.send(output);
        });
        (sink, receiver)
    }

    #[test]
    fn a_process_that_dies_drops_what_waited_and_tells_a_missing_session() {
        let (sink, outputs) = collecting_sink();
        let queue = Queue::default();
        queue.state().waiting.insert("m2".into());
        let mut forward = SharedSink::new(sink).for_attempt(0, queue);
        forward(ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Failed {
                reason: TurnFailure::ProcessExited {
                    detail: "exit code: 1: No conversation found with session ID: s9".into(),
                },
            },
        });
        let told: Vec<_> = outputs.try_iter().collect();
        assert_eq!(dropped(&told), ["m2"]);
        assert!(matches!(
            completions(&told).as_slice(),
            [TurnOutcome::Failed {
                reason: TurnFailure::ResumeFailed { .. }
            }]
        ));
    }

    /// A command that runs `windows` with `cmd` or `unix` with `sh`, from a script of its own.
    fn script(windows: &str, unix: &str) -> Command {
        let dir = std::env::temp_dir().join(format!("hoshi-claude-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        if cfg!(windows) {
            let path = dir.join("cli.cmd");
            std::fs::write(&path, format!("@echo off\r\n{windows}\r\n")).unwrap();
            let mut command = Command::new("cmd");
            command.arg("/C").arg(path);
            command
        } else {
            let path = dir.join("cli.sh");
            std::fs::write(&path, unix).unwrap();
            let mut command = Command::new("sh");
            command.arg(path);
            command
        }
    }

    const SILENT: (&str, &str) = ("ping -n 30 127.0.0.1 > nul", "sleep 30");

    fn starting(
        command: Command,
        mcp: bool,
        retries: bool,
        deadlines: Deadlines,
    ) -> mpsc::Receiver<ProviderOutput> {
        let (sink, outputs) = collecting_sink();
        let startup = Arc::new(Startup {
            command,
            prompt: "hi".into(),
            attachments: Vec::new(),
            earlier_media: Vec::new(),
            mcp: mcp.then(|| McpServer {
                url: "http://127.0.0.1:9/mcp".into(),
                authorization: "Bearer x".into(),
            }),
            model: None,
            effort: None,
            queue: Queue::default(),
            requests: Requests::default(),
            live_output: LiveOutput::default(),
            sink: SharedSink::new(sink),
            process: LiveProcess::default(),
            retries,
            deadlines,
        });
        startup.launch(0).unwrap();
        outputs
    }

    /// What the turn told until it ended.
    fn until_completed(outputs: &mpsc::Receiver<ProviderOutput>) -> Vec<ProviderOutput> {
        let mut told = Vec::new();
        while let Ok(output) = outputs.recv_timeout(Duration::from_secs(20)) {
            let done = matches!(output, ProviderOutput::TurnCompleted { .. });
            told.push(output);
            if done {
                break;
            }
        }
        told
    }

    #[test]
    fn a_cli_that_prints_nothing_in_time_fails_to_start() {
        let deadlines = Deadlines {
            boot: Duration::from_millis(300),
            mcp: Duration::from_secs(10),
        };
        let told = until_completed(&starting(
            script(SILENT.0, SILENT.1),
            false,
            false,
            deadlines,
        ));
        assert!(matches!(
            completions(&told).as_slice(),
            [TurnOutcome::Failed {
                reason: TurnFailure::StartupTimedOut { .. }
            }]
        ));
    }

    #[test]
    fn a_cli_that_never_connects_hoshi_s_server_fails_to_start() {
        let deadlines = Deadlines {
            boot: Duration::from_secs(10),
            mcp: Duration::from_millis(300),
        };
        let queued = r#"{"type":"command_lifecycle","command_uuid":"x","state":"queued"}"#;
        let command = script(
            &format!("echo {queued}\r\n{}", SILENT.0),
            &format!("echo '{queued}'\n{}", SILENT.1),
        );
        let told = until_completed(&starting(command, true, false, deadlines));
        assert!(matches!(
            completions(&told).as_slice(),
            [TurnOutcome::Failed {
                reason: TurnFailure::StartupTimedOut { detail }
            }] if detail.contains("MCP")
        ));
    }

    #[test]
    fn a_resumed_session_that_does_not_start_gets_a_second_try() {
        let deadlines = Deadlines {
            boot: Duration::from_millis(1500),
            mcp: Duration::from_secs(10),
        };
        let init = r#"{"type":"system","subtype":"init","session_id":"s2"}"#;
        let command = script(
            &format!(
                "if exist \"%~dp0tried\" (\r\necho {init}\r\n) else (\r\ntype nul > \"%~dp0tried\"\r\n{}\r\n)",
                SILENT.0
            ),
            &format!(
                "dir=$(dirname \"$0\")\nif [ -e \"$dir/tried\" ]; then echo '{init}'; else touch \"$dir/tried\"; {}; fi",
                SILENT.1
            ),
        );
        let told = until_completed(&starting(command, false, true, deadlines));
        assert!(
            told.contains(&ProviderOutput::SessionStarted {
                session_id: "s2".into()
            }),
            "{told:#?}"
        );
        assert!(matches!(
            completions(&told).as_slice(),
            [TurnOutcome::Failed {
                reason: TurnFailure::ProcessExited { .. }
            }]
        ));
    }

    #[test]
    fn a_stopped_turn_s_process_is_killed_once_its_grace_ends_even_with_stdin_closed() {
        let (sink, outputs) = collecting_sink();
        let (silent_windows, silent_unix) = SILENT;
        let process = spawn_turn(
            script(silent_windows, silent_unix),
            String::new(),
            sink,
            |_, _| Vec::new(),
            || true,
        )
        .unwrap();
        process.stdin().close();
        let live = LiveProcess::default();
        live.replace(process);
        kill_after(live, Duration::from_millis(100));
        assert!(matches!(
            outputs.recv_timeout(Duration::from_secs(10)),
            Ok(ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Interrupted
            })
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
        assert_eq!(
            session_settings(&model(false, false), ToolSearch::Deferred),
            None
        );
        assert_eq!(
            session_settings(&model(false, false), ToolSearch::Off),
            None
        );
    }

    #[test]
    fn tool_search_on_auto_is_forced_on_so_hoshi_s_tools_stay_deferred() {
        assert_eq!(
            session_settings(&model(true, false), ToolSearch::Auto),
            Some(json!({ "env": { "ENABLE_TOOL_SEARCH": "true" }, "fastMode": true }))
        );
    }

    #[test]
    fn with_tool_search_off_the_instructions_don_t_say_to_load_tools() {
        let own = format!("hoshi has tools{DEFERRED_TOOLS}. More.");
        let thread = TurnRequest {
            instructions: Some(own.clone()),
            ..request("/repo", "hi")
        };
        assert_eq!(
            system_prompt(&thread, None, ToolSearch::Off).as_deref(),
            Some("hoshi has tools. More.")
        );
        assert_eq!(system_prompt(&thread, None, ToolSearch::Auto), Some(own));
    }

    #[test]
    fn fast_mode_and_ultracode_go_through_settings() {
        assert_eq!(
            session_settings(&model(true, false), ToolSearch::Deferred),
            Some(json!({ "fastMode": true }))
        );
        assert_eq!(
            session_settings(&model(false, true), ToolSearch::Deferred),
            Some(json!({ "ultracode": true, "permissions": { "allow": ["Workflow"] } }))
        );
    }

    fn parsed(line: &str) -> Value {
        serde_json::from_str(line).unwrap()
    }

    fn completed(outputs: &[ProviderOutput]) -> Vec<(Option<String>, String, String)> {
        outputs
            .iter()
            .filter_map(|output| match output {
                ProviderOutput::ToolCallCompleted {
                    item_id, output, ..
                } => Some((None, item_id.clone(), output.clone())),
                ProviderOutput::Subagent {
                    tool_use_id,
                    output,
                } => match &**output {
                    ProviderOutput::ToolCallCompleted {
                        item_id, output, ..
                    } => Some((Some(tool_use_id.clone()), item_id.clone(), output.clone())),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    fn workflow_member(index: u64, label: &str, state: &str, result: Option<&str>) -> Value {
        let mut member = json!({ "type": "workflow_agent", "index": index, "label": label,
            "phaseIndex": 1, "phaseTitle": "Reply", "model": "claude-haiku-5-5", "state": state,
            "startedAt": 1_794_765_470_387_u64, "queuedAt": 1_794_765_470_383_u64, "attempt": 1,
            "promptPreview": "Reply with exactly: ONE" });
        if let Some(result) = result {
            member["resultPreview"] = json!(result);
        }
        member
    }

    fn workflow_progress(members: Vec<Value>) -> Value {
        let mut entries = vec![json!({ "type": "workflow_phase", "index": 1, "title": "Reply" })];
        entries.extend(members);
        Value::Array(entries)
    }

    /// The agent launches a workflow in the background; `ack` is the CLI's reading of the
    /// launch.
    fn workflow_launched(ack: Value) -> Vec<Value> {
        let roster_entry = json!({ "task_id": "w1", "run_id": "run-w1",
            "task_type": "local_workflow", "description": "Two agents reply" });
        vec![
            json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
            json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "id": "msg_1",
                "content": [{ "type": "tool_use", "id": "toolu_workflow", "name": "Workflow",
                    "input": { "script": "phase('Reply')" } }] } }),
            roster(json!([roster_entry])),
            json!({ "type": "system", "subtype": "task_started", "task_id": "w1",
                "run_id": "run-w1", "tool_use_id": "toolu_workflow",
                "description": "Two agents reply", "task_type": "local_workflow",
                "workflow_name": "one-two" }),
            json!({ "type": "user", "parent_tool_use_id": null, "tool_use_result": ack,
                "message": { "role": "user", "content": [{ "type": "tool_result",
                    "tool_use_id": "toolu_workflow",
                    "content": "Workflow launched in background. Task ID: w1" }] } }),
            json!({ "type": "system", "subtype": "task_progress", "task_id": "w1",
                "run_id": "run-w1", "tool_use_id": "toolu_workflow",
                "description": "Reply: reply-one", "workflow_progress": workflow_progress(vec![
                    workflow_member(1, "reply-one", "done", Some("ONE")),
                    workflow_member(2, "reply-two", "start", None),
                ]) }),
            json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "id": "msg_2",
                "content": [{ "type": "text", "text": "WF_LAUNCHED" }] } }),
            run_result(),
        ]
    }

    fn workflow_ended() -> Vec<Value> {
        vec![
            json!({ "type": "system", "subtype": "task_notification", "task_id": "w1",
                "run_id": "run-w1", "tool_use_id": "toolu_workflow", "status": "completed",
                "summary": "Dynamic workflow \"Two agents reply\" completed",
                "workflow_progress": workflow_progress(vec![
                    workflow_member(1, "reply-one", "done", Some("ONE")),
                    workflow_member(2, "reply-two", "done", Some("TWO")),
                ]) }),
            roster(json!([])),
            json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
            json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "id": "msg_3",
                "content": [{ "type": "text", "text": "WF_DONE" }] } }),
            wake_result("run-w1"),
        ]
    }

    #[test]
    fn a_workflow_holds_its_call_and_the_turn_and_shows_each_agent() {
        let mut parser = ClaudeParser::new(None);
        let ack = json!({ "status": "async_launched", "taskId": "w1",
            "taskType": "local_workflow", "runId": "wf_1", "transcriptDir": "/repo/wf" });
        let launched = feed(&mut parser, &workflow_launched(ack));

        assert!(completions(&launched).is_empty(), "{launched:#?}");
        assert!(completed(&launched).is_empty(), "{launched:#?}");
        assert!(launched.contains(&ProviderOutput::RunEnded { run: 1 }));
        assert!(launched.contains(&ProviderOutput::ToolOutput {
            item_id: "toolu_workflow".into(),
            output: "Reply\n  ✓ reply-one: ONE\n  ● reply-two".into(),
        }));
        assert_eq!(
            parser.live_output.agent_task("toolu_workflow").as_deref(),
            Some("w1")
        );

        let ended = feed(&mut parser, &workflow_ended());
        assert_eq!(
            completed(&ended),
            [(
                None,
                "toolu_workflow".into(),
                "Dynamic workflow \"Two agents reply\" completed\n\nReply\n  ✓ reply-one: ONE\n  ✓ reply-two: TWO"
                    .into()
            )]
        );
        assert_eq!(told_ended(&ended), ["w1"]);
        assert_eq!(completions(&ended), [TurnOutcome::Completed]);
        assert!(parser.queue.state().closing);
    }

    #[test]
    fn a_workflow_launch_that_only_names_its_run_holds_its_call_too() {
        let mut parser = ClaudeParser::new(None);
        let ack = json!({ "taskType": "local_workflow", "runId": "wf_1",
            "transcriptDir": "/repo/wf" });
        let launched = feed(&mut parser, &workflow_launched(ack));

        assert!(completed(&launched).is_empty(), "{launched:#?}");
        assert_eq!(
            completed(&feed(&mut parser, &workflow_ended()))[0].1,
            "toolu_workflow"
        );
    }

    fn agent_a_input() -> Value {
        json!({ "description": "Agent A", "model": "haiku",
            "prompt": "Reply with exactly: A_FIRST", "run_in_background": true })
    }

    /// The agent starts subagent `a1` in the background, which answers `A_FIRST`; Claude Code
    /// tells the agent, which starts a run about it.
    fn subagent_answers_once(parser: &mut ClaudeParser) -> Vec<ProviderOutput> {
        feed(
            parser,
            &[
                json!({ "type": "system", "subtype": "init", "session_id": "s1", "cwd": "/repo" }),
                json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                    "id": "msg_1", "content": [{ "type": "tool_use", "id": "toolu_agent",
                    "name": "Agent", "input": agent_a_input() }] } }),
                roster(json!([agent_roster("run-a1")])),
                json!({ "type": "system", "subtype": "task_started", "task_id": "a1",
                    "run_id": "run-a1", "tool_use_id": "toolu_agent", "description": "Agent A",
                    "is_backgrounded": true, "spawn_depth": 1, "task_type": "local_agent" }),
                json!({ "type": "user", "parent_tool_use_id": null,
                    "tool_use_result": { "isAsync": true, "status": "async_launched",
                        "agentId": "a1" },
                    "message": { "role": "user", "content": [{ "type": "tool_result",
                        "tool_use_id": "toolu_agent",
                        "content": "Async agent launched successfully." }] } }),
                run_result(),
                parsed(&subagent_frame("toolu_agent", "claude-haiku-5-5")),
                agent_ended("run-a1", "A_FIRST"),
                roster(json!([])),
                json!({ "type": "system", "subtype": "init", "session_id": "s1", "cwd": "/repo" }),
            ],
        )
    }

    /// The agent resumes `a1` with `SendMessage`, whose `task_started` names that call, and
    /// its run ends with `result`.
    fn resume_with_send_message(result: Value) -> Vec<Value> {
        vec![
            json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                "id": "msg_4", "content": [{ "type": "tool_use", "id": "toolu_send",
                "name": "SendMessage", "input": { "to": "a1",
                    "message": "Reply with exactly: A_SECOND" } }] } }),
            roster(json!([agent_roster("run-a1-again")])),
            json!({ "type": "system", "subtype": "task_started", "task_id": "a1",
                "run_id": "run-a1-again", "tool_use_id": "toolu_send",
                "description": "Agent A", "is_backgrounded": true, "spawn_depth": 1,
                "task_type": "local_agent", "prompt": "Reply with exactly: A_SECOND" }),
            json!({ "type": "user", "parent_tool_use_id": null,
                "tool_use_result": { "success": true, "message": "Resuming agent a1",
                    "resumedAgentId": "a1" },
                "message": { "role": "user", "content": [{ "type": "tool_result",
                    "tool_use_id": "toolu_send",
                    "content": "{\"success\":true,\"resumedAgentId\":\"a1\"}" }] } }),
            result,
        ]
    }

    /// The resumed `a1` answers `A_SECOND`, in the transcript of the call that first started
    /// it, and the agent answers in a run of its own.
    fn resumed_subagent_answers() -> Vec<Value> {
        vec![
            parsed(&subagent_frame("toolu_agent", "claude-haiku-5-5")),
            json!({ "type": "system", "subtype": "task_notification", "task_id": "a1",
                "run_id": "run-a1-again", "tool_use_id": "toolu_send",
                "status": "completed", "summary": "A_SECOND" }),
            roster(json!([])),
            json!({ "type": "system", "subtype": "init", "session_id": "s1", "cwd": "/repo" }),
            wake_result("run-a1-again"),
        ]
    }

    fn agent_a_runs_again(outputs: &[ProviderOutput]) -> bool {
        outputs.contains(&ProviderOutput::ToolCallStarted {
            item_id: "toolu_agent".into(),
            name: "Agent".into(),
            input: agent_a_input().to_string(),
        })
    }

    #[test]
    fn a_subagent_resumed_with_send_message_runs_its_first_call_again() {
        let mut parser = ClaudeParser::new(None);
        let first = subagent_answers_once(&mut parser);
        assert_eq!(
            completed(&first),
            [(None, "toolu_agent".into(), "A_FIRST".into())]
        );

        let resumed = feed(
            &mut parser,
            &resume_with_send_message(wake_result("run-a1")),
        );
        assert!(agent_a_runs_again(&resumed), "{resumed:#?}");
        assert_eq!(completed(&resumed)[0].1, "toolu_send");
        assert!(completions(&resumed).is_empty(), "{resumed:#?}");
        assert_eq!(
            parser.live_output.agent_task("toolu_agent").as_deref(),
            Some("a1")
        );

        let answered = feed(&mut parser, &resumed_subagent_answers());
        assert_eq!(
            completed(&answered),
            [(None, "toolu_agent".into(), "A_SECOND".into())]
        );
        assert_eq!(completions(&answered), [TurnOutcome::Completed]);
    }

    #[test]
    fn a_subagent_of_an_earlier_process_resumed_runs_its_first_call_again() {
        let projects = std::env::temp_dir().join(format!("hoshi-resume-{}", std::process::id()));
        let folder = projects.join("-repo");
        std::fs::create_dir_all(folder.join("s1").join("subagents")).unwrap();
        std::fs::write(
            folder
                .join("s1")
                .join("subagents")
                .join("agent-a1.meta.json"),
            json!({ "agentType": "general-purpose", "toolUseId": "toolu_agent" }).to_string(),
        )
        .unwrap();
        let call = json!({ "type": "assistant", "message": { "content": [{ "type": "tool_use",
            "id": "toolu_agent", "name": "Agent", "input": agent_a_input() }] } });
        std::fs::write(folder.join("s1.jsonl"), format!("{call}\n")).unwrap();
        let mut parser = ClaudeParser {
            subagents: Subagents::kept_in(&projects),
            ..ClaudeParser::new(None)
        };

        let mut lines = vec![json!({ "type": "system", "subtype": "init", "session_id": "s1" })];
        lines.extend(resume_with_send_message(run_result()));
        let resumed = feed(&mut parser, &lines);
        std::fs::remove_dir_all(&projects).unwrap();

        assert!(agent_a_runs_again(&resumed), "{resumed:#?}");
        assert!(completions(&resumed).is_empty(), "{resumed:#?}");
        assert_eq!(
            parser.live_output.agent_task("toolu_agent").as_deref(),
            Some("a1")
        );
        let answered = feed(&mut parser, &resumed_subagent_answers());
        assert_eq!(
            completed(&answered),
            [(None, "toolu_agent".into(), "A_SECOND".into())]
        );
        assert_eq!(completions(&answered), [TurnOutcome::Completed]);
    }

    #[test]
    fn a_subagent_s_background_subagent_holds_the_call_that_started_it() {
        let mut parser = ClaudeParser::new(None);
        let child = json!({ "task_id": "a1", "run_id": "run-a1", "task_type": "local_agent",
            "description": "Child" });
        let grandchild = json!({ "task_id": "a2", "run_id": "run-a2", "task_type": "local_agent",
            "description": "Grandchild", "parent_task_id": "a1" });
        let mut lines = vec![
            json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
            parsed(&agent_call(
                "toolu_child",
                None,
                json!({ "run_in_background": true }),
            )),
            roster(json!([child.clone()])),
            json!({ "type": "system", "subtype": "task_started", "task_id": "a1",
                "run_id": "run-a1", "tool_use_id": "toolu_child", "is_backgrounded": true,
                "spawn_depth": 1, "task_type": "local_agent" }),
            json!({ "type": "user", "parent_tool_use_id": null,
                "tool_use_result": { "isAsync": true, "status": "async_launched" },
                "message": { "role": "user", "content": [{ "type": "tool_result",
                    "tool_use_id": "toolu_child",
                    "content": "Async agent launched successfully." }] } }),
            run_result(),
        ];
        lines.extend([
            parsed(&agent_call(
                "toolu_grandchild",
                Some("toolu_child"),
                json!({ "run_in_background": true }),
            )),
            roster(json!([child.clone(), grandchild])),
            json!({ "type": "system", "subtype": "task_started", "task_id": "a2",
                "run_id": "run-a2", "tool_use_id": "toolu_grandchild", "is_backgrounded": true,
                "spawn_depth": 2, "parent_task_id": "a1", "task_type": "local_agent" }),
            json!({ "type": "user", "parent_tool_use_id": "toolu_child",
                "message": { "role": "user", "content": [{ "type": "tool_result",
                    "tool_use_id": "toolu_grandchild", "content": [{ "type": "text",
                    "text": "Async agent launched successfully. (This tool result is internal metadata.)\nagentId: a2" }] }] } }),
        ]);
        let launched = feed(&mut parser, &lines);
        assert!(completed(&launched).is_empty(), "{launched:#?}");

        let ended = feed(
            &mut parser,
            &[
                json!({ "type": "system", "subtype": "task_notification", "task_id": "a2",
                    "run_id": "run-a2", "tool_use_id": "toolu_grandchild",
                    "status": "completed", "summary": "GRANDCHILD_DONE" }),
                roster(json!([child])),
            ],
        );
        assert_eq!(
            completed(&ended),
            [(
                Some("toolu_child".into()),
                "toolu_grandchild".into(),
                "GRANDCHILD_DONE".into()
            )]
        );
        assert!(told_ended(&ended).is_empty(), "{ended:#?}");
    }

    #[test]
    fn a_background_subagent_two_levels_down_wakes_its_owner_not_the_agent() {
        let mut parser = ClaudeParser::new(None);
        let child = json!({ "task_id": "a1", "run_id": "run-a1", "task_type": "local_agent",
            "description": "Child" });
        let grandchild = json!({ "task_id": "a2", "run_id": "run-a2", "task_type": "local_agent",
            "description": "Grandchild" });
        let outputs = feed(
            &mut parser,
            &[
                json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
                roster(json!([child.clone()])),
                json!({ "type": "system", "subtype": "task_started", "task_id": "a1",
                    "run_id": "run-a1", "tool_use_id": "toolu_child", "is_backgrounded": true,
                    "spawn_depth": 1, "task_type": "local_agent" }),
                run_result(),
                roster(json!([child.clone(), grandchild])),
                json!({ "type": "system", "subtype": "task_started", "task_id": "a2",
                    "run_id": "run-a2", "tool_use_id": "toolu_grandchild",
                    "is_backgrounded": true, "spawn_depth": 2, "task_type": "local_agent" }),
                json!({ "type": "system", "subtype": "task_notification", "task_id": "a2",
                    "run_id": "run-a2", "tool_use_id": "toolu_grandchild",
                    "status": "completed", "summary": "GRANDCHILD_DONE" }),
                roster(json!([child])),
                json!({ "type": "system", "subtype": "task_notification", "task_id": "a1",
                    "run_id": "run-a1", "tool_use_id": "toolu_child", "status": "completed",
                    "summary": "CHILD_DONE" }),
                roster(json!([])),
                json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
                wake_result("run-a1"),
            ],
        );

        assert_eq!(told_ended(&outputs), ["a1"]);
        assert_eq!(completions(&outputs), [TurnOutcome::Completed]);
    }

    fn schedule_wakeup(scheduled_for: u64) -> Vec<Value> {
        vec![
            json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
            json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "id": "msg_1",
                "content": [{ "type": "tool_use", "id": "toolu_wakeup", "name": "ScheduleWakeup",
                    "input": { "delaySeconds": 60, "prompt": "Reply with exactly: WOKE",
                        "reason": "Check back" } }] } }),
            json!({ "type": "user", "parent_tool_use_id": null,
                "tool_use_result": { "scheduledFor": scheduled_for, "clampedDelaySeconds": 60,
                    "wasClamped": false },
                "message": { "role": "user", "content": [{ "type": "tool_result",
                    "tool_use_id": "toolu_wakeup",
                    "content": "Next wakeup scheduled for 14:59:00 (in 91s)." }] } }),
            json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "id": "msg_2",
                "content": [{ "type": "text", "text": "SCHEDULED" }] } }),
            run_result(),
        ]
    }

    #[test]
    fn a_scheduled_wakeup_keeps_the_process_for_the_run_it_starts() {
        let mut parser = ClaudeParser::new(None);
        let later = unix_millis() + 60 * 60 * 1000;
        let scheduled = feed(&mut parser, &schedule_wakeup(later));
        assert_eq!(completions(&scheduled), [TurnOutcome::Completed]);
        {
            let queue = parser.queue.state();
            assert!(!queue.closing && !queue.turn_open);
        }

        let woke = feed(
            &mut parser,
            &[
                json!({ "type": "command_lifecycle", "command_uuid": "c1", "state": "started",
                    "wakeup_source": "loop_wakeup", "scheduled_for": later }),
                json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
                json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                    "id": "msg_3", "content": [{ "type": "text", "text": "WOKE" }] } }),
                json!({ "type": "result", "subtype": "success", "is_error": false,
                    "result": "WOKE" }),
                json!({ "type": "command_lifecycle", "command_uuid": "c1",
                    "state": "completed" }),
            ],
        );
        assert_eq!(woke[0], ProviderOutput::Woke);
        assert_eq!(completions(&woke), [TurnOutcome::Completed]);
        assert!(parser.queue.state().closing, "nothing else is scheduled");
    }

    #[test]
    fn a_wakeup_called_off_lets_the_process_end() {
        let mut parser = ClaudeParser::new(None);
        feed(&mut parser, &schedule_wakeup(0));
        assert!(parser.queue.state().closing);
    }

    /// The agent's subagent `a1` runs in the background and the turn waits for it.
    fn wait_for_subagent(parser: &mut ClaudeParser) {
        feed(
            parser,
            &[
                json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
                roster(json!([agent_roster("run-a1")])),
                json!({ "type": "system", "subtype": "task_started", "task_id": "a1",
                    "run_id": "run-a1", "tool_use_id": "toolu_agent", "is_backgrounded": true,
                    "spawn_depth": 1, "task_type": "local_agent" }),
                json!({ "type": "user", "parent_tool_use_id": null,
                    "tool_use_result": { "isAsync": true, "status": "async_launched" },
                    "message": { "role": "user", "content": [{ "type": "tool_result",
                        "tool_use_id": "toolu_agent",
                        "content": "Async agent launched successfully." }] } }),
                run_result(),
            ],
        );
    }

    #[test]
    fn a_stopped_background_subagent_is_still_answered() {
        let mut parser = ClaudeParser::new(None);
        wait_for_subagent(&mut parser);

        let stopped = feed(
            &mut parser,
            &[
                json!({ "type": "system", "subtype": "task_notification", "task_id": "a1",
                    "run_id": "run-a1", "tool_use_id": "toolu_agent", "status": "stopped",
                    "summary": "Agent C" }),
                roster(json!([])),
            ],
        );
        assert!(completions(&stopped).is_empty(), "{stopped:#?}");
        assert!(stopped.contains(&ProviderOutput::ToolCallStopped {
            item_id: "toolu_agent".into()
        }));

        let answered = feed(
            &mut parser,
            &[
                json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
                json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                    "id": "msg_2", "content": [{ "type": "text", "text": "Agent C was stopped." }] } }),
                wake_result("run-a1"),
            ],
        );
        assert!(answered.contains(&ProviderOutput::AssistantMessage {
            item_id: "msg_2:0".into(),
            text: "Agent C was stopped.".into(),
        }));
        assert_eq!(completions(&answered), [TurnOutcome::Completed]);
    }

    #[test]
    fn a_resumed_turn_outlasts_the_run_about_a_subagent_an_earlier_process_left() {
        let mut parser = ClaudeParser::new(None);
        let left = feed(
            &mut parser,
            &[
                json!({ "type": "system", "subtype": "task_notification", "task_id": "a9",
                    "status": "stopped", "output_file": "/tmp/a9.output",
                    "summary": "Background agent \"Agent D\" didn't finish before the previous session ended" }),
                json!({ "type": "system", "subtype": "task_notification", "task_id": "b9",
                    "tool_use_id": "toolu_old", "status": "stopped", "output_file": "",
                    "summary": "Orphaned by a previous Claude Code process exit and reported in an aggregate summary." }),
                json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
                json!({ "type": "result", "subtype": "success", "is_error": false,
                    "num_turns": 0, "result": "", "origin": { "kind": "task-notification" } }),
            ],
        );
        assert!(completions(&left).is_empty(), "{left:#?}");
        assert!(told_ended(&left).is_empty() && completed(&left).is_empty());
        assert!(!left.contains(&ProviderOutput::RunEnded { run: 1 }));

        let answered = feed(
            &mut parser,
            &[
                json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
                json!({ "type": "result", "subtype": "success", "is_error": false,
                    "num_turns": 1, "result": "BACK" }),
            ],
        );
        assert_eq!(completions(&answered), [TurnOutcome::Completed]);
    }
}
