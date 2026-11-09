//! Provider adapters. Each one drives an agent CLI (Claude Code first) and
//! translates its native protocol into provider-neutral `ProviderOutput`.

mod claude;
mod claude_account;
mod claude_commands;
mod claude_control;
mod claude_install;
mod claude_models;
mod claude_output;
mod claude_prompt;
mod claude_requests;
mod claude_titles;
mod claude_tools;
mod claude_usage;
mod claude_writing;
mod process;
mod updater;

pub use claude::{ClaudeCodeAdapter, ClaudeParser};
use hoshi_core::{
    AccessMode, Effort, MessageAttachment, ProviderAccountStatus, ProviderModels, ProviderOutput,
    RequestResponse, SlashCommand, TurnFailure, UsageLimits,
};
use std::path::{Path, PathBuf};
pub use updater::{UpdateFailure, Updater, run as run_update};

pub struct TurnRequest {
    /// The thread's working directory: its worktree, or the project root.
    pub cwd: String,
    pub prompt: String,
    pub attachments: Vec<TurnAttachment>,
    pub resume_session_id: Option<String>,
    /// Resumes the session after this message of it, leaving out the rest, as a rewind asks.
    pub resume_session_at: Option<String>,
    /// Overrides the adapter's default model.
    pub model: Option<String>,
    pub effort: Option<Effort>,
    pub fast_mode: bool,
    pub ultracode: bool,
    pub access_mode: AccessMode,
    pub plan_mode: bool,
    /// hoshi's own MCP server for the agent, whose tools it may call without asking.
    pub mcp: Option<McpServer>,
    /// Added to the agent's system prompt.
    pub instructions: Option<String>,
}

/// An MCP server reached over HTTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    pub url: String,
    /// The `Authorization` header that says which thread calls.
    pub authorization: String,
}

/// The name agents know hoshi's MCP server by; its tools are `mcp__hoshi__<tool>`.
pub const MCP_SERVER_NAME: &str = "hoshi";

/// Something the message carries besides its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnAttachment {
    pub attachment: MessageAttachment,
    /// Where an image's file is; `None` for other attachments.
    pub path: Option<PathBuf>,
}

/// What a thread's title is generated from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleRequest {
    /// The first message; when regenerating, the thread's messages as `USER:` and
    /// `ASSISTANT:` sections.
    pub context: String,
    /// Set when regenerating, so an accurate title can stay.
    pub previous_title: Option<String>,
    /// Names of the images the messages carry.
    pub images: Vec<String>,
    /// `None` uses the provider's quick model.
    pub model: Option<String>,
    /// The user's own guidance for titles.
    pub instructions: String,
}

/// What a commit message is written from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitMessageRequest {
    pub branch: String,
    /// `git diff --cached --name-status` of what the commit holds.
    pub files: String,
    pub patch: String,
    /// The repository's latest commit subjects, newest first: the style to follow.
    pub recent_subjects: Vec<String>,
    /// Also name a branch for the change, as when it leaves the default branch.
    pub wants_branch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitMessage {
    pub subject: String,
    /// Empty when the subject says it all.
    pub body: String,
    /// Asked for with [`CommitMessageRequest::wants_branch`]; a name without prefix.
    pub branch: Option<String>,
}

/// What a pull request's title and description are written from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestTextRequest {
    pub base: String,
    pub head: String,
    /// `git log --oneline base..head`.
    pub commits: String,
    /// `git diff --stat base...head`.
    pub stat: String,
    pub patch: String,
    /// The repository's pull request template, to fill in.
    pub template: Option<String>,
    pub recent_subjects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestText {
    pub title: String,
    /// Markdown.
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedTitle {
    pub title: String,
    /// The subject is still unknown, such as "fix this"; worth another try once the
    /// agent answered.
    pub needs_refinement: bool,
}

/// A provider's installed CLI against its latest release.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Installation {
    /// `None` when the CLI can't be run.
    pub installed: Option<String>,
    /// `None` when the latest release can't be looked up.
    pub latest: Option<String>,
    /// `None` when hoshi can't update it, as with a CLI a system package manager installed.
    pub updater: Option<Updater>,
}

impl Installation {
    pub fn update_available(&self) -> bool {
        let version =
            |text: &Option<String>| text.as_deref().and_then(claude_models::parse_version);
        matches!(
            (version(&self.installed), version(&self.latest)),
            (Some(installed), Some(latest)) if installed < latest
        )
    }
}

