//! GitHub Copilot CLI, driven over the Agent Client Protocol (`copilot --acp`). A turn starts
//! a process with a new session, or the thread's earlier one loaded again; once it completes,
//! the process waits a while for that session's next turn.

mod account;
mod install;

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

mod pool;
mod turn;

use pool::*;
use turn::*;

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
        Some(install::installation(self.binary(), installed))
    }

    fn forget_version(&self) {
        *lock(&self.version) = None;
        *lock(&self.models) = None;
        self.idle.end_all();
    }

    fn end_idle_processes(&self) {
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
    crate::updater::with_search_path(&mut command);
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
}
