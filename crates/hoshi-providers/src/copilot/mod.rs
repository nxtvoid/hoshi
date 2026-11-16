//! GitHub Copilot CLI, driven over the Agent Client Protocol (`copilot --acp`). A turn starts
//! a process with a new session, or the thread's earlier one loaded again; once it completes,
//! the process waits a while for that session's next turn.

mod account;

use crate::acp::{Translator, normalize_tool};
use crate::jsonrpc::{Connection, Incoming, RpcError};
use crate::process::{hide_console, kill_tree};
use crate::titles;
use crate::updater::find_program;
use crate::version::{Version, parse_version};
use crate::{
    GeneratedTitle, Installation, McpServer, OutputSink, ProviderAdapter, RunningTurn,
    TitleRequest, TurnAttachment, TurnRequest, truncate_json,
};
use base64::Engine as _;
use hoshi_core::{
    AccessMode, ApprovalDecision, Effort, ItemContent, MessageAttachment, ModelInfo,
    ProviderAccountStatus, ProviderModels, ProviderOutput, RequestResponse, RequestStatus,
    SlashCommand, ToolDetail, TurnFailure, TurnOutcome,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak, mpsc};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) const PROVIDER: &str = "copilot";

/// Runs the model the plan gives, for plans that let no one choose, such as Copilot Student.
const DEFAULT_MODEL: &str = "default";

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);
const LOAD_TIMEOUT: Duration = Duration::from_secs(120);
const SETTING_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a probe waits for the session's late updates, such as its commands.
const PROBE_SETTLE: Duration = Duration::from_millis(1_500);
const TITLE_TIMEOUT: Duration = Duration::from_secs(90);
/// How long a cancelled turn may take to stop before its process is killed.
const CANCEL_GRACE: Duration = Duration::from_secs(5);
const STDERR_TAIL_BYTES: usize = 2_000;
const APPROVAL_INPUT_MAX_CHARS: usize = 2_000;
/// How long a process kept after its turn waits for its session's next one.
const IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const IDLE_SWEEP: Duration = Duration::from_secs(30);

/// Copilot fails every prompt of a session with these once it lost it, until its process is
/// replaced.
const LOST_SESSION: [&str; 2] = [
    "session event delivery failed: session not found: ",
    "session lock unavailable during is_session_generation_current: acquisition timed out",
];

const PLAN_MODE: &str = "https://agentclientprotocol.com/protocol/session-modes#plan";
const AGENT_MODE: &str = "https://agentclientprotocol.com/protocol/session-modes#agent";
/// What a plan the user accepted is answered with.
const IMPLEMENT_PLAN: &str = "The plan is approved. Implement it now.";
/// How hoshi names its own MCP tools.
const HOSHI_TOOL_PREFIX: &str = "mcp__hoshi__";

/// The raw events Copilot sends besides the protocol's own updates.
const SESSION_EVENT: &str = "github.com/copilot/sessionEvent";

fn client_capabilities() -> Value {
    json!({
        "fs": { "readTextFile": false, "writeTextFile": false },
        "terminal": false,
        "_meta": {
            "github.com/copilot": {
                "events": [
                    "subagent.started",
                    "subagent.completed",
                    "subagent.failed",
                    "session.compaction_complete",
                    "assistant.turn_start",
                    "assistant.idle",
                ],
            },
        },
    })
}

pub struct CopilotAdapter {
    /// Empty means `copilot` on the PATH.
    binary_path: Option<String>,
    /// Read once, from a probe session or the CLI's help.
    models: Mutex<Option<ProviderModels>>,
    /// `None` until asked; then `Some(None)` when the CLI can't be run.
    version: Mutex<Option<Option<Version>>>,
    /// Who the last [`ProviderAdapter::account_status`] found signed in.
    login: Mutex<Option<String>>,
    idle: Arc<Pool>,
}

impl CopilotAdapter {
    pub fn new(binary_path: Option<String>) -> Self {
        Self {
            binary_path: binary_path.filter(|path| !path.trim().is_empty()),
            models: Mutex::default(),
            version: Mutex::default(),
            login: Mutex::default(),
            idle: Arc::default(),
        }
    }

    /// The GitHub login Copilot ran as when [`ProviderAdapter::account_status`] last looked.
    pub fn login(&self) -> Option<String> {
        lock(&self.login).clone()
    }

    /// The process kept for `session_id`'s next turn, by its id.
    #[doc(hidden)]
    pub fn kept_process(&self, session_id: &str) -> Option<u32> {
        lock(&self.idle.kept)
            .get(session_id)
            .map(|kept| kept.live.agent.pid)
    }

    /// The CLI to run: `copilot.cmd` and the like count on Windows, as npm installs it so.
    fn binary(&self) -> Option<PathBuf> {
        find_program(self.binary_path.as_deref().unwrap_or("copilot"), true)
    }

