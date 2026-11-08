use crate::claude_commands;
use crate::claude_install;
use crate::claude_models::{TurnModel, Version, manifest, parse_version};
use crate::claude_prompt;
use crate::claude_titles;
use crate::claude_usage;
use crate::process::{hide_console, spawn_turn};
use crate::{
    GeneratedTitle, Installation, OutputSink, ProviderAdapter, RunningTurn, TitleRequest,
    TurnRequest, truncate,
};
use hoshi_core::{
    AccessMode, ContextUsage, ProviderModels, ProviderOutput, SlashCommand, TurnFailure,
    TurnOutcome, UsageLimits,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const TOOL_INPUT_MAX_CHARS: usize = 2_000;
const TOOL_OUTPUT_MAX_CHARS: usize = 8_000;

pub struct ClaudeCodeAdapter {
    /// Empty means `claude` on the PATH.
    binary_path: Option<String>,
    /// Runs threads without a model of their own; empty means the manifest's default.
    model: Option<String>,
    /// `None` until asked; then `Some(None)` when the CLI can't be run.
    version: Mutex<Option<Option<Version>>>,
}

impl ClaudeCodeAdapter {
    pub fn new(binary_path: Option<String>, model: Option<String>) -> Self {
        Self {
            binary_path,
            model,
            version: Mutex::default(),
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
}

impl ProviderAdapter for ClaudeCodeAdapter {
    fn models(&self) -> ProviderModels {
        let today = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| (elapsed.as_secs() / 86_400) as i64)
            .unwrap_or_default();
        manifest().catalog(self.version(), today)
    }

    fn usage_limits(&self) -> Option<UsageLimits> {
        claude_usage::probe(self.binary())
    }

    fn installation(&self) -> Option<Installation> {
        let installed = self
            .version()
            .map(|(major, minor, patch)| format!("{major}.{minor}.{patch}"));
        Some(claude_install::installation(self.binary(), installed))
    }

    fn forget_version(&self) {
        *self
            .version
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    fn slash_commands(&self, cwd: &Path) -> Option<Vec<SlashCommand>> {
        claude_commands::probe(self.binary(), cwd)
    }

    fn generate_title(&self, request: &TitleRequest) -> Option<GeneratedTitle> {
        claude_titles::generate(self.binary(), request)
    }

    fn start_turn(
        &self,
        request: TurnRequest,
        sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
        let turn_model = manifest().resolve(
            request.model.as_deref().or(self.model.as_deref()),
            request.effort,
            request.fast_mode,
            request.ultracode,
        );
        let mut command = Command::new(self.binary());
        command
            .current_dir(&request.cwd)
            .args(["-p", "--input-format", "stream-json"])
            .args(["--output-format", "stream-json", "--verbose"])
            .arg("--include-partial-messages")
            .args(["--thinking-display", "summarized"])
            .args(["--permission-mode", permission_mode(&request)]);
        if request.access_mode == AccessMode::FullAccess && !request.plan_mode {
            command.arg("--allow-dangerously-skip-permissions");
        }
        command.args(["--model", &turn_model.slug]);
        if let Some(effort) = &turn_model.effort {
            command.args(["--effort", effort]);
        }
        if let Some(settings) = session_settings(&turn_model) {
            command.args(["--settings", &settings.to_string()]);
        }
        if let Some(session_id) = &request.resume_session_id {
            command.args(["--resume", session_id]);
        }
        let input = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": claude_prompt::message_content(&request.prompt, &request.attachments),
            },
        });
        let mut parser = ClaudeParser::new(manifest().context_window(&turn_model.slug));
        let turn = spawn_turn(command, format!("{input}\n"), sink, move |line| {
            parser.parse_line(line)
        })?;
        Ok(Box::new(turn))
    }
}

fn permission_mode(request: &TurnRequest) -> &'static str {
    if request.plan_mode {
        return "plan";
    }
    match request.access_mode {
        AccessMode::AcceptEdits => "acceptEdits",
        AccessMode::Auto => "auto",
        AccessMode::FullAccess => "bypassPermissions",
    }
}

/// Ultracode runs through the `Workflow` tool, which `-p` would refuse
/// without an allow rule since it never prompts.
fn session_settings(model: &TurnModel) -> Option<Value> {
    if !model.fast_mode && !model.ultracode {
        return None;
    }
    let mut settings = json!({});
    if model.fast_mode {
        settings["fastMode"] = json!(true);
    }
    if model.ultracode {
        settings["ultracode"] = json!(true);
        settings["permissions"] = json!({ "allow": ["Workflow"] });
    }
    Some(settings)
}

/// Translates Claude Code's `stream-json` output, one line at a time.
#[derive(Default)]
pub struct ClaudeParser {
    /// Content blocks seen so far per API message, to number assistant items
    /// the same way the stream numbers its deltas.
    blocks_seen: HashMap<String, usize>,
    /// The window of the turn's model, until the turn's result reports the real one.
    context_window: Option<u64>,
    /// The model of the agent's latest call, whose window the result reports.
    model: Option<String>,
    used_tokens: Option<u64>,
    /// Calls of the tools that move the session, by id, to read where their results lead.
    moves: HashMap<String, WorktreeMove>,
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

