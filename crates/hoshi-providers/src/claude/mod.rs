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

/// Asks the process to stop, and kills it after `grace` if it still runs then.
fn ask_to_stop(process: &LiveProcess, grace: Duration) {
    let request = json!({
        "type": "control_request",
        "request_id": INTERRUPT_REQUEST_ID,
        "request": { "subtype": "interrupt" },
    });
    kill_after(process.clone(), grace);
    if !process.stdin().write_line(&request.to_string()) {
        process.interrupt();
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
mod tests;
