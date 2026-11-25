//! OpenAI Codex, driven over its app-server (`codex app-server`): JSON-RPC 2.0 over stdio.
//! Each turn runs its own server, on a new thread or the thread's earlier one resumed. A
//! server whose turn left commands running in the background stays for them, and runs the
//! thread's next turn.

mod catalog;
pub(crate) mod install;
mod items;
pub(crate) mod login;
mod requests;

use crate::jsonrpc::{Connection, Incoming, RpcError, stderr_tail};
use crate::process::{hide_console, kill_tree};
use crate::updater::find_program;
use crate::version::{Version, parse_version};
use crate::{
    CommitMessage, CommitMessageRequest, GeneratedTitle, Installation, McpServer, OutputSink,
    ProviderAdapter, PullRequestText, PullRequestTextRequest, RunningTurn, TitleRequest,
    TurnAttachment, TurnRequest, attachments, titles, writing,
};
use catalog::ULTRA_EFFORT;
use hoshi_core::{
    AccessMode, ApprovalDecision, BankedResetFailure, BankedResetSpend, ContextHandoff,
    ItemContent, MessageAttachment, ProviderAccountStatus, ProviderModels, ProviderOutput,
    RequestResponse, RequestStatus, SlashCommand, TurnFailure, TurnOutcome, UsageLimits,
};
use items::{Conversation, plan_request_id};
use requests::Kind;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

mod session;
mod turn;

use session::*;
use turn::*;

pub(crate) const PROVIDER: &str = "codex";

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);
const THREAD_TIMEOUT: Duration = Duration::from_secs(120);
const QUERY_TIMEOUT: Duration = Duration::from_secs(30);
const STEER_TIMEOUT: Duration = Duration::from_secs(15);
const TITLE_TIMEOUT: Duration = Duration::from_secs(90);
const WRITING_TIMEOUT: Duration = Duration::from_secs(120);
/// How long an interrupted turn may take to stop before its server is killed.
const CANCEL_GRACE: Duration = Duration::from_secs(5);
const STDERR_TAIL_BYTES: usize = 2_000;
const MODEL_PAGES: usize = 10;
/// Longer than the longest of hoshi's tool calls, a wait for a delegated task.
const MCP_TOOL_TIMEOUT_SECS: u64 = 65 * 60;
/// How long Codex may retry a model call without an answer before the turn fails.
const RETRY_STALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// How often a waiting turn looks whether its retry stalled.
const STALL_POLL: Duration = Duration::from_secs(1);
/// How long to wait, and how often, for a thread Codex still closes or another server still
/// holds before resuming it.
const CLOSING_WAIT: Duration = Duration::from_secs(1);
const CLOSING_TRIES: usize = 10;
/// What Codex says when another client holds the thread, as the Codex app or an IDE does.
const ACTIVE_WRITER: &str = "already has an active writer";
/// Notifications that show the model answers again after a retry.
const MODEL_OUTPUT: [&str; 6] = [
    "item/started",
    "item/agentMessage/delta",
    "item/reasoning/summaryTextDelta",
    "item/reasoning/textDelta",
    "item/plan/delta",
    "turn/plan/updated",
];

const CODEX_HOME: &str = "CODEX_HOME";
/// What a plan the user accepted is answered with.
const IMPLEMENT_PLAN: &str = "The plan is approved. Implement it now.";
/// The prompt hoshi compacts a conversation with.
const COMPACT_COMMAND: &str = "/compact";
/// Notifications hoshi has no use for.
const OPTED_OUT: [&str; 3] = [
    "turn/diff/updated",
    "turn/moderationMetadata",
    "item/fileChange/outputDelta",
];

pub struct CodexAdapter {
    /// Empty means `codex` on the PATH.
    binary_path: Option<String>,
    /// Set as `CODEX_HOME` on every process; `None` leaves Codex's own default.
    codex_home: Option<String>,
    /// Read once, from `model/list`.
    catalog: Mutex<Option<Catalog>>,
    /// `None` until asked; then `Some(None)` when the CLI can't be run.
    version: Mutex<Option<Option<Version>>>,
    session_approvals: Arc<Mutex<SessionApprovals>>,
}

