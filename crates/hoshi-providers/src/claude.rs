use crate::process::spawn_turn;
use crate::{OutputSink, ProviderAdapter, RunningTurn, TurnRequest, truncate};
use hoshi_core::{ProviderOutput, TurnFailure, TurnOutcome};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::process::Command;

const TOOL_INPUT_MAX_CHARS: usize = 2_000;
const TOOL_OUTPUT_MAX_CHARS: usize = 8_000;

pub struct ClaudeCodeAdapter {
    /// Empty means `claude` on the PATH.
    pub binary_path: Option<String>,
    /// Empty means Claude Code's own default.
    pub model: Option<String>,
    pub permission_mode: String,
}

impl ProviderAdapter for ClaudeCodeAdapter {
    fn start_turn(
        &self,
        request: TurnRequest,
        sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
        let mut command = Command::new(self.binary_path.as_deref().unwrap_or("claude"));
        command
            .current_dir(&request.cwd)
            .args(["-p", "--input-format", "stream-json"])
            .args(["--output-format", "stream-json", "--verbose"])
            .arg("--include-partial-messages")
            .args(["--permission-mode", &self.permission_mode]);
        if let Some(model) = request.model.as_ref().or(self.model.as_ref()) {
            command.args(["--model", model]);
        }
        if let Some(effort) = request.effort {
            command.args(["--effort", effort.as_str()]);
        }
        if let Some(session_id) = &request.resume_session_id {
            command.args(["--resume", session_id]);
        }
        let input = json!({
            "type": "user",
            "message": { "role": "user", "content": request.prompt },
        });
        let mut parser = ClaudeParser::default();
        let turn = spawn_turn(command, format!("{input}\n"), sink, move |line| {
            parser.parse_line(line)
        })?;
        Ok(Box::new(turn))
    }
}

/// Translates Claude Code's `stream-json` output, one line at a time.
#[derive(Default)]
pub struct ClaudeParser {
    /// Content blocks seen so far per API message, to number assistant items
    /// the same way the stream numbers its deltas.
    blocks_seen: HashMap<String, usize>,
}

impl ClaudeParser {
    pub fn parse_line(&mut self, line: &str) -> Vec<ProviderOutput> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        // Subagent traffic belongs to its parent tool call; the transcript shows only the top level.
        if !value["parent_tool_use_id"].is_null() {
            return Vec::new();
        }
        match value["type"].as_str() {
            Some("system") if value["subtype"] == "init" => value["session_id"]
                .as_str()
                .map(|session_id| ProviderOutput::SessionStarted {
                    session_id: session_id.to_owned(),
                })
                .into_iter()
                .collect(),
            Some("stream_event") => self.parse_stream_event(&value),
            Some("assistant") => self.parse_assistant(&value["message"]),
            Some("user") => parse_tool_results(&value["message"]),
            Some("result") => vec![ProviderOutput::TurnCompleted {
                outcome: parse_result(&value),
            }],
            _ => Vec::new(),
        }
    }

    fn parse_stream_event(&self, value: &Value) -> Vec<ProviderOutput> {
        let event = &value["event"];
        if event["type"] != "content_block_delta" || event["delta"]["type"] != "text_delta" {
            return Vec::new();
        }
        let (Some(message_id), Some(index), Some(delta)) = (
            value["api_message_id"].as_str(),
            event["index"].as_u64(),
            event["delta"]["text"].as_str(),
        ) else {
            return Vec::new();
        };
        vec![ProviderOutput::TextDelta {
            item_id: format!("{message_id}:{index}"),
            delta: delta.to_owned(),
        }]
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
                Some("tool_use") => outputs.push(ProviderOutput::ToolCallStarted {
                    item_id: block["id"].as_str().unwrap_or(&item_id).to_owned(),
                    name: block["name"].as_str().unwrap_or_default().to_owned(),
                    input: truncate(&block["input"].to_string(), TOOL_INPUT_MAX_CHARS),
                }),
                _ => {}
            }
        }
        outputs
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