    fn command(&self) -> Option<Command> {
        Some(command(&self.binary()?))
    }

    fn version(&self) -> Option<Version> {
        let mut version = lock(&self.version);
        *version.get_or_insert_with(|| {
            let mut command = self.command()?;
            command
                .arg("--version")
                .stdin(Stdio::null())
                .stderr(Stdio::null());
            let output = command.output().ok()?;
            let text = String::from_utf8_lossy(&output.stdout);
            parse_version(text.lines().next()?.trim_end_matches('.'))
        })
    }

    /// Opens a session in `cwd` only to read what it offers, then ends it.
    fn probe(&self, cwd: &Path) -> Option<Probe> {
        let mut command = self.command()?;
        command.arg("--acp").current_dir(cwd);
        let mut agent = Agent::spawn(command).ok()?;
        let seen = Arc::new(Mutex::new(Probe::default()));
        let collected = seen.clone();
        let connection = agent.connect(
            move |incoming| {
                if let Incoming::Notification { params, .. } = incoming {
                    collected.lock().unwrap().read(&params["update"]);
                }
            },
            || {},
        );
        let result = (|| {
            connection
                .call(
                    "initialize",
                    json!({ "protocolVersion": 1, "clientCapabilities": client_capabilities() }),
                    HANDSHAKE_TIMEOUT,
                )
                .ok()?;
            let session = connection
                .call(
                    "session/new",
                    json!({ "cwd": cwd, "mcpServers": [] }),
                    HANDSHAKE_TIMEOUT,
                )
                .ok()?;
            seen.lock().unwrap().read_options(&session["configOptions"]);
            thread::sleep(PROBE_SETTLE);
            Some(seen.lock().unwrap().clone())
        })();
        connection.close();
        agent.kill();
        result
    }
}

