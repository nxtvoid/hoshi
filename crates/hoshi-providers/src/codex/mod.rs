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

pub(crate) const PROVIDER: &str = "codex";

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);
const THREAD_TIMEOUT: Duration = Duration::from_secs(120);
const QUERY_TIMEOUT: Duration = Duration::from_secs(30);
const STEER_TIMEOUT: Duration = Duration::from_secs(15);
const TITLE_TIMEOUT: Duration = Duration::from_secs(90);
const WRITING_TIMEOUT: Duration = Duration::from_secs(120);
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);
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

const NPM_LATEST: &str = "https://registry.npmjs.org/@openai/codex/latest";
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
            latest: installed.as_ref().and_then(|_| npm_latest()),
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

fn npm_latest() -> Option<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(LOOKUP_TIMEOUT))
        .build()
        .into();
    let text = agent
        .get(NPM_LATEST)
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    let package: Value = serde_json::from_str(&text).ok()?;
    Some(package["version"].as_str()?.to_owned())
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

/// A turn's state, shared by the thread that runs it and the one that reads the server.
struct Turn {
    sink: Mutex<OutputSink>,
    /// Set once the thread is open.
    conversation: Mutex<Option<Conversation>>,
    connection: Mutex<Option<Connection>>,
    finished: AtomicBool,
    cancelled: AtomicBool,
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
    lingering: AtomicBool,
    /// Until the server is shut down.
    server: Mutex<Option<Arc<Server>>>,
}

/// A model call Codex retries.
struct Retry {
    attempt: u32,
    /// When Codex last said it retries.
    since: Instant,
    /// Its `TurnError`.
    error: Value,
}