/// The kinds of approval the user accepted for the session, by Codex thread, with the access
/// mode they were accepted in: they hold only while the thread runs in it.
#[derive(Debug, Default)]
struct SessionApprovals(HashMap<String, (AccessMode, HashSet<Kind>)>);

impl SessionApprovals {
    /// A turn of `thread` runs in `access`, which ends what was accepted in another mode.
    fn begin(&mut self, thread: &str, access: AccessMode) {
        if self.0.get(thread).is_some_and(|(mode, _)| *mode != access) {
            self.0.remove(thread);
        }
    }

    fn allows(&self, thread: &str, access: AccessMode, kind: Kind) -> bool {
        self.0
            .get(thread)
            .is_some_and(|(mode, kinds)| *mode == access && kinds.contains(&kind))
    }

    fn grant(&mut self, thread: &str, access: AccessMode, kind: Kind) {
        let entry = self
            .0
            .entry(thread.to_owned())
            .or_insert_with(|| (access, HashSet::new()));
        if entry.0 != access {
            *entry = (access, HashSet::new());
        }
        entry.1.insert(kind);
    }
}

#[derive(Debug, Clone, Default)]
struct Catalog {
    models: ProviderModels,
    /// The service tier a fast run asks for, by model.
    fast_tiers: HashMap<String, String>,
}

impl CodexAdapter {
    pub fn new(binary_path: Option<String>, codex_home: Option<String>) -> Self {
        Self {
            binary_path: binary_path.filter(|path| !path.trim().is_empty()),
            codex_home: codex_home.filter(|home| !home.trim().is_empty()),
            catalog: Mutex::default(),
            version: Mutex::default(),
            session_approvals: Arc::default(),
        }
    }

    /// The CLI to run: `codex.cmd` and the like count on Windows, as npm installs it so.
    fn binary(&self) -> Option<PathBuf> {
        find_program(self.binary_path.as_deref().unwrap_or("codex"), true)
    }

