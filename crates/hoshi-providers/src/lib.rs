//! Provider adapters. Each one drives an agent CLI (Claude Code first) and
//! translates its native protocol into provider-neutral `ProviderOutput`.

mod acp;
pub mod agent_processes;
mod attachments;
mod claude;
mod codex;
mod copilot;
mod history;
mod jsonrpc;
mod packages;
mod process;
mod titles;
mod updater;
mod version;
mod writing;

pub use claude::{ClaudeCodeAdapter, ClaudeParser};
pub use codex::CodexAdapter;
pub use codex::install::{
    CodexAdapterFactory, CodexBinary, CodexInstall, CodexSetup, ManagedCodex,
    latest_version as codex_latest_version,
};
pub use codex::login::{
    CodexSignIn, account as codex_account, codex_home, sign_out as codex_sign_out,
    start_sign_in as start_codex_sign_in,
};
pub use copilot::CopilotAdapter;
pub use history::SessionFolder;
use hoshi_core::{
    AccessMode, BankedResetSpend, ContextHandoff, Effort, MessageAttachment, ProviderAccountStatus,
    ProviderModels, ProviderOutput, RequestResponse, SlashCommand, SubagentEffort, TurnFailure,
    UsageLimits,
};
use std::path::{Path, PathBuf};
pub use updater::{UpdateFailure, UpdateProgress, Updater, run as run_update};

#[derive(Clone)]
pub struct TurnRequest {
    /// The thread's working directory: its worktree, or the project root.
    pub cwd: String,
    pub prompt: String,
    pub attachments: Vec<TurnAttachment>,
    /// The images and files earlier messages carried since the conversation last compacted,
    /// oldest first: what of them the provider took in counts against what it takes in now.
    pub earlier_media: Vec<TurnAttachment>,
    pub resume_session_id: Option<String>,
    /// Resumes the session after this message of it, leaving out the rest, as a rewind asks.
    pub resume_session_at: Option<String>,
    /// Resumes a copy of the session under a new id, leaving the session as it is: a fork.
    pub fork_session: bool,
    /// Context the agent takes in before the prompt, as one that did not see part of the
    /// thread's conversation.
    pub handoff: Option<ContextHandoff>,
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
    /// How the agent chooses the model and effort of the subagents and tasks it starts.
    pub subagent_effort: SubagentEffort,
    /// Set on the agent's process, so everything it starts inherits them.
    pub environment: Vec<(String, String)>,
}

impl TurnRequest {
    /// The request with its context handoff written ahead of its prompt, for a provider that
    /// takes it inline.
    pub fn with_inline_handoff(mut self) -> Self {
        if let Some(handoff) = self.handoff.take() {
            self.prompt = handoff.inline_prompt(&self.prompt);
        }
        self
    }
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

/// Where hoshi's instructions say its tools are deferred, which the adapter leaves out when
/// the user turned Claude Code's tool search off.
pub const DEFERRED_TOOLS: &str = ", deferred: load them with ToolSearch, several at once (\"select:mcp__hoshi__<a>,mcp__hoshi__<b>\")";

/// Something the message carries besides its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnAttachment {
    pub attachment: MessageAttachment,
    /// Where an image's or file's file is; `None` for other attachments.
    pub path: Option<PathBuf>,
    /// The text of a document file, where it was written beside it.
    pub extracted_text: Option<PathBuf>,
}

