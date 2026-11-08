//! Provider adapters. Each one drives an agent CLI (Claude Code first) and
//! translates its native protocol into provider-neutral `ProviderOutput`.

mod claude;
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
mod process;
mod updater;

pub use claude::{ClaudeCodeAdapter, ClaudeParser};
use hoshi_core::{
    AccessMode, Effort, MessageAttachment, ProviderModels, ProviderOutput, RequestResponse,
    SlashCommand, TurnFailure, UsageLimits,
};
use std::path::{Path, PathBuf};
pub use updater::{UpdateFailure, Updater, run as run_update};

pub struct TurnRequest {
    /// The thread's working directory: its worktree, or the project root.
    pub cwd: String,
    pub prompt: String,
    pub attachments: Vec<TurnAttachment>,
    pub resume_session_id: Option<String>,
    /// Overrides the adapter's default model.
    pub model: Option<String>,
    pub effort: Option<Effort>,
    pub fast_mode: bool,
    pub ultracode: bool,
    pub access_mode: AccessMode,
    pub plan_mode: bool,
}

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

/// Receives everything a turn reports, ending with `ProviderOutput::TurnCompleted`.
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
}

/// Truncates long tool inputs and outputs so a transcript stays light.
pub(crate) fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => format!("{}…", &text[..index]),
        None => text.to_owned(),
    }
}