/// Receives everything a turn reports, ending with `ProviderOutput::TurnCompleted`. When the
/// turn's process outlives it ([`RunningTurn::lingers`]), it goes on with what that process
/// reports later: the turns [`RunningTurn::continue_with`] or `ProviderOutput::Woke` start in
/// it, and `ProviderOutput::ProcessEnded` once it is gone.
pub type OutputSink = Box<dyn FnMut(ProviderOutput) + Send>;

pub trait ProviderAdapter: Send + Sync {
    /// Can run the CLI, so it may take a moment.
    fn models(&self) -> ProviderModels;

    /// The plan's rolling limits; `None` when they can't be read. Can run the CLI.
    fn usage_limits(&self) -> Option<UsageLimits> {
        None
    }

    /// The installed CLI, its latest release and how to update it; `None` when the
    /// provider can't tell. Runs the CLI and reaches the network.
    fn installation(&self) -> Option<Installation> {
        None
    }

    /// Reads the CLI's version again next time, after an update replaced it.
    fn forget_version(&self) {}

    /// Whether the CLI is installed and signed in. Runs the CLI, so it can take a few
    /// seconds; `Unknown` when it can't tell.
    fn account_status(&self) -> ProviderAccountStatus {
        ProviderAccountStatus::Unknown
    }

    /// What a prompt can start with in `cwd`, which may add its own; `None` when
    /// they can't be read. Can run the CLI.
    fn slash_commands(&self, _cwd: &Path) -> Option<Vec<SlashCommand>> {
        None
    }

    /// Names a thread. `None` when the provider can't. Runs the CLI, so it takes a
    /// few seconds.
    fn generate_title(&self, _request: &TitleRequest) -> Option<GeneratedTitle> {
        None
    }

    /// Writes a commit message for the staged change. `None` when the provider can't.
    fn write_commit_message(&self, _request: &CommitMessageRequest) -> Option<CommitMessage> {
        None
    }

    /// Writes a pull request's title and description. `None` when the provider can't.
    fn write_pull_request(&self, _request: &PullRequestTextRequest) -> Option<PullRequestText> {
        None
    }

    fn start_turn(
        &self,
        request: TurnRequest,
        sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure>;
}

pub trait RunningTurn: Send {
    /// Stops the turn. The sink still receives a final `TurnCompleted`.
    fn interrupt(&self);

    /// Answers a request the turn reported with `ProviderOutput::RequestOpened`. Ignored
    /// once the agent no longer waits for it.
    fn respond(&self, request_id: &str, response: &RequestResponse);

    /// Stops the subagent that call `tool_use_id` started; the turn goes on. Ignored when it
    /// already ended or the provider can't stop one alone.
    fn stop_subagent(&self, _tool_use_id: &str) {}

    /// Stops a command the turn left running in the background, by the provider's task id.
    /// Ignored when it already ended.
    fn stop_background_task(&self, _task_id: &str) {}

    /// Gives the running turn another message, which the agent reads at its next step.
    /// `false` when the turn is ending and can't take it: it then needs a turn of its own.
    fn steer(&self, _message_id: &str, _prompt: &str, _attachments: &[TurnAttachment]) -> bool {
        false
    }

    /// Makes the turn wait for background commands it started, or stop waiting with
    /// `awaited` false. Returns the ids among `task_ids` that run; `None` when the provider
    /// can't wait for them.
    fn await_background_tasks(&self, _task_ids: &[String], _awaited: bool) -> Option<Vec<String>> {
        None
    }

    /// Whether the process stays once its turn completed, keeping the commands the turn left
    /// running in the background.
    fn lingers(&self) -> bool {
        false
    }

    /// Runs the thread's next turn in the process a completed turn left running, reporting to
    /// that turn's sink. `false` when it can't, as when the request changed what the process
    /// was started with: the caller then closes it and starts a turn of its own.
    fn continue_with(&self, _request: &TurnRequest) -> bool {
        false
    }

    /// Ends the process and everything it started.
    fn close(&self) {
        self.interrupt();
    }
}

/// Truncates long tool inputs and outputs so a transcript stays light.
pub(crate) fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => format!("{}…", &text[..index]),
        None => text.to_owned(),
    }
}