    fn command(&self) -> Option<Command> {
        let mut command = Command::new(self.binary()?);
        if let Some(home) = &self.codex_home {
            command.env(CODEX_HOME, home);
        }
        hide_console(&mut command);
        Some(command)
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
            parse_version(&String::from_utf8_lossy(&output.stdout))
        })
    }

    fn server(&self, cwd: &Path, environment: &[(String, String)]) -> Result<Server, TurnFailure> {
        let mut command = self.command().ok_or_else(|| TurnFailure::SpawnFailed {
            detail: "codex was not found".into(),
        })?;
        command
            .arg("app-server")
            .current_dir(cwd)
            .envs(environment.iter().cloned());
        Server::spawn(command)
    }

    /// Starts a server only to ask it something, then ends it.
    fn query<T>(&self, cwd: &Path, ask: impl FnOnce(&Connection) -> Option<T>) -> Option<T> {
        let mut server = self.server(cwd, &[]).ok()?;
        let _drained = server.stderr();
        let connection = server.connect(|_| {}, || {});
        let answer = initialize(&connection).ok().and_then(|_| ask(&connection));
        connection.close();
        server.kill();
        answer
    }

    fn catalog(&self) -> Option<Catalog> {
        if let Some(catalog) = lock(&self.catalog).clone() {
            return Some(catalog);
        }
        self.version()?;
        let models = self.query(&std::env::temp_dir(), list_models)?;
        let catalog = Catalog {
            models: catalog::catalog(&models),
            fast_tiers: models
                .iter()
                .filter_map(|model| {
                    Some((
                        model["model"].as_str()?.to_owned(),
                        catalog::fast_tier(model)?,
                    ))
                })
                .collect(),
        };
        *lock(&self.catalog) = Some(catalog.clone());
        Some(catalog)
    }

    /// One tool-less, read-only run of the quick model that answers `prompt`.
    fn exec(&self, prompt: &str, model: Option<&str>, timeout: Duration) -> Option<String> {
        let model = model.map(str::to_owned).or_else(|| {
            lock(&self.catalog)
                .as_ref()
                .and_then(|catalog| catalog.models.default_model.clone())
        });
        let mut command = self.command()?;
        command
            .current_dir(std::env::temp_dir())
            .args(["exec", "--ephemeral", "--skip-git-repo-check"])
            .args(["--sandbox", "read-only"])
            .args(["-c", "mcp_servers={}", "-c", "model_reasoning_effort=low"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(model) = &model {
            command.args(["--model", model]);
        }
        command.arg("-");
        run_with_timeout(command, prompt, timeout)
    }
}

fn list_models(connection: &Connection) -> Option<Vec<Value>> {
    let mut models = Vec::new();
    let mut cursor = Value::Null;
    for _ in 0..MODEL_PAGES {
        let page = connection
            .call("model/list", json!({ "cursor": cursor }), QUERY_TIMEOUT)
            .ok()?;
        models.extend(page["data"].as_array().cloned().unwrap_or_default());
        cursor = page["nextCursor"].clone();
        if cursor.is_null() {
            break;
        }
    }
    Some(models)
}

/// Opens the JSON-RPC session, as every client must before anything else.
fn initialize(connection: &Connection) -> Result<Value, RpcError> {
    let answer = connection.call(
        "initialize",
        json!({
            "clientInfo": {
                "name": "hoshi",
                "title": "hoshi",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "capabilities": {
                "experimentalApi": true,
                "optOutNotificationMethods": OPTED_OUT,
            },
        }),
        HANDSHAKE_TIMEOUT,
    )?;
    connection.notify("initialized", Value::Null);
    Ok(answer)
}

impl ProviderAdapter for CodexAdapter {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    fn session_folders(&self) -> Vec<crate::SessionFolder> {
        login::codex_home(self.codex_home.as_deref().map(Path::new))
            .map(|home| crate::history::codex_folders(&home.join("sessions")))
            .unwrap_or_default()
    }

    fn models(&self) -> ProviderModels {
        self.catalog()
            .map(|catalog| catalog.models)
            .unwrap_or_else(|| ProviderModels {
                provider: PROVIDER.into(),
                ..ProviderModels::default()
            })
    }

    fn reports_usage_limits(&self) -> bool {
        true
    }

    fn forks_sessions(&self) -> bool {
        true
    }

    fn injects_context(&self) -> bool {
        true
    }

    fn restarts_to_switch(&self) -> bool {
        true
    }

    fn usage_limits(&self) -> Option<UsageLimits> {
        self.query(&std::env::temp_dir(), |connection| {
            let limits = connection
                .call("account/rateLimits/read", Value::Null, QUERY_TIMEOUT)
                .ok()?;
            let account = connection
                .call("account/read", json!({}), QUERY_TIMEOUT)
                .ok();
            Some(UsageLimits {
                account: account.as_ref().and_then(catalog::account),
                ..catalog::usage_limits(&limits)
            })
        })
    }

    fn spend_banked_reset(&self, grant_id: &str, request_id: &str) -> BankedResetSpend {
        let answer = self.query(&std::env::temp_dir(), |connection| {
            Some(connection.call(
                "account/rateLimitResetCredit/consume",
                json!({ "creditId": grant_id, "idempotencyKey": request_id }),
                QUERY_TIMEOUT,
            ))
        });
        let failed = |reason| BankedResetSpend::Failed { reason };
        match answer {
            None => failed(BankedResetFailure::RequestFailed),
            Some(Err(_)) => failed(BankedResetFailure::Unconfirmed),
            Some(Ok(answer)) => match answer["outcome"].as_str() {
                Some("reset") => BankedResetSpend::Reset,
                Some("nothingToReset") => BankedResetSpend::NothingToReset,
                Some("alreadyRedeemed") => BankedResetSpend::AlreadyUsed,
                Some("noCredit") => BankedResetSpend::NoneLeft,
                _ => failed(BankedResetFailure::Unconfirmed),
            },
        }
    }

    fn installation(&self) -> Option<Installation> {
        let installed = self
            .version()
            .map(|(major, minor, patch)| format!("{major}.{minor}.{patch}"));
        Some(Installation {
            latest: installed.as_ref().and_then(|_| install::latest_version()),
            installed,
            updater: None,
        })
    }

    fn forget_version(&self) {
        *lock(&self.version) = None;
        *lock(&self.catalog) = None;
    }

    fn account_status(&self) -> ProviderAccountStatus {
        if self.version().is_none() {
            return ProviderAccountStatus::NotInstalled;
        }
        let account = self.query(&std::env::temp_dir(), |connection| {
            connection
                .call("account/read", json!({}), QUERY_TIMEOUT)
                .ok()
        });
        match account {
            Some(account) if catalog::signed_out(&account) => ProviderAccountStatus::SignedOut,
            Some(_) => ProviderAccountStatus::Ready,
            None => ProviderAccountStatus::Unknown,
        }
    }

    fn slash_commands(&self, cwd: &Path) -> Option<Vec<SlashCommand>> {
        let skills = self.query(cwd, |connection| skills(connection, cwd))?;
        Some(
            skills
                .iter()
                .filter(|skill| skill["enabled"] != false)
                .filter_map(|skill| {
                    let description = skill["interface"]["shortDescription"]
                        .as_str()
                        .or_else(|| skill["shortDescription"].as_str())
                        .or_else(|| skill["description"].as_str())
                        .unwrap_or_default();
                    Some(SlashCommand {
                        name: skill["name"].as_str()?.to_owned(),
                        description: description.to_owned(),
                        argument_hint: None,
                        aliases: Vec::new(),
                        custom: true,
                    })
                })
                .collect(),
        )
    }

    fn generate_title(&self, request: &TitleRequest) -> Option<GeneratedTitle> {
        let prompt = format!(
            "{}\n\nReply with the JSON object only.",
            titles::prompt(request)
        );
        let output = self.exec(&prompt, request.model.as_deref(), TITLE_TIMEOUT)?;
        parse_title(&output)
    }

    fn write_commit_message(&self, request: &CommitMessageRequest) -> Option<CommitMessage> {
        let prompt = format!(
            "{}\n\nReply with the JSON object only.",
            writing::commit_prompt(request)
        );
        let output = self.exec(&prompt, None, WRITING_TIMEOUT)?;
        writing::parse_commit(&structured(&output)?)
    }

    fn write_pull_request(&self, request: &PullRequestTextRequest) -> Option<PullRequestText> {
        let prompt = format!(
            "{}\n\nReply with the JSON object only.",
            writing::pull_request_prompt(request)
        );
        let output = self.exec(&prompt, None, WRITING_TIMEOUT)?;
        writing::parse_pull_request(&structured(&output)?)
    }

    fn start_turn(
        &self,
        request: TurnRequest,
        sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
        let mut server = self.server(Path::new(&request.cwd), &request.environment)?;
        let fast_tiers = lock(&self.catalog).clone().unwrap_or_default();
        let fast_tier = fast_tiers.fast_tier(&request);
        let turn = Arc::new(Turn::new(sink, self.session_approvals.clone()));
        let stderr = server.stderr();
        let (reader, gone) = (turn.clone(), turn.clone());
        let connection = server.connect(
            move |incoming| reader.receive(incoming),
            move || gone.gone(stderr.join().unwrap_or_default()),
        );
        *lock(&turn.connection) = Some(connection.clone());
        let server = Arc::new(server);
        *lock(&turn.server) = Some(server.clone());
        let launch = Launch::of(&request);
        let worker = turn.clone();
        thread::spawn(move || {
            let outcome = worker.run(&connection, &request, fast_tier);
            worker.end(outcome);
        });
        Ok(Box::new(CodexTurn {
            turn,
            server,
            launch,
            catalog: fast_tiers,
        }))
    }
}

impl Catalog {
    /// The service tier a fast run of `request` asks for.
    fn fast_tier(&self, request: &TurnRequest) -> Option<String> {
        if !request.fast_mode {
            return None;
        }
        let model = request
            .model
            .as_ref()
            .or(self.models.default_model.as_ref())?;
        self.fast_tiers.get(model).cloned()
    }
}

/// What a server's thread was opened with, which a later turn in it can't change.
#[derive(Debug, Clone, PartialEq)]
struct Launch {
    cwd: String,
    environment: Vec<(String, String)>,
    mcp: Option<McpServer>,
    instructions: Option<String>,
}

impl Launch {
    fn of(request: &TurnRequest) -> Self {
        Self {
            cwd: request.cwd.clone(),
            environment: request.environment.clone(),
            mcp: request.mcp.clone(),
            instructions: request.instructions.clone(),
        }
    }
}

fn copy(request: &TurnRequest) -> TurnRequest {
    TurnRequest {
        cwd: request.cwd.clone(),
        prompt: request.prompt.clone(),
        attachments: request.attachments.clone(),
        earlier_media: request.earlier_media.clone(),
        resume_session_id: request.resume_session_id.clone(),
        resume_session_at: request.resume_session_at.clone(),
        fork_session: request.fork_session,
        handoff: request.handoff.clone(),
        model: request.model.clone(),
        effort: request.effort,
        fast_mode: request.fast_mode,
        ultracode: request.ultracode,
        access_mode: request.access_mode,
        plan_mode: request.plan_mode,
        mcp: request.mcp.clone(),
        instructions: request.instructions.clone(),
        subagent_effort: request.subagent_effort,
        environment: request.environment.clone(),
    }
}

fn skills(connection: &Connection, cwd: &Path) -> Option<Vec<Value>> {
    let answer = connection
        .call("skills/list", json!({ "cwds": [cwd] }), QUERY_TIMEOUT)
        .ok()?;
    Some(
        answer["data"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|entry| entry["skills"].as_array().cloned().unwrap_or_default())
            .collect(),
    )
}

/// The first JSON object a one-shot answer holds.
fn json_object(output: &str) -> Option<Value> {
    let start = output.find('{')?;
    let end = output.rfind('}')?;
    serde_json::from_str(output.get(start..=end)?)
        .ok()
        .filter(Value::is_object)
}

fn parse_title(output: &str) -> Option<GeneratedTitle> {
    let answer = json_object(output)?;
    let title = answer["title"].as_str()?.trim();
    let title = title
        .trim_end_matches(['.', '!', '?', ':'])
        .trim_matches('"');
    (!title.is_empty()).then(|| GeneratedTitle {
        title: title.to_owned(),
        needs_refinement: answer["needsRefinement"] == true,
    })
}

/// A one-shot answer as the result the writing helpers read.
fn structured(output: &str) -> Option<String> {
    Some(json!({ "is_error": false, "structured_output": json_object(output)? }).to_string())
}

/// Runs `command` with `input` on its stdin, as `codex exec -` reads a prompt: a prompt on
/// several lines can't be an argument of a `.cmd`.
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

/// A running `codex app-server`.
struct Server {
    child: Arc<Mutex<Child>>,
    pid: u32,
    exited: Arc<AtomicBool>,
    noted: Option<crate::agent_processes::Noted>,
    stderr: Option<ChildStderr>,
    pipes: Option<(ChildStdin, ChildStdout)>,
}

impl Server {
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
        stderr_tail(self.stderr.take(), STDERR_TAIL_BYTES)
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

/// A request Codex waits on the user for.
struct Pending {
    rpc_id: Value,
    kind: Kind,
    params: Value,
}

struct CodexTurn {
    turn: Arc<Turn>,
    server: Arc<Server>,
    launch: Launch,
    /// The models as the server's first turn knew them.
    catalog: Catalog,
}

impl RunningTurn for CodexTurn {
    fn interrupt(&self) {
        if self.turn.finished.load(Ordering::SeqCst) {
            return;
        }
        self.turn.cancel();
        let (turn, server) = (self.turn.clone(), self.server.clone());
        thread::spawn(move || {
            thread::sleep(CANCEL_GRACE);
            if !turn.finished.load(Ordering::SeqCst) {
                server.kill();
            }
        });
    }

    fn respond(&self, request_id: &str, response: &RequestResponse) {
        self.turn.respond(request_id, response);
    }

    fn stop_subagent(&self, tool_use_id: &str) {
        self.turn.stop_subagent(tool_use_id);
    }

    fn steer(
        &self,
        message_id: &str,
        prompt: &str,
        attachments: &[TurnAttachment],
        _at_once: bool,
    ) -> bool {
        self.turn.steer(message_id, prompt, attachments)
    }

    fn stop_background_task(&self, task_id: &str) {
        self.turn.stop_background_task(task_id);
    }

    fn lingers(&self) -> bool {
        self.turn.lingering.load(Ordering::SeqCst) && !self.server.exited.load(Ordering::SeqCst)
    }

    fn continue_with(&self, request: &TurnRequest) -> bool {
        self.turn
            .continue_with(request, &self.launch, self.catalog.fast_tier(request))
    }

    fn close(&self) {
        self.turn.cancelled.store(true, Ordering::SeqCst);
        self.server.kill();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests;
