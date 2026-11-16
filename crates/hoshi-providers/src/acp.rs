//! The Agent Client Protocol, over [`crate::jsonrpc`]: [`Translator`] turns a session's
//! updates into provider-neutral output.

use crate::{truncate, truncate_json};
use hoshi_core::{ContextUsage, ProviderOutput, Task, TaskStatus, ToolDetail};
use serde_json::{Value, json};
use std::collections::HashMap;

const TOOL_INPUT_MAX_CHARS: usize = 2_000;
const TOOL_OUTPUT_MAX_CHARS: usize = 20_000;
const DIFF_MAX_CHARS: usize = 60_000;

/// A message or reasoning being streamed.
#[derive(Debug, Default)]
struct OpenText {
    item_id: String,
    text: String,
    reasoning: bool,
}

/// A tool call reported so far.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolCall {
    pub name: String,
    pub input: Value,
    pub done: bool,
}

/// Turns `session/update` notifications into provider output, item by item.
#[derive(Debug, Default)]
pub(crate) struct Translator {
    open: Option<OpenText>,
    tools: HashMap<String, ToolCall>,
    /// Numbers messages the agent sent without an id of their own.
    unnamed: u64,
}

impl Translator {
    pub fn tool(&self, tool_call_id: &str) -> Option<&ToolCall> {
        self.tools.get(tool_call_id)
    }

    /// The output one update makes. Updates hoshi does not show make none.
    pub fn update(&mut self, update: &Value) -> Vec<ProviderOutput> {
        let mut outputs = Vec::new();
        match update["sessionUpdate"].as_str().unwrap_or_default() {
            kind @ ("agent_message_chunk" | "agent_thought_chunk") => {
                let Some(text) = update["content"]["text"].as_str() else {
                    return outputs;
                };
                let reasoning = kind == "agent_thought_chunk";
                let id = update["messageId"].as_str();
                let continues = self.open.as_ref().is_some_and(|open| {
                    open.reasoning == reasoning && id.is_none_or(|id| open.item_id == id)
                });
                if !continues {
                    self.flush(&mut outputs);
                    let item_id = match id {
                        Some(id) => id.to_owned(),
                        None => {
                            self.unnamed += 1;
                            format!("message-{}", self.unnamed)
                        }
                    };
                    self.open = Some(OpenText {
                        item_id,
                        text: String::new(),
                        reasoning,
                    });
                }
                let open = self.open.as_mut().expect("opened above");
                open.text.push_str(text);
                let (item_id, delta) = (open.item_id.clone(), text.to_owned());
                outputs.push(if reasoning {
                    ProviderOutput::ReasoningDelta { item_id, delta }
                } else {
                    ProviderOutput::TextDelta { item_id, delta }
                });
            }
            "tool_call" => {
                self.flush(&mut outputs);
                self.tool_call(update, &mut outputs);
            }
            "tool_call_update" => self.tool_call(update, &mut outputs),
            "plan" => {
                self.flush(&mut outputs);
                let tasks = update["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|entry| Task {
                        subject: entry["content"].as_str().unwrap_or_default().to_owned(),
                        active_form: None,
                        status: match entry["status"].as_str() {
                            Some("in_progress") => TaskStatus::InProgress,
                            Some("completed") => TaskStatus::Completed,
                            _ => TaskStatus::Pending,
                        },
                    })
                    .collect();
                outputs.push(ProviderOutput::TaskList { tasks });
            }
            "usage_update" => {
                if let Some(used) = update["used"].as_u64() {
                    outputs.push(ProviderOutput::ContextUsage(ContextUsage {
                        used_tokens: used,
                        max_tokens: update["size"].as_u64(),
                        cache_ttl_secs: None,
                        cache_expires_at: None,
                    }));
                }
            }
            _ => {}
        }
        outputs
    }

    fn tool_call(&mut self, update: &Value, outputs: &mut Vec<ProviderOutput>) {
        let Some(id) = update["toolCallId"].as_str() else {
            return;
        };
        if !self.tools.contains_key(id) {
            let (name, input) = normalize_tool(update);
            outputs.push(ProviderOutput::ToolCallStarted {
                item_id: id.to_owned(),
                name: name.clone(),
                input: truncate_json(&input, TOOL_INPUT_MAX_CHARS),
            });
            self.tools.insert(
                id.to_owned(),
                ToolCall {
                    name,
                    input,
                    done: false,
                },
            );
        }
        let tool = self.tools.get_mut(id).expect("inserted above");
        if tool.done {
            return;
        }
        let status = update["status"].as_str();
        let output = tool_output(update);
        match status {
            Some("completed" | "failed") => {
                tool.done = true;
                outputs.push(ProviderOutput::ToolCallCompleted {
                    item_id: id.to_owned(),
                    output: truncate(&output, TOOL_OUTPUT_MAX_CHARS),
                    is_error: status == Some("failed"),
                    detail: file_change(&tool.name, &tool.input, update),
                });
            }
            _ if !output.is_empty() => outputs.push(ProviderOutput::ToolOutput {
                item_id: id.to_owned(),
                output: truncate(&output, TOOL_OUTPUT_MAX_CHARS),
            }),
            _ => {}
        }
    }

    /// Ends the message or reasoning being streamed, if any.
    pub fn flush(&mut self, outputs: &mut Vec<ProviderOutput>) {
        let Some(open) = self.open.take() else {
            return;
        };
        let OpenText {
            item_id,
            text,
            reasoning,
        } = open;
        outputs.push(if reasoning {
            ProviderOutput::Reasoning { item_id, text }
        } else {
            ProviderOutput::AssistantMessage { item_id, text }
        });
    }
}