    pub fn parse_line(&mut self, line: &str) -> Vec<ProviderOutput> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        // Subagent traffic belongs to its parent tool call; the transcript shows only the top level.
        if !value["parent_tool_use_id"].is_null() {
            return Vec::new();
        }
        match value["type"].as_str() {
            Some("system") if value["subtype"] == "init" => {
                let session =
                    value["session_id"]
                        .as_str()
                        .map(|session_id| ProviderOutput::SessionStarted {
                            session_id: session_id.to_owned(),
                        });
                // A resumed session reopens the worktree it was in, wherever it was started.
                let cwd = value["cwd"]
                    .as_str()
                    .map(|path| ProviderOutput::WorkingDirectory {
                        path: path.to_owned(),
                    });
                session.into_iter().chain(cwd).collect()
            }
            Some("stream_event") => self.parse_stream_event(&value),
            Some("assistant") => self.parse_assistant(&value["message"]),
            Some("user") => {
                let mut outputs = parse_tool_results(&value["message"]);
                outputs.extend(self.worktree_move(&value));
                outputs
            }
            Some("rate_limit_event") => claude_usage::rate_limit_event(&value["rate_limit_info"])
                .map(ProviderOutput::UsageLimits)
                .into_iter()
                .collect(),
            Some("result") => {
                let mut outputs: Vec<_> = self.reported_window(&value).into_iter().collect();
                outputs.push(ProviderOutput::TurnCompleted {
                    outcome: parse_result(&value),
                });
                outputs
            }
            _ => Vec::new(),
        }
    }

    fn parse_stream_event(&mut self, value: &Value) -> Vec<ProviderOutput> {
        let event = &value["event"];
        match event["type"].as_str() {
            Some("message_start") => {
                let message = &event["message"];
                if let Some(model) = message["model"].as_str() {
                    self.model = Some(model.to_owned());
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
        match (
            delta["type"].as_str(),
            delta["text"].as_str(),
            delta["thinking"].as_str(),
        ) {
            (Some("text_delta"), Some(text), _) => vec![ProviderOutput::TextDelta {
                item_id,
                delta: text.to_owned(),
            }],
            (Some("thinking_delta"), _, Some(thinking)) => vec![ProviderOutput::ReasoningDelta {
                item_id,
                delta: thinking.to_owned(),
            }],
            _ => Vec::new(),
        }
    }

    fn parse_assistant(&mut self, message: &Value) -> Vec<ProviderOutput> {
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
                    outputs.push(ProviderOutput::ToolCallStarted {
                        item_id: block["id"].as_str().unwrap_or(&item_id).to_owned(),
                        name: name.to_owned(),
                        input: truncate(&block["input"].to_string(), TOOL_INPUT_MAX_CHARS),
                    });
                }
                _ => {}
            }
        }
        outputs
    }
}

impl ClaudeParser {
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
        }))
    }
}

fn parse_tool_results(message: &Value) -> Vec<ProviderOutput> {
    let Some(blocks) = message["content"].as_array() else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter(|block| block["type"] == "tool_result")
        .filter_map(|block| {
            let item_id = block["tool_use_id"].as_str()?.to_owned();
            let output = match &block["content"] {
                Value::String(text) => text.clone(),
                Value::Array(parts) => parts
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            };
            Some(ProviderOutput::ToolCallCompleted {
                item_id,
                output: truncate(&output, TOOL_OUTPUT_MAX_CHARS),
                is_error: block["is_error"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

fn parse_result(value: &Value) -> TurnOutcome {
    if value["is_error"].as_bool().unwrap_or(false) {
        let subtype = value["subtype"].as_str().unwrap_or("error");
        let result = value["result"].as_str().unwrap_or_default();
        TurnOutcome::Failed {
            reason: TurnFailure::ProviderError {
                detail: format!("{subtype}: {result}"),
            },
        }
    } else {
        TurnOutcome::Completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> TurnRequest {
        TurnRequest {
            cwd: ".".into(),
            prompt: "hi".into(),
            attachments: Vec::new(),
            resume_session_id: None,
            model: None,
            effort: None,
            fast_mode: false,
            ultracode: false,
            access_mode: AccessMode::AcceptEdits,
            plan_mode: false,
        }
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
        let mut request = request();
        assert_eq!(permission_mode(&request), "acceptEdits");
        request.access_mode = AccessMode::FullAccess;
        assert_eq!(permission_mode(&request), "bypassPermissions");
        request.plan_mode = true;
        assert_eq!(permission_mode(&request), "plan");
    }

    #[test]
    fn no_settings_are_passed_when_both_modes_are_off() {
        assert_eq!(session_settings(&model(false, false)), None);
    }

    #[test]
    fn fast_mode_and_ultracode_go_through_settings() {
        assert_eq!(
            session_settings(&model(true, false)),
            Some(json!({ "fastMode": true }))
        );
        assert_eq!(
            session_settings(&model(false, true)),
            Some(json!({ "ultracode": true, "permissions": { "allow": ["Workflow"] } }))
        );
    }
}