impl ProviderAdapter for CopilotAdapter {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    fn session_folders(&self) -> Vec<crate::SessionFolder> {
        std::env::var_os("COPILOT_HOME")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::home_dir().map(|home| home.join(".copilot")))
            .map(|dir| crate::history::copilot_folders(&dir.join("session-state")))
            .unwrap_or_default()
    }

    fn models(&self) -> ProviderModels {
        if let Some(models) = lock(&self.models).clone() {
            return models;
        }
        if self.version().is_none() {
            return ProviderModels {
                provider: PROVIDER.into(),
                ..ProviderModels::default()
            };
        }
        let probe = self.probe(&std::env::temp_dir()).unwrap_or_default();
        let models = catalog(probe.models, probe.current_model, &probe.efforts);
        *lock(&self.models) = Some(models.clone());
        models
    }

    fn installation(&self) -> Option<Installation> {
        let installed = self
            .version()
            .map(|(major, minor, patch)| format!("{major}.{minor}.{patch}"));
        Some(Installation {
            installed,
            latest: None,
            updater: None,
        })
    }

    fn forget_version(&self) {
        *lock(&self.version) = None;
        *lock(&self.models) = None;
        self.idle.end_all();
    }

    fn signed_in_as(&self) -> Option<String> {
        self.login()
    }

    fn account_status(&self) -> ProviderAccountStatus {
        if self.version().is_none() {
            *lock(&self.login) = None;
            return ProviderAccountStatus::NotInstalled;
        }
        let account = self.command().and_then(account::read);
        *lock(&self.login) = account.as_ref().and_then(|account| account.login.clone());
        account.map_or(ProviderAccountStatus::Unknown, |account| account.status)
    }

    fn slash_commands(&self, cwd: &Path) -> Option<Vec<SlashCommand>> {
        Some(self.probe(cwd)?.commands)
    }

    fn generate_title(&self, request: &TitleRequest) -> Option<GeneratedTitle> {
        let prompt = format!(
            "{}\n\nReply with the JSON object only.",
            titles::prompt(request)
        );
        let mut command = self.command()?;
        command
            .current_dir(std::env::temp_dir())
            .args([
                "-s",
                "--no-custom-instructions",
                "--no-ask-user",
                "--available-tools",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(model) = chosen(request.model.as_deref()) {
            command.args(["--model", model]);
        }
        parse_title(&run_with_timeout(command, &prompt, TITLE_TIMEOUT)?)
    }

    fn start_turn(
        &self,
        request: TurnRequest,
        sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
        let binary = self.binary().ok_or_else(|| TurnFailure::SpawnFailed {
            detail: "copilot was not found".into(),
        })?;
        let request = request.with_inline_handoff();
        let launch = Launch::of(&request);
        let kept = request
            .resume_session_id
            .as_deref()
            .filter(|_| request.resume_session_at.is_none())
            .and_then(|session_id| self.idle.take(session_id, &launch));
        let reused = kept.is_some();
        let live = match kept {
            Some(live) => live,
            None => Live::start(&binary, &launch)?,
        };
        let turn = Arc::new(Turn::new(sink, request.access_mode));
        let live = Arc::new(Mutex::new(live));
        let work = Work {
            turn: turn.clone(),
            live: live.clone(),
            idle: Arc::downgrade(&self.idle),
            binary,
            launch,
        };
        thread::spawn(move || work.run(&request, reused));
        Ok(Box::new(CopilotTurn { turn, live }))
    }
}

impl Drop for CopilotAdapter {
    fn drop(&mut self) {
        self.idle.end_all();
    }
}

fn command(binary: &Path) -> Command {
    let mut command = Command::new(binary);
    hide_console(&mut command);
    command
}

/// What a process was started with. A turn that asks for anything else needs a new one.
#[derive(Debug, Clone, PartialEq)]
struct Launch {
    cwd: String,
    model: Option<String>,
    effort: Option<Effort>,
    allow_all: bool,
    environment: Vec<(String, String)>,
    /// hoshi's MCP server's address and authorization, which the session took as it opened.
    mcp: Option<(String, String)>,
}

impl Launch {
    fn of(request: &TurnRequest) -> Self {
        Self {
            cwd: request.cwd.clone(),
            model: chosen(request.model.as_deref()).map(str::to_owned),
            effort: request.effort,
            allow_all: request.access_mode == AccessMode::FullAccess,
            environment: request.environment.clone(),
            mcp: request
                .mcp
                .as_ref()
                .map(|server| (server.url.clone(), server.authorization.clone())),
        }
    }

    fn command(&self, binary: &Path) -> Command {
        let mut command = command(binary);
        command
            .arg("--acp")
            .current_dir(&self.cwd)
            .envs(self.environment.iter().cloned());
        if let Some(model) = &self.model {
            command.args(["--model", model]);
        }
        if let Some(effort) = self.effort {
            command.args(["--reasoning-effort", effort.as_str()]);
        }
        if self.allow_all {
            command.arg("--allow-all");
        }
        command
    }
}

/// A `copilot --acp` process, and the turn it runs while it runs one.
struct Live {
    agent: Agent,
    connection: Connection,
    turn: Arc<Mutex<Option<Arc<Turn>>>>,
}

impl Live {
    fn start(binary: &Path, launch: &Launch) -> Result<Arc<Self>, TurnFailure> {
        let mut agent = Agent::spawn(launch.command(binary))?;
        let stderr = agent.stderr();
        let turn: Arc<Mutex<Option<Arc<Turn>>>> = Arc::default();
        let replies: Arc<OnceLock<Connection>> = Arc::default();
        let (reader, ended, idle) = (turn.clone(), turn.clone(), replies.clone());
        let connection = agent.connect(
            move |incoming| {
                let current = lock(&reader).clone();
                match (current, incoming) {
                    (Some(turn), incoming) => turn.receive(incoming),
                    (None, Incoming::Request { id, method, .. }) => {
                        let Some(connection) = idle.get() else {
                            return;
                        };
                        if method == "session/request_permission" {
                            connection.reply(&id, json!({ "outcome": { "outcome": "cancelled" } }));
                        } else {
                            connection.reply_error(&id, -32601, "not supported by this client");
                        }
                    }
                    (None, Incoming::Notification { .. }) => {}
                }
            },
            move || {
                let stderr = stderr.join().unwrap_or_default();
                if let Some(turn) = lock(&ended).clone() {
                    turn.process_ended(stderr);
                }
            },
        );
        let _ = replies.set(connection.clone());
        Ok(Arc::new(Self {
            agent,
            connection,
            turn,
        }))
    }

    fn attach(&self, turn: &Arc<Turn>) {
        *lock(&turn.connection) = Some(self.connection.clone());
        *lock(&self.turn) = Some(turn.clone());
    }

    fn detach(&self) {
        lock(&self.turn).take();
    }

    fn running(&self) -> bool {
        !self.agent.exited.load(Ordering::SeqCst)
    }

    fn end(&self) {
        self.detach();
        self.connection.close();
        self.agent.kill();
    }
}

/// The processes kept after their turn completed, by the session they hold.
#[derive(Default)]
struct Pool {
    kept: Mutex<HashMap<String, Kept>>,
    sweeping: AtomicBool,
}

struct Kept {
    launch: Launch,
    live: Arc<Live>,
    since: Instant,
}

impl Pool {
    /// The process kept for `session_id`, when it was started as `launch` asks and still runs.
    /// One that wasn't ends.
    fn take(&self, session_id: &str, launch: &Launch) -> Option<Arc<Live>> {
        let kept = lock(&self.kept).remove(session_id)?;
        if kept.launch == *launch && kept.live.running() {
            Some(kept.live)
        } else {
            kept.live.end();
            None
        }
    }

    fn keep(self: &Arc<Self>, session_id: String, launch: Launch, live: Arc<Live>) {
        let kept = Kept {
            launch,
            live,
            since: Instant::now(),
        };
        if let Some(previous) = lock(&self.kept).insert(session_id, kept) {
            previous.live.end();
        }
        if self.sweeping.swap(true, Ordering::SeqCst) {
            return;
        }
        let pool = Arc::downgrade(self);
        thread::spawn(move || {
            loop {
                thread::sleep(IDLE_SWEEP);
                let Some(pool) = pool.upgrade() else {
                    return;
                };
                pool.end_idle(IDLE_TIMEOUT);
            }
        });
    }

    /// Ends the processes kept for longer than `timeout`.
    fn end_idle(&self, timeout: Duration) {
        let ended: Vec<Kept> = {
            let mut kept = lock(&self.kept);
            let expired: Vec<String> = kept
                .iter()
                .filter(|(_, kept)| kept.since.elapsed() >= timeout || !kept.live.running())
                .map(|(session_id, _)| session_id.clone())
                .collect();
            expired
                .iter()
                .filter_map(|session_id| kept.remove(session_id))
                .collect()
        };
        for kept in ended {
            kept.live.end();
        }
    }

    fn end_all(&self) {
        let ended: Vec<Kept> = lock(&self.kept).drain().map(|(_, kept)| kept).collect();
        for kept in ended {
            kept.live.end();
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.end_all();
    }
}

/// Runs a turn in its process, and keeps the process for the session's next turn once it
/// completes.
struct Work {
    turn: Arc<Turn>,
    live: Arc<Mutex<Arc<Live>>>,
    idle: Weak<Pool>,
    binary: PathBuf,
    launch: Launch,
}

impl Work {
    fn run(self, request: &TurnRequest, mut reused: bool) {
        loop {
            let live = lock(&self.live).clone();
            live.attach(&self.turn);
            self.turn.reused.store(reused, Ordering::SeqCst);
            let attempt = self.turn.run(&live.connection, request, reused);
            live.detach();
            let (outcome, reusable) = match attempt {
                Attempt::Retry => {
                    live.end();
                    match Live::start(&self.binary, &self.launch) {
                        Ok(fresh) => {
                            *lock(&self.live) = fresh;
                            reused = false;
                            continue;
                        }
                        Err(reason) => (TurnOutcome::Failed { reason }, false),
                    }
                }
                Attempt::Done { outcome, reusable } => (outcome, reusable),
            };
            let session_id = lock(&self.turn.session_id).clone();
            let keep = reusable
                && live.running()
                && !self.turn.closed.load(Ordering::SeqCst)
                && lock(&self.turn.subagents).is_empty();
            match (session_id, self.idle.upgrade()) {
                (Some(session_id), Some(idle)) if keep => idle.keep(session_id, self.launch, live),
                _ => live.end(),
            }
            self.turn.finish(outcome);
            return;
        }
    }
}

/// How one try at a turn ended.
enum Attempt {
    Done {
        outcome: TurnOutcome,
        /// Whether its process is idle and can take the session's next turn.
        reusable: bool,
    },
    /// The kept process had lost the session before it did anything: the turn goes on in a
    /// new one.
    Retry,
}

/// Whether a kept process failed `error` for having lost its session, or by ending.
fn lost(error: &RpcError) -> bool {
    let local = error.code == 0 && error.data.is_null();
    let detail = error.detail();
    local || LOST_SESSION.iter().any(|marker| detail.contains(marker))
}

/// What a probe session offered.
#[derive(Debug, Clone, Default, PartialEq)]
struct Probe {
    /// Slug and name.
    models: Vec<(String, Option<String>)>,
    current_model: Option<String>,
    efforts: Vec<Effort>,
    commands: Vec<SlashCommand>,
}

impl Probe {
    fn read(&mut self, update: &Value) {
        match update["sessionUpdate"].as_str() {
            Some("config_option_update") => self.read_options(&update["configOptions"]),
            Some("available_commands_update") => {
                self.commands = update["availableCommands"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|command| {
                        Some(SlashCommand {
                            name: command["name"].as_str()?.to_owned(),
                            description: command["description"]
                                .as_str()
                                .unwrap_or_default()
                                .to_owned(),
                            argument_hint: command["input"]["hint"].as_str().map(str::to_owned),
                            aliases: Vec::new(),
                            custom: false,
                        })
                    })
                    .collect();
            }
            _ => {}
        }
    }

    /// The model and effort choices among a session's options, when it offers them.
    fn read_options(&mut self, options: &Value) {
        for option in options.as_array().into_iter().flatten() {
            let choices = || {
                option["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|choice| {
                        Some((
                            choice["value"].as_str()?.to_owned(),
                            choice["name"].as_str().map(str::to_owned),
                        ))
                    })
                    .collect::<Vec<_>>()
            };
            match option["category"].as_str() {
                Some("model") => {
                    self.models = choices();
                    self.current_model = option["currentValue"].as_str().map(str::to_owned);
                }
                Some("thought_level") => {
                    self.efforts = choices()
                        .into_iter()
                        .filter_map(|(value, _)| effort(&value))
                        .collect();
                }
                _ => {}
            }
        }
    }
}

fn effort(value: &str) -> Option<Effort> {
    match value {
        "low" => Some(Effort::Low),
        "medium" => Some(Effort::Medium),
        "high" => Some(Effort::High),
        "xhigh" => Some(Effort::Xhigh),
        "max" => Some(Effort::Max),
        _ => None,
    }
}

/// The catalog hoshi shows for Copilot's models, named as a person would. A session that
/// offers no models gets only the plan's default one.
fn catalog(
    mut models: Vec<(String, Option<String>)>,
    mut default_model: Option<String>,
    efforts: &[Effort],
) -> ProviderModels {
    let (efforts, default_effort) = if efforts.is_empty() {
        (
            vec![
                Effort::Low,
                Effort::Medium,
                Effort::High,
                Effort::Xhigh,
                Effort::Max,
            ],
            Some(Effort::Medium),
        )
    } else {
        (efforts.to_vec(), None)
    };
    if models.is_empty() {
        models.push((DEFAULT_MODEL.into(), Some("Default".into())));
        default_model = Some(DEFAULT_MODEL.into());
    }
    ProviderModels {
        provider: PROVIDER.into(),
        default_model,
        models: models
            .into_iter()
            .filter(|(slug, _)| !slug.is_empty())
            .map(|(slug, name)| {
                let name = name
                    .filter(|name| name != &slug)
                    .unwrap_or_else(|| model_name(&slug));
                ModelInfo {
                    short_name: name.strip_prefix("Claude ").unwrap_or(&name).to_owned(),
                    name,
                    slug,
                    legacy: false,
                    is_new: false,
                    update_required: false,
                    efforts: efforts.clone(),
                    default_effort,
                    fast_mode: false,
                    ultracode: false,
                }
            })
            .collect(),
        ..ProviderModels::default()
    }
}

/// `claude-sonnet-5.5` as "Claude Sonnet 5.5", `gpt-6.1-sol` as "GPT-6.1 Sol".
fn model_name(slug: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    for part in slug.split('-') {
        let word = match part {
            "gpt" | "mai" => part.to_uppercase(),
            _ if part.starts_with(|c: char| c.is_ascii_digit()) => match words.last_mut() {
                Some(last) if *last == "GPT" => {
                    last.push('-');
                    last.push_str(part);
                    continue;
                }
                _ => part.to_owned(),
            },
            _ => {
                let mut chars = part.chars();
                chars.next().map_or_else(String::new, |first| {
                    first.to_uppercase().chain(chars).collect()
                })
            }
        };
        words.push(word);
    }
    words.join(" ")
}

/// The title in a one-shot answer: the first JSON object it prints.
fn parse_title(output: &str) -> Option<GeneratedTitle> {
    let start = output.find('{')?;
    let end = output.rfind('}')?;
    let answer: Value = serde_json::from_str(output.get(start..=end)?).ok()?;
    let title = answer["title"].as_str()?.trim();
    let title = title
        .trim_end_matches(['.', '!', '?', ':'])
        .trim_matches('"');
    (!title.is_empty()).then(|| GeneratedTitle {
        title: title.to_owned(),
        needs_refinement: answer["needsRefinement"] == true,
    })
}

/// Runs `command` with `input` on its stdin, as the CLI reads a prompt piped to it: a
/// prompt on several lines can't be an argument of a `.cmd`.
fn run_with_timeout(mut command: Command, input: &str, timeout: Duration) -> Option<String> {
    let mut child = command.spawn().ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let mut stdout = child.stdout.take()?;
    let pid = child.id();
    let child = Mutex::new(child);
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut output = String::new();
        let _ = stdout.read_to_string(&mut output);
        let _ = sender.send(output);
    });
    let output = receiver.recv_timeout(timeout).ok();
    if output.is_none() {
        kill_tree(pid, &child);
    }
    let _ = lock(&child).wait();
    output
}