/// What a thread's title is generated from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleRequest {
    /// The first message; when regenerating, the thread's messages as `USER:` and
    /// `ASSISTANT:` sections.
    pub context: String,
    /// Set when regenerating, so an accurate title can stay.
    pub previous_title: Option<String>,
    /// Names of the images and files the messages carry.
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
        let version = |text: &Option<String>| text.as_deref().and_then(version::parse_version);
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
    /// The provider's id, as threads and model choices name it.
    fn provider(&self) -> &'static str;

    /// Can run the CLI, so it may take a moment.
    fn models(&self) -> ProviderModels;

    /// The models known without running the CLI, such as a catalog bundled with hoshi.
    fn offline_models(&self) -> Option<ProviderModels> {
        None
    }

    /// The plan's rolling limits; `None` when they can't be read. Can run the CLI.
    fn usage_limits(&self) -> Option<UsageLimits> {
        None
    }

    /// Whether [`ProviderAdapter::usage_limits`] can read the plan's limits at all.
    fn reports_usage_limits(&self) -> bool {
        false
    }

    /// Spends the banked reset of grant `grant_id`, which [`UsageLimits`] named. Retrying with
    /// the same `request_id` repeats the same spend instead of spending another. Reaches the
    /// network.
    fn spend_banked_reset(&self, _grant_id: &str, _request_id: &str) -> BankedResetSpend {
        BankedResetSpend::NoneLeft
    }

    /// The installed CLI, its latest release and how to update it; `None` when the
    /// provider can't tell. Runs the CLI and reaches the network.
    fn installation(&self) -> Option<Installation> {
        None
    }

    /// Reads the CLI's version again next time, after an update replaced it.
    fn forget_version(&self) {}

    /// Ends the processes kept between turns, which hold the files an update replaces.
    fn end_idle_processes(&self) {}

    /// Whether the CLI is installed and signed in. Runs the CLI, so it can take a few
    /// seconds; `Unknown` when it can't tell.
    fn account_status(&self) -> ProviderAccountStatus {
        ProviderAccountStatus::Unknown
    }

    /// The account name the last [`ProviderAdapter::account_status`] found signed in, for
    /// providers that report one that way.
    fn signed_in_as(&self) -> Option<String> {
        None
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

    /// The folders the CLI ran sessions in, read from the files it keeps them in.
    fn session_folders(&self) -> Vec<SessionFolder> {
        Vec::new()
    }

    /// Whether an idle thread over [`hoshi_core::IDLE_COMPACTION_TOKENS`] is worth compacting
    /// with `/compact` around when its prompt cache expires, which the provider reports in
    /// its [`hoshi_core::ContextUsage`].
    fn compacts_idle_threads(&self) -> bool {
        false
    }

    /// Whether [`TurnRequest::fork_session`] resumes a copy of a session at
    /// [`TurnRequest::resume_session_at`], leaving the session itself untouched.
    fn forks_sessions(&self) -> bool {
        false
    }

    /// Whether the adapter reports how it delivered a turn's
    /// [`TurnRequest::handoff`] with `ProviderOutput::ContextHandoffDelivered`; one that does
    /// not writes it ahead of the prompt.
    fn injects_context(&self) -> bool {
        false
    }

    /// Whether a running turn is stopped so a message can move its thread to another provider
    /// at once; otherwise the message waits for the turn to end.
    fn restarts_to_switch(&self) -> bool {
        false
    }

    /// How the agent goes on with a subagent of its own that ended before it finished, by the
    /// `resume_id` its call's [`hoshi_core::ToolDetail::Subagent`] reports; `None` when it
    /// can't.
    fn subagent_resumption(&self, _resume_id: &str) -> Option<String> {
        None
    }

    /// The context window of `model`, `None` for the default one, when known before a turn
    /// reports it.
    fn context_window(&self, _model: Option<&str>) -> Option<u64> {
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

    /// Gives the running turn another message. The agent reads it `at_once`, as when the
    /// user steers it, or once the step it is on is over, as fits news it didn't ask for.
    /// `false` when the turn is ending and can't take it: it then needs a turn of its own.
    fn steer(
        &self,
        _message_id: &str,
        _prompt: &str,
        _attachments: &[TurnAttachment],
        _at_once: bool,
    ) -> bool {
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

    /// The kept process has work ahead besides its background commands, such as a run the
    /// agent scheduled, so it may stay quiet as long as they may.
    fn awaits_work(&self) -> bool {
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

/// A tool input as JSON within `max_chars`: its texts are cut short, so it still parses.
pub(crate) fn truncate_json(value: &serde_json::Value, max_chars: usize) -> String {
    let fits = |text: &str| text.chars().nth(max_chars).is_none();
    let whole = value.to_string();
    if fits(&whole) {
        return whole;
    }
    let (mut low, mut high) = (0, longest_text(value));
    let mut best = None;
    while low <= high {
        let limit = low + (high - low) / 2;
        let text = cut_texts(value, limit).to_string();
        if fits(&text) {
            best = Some(text);
            low = limit + 1;
        } else if limit == 0 {
            break;
        } else {
            high = limit - 1;
        }
    }
    best.unwrap_or_else(|| truncate(&whole, max_chars))
}

fn longest_text(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::String(text) => text.chars().count(),
        serde_json::Value::Array(items) => items.iter().map(longest_text).max().unwrap_or(0),
        serde_json::Value::Object(fields) => fields.values().map(longest_text).max().unwrap_or(0),
        _ => 0,
    }
}

fn cut_texts(value: &serde_json::Value, limit: usize) -> serde_json::Value {
    match value {
        serde_json::Value::String(text) => serde_json::Value::String(truncate(text, limit)),
        serde_json::Value::Array(items) => {
            items.iter().map(|item| cut_texts(item, limit)).collect()
        }
        serde_json::Value::Object(fields) => fields
            .iter()
            .map(|(key, field)| (key.clone(), cut_texts(field, limit)))
            .collect(),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_long_input_still_parses_and_keeps_its_short_fields() {
        let input = json!({
            "description": "List files",
            "subagent_type": "Explore",
            "prompt": "x".repeat(5_000),
        });
        let text = truncate_json(&input, 2_000);
        assert!(text.chars().count() <= 2_000);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["description"], "List files");
        assert_eq!(parsed["subagent_type"], "Explore");
        assert!(parsed["prompt"].as_str().unwrap().ends_with('…'));
    }

    #[test]
    fn an_inline_handoff_goes_ahead_of_the_user_s_message() {
        let request = TurnRequest {
            cwd: "/repo".into(),
            prompt: "Go on.".into(),
            attachments: Vec::new(),
            earlier_media: Vec::new(),
            resume_session_id: None,
            resume_session_at: None,
            fork_session: false,
            handoff: Some(ContextHandoff {
                context: "Context handoff (full_thread_summary):".into(),
                messages: Vec::new(),
                omitted_items: 0,
            }),
            model: None,
            effort: None,
            fast_mode: false,
            ultracode: false,
            access_mode: AccessMode::default(),
            plan_mode: false,
            mcp: None,
            instructions: None,
            subagent_effort: SubagentEffort::Thread,
            environment: Vec::new(),
        };

        let inline = request.with_inline_handoff();

        assert_eq!(
            inline.prompt,
            "Context handoff (full_thread_summary):\n\nUser message:\nGo on."
        );
        assert!(inline.handoff.is_none());
    }

    #[test]
    fn a_short_input_stays_whole() {
        let input = json!({ "command": "ls" });
        assert_eq!(truncate_json(&input, 2_000), input.to_string());
    }
}