/// A tool call as hoshi names its tools, so the transcript shows it like any other agent's:
/// shell commands as `Bash`, reads as `Read`, and so on. Anything else keeps the agent's
/// title and input.
pub(crate) fn normalize_tool(call: &Value) -> (String, Value) {
    let raw = &call["rawInput"];
    let title = call["title"].as_str().unwrap_or("Tool").trim();
    let text = |key: &str| raw[key].as_str().map(str::to_owned);
    let path = || {
        text("path")
            .or_else(|| text("file_path"))
            .or_else(|| call["locations"][0]["path"].as_str().map(str::to_owned))
    };
    match call["kind"].as_str().unwrap_or_default() {
        "execute" if raw["command"].is_string() => (
            "Bash".into(),
            json!({ "command": text("command"), "description": text("description") }),
        ),
        "read" if path().is_some() => ("Read".into(), json!({ "file_path": path() })),
        "edit" if raw.get("file_text").is_some() => (
            "Write".into(),
            json!({ "file_path": path(), "content": text("file_text") }),
        ),
        "edit" if path().is_some() => (
            "Edit".into(),
            json!({
                "file_path": path(),
                "old_string": text("old_str"),
                "new_string": text("new_str"),
            }),
        ),
        "fetch" if raw["url"].is_string() => ("WebFetch".into(), json!({ "url": text("url") })),
        "fetch" | "search" if raw["query"].is_string() => {
            ("WebSearch".into(), json!({ "query": text("query") }))
        }
        _ if raw["agent_type"].is_string() && raw["prompt"].is_string() => (
            "Task".into(),
            json!({
                "subagent_type": text("agent_type"),
                "description": text("description").or_else(|| text("name")),
                "prompt": text("prompt"),
            }),
        ),
        "other" if mcp_tool(title).is_some() => (mcp_tool(title).unwrap_or_default(), raw.clone()),
        _ if raw["pattern"].is_string() => {
            let paths = text("path").or_else(|| text("paths"));
            if raw.get("output_mode").is_some() || raw.get("-n").is_some() {
                (
                    "Grep".into(),
                    json!({ "pattern": text("pattern"), "path": paths }),
                )
            } else {
                (
                    "Glob".into(),
                    json!({ "pattern": text("pattern"), "path": paths }),
                )
            }
        }
        _ => (title.to_owned(), raw.clone()),
    }
}

/// hoshi's MCP tool a call titled `hoshi-<tool>` runs, as hoshi names it.
fn mcp_tool(title: &str) -> Option<String> {
    let tool = title.strip_prefix(&format!("{}-", crate::MCP_SERVER_NAME))?;
    (!tool.is_empty() && !tool.contains(char::is_whitespace))
        .then(|| format!("mcp__{}__{tool}", crate::MCP_SERVER_NAME))
}