/// A running `copilot --acp`.
struct Agent {
    child: Arc<Mutex<Child>>,
    pid: u32,
    exited: Arc<AtomicBool>,
    noted: Option<crate::agent_processes::Noted>,
    stderr: Option<std::process::ChildStderr>,
    pipes: Option<(std::process::ChildStdin, std::process::ChildStdout)>,
}

impl Agent {
    fn spawn(mut command: Command) -> Result<Self, TurnFailure> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|error| TurnFailure::SpawnFailed {
            detail: error.to_string(),
        })?;
        let pipes = child.stdin.take().zip(child.stdout.take());
        Ok(Self {
            pid: child.id(),
            noted: Some(crate::agent_processes::Noted::new(child.id())),
            stderr: child.stderr.take(),
            child: Arc::new(Mutex::new(child)),
            exited: Arc::default(),
            pipes,
        })
    }

    fn stderr(&mut self) -> thread::JoinHandle<String> {
        crate::jsonrpc::stderr_tail(self.stderr.take(), STDERR_TAIL_BYTES)
    }

    fn connect(
        &mut self,
        incoming: impl FnMut(Incoming) + Send + 'static,
        ended: impl FnOnce() + Send + 'static,
    ) -> Connection {
        let (stdin, stdout) = self.pipes.take().expect("connected once");
        let (child, exited) = (self.child.clone(), self.exited.clone());
        let noted = self.noted.take();
        Connection::start(stdin, stdout, incoming, move || {
            let _ = lock(&child).wait();
            drop(noted);
            exited.store(true, Ordering::SeqCst);
            ended();
        })
    }

    fn kill(&self) {
        if !self.exited.load(Ordering::SeqCst) {
            kill_tree(self.pid, &self.child);
        }
    }
}

