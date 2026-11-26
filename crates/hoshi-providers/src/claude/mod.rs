mod account;
mod commands;
mod control;
pub(crate) mod install;
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

mod parser;
mod process;
mod queue;
mod turn;

pub use parser::ClaudeParser;
use process::*;
use queue::*;
use turn::*;

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
use std::path::{Path, PathBuf};
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
    /// Empty means `claude` on the PATH, else where the native installer puts it.
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

    /// Looked up on each use, so a CLI installed since hoshi started is found.
    fn binary(&self) -> Option<PathBuf> {
        install::resolve(
            self.binary_path.as_deref(),
            crate::updater::search_path,
            std::env::home_dir().as_deref(),
        )
    }

    /// Asks the CLI once; `None` when it can't be run.
    fn version(&self) -> Option<Version> {
        let mut version = self
            .version
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *version.get_or_insert_with(|| {
            let mut command = Command::new(self.binary()?);
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
        let mut limits = usage::probe(&self.binary()?)?;
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
        let status = account::probe(self.binary().as_deref());
        self.forget_version();
        status
    }

    fn slash_commands(&self, cwd: &Path) -> Option<Vec<SlashCommand>> {
        commands::probe(&self.binary()?, cwd)
    }

    fn generate_title(&self, request: &TitleRequest) -> Option<GeneratedTitle> {
        titles::generate(&self.binary()?, request)
    }

    fn write_commit_message(&self, request: &CommitMessageRequest) -> Option<CommitMessage> {
        writing::write_commit(&self.binary()?, request)
    }

    fn write_pull_request(&self, request: &PullRequestTextRequest) -> Option<PullRequestText> {
        writing::write_pull_request(&self.binary()?, request)
    }

    fn compacts_idle_threads(&self) -> bool {
        true
    }

    fn forks_sessions(&self) -> bool {
        true
    }

    fn subagent_resumption(&self, resume_id: &str) -> Option<String> {
        Some(format!("resume it with SendMessage to '{resume_id}'"))
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
        let binary = self.binary().ok_or_else(|| TurnFailure::SpawnFailed {
            detail: "claude was not found".into(),
        })?;
        let mut command = Command::new(binary);
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

#[cfg(test)]
mod tests;