/// The text a tool call reported: its content blocks, else its raw output.
fn tool_output(update: &Value) -> String {
    let blocks: Vec<&str> = update["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| block["content"]["text"].as_str())
        .collect();
    if !blocks.is_empty() {
        return blocks.join("\n");
    }
    match &update["rawOutput"] {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        output => output["content"]
            .as_str()
            .map_or_else(|| output.to_string(), str::to_owned),
    }
}

/// The change an edit made, from the diff the agent reported with it.
fn file_change(name: &str, input: &Value, update: &Value) -> Option<ToolDetail> {
    if !matches!(name, "Edit" | "Write") {
        return None;
    }
    let path = input["file_path"].as_str()?.to_owned();
    let diff = update["rawOutput"]["detailedContent"]
        .as_str()
        .filter(|diff| diff.contains("@@"))?;
    let hunks = &diff[diff.find("@@")?..];
    let count = |sign: char| {
        hunks
            .lines()
            .filter(|line| line.starts_with(sign) && !line.starts_with("+++"))
            .filter(|line| !line.starts_with("---"))
            .count() as u32
    };
    Some(ToolDetail::FileChange {
        path,
        additions: count('+'),
        deletions: count('-'),
        diff: Some(truncate(hunks, DIFF_MAX_CHARS)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(kind: &str, id: Option<&str>, text: &str) -> Value {
        let mut update = json!({
            "sessionUpdate": kind,
            "content": { "type": "text", "text": text },
        });
        if let Some(id) = id {
            update["messageId"] = json!(id);
        }
        update
    }

    #[test]
    fn chunks_stream_a_message_that_ends_when_something_else_starts() {
        let mut translator = Translator::default();
        let mut outputs = translator.update(&chunk("agent_thought_chunk", Some("r"), "hmm"));
        outputs.extend(translator.update(&chunk("agent_message_chunk", Some("m"), "Hel")));
        outputs.extend(translator.update(&chunk("agent_message_chunk", Some("m"), "lo")));
        translator.flush(&mut outputs);

        assert_eq!(
            outputs,
            [
                ProviderOutput::ReasoningDelta {
                    item_id: "r".into(),
                    delta: "hmm".into()
                },
                ProviderOutput::Reasoning {
                    item_id: "r".into(),
                    text: "hmm".into()
                },
                ProviderOutput::TextDelta {
                    item_id: "m".into(),
                    delta: "Hel".into()
                },
                ProviderOutput::TextDelta {
                    item_id: "m".into(),
                    delta: "lo".into()
                },
                ProviderOutput::AssistantMessage {
                    item_id: "m".into(),
                    text: "Hello".into()
                },
            ]
        );
    }

    #[test]
    fn a_message_without_an_id_gets_one_of_its_own() {
        let mut translator = Translator::default();
        translator.update(&chunk("agent_message_chunk", None, "Error: no model"));
        let mut outputs = Vec::new();
        translator.flush(&mut outputs);
        assert_eq!(
            outputs,
            [ProviderOutput::AssistantMessage {
                item_id: "message-1".into(),
                text: "Error: no model".into()
            }]
        );
    }

    #[test]
    fn a_shell_command_runs_as_bash_and_completes_with_its_output() {
        let mut translator = Translator::default();
        let started = translator.update(&json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "c1",
            "title": "Run echo hi",
            "kind": "execute",
            "status": "pending",
            "rawInput": { "command": "echo hi", "description": "Run echo hi", "mode": "sync" },
        }));
        let running = translator.update(&json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "c1",
            "content": [{ "type": "content", "content": { "type": "text", "text": "hi\n" } }],
        }));
        let done = translator.update(&json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "c1",
            "status": "completed",
            "content": [{ "type": "content", "content": { "type": "text", "text": "hi\nexit 0" } }],
        }));

        assert_eq!(
            started,
            [ProviderOutput::ToolCallStarted {
                item_id: "c1".into(),
                name: "Bash".into(),
                input: r#"{"command":"echo hi","description":"Run echo hi"}"#.into(),
            }]
        );
        assert_eq!(
            running,
            [ProviderOutput::ToolOutput {
                item_id: "c1".into(),
                output: "hi\n".into()
            }]
        );
        assert_eq!(
            done,
            [ProviderOutput::ToolCallCompleted {
                item_id: "c1".into(),
                output: "hi\nexit 0".into(),
                is_error: false,
                detail: None,
            }]
        );
    }

    #[test]
    fn tools_take_the_names_the_transcript_knows() {
        let name = |call: Value| normalize_tool(&call).0;
        assert_eq!(
            name(json!({ "kind": "read", "rawInput": { "path": "a.txt" } })),
            "Read"
        );
        assert_eq!(
            name(
                json!({ "kind": "edit", "rawInput": { "path": "a", "old_str": "x", "new_str": "y" } })
            ),
            "Edit"
        );
        assert_eq!(
            name(json!({ "kind": "edit", "rawInput": { "path": "a", "file_text": "x" } })),
            "Write"
        );
        assert_eq!(
            name(
                json!({ "kind": "other", "rawInput": { "pattern": "hello", "output_mode": "content" } })
            ),
            "Grep"
        );
        assert_eq!(
            name(
                json!({ "kind": "other", "rawInput": { "agent_type": "explore", "prompt": "look" } })
            ),
            "Task"
        );
        assert_eq!(
            name(json!({ "kind": "other", "title": "Custom thing", "rawInput": {} })),
            "Custom thing"
        );
        assert_eq!(
            name(json!({ "kind": "other", "title": "hoshi-delegate_task", "rawInput": {} })),
            "mcp__hoshi__delegate_task"
        );
    }

    #[test]
    fn an_edit_reports_its_change_from_the_agents_diff() {
        let mut translator = Translator::default();
        translator.update(&json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "e1",
            "kind": "edit",
            "rawInput": { "path": "src/a.rs", "old_str": "a", "new_str": "b" },
        }));
        let done = translator.update(&json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "e1",
            "status": "completed",
            "rawOutput": {
                "content": "File updated",
                "detailedContent": "\ndiff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,2 +1,2 @@\n-a\n+b\n same\n",
            },
        }));
        let ProviderOutput::ToolCallCompleted { detail, output, .. } = &done[0] else {
            panic!("expected a completed call, got {done:?}");
        };
        assert_eq!(output, "File updated");
        assert_eq!(
            detail,
            &Some(ToolDetail::FileChange {
                path: "src/a.rs".into(),
                additions: 1,
                deletions: 1,
                diff: Some("@@ -1,2 +1,2 @@\n-a\n+b\n same\n".into()),
            })
        );
    }
}