/// A permission the agent waits for.
struct Permission {
    id: Value,
    /// Option ids by kind, such as `allow_once`.
    options: HashMap<String, String>,
}

/// A turn's state, shared by the thread that runs it and the one that reads the agent.
struct Turn {
    sink: Mutex<OutputSink>,
    translator: Mutex<Translator>,
    session_id: Mutex<Option<String>>,
    /// While a session loads, the agent replays its history, which the transcript has.
    loading: AtomicBool,
    finished: AtomicBool,
    cancelled: AtomicBool,
    /// Ended by [`RunningTurn::close`], so its process is not kept.
    closed: AtomicBool,
    /// Runs in a process an earlier turn of its session left.
    reused: AtomicBool,
    /// Whether the agent showed anything of the turn yet.
    progressed: AtomicBool,
    permissions: Mutex<HashMap<String, Permission>>,
    access: AccessMode,
    connection: Mutex<Option<Connection>>,
    /// The plan file the agent wrote in plan mode, which it proposes once its prompt ends.
    plan_file: Mutex<Option<std::path::PathBuf>>,
    /// Answers the plan proposal the turn waits on.
    plan_answer: Mutex<Option<(String, mpsc::Sender<RequestResponse>)>>,
    subagents: Mutex<Subagents>,
}

/// The subagents running in the turn, by Copilot's agent id.
#[derive(Default)]
struct Subagents {
    /// The call that started each one.
    calls: HashMap<String, String>,
    translators: HashMap<String, Translator>,
}