impl Turn {
    fn new(sink: OutputSink, session_approvals: Arc<Mutex<SessionApprovals>>) -> Self {
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

    fn emit(&self, outputs: Vec<ProviderOutput>) {
        if outputs.is_empty() || self.finished.load(Ordering::SeqCst) {
            return;
        }
        let mut sink = lock(&self.sink);
        for output in outputs {
            sink(output);
        }
    }

    /// Ends the turn once.
    fn finish(&self, outcome: TurnOutcome) {
        if self.finished.swap(true, Ordering::SeqCst) {
            return;
        }
        lock(&self.sink)(ProviderOutput::TurnCompleted { outcome });
    }

    /// Ends the turn once it ran. A turn that completed with commands still running leaves
    /// them in the background and keeps the server for them; otherwise the server ends.
    fn end(&self, outcome: TurnOutcome) {
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
    fn shut_down_when_idle(&self) {
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
    fn gone(&self, stderr: String) {
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
    fn continue_with(
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
    fn stop_background_task(&self, task_id: &str) {
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

    fn connection(&self) -> Option<Connection> {
        lock(&self.connection).clone()
    }

    fn thread_id(&self) -> Option<String> {
        lock(&self.conversation)
            .as_ref()
            .map(|conversation| conversation.thread_id().to_owned())
    }

    /// The thread's running turn, as `(thread, turn)`.
    fn running(&self) -> Option<(String, String)> {
        let conversation = lock(&self.conversation);
        let conversation = conversation.as_ref()?;
        Some((
            conversation.thread_id().to_owned(),
            conversation.turn_id()?.to_owned(),
        ))
    }

    fn receive(&self, incoming: Incoming) {
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

    fn notification(&self, method: &str, params: &Value) {
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
    fn own(&self, params: &Value) -> bool {
        let thread = self.thread_id();
        thread.is_some() && params["threadId"].as_str() == thread.as_deref()
    }

    /// Codex failed to reach the model and tries again.
    fn retrying(&self, error: &Value) {
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
    fn stalled_retry(&self, now: Instant) -> Option<TurnFailure> {
        let retry = lock(&self.retry);
        let retry = retry
            .as_ref()
            .filter(|retry| now.saturating_duration_since(retry.since) >= RETRY_STALL_TIMEOUT)?;
        Some(turn_failure(&retry.error, lock(&self.limits).as_ref()))
    }

    /// Opens the thread's conversation, with what came before it was open.
    fn opened(&self, thread_id: String) {
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
    fn resolved(&self, rpc_id: &Value) {
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

    fn request(&self, rpc_id: Value, kind: Kind, params: Value) {
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

    fn respond(&self, request_id: &str, response: &RequestResponse) {
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

    fn cancel(&self) {
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

    fn interrupt_running(&self, connection: &Connection) {
        if let Some((thread, turn)) = self.running() {
            let _ = connection.request(
                "turn/interrupt",
                json!({ "threadId": thread, "turnId": turn }),
            );
        }
    }

    fn steer(&self, message_id: &str, prompt: &str, attachments: &[TurnAttachment]) -> bool {
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

    fn stop_subagent(&self, tool_use_id: &str) {
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
    fn run(
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
    fn turns(
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

fn provider_failed(detail: String) -> TurnOutcome {
    TurnOutcome::Failed {
        reason: TurnFailure::ProviderError { detail },
    }
}

/// The error a turn failed with: the one it completed with, filled in with what Codex said
/// before it.
fn latest_error(completed: &Value, notified: Option<Value>) -> Value {
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
fn turn_failure(error: &Value, limits: Option<&UsageLimits>) -> TurnFailure {
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
fn error_detail(error: &Value) -> String {
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

/// How the thread's turns run.
struct TurnSettings {
    model: String,
    effort: Option<String>,
    fast_tier: Option<String>,
    access: AccessMode,
    cwd: String,
}

impl TurnSettings {
    fn turn_params(&self, thread_id: &str, input: &[Value], planning: bool) -> Value {
        let (approval_policy, reviewer, sandbox) = policy(self.access);
        let mut params = json!({
            "threadId": thread_id,
            "input": input,
            "cwd": self.cwd,
            "summary": "auto",
            "approvalPolicy": approval_policy,
            "approvalsReviewer": reviewer,
            "sandboxPolicy": sandbox,
        });
        if !self.model.is_empty() {
            params["model"] = json!(self.model);
            params["collaborationMode"] = json!({
                "mode": if planning { "plan" } else { "default" },
                "settings": {
                    "model": self.model,
                    "reasoning_effort": self.effort,
                    "developer_instructions": null,
                },
            });
        }
        if let Some(effort) = &self.effort {
            params["effort"] = json!(effort);
        }
        if let Some(tier) = &self.fast_tier {
            params["serviceTier"] = json!(tier);
        }
        params
    }
}

/// Codex's approval policy, who reviews its approvals and its sandbox, for hoshi's access mode.
fn policy(access: AccessMode) -> (&'static str, &'static str, Value) {
    let workspace = json!({
        "type": "workspaceWrite",
        "writableRoots": [],
        "networkAccess": false,
        "excludeTmpdirEnvVar": false,
        "excludeSlashTmp": false,
    });
    match access {
        AccessMode::Ask => (
            "untrusted",
            "user",
            json!({ "type": "readOnly", "networkAccess": false }),
        ),
        AccessMode::AcceptEdits => ("on-request", "user", workspace),
        AccessMode::Auto => ("on-request", "auto_review", workspace),
        AccessMode::FullAccess => ("never", "user", json!({ "type": "dangerFullAccess" })),
    }
}

/// Starts the thread, or resumes the thread's earlier one, forked after the message a rewind
/// goes back to; its id and the model it runs. A thread that can't be resumed fails the turn:
/// a new one would not know the conversation.
fn open_thread(
    call: &dyn Fn(&str, Value) -> Result<Value, RpcError>,
    request: &TurnRequest,
    closing_wait: Duration,
) -> Result<(String, Option<String>), TurnFailure> {
    let mut params = json!({
        "cwd": request.cwd,
        "config": thread_config(request),
    });
    if let Some(model) = &request.model {
        params["model"] = json!(model);
    }
    if let Some(instructions) = &request.instructions {
        params["developerInstructions"] = json!(codex_instructions(instructions));
    }
    let opened = |answer: Value| {
        let id = answer["thread"]["id"].as_str()?.to_owned();
        Some((id, answer["model"].as_str().map(str::to_owned)))
    };
    if let Some(resumed) = &request.resume_session_id {
        let mut resume = params.clone();
        resume["threadId"] = json!(resumed);
        resume["excludeTurns"] = json!(true);
        if let Some(turn) = &request.resume_session_at {
            resume["lastTurnId"] = json!(turn);
        }
        let method = if request.resume_session_at.is_some() || request.fork_session {
            "thread/fork"
        } else {
            "thread/resume"
        };
        let answer = resume_thread(call, method, &resume, resumed, closing_wait)
            .map_err(|error| resume_failure(&error))?;
        return opened(answer).ok_or_else(|| TurnFailure::ResumeFailed {
            detail: format!("{method} answered no thread"),
        });
    }
    let answer = call("thread/start", params).map_err(|error| TurnFailure::ProviderError {
        detail: error.detail(),
    })?;
    opened(answer).ok_or_else(|| TurnFailure::ProviderError {
        detail: "the app-server started no thread".into(),
    })
}

/// Resumes or forks a thread: one archived elsewhere once unarchived, one Codex still closes
/// or another server still holds once let go, as the server of the thread's last turn that
/// ends just now.
fn resume_thread(
    call: &dyn Fn(&str, Value) -> Result<Value, RpcError>,
    method: &str,
    params: &Value,
    thread_id: &str,
    closing_wait: Duration,
) -> Result<Value, RpcError> {
    let mut answer = call(method, params.clone());
    if answer.as_ref().is_err_and(archived) {
        call("thread/unarchive", json!({ "threadId": thread_id }))?;
        answer = call(method, params.clone());
    }
    for _ in 0..CLOSING_TRIES {
        if !answer
            .as_ref()
            .is_err_and(|error| closing(error) || in_use(error))
        {
            break;
        }
        thread::sleep(closing_wait);
        answer = call(method, params.clone());
    }
    answer
}

/// Codex keeps an archived session from being resumed until it is unarchived.
fn archived(error: &RpcError) -> bool {
    let message = error.message.to_ascii_lowercase();
    (message.contains("session ") && message.contains(" is archived"))
        || message.contains("codex unarchive")
}

/// Codex still unloads the thread, as after an unsubscribe.
fn closing(error: &RpcError) -> bool {
    let message = error.message.to_ascii_lowercase();
    message.contains("thread ") && message.contains(" is closing")
}

/// Another client writes to the thread.
fn in_use(error: &RpcError) -> bool {
    error.code == -32600 && error.message.contains(ACTIVE_WRITER)
}

/// Why a thread could not be resumed: another client holds it, or Codex's own error.
fn resume_failure(error: &RpcError) -> TurnFailure {
    let detail = if in_use(error) {
        format!("thread in use: {}", error.detail())
    } else {
        error.detail()
    };
    TurnFailure::ResumeFailed { detail }
}

/// What every thread runs with: hoshi's MCP server, its tools allowed without asking, and the
/// environment the commands it runs see.
fn thread_config(request: &TurnRequest) -> Value {
    let mut config = json!({
        "tools.update_plan.enabled": true,
        "features.concurrent_reasoning_summaries": true,
    });
    if let Some(server) = &request.mcp {
        config[format!("mcp_servers.{}", crate::MCP_SERVER_NAME)] = mcp(server);
    }
    for (name, value) in &request.environment {
        config[format!("shell_environment_policy.set.{name}")] = json!(value);
    }
    config
}

/// hoshi's MCP server as Codex's config describes one reached over HTTP.
fn mcp(server: &McpServer) -> Value {
    json!({
        "url": server.url,
        "http_headers": { "Authorization": server.authorization },
        "default_tools_approval_mode": "approve",
        "tool_timeout_sec": MCP_TOOL_TIMEOUT_SECS,
    })
}

/// hoshi's instructions as Codex reads them: its tools are never deferred.
fn codex_instructions(instructions: &str) -> String {
    instructions.replace(crate::DEFERRED_TOOLS, "")
}

/// Where the skills the prompt mentions are, by name.
fn skill_paths(connection: &Connection, request: &TurnRequest) -> HashMap<String, String> {
    let wanted: HashSet<&str> = request
        .attachments
        .iter()
        .filter_map(|attachment| match &attachment.attachment {
            MessageAttachment::Skill { name } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    if wanted.is_empty() {
        return HashMap::new();
    }
    skills(connection, Path::new(&request.cwd))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|skill| {
            let name = skill["name"]
                .as_str()
                .filter(|name| wanted.contains(name))?;
            Some((name.to_owned(), skill["path"].as_str()?.to_owned()))
        })
        .collect()
}

/// A context handoff as items of the conversation: what it covers as the user's, then each
/// message as its role's.
fn history_items(handoff: &ContextHandoff) -> Vec<Value> {
    let item = |role: &str, text: String| {
        let kind = if role == "user" {
            "input_text"
        } else {
            "output_text"
        };
        json!({ "type": "message", "role": role, "content": [{ "type": kind, "text": text }] })
    };
    std::iter::once(item("user", handoff.context.clone()))
        .chain(
            handoff
                .messages
                .iter()
                .map(|message| item(message.role.as_str(), message.rendered())),
        )
        .collect()
}

fn text_input(text: &str) -> Value {
    json!({ "type": "text", "text": text, "text_elements": [] })
}

/// The message as Codex takes it: its text, with a line saying where each image and file is
/// saved, then its images and the skills it mentions.
fn user_input(
    prompt: &str,
    attachments: &[TurnAttachment],
    skills: &HashMap<String, String>,
) -> Vec<Value> {
    let text = attachments::with_context(prompt.to_owned(), attachments);
    let mut input = vec![text_input(&text)];
    for attachment in attachments {
        match (&attachment.attachment, &attachment.path) {
            (MessageAttachment::Image { .. }, Some(path)) if path.is_file() => {
                input.push(json!({ "type": "localImage", "path": path }));
            }
            (MessageAttachment::Skill { name }, _) => {
                if let Some(path) = skills.get(name) {
                    input.push(json!({ "type": "skill", "name": name, "path": path }));
                }
            }
            _ => {}
        }
    }
    input
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