impl Subagents {
    /// What `update` makes inside the subagent it belongs to; `None` for the agent's own.
    fn update(&mut self, update: &Value) -> Option<Vec<ProviderOutput>> {
        let agent = update["_meta"]["github.com/copilot"]["agentId"].as_str()?;
        let call = self.calls.get(agent)?.clone();
        let outputs = self
            .translators
            .entry(agent.to_owned())
            .or_default()
            .update(update);
        Some(within(&call, outputs))
    }

    fn started(&mut self, agent: &str, call: &str) {
        self.calls.insert(agent.to_owned(), call.to_owned());
    }

    fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    /// The subagent's last streamed text, as it ends.
    fn ended(&mut self, agent: &str) -> Vec<ProviderOutput> {
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
fn within(call: &str, outputs: Vec<ProviderOutput>) -> Vec<ProviderOutput> {
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
    fn new(sink: OutputSink, access: AccessMode) -> Self {
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

    fn emit(&self, outputs: Vec<ProviderOutput>) {
        if outputs.is_empty() {
            return;
        }
        self.progressed.store(true, Ordering::SeqCst);
        self.send(outputs);
    }

    fn send(&self, outputs: Vec<ProviderOutput>) {
        if self.finished.load(Ordering::SeqCst) {
            return;
        }
        let mut sink = lock(&self.sink);
        for output in outputs {
            sink(output);
        }
    }

    /// Reports the turn's session, once.
    fn announce(&self, session_id: &str) {
        let previous = lock(&self.session_id).replace(session_id.to_owned());
        if previous.as_deref() != Some(session_id) {
            self.send(vec![ProviderOutput::SessionStarted {
                session_id: session_id.to_owned(),
            }]);
        }
    }

    /// Whether the turn can still start over in a new process, having lost the one it reused.
    fn retryable(&self) -> bool {
        self.reused.load(Ordering::SeqCst)
            && !self.progressed.load(Ordering::SeqCst)
            && !self.cancelled.load(Ordering::SeqCst)
    }

    fn process_ended(&self, stderr: String) {
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
    fn finish(&self, outcome: TurnOutcome) {
        let mut outputs = Vec::new();
        lock(&self.translator).flush(&mut outputs);
        self.emit(outputs);
        if self.finished.swap(true, Ordering::SeqCst) {
            return;
        }
        lock(&self.sink)(ProviderOutput::TurnCompleted { outcome });
    }

    fn connection(&self) -> Option<Connection> {
        lock(&self.connection).clone()
    }

    fn receive(&self, incoming: Incoming) {
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

    fn session_event(&self, event: &Value) {
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

    fn permission(&self, id: Value, params: &Value) {
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
    fn answer(&self, permission: &Permission, kind: &str) {
        let Some(connection) = self.connection() else {
            return;
        };
        let outcome = match permission.options.get(kind) {
            Some(option) => json!({ "outcome": "selected", "optionId": option }),
            None => json!({ "outcome": "cancelled" }),
        };
        connection.reply(&permission.id, json!({ "outcome": outcome }));
    }

    fn respond(&self, request_id: &str, response: &RequestResponse) {
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

    fn cancel(&self) {
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
    fn run(&self, connection: &Connection, request: &TurnRequest, reused: bool) -> Attempt {
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
            *lock(&self.plan_answer) = Some((request_id.clone(), sender));
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
    fn open(
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
fn chosen(model: Option<&str>) -> Option<&str> {
    model.filter(|model| *model != DEFAULT_MODEL)
}

/// hoshi's MCP server as ACP describes one reached over HTTP.
fn mcp(server: &McpServer) -> Value {
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

fn resource_link(path: &Path, name: &str) -> Value {
    let path = path.to_string_lossy().replace('\\', "/");
    let uri = if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    };
    json!({ "type": "resource_link", "uri": uri, "name": name })
}

struct CopilotTurn {
    turn: Arc<Turn>,
    /// The process the turn runs in, which a kept one that failed it gives way to.
    live: Arc<Mutex<Arc<Live>>>,
}

impl RunningTurn for CopilotTurn {
    fn interrupt(&self) {
        if self.turn.finished.load(Ordering::SeqCst) {
            return;
        }
        self.turn.cancel();
        let (turn, live) = (self.turn.clone(), self.live.clone());
        thread::spawn(move || {
            thread::sleep(CANCEL_GRACE);
            if !turn.finished.load(Ordering::SeqCst) {
                lock(&live).agent.kill();
            }
        });
    }

    fn respond(&self, request_id: &str, response: &RequestResponse) {
        self.turn.respond(request_id, response);
    }

    /// Once the turn completed, its process belongs to the session's next turn.
    fn close(&self) {
        self.turn.closed.store(true, Ordering::SeqCst);
        self.turn.cancelled.store(true, Ordering::SeqCst);
        if !self.turn.finished.load(Ordering::SeqCst) {
            lock(&self.live).agent.kill();
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_read_as_names() {
        assert_eq!(model_name("claude-sonnet-5.5"), "Claude Sonnet 5.5");
        assert_eq!(model_name("gpt-6.1-sol"), "GPT-6.1 Sol");
        assert_eq!(model_name("gemini-3.8-flash"), "Gemini 3.8 Flash");
        assert_eq!(model_name("mai-code-1.1-flash"), "MAI Code 1.1 Flash");
    }

    #[test]
    fn a_session_offers_its_models_and_efforts_through_its_options() {
        let mut probe = Probe::default();
        probe.read_options(&json!([
            { "id": "mode", "category": "mode", "currentValue": "agent", "options": [] },
            {
                "id": "model",
                "category": "model",
                "currentValue": "claude-sonnet-5.5",
                "options": [
                    { "value": "claude-sonnet-5.5", "name": "Claude Sonnet 5.5" },
                    { "value": "gpt-6.1-sol", "name": "gpt-6.1-sol" },
                ],
            },
            {
                "id": "reasoning_effort",
                "category": "thought_level",
                "currentValue": "medium",
                "options": [{ "value": "medium" }, { "value": "xhigh" }, { "value": "none" }],
            },
        ]));
        let catalog = catalog(probe.models, probe.current_model, &probe.efforts);

        assert_eq!(catalog.default_model.as_deref(), Some("claude-sonnet-5.5"));
        let names: Vec<_> = catalog
            .models
            .iter()
            .map(|model| model.name.as_str())
            .collect();
        assert_eq!(names, ["Claude Sonnet 5.5", "GPT-6.1 Sol"]);
        assert_eq!(catalog.models[0].short_name, "Sonnet 5.5");
        assert_eq!(catalog.models[0].efforts, [Effort::Medium, Effort::Xhigh]);
    }

    #[test]
    fn a_title_is_read_from_the_json_the_model_printed() {
        let title = parse_title("{\"title\":\"Fix login on Safari.\",\"needsRefinement\":false}\n");
        assert_eq!(
            title,
            Some(GeneratedTitle {
                title: "Fix login on Safari".into(),
                needs_refinement: false,
            })
        );
        assert_eq!(parse_title("Execution failed: no model"), None);
    }

    #[test]
    fn a_subagents_messages_go_under_the_call_that_started_it() {
        let mut subagents = Subagents::default();
        let own = json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "hi" },
        });
        let theirs = json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "found it" },
            "messageId": "m",
            "_meta": { "github.com/copilot": { "agentId": "a1" } },
        });

        assert_eq!(subagents.update(&own), None);
        assert_eq!(subagents.update(&theirs), None);
        subagents.started("a1", "task-call");
        let streamed = subagents.update(&theirs).unwrap();
        let ended = subagents.ended("a1");

        assert_eq!(
            streamed,
            [ProviderOutput::Subagent {
                tool_use_id: "task-call".into(),
                output: Box::new(ProviderOutput::TextDelta {
                    item_id: "m".into(),
                    delta: "found it".into(),
                }),
            }]
        );
        assert_eq!(
            ended,
            [ProviderOutput::Subagent {
                tool_use_id: "task-call".into(),
                output: Box::new(ProviderOutput::AssistantMessage {
                    item_id: "m".into(),
                    text: "found it".into(),
                }),
            }]
        );
    }

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
    fn a_plan_without_a_choice_of_models_offers_its_default() {
        let catalog = catalog(Vec::new(), None, &[]);

        assert_eq!(catalog.default_model.as_deref(), Some(DEFAULT_MODEL));
        assert_eq!(catalog.models.len(), 1);
        assert_eq!(catalog.models[0].name, "Default");
        assert_eq!(catalog.models[0].default_effort, Some(Effort::Medium));
        assert_eq!(chosen(Some(DEFAULT_MODEL)), None);
        assert_eq!(chosen(Some("gpt-6.1-sol")), Some("gpt-6.1-sol"));
    }

    fn turn_request() -> TurnRequest {
        TurnRequest {
            cwd: "C:/repo".into(),
            prompt: "hi".into(),
            attachments: Vec::new(),
            earlier_media: Vec::new(),
            resume_session_id: Some("s1".into()),
            resume_session_at: None,
            fork_session: false,
            handoff: None,
            model: Some(DEFAULT_MODEL.into()),
            effort: None,
            fast_mode: false,
            ultracode: false,
            access_mode: AccessMode::Ask,
            plan_mode: false,
            mcp: None,
            instructions: None,
            subagent_effort: hoshi_core::SubagentEffort::default(),
            environment: Vec::new(),
        }
    }

    #[test]
    fn a_kept_process_takes_turns_only_as_it_was_started() {
        let launch = Launch::of(&turn_request());
        let same = Launch::of(&TurnRequest {
            access_mode: AccessMode::AcceptEdits,
            plan_mode: true,
            model: None,
            prompt: "next".into(),
            ..turn_request()
        });
        let changed = [
            TurnRequest {
                access_mode: AccessMode::FullAccess,
                ..turn_request()
            },
            TurnRequest {
                model: Some("gpt-6.1-sol".into()),
                ..turn_request()
            },
            TurnRequest {
                effort: Some(Effort::High),
                ..turn_request()
            },
            TurnRequest {
                cwd: "C:/repo/worktree".into(),
                ..turn_request()
            },
            TurnRequest {
                mcp: Some(McpServer {
                    url: "http://127.0.0.1:1/mcp".into(),
                    authorization: "Bearer t".into(),
                }),
                ..turn_request()
            },
        ];

        assert_eq!(launch, same);
        for request in &changed {
            assert_ne!(Launch::of(request), launch);
        }
    }

    #[test]
    fn a_kept_process_gives_way_once_it_lost_its_session() {
        let error = |code: i64, message: &str| RpcError {
            code,
            message: message.into(),
            data: Value::Null,
        };
        let lost_session = RpcError {
            data: json!({ "details": "session event delivery failed: session not found: s1" }),
            ..error(-32603, "Internal error")
        };

        assert!(lost(&RpcError::gone()));
        assert!(lost(&lost_session));
        assert!(lost(&error(
            -32603,
            "session lock unavailable during is_session_generation_current: acquisition timed out"
        )));
        assert!(!lost(&error(-32603, "quota exceeded")));
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
