//! The turn's running tasks. A shell command's output: the CLI only sends it with the tool
//! result, but answers `get_task_output` for a running task, so the turn asks for it now
//! and then. And subagents, which `stop_task` stops one at a time.

use crate::claude_requests::strip_ansi;
use crate::process::Stdin;
use hoshi_core::ProviderOutput;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

const POLL_EVERY: Duration = Duration::from_millis(400);
const REQUEST_PREFIX: &str = "hoshi-output:";
/// How much of a running command's output is sent each time it grows: its latest lines.
const LIVE_TAIL_BYTES: usize = 32 * 1024;

/// The output's last `LIVE_TAIL_BYTES`, from the start of a line when it was cut.
fn tail(output: &str) -> &str {
    if output.len() <= LIVE_TAIL_BYTES {
        return output;
    }
    let mut start = output.len() - LIVE_TAIL_BYTES;
    while !output.is_char_boundary(start) {
        start += 1;
    }
    let cut = &output[start..];
    match cut.find('\n') {
        Some(line_end) if !cut[line_end + 1..].is_empty() => &cut[line_end + 1..],
        _ => cut,
    }
}

#[derive(Debug)]
struct Running {
    tool_use_id: String,
    /// Bytes the command had written at the last answer, to report only changes.
    seen: Option<u64>,
}

#[derive(Debug, Default)]
struct Tasks {
    /// Shell commands still running, by task id.
    commands: HashMap<String, Running>,
    /// Subagents still running: their task id, by the call that started them.
    agents: HashMap<String, String>,
}

/// The turn's running tasks, shared by the stdout reader, the poller and the running turn.
#[derive(Debug, Clone, Default)]
pub(crate) struct LiveOutput(Arc<Mutex<Tasks>>);

impl LiveOutput {
    fn tasks(&self) -> MutexGuard<'_, Tasks> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The task of the subagent that call `tool_use_id` started, while it runs.
    pub fn agent_task(&self, tool_use_id: &str) -> Option<String> {
        self.tasks().agents.get(tool_use_id).cloned()
    }

    /// Follows a command or subagent once its task starts; a command sent to the background
    /// has no output to wait for in the turn.
    pub fn started(&self, value: &Value) {
        let (Some(task_id), Some(tool_use_id)) =
            (value["task_id"].as_str(), value["tool_use_id"].as_str())
        else {
            return;
        };
        if value["task_type"] == "local_agent" {
            self.tasks()
                .agents
                .insert(tool_use_id.to_owned(), task_id.to_owned());
            return;
        }
        if value["task_type"] != "local_bash" || value["is_backgrounded"] == true {
            return;
        }
        self.tasks().commands.insert(
            task_id.to_owned(),
            Running {
                tool_use_id: tool_use_id.to_owned(),
                seen: None,
            },
        );
    }

    pub fn follows(&self, task_id: &str) -> bool {
        self.tasks().commands.contains_key(task_id)
    }

    /// Whether a command or subagent runs, which a message sent now would move aside.
    pub fn anything_running(&self) -> bool {
        let tasks = self.tasks();
        !tasks.commands.is_empty() || !tasks.agents.is_empty()
    }

    pub fn ended(&self, value: &Value) {
        if let Some(task_id) = value["task_id"].as_str() {
            let mut tasks = self.tasks();
            tasks.commands.remove(task_id);
            tasks.agents.retain(|_, agent| agent != task_id);
        }
    }

    /// The command's output so far, from the answer to one of the turn's requests.
    pub fn answer(&self, value: &Value) -> Option<ProviderOutput> {
        let response = &value["response"];
        let task_id = response["request_id"]
            .as_str()?
            .strip_prefix(REQUEST_PREFIX)?;
        let output = &response["response"];
        let total = output["total_bytes"].as_u64()?;
        let mut tasks = self.tasks();
        let running = tasks.commands.get_mut(task_id)?;
        if running.seen == Some(total) {
            return None;
        }
        running.seen = Some(total);
        Some(ProviderOutput::ToolOutput {
            item_id: running.tool_use_id.clone(),
            output: strip_ansi(tail(output["output"].as_str().unwrap_or_default())),
        })
    }

    /// Asks for the running commands' output until the turn's stdin closes.
    pub fn poll(&self, stdin: Stdin) {
        let live = self.clone();
        thread::spawn(move || {
            while stdin.is_open() {
                thread::sleep(POLL_EVERY);
                let tasks: Vec<String> = live.tasks().commands.keys().cloned().collect();
                for task_id in tasks {
                    let request = json!({
                        "type": "control_request",
                        "request_id": format!("{REQUEST_PREFIX}{task_id}"),
                        "request": { "subtype": "get_task_output", "task_id": task_id },
                    });
                    stdin.write_line(&request.to_string());
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(total: u64, output: &str) -> Value {
        json!({ "type": "control_response", "response": {
            "subtype": "success", "request_id": "hoshi-output:b1",
            "response": { "output": output, "total_bytes": total, "truncated": false } } })
    }

    #[test]
    fn a_running_command_reports_its_output_when_it_grows() {
        let live = LiveOutput::default();
        live.started(&json!({ "task_id": "b1", "tool_use_id": "toolu_1",
            "task_type": "local_bash", "is_backgrounded": false }));

        assert_eq!(
            live.answer(&answer(7, "\u{1b}[32mtick 1\u{1b}[0m\r\n")),
            Some(ProviderOutput::ToolOutput {
                item_id: "toolu_1".into(),
                output: "tick 1\r\n".into(),
            })
        );
        assert_eq!(live.answer(&answer(7, "tick 1\r\n")), None);

        live.ended(&json!({ "task_id": "b1" }));
        assert_eq!(live.answer(&answer(14, "tick 1\r\ntick 2\r\n")), None);
    }

    #[test]
    fn one_long_final_line_keeps_its_bounded_suffix() {
        let output = format!("{}\n", "x".repeat(LIVE_TAIL_BYTES * 2));

        let sent = tail(&output);

        assert_eq!(sent.len(), LIVE_TAIL_BYTES);
        assert!(sent.ends_with('\n'));
    }

    #[test]
    fn a_long_output_is_sent_as_its_latest_whole_lines() {
        let live = LiveOutput::default();
        live.started(&json!({ "task_id": "b1", "tool_use_id": "toolu_1",
            "task_type": "local_bash", "is_backgrounded": false }));
        let line = format!("{}\n", "é".repeat(99));
        let output = line.repeat(1000);

        let Some(ProviderOutput::ToolOutput { output: sent, .. }) =
            live.answer(&answer(output.len() as u64, &output))
        else {
            panic!("no output");
        };
        assert!(sent.len() <= LIVE_TAIL_BYTES);
        assert!(sent.len() > LIVE_TAIL_BYTES - line.len());
        assert!(sent.starts_with('é') && output.ends_with(&sent));
    }

    #[test]
    fn a_running_subagent_is_known_by_the_call_that_started_it() {
        let live = LiveOutput::default();
        live.started(&json!({ "task_id": "a1", "tool_use_id": "toolu_agent",
            "task_type": "local_agent", "is_backgrounded": false }));
        assert_eq!(live.agent_task("toolu_agent").as_deref(), Some("a1"));

        live.ended(&json!({ "task_id": "a1" }));
        assert_eq!(live.agent_task("toolu_agent"), None);
    }

    #[test]
    fn background_commands_and_other_answers_are_left_alone() {
        let live = LiveOutput::default();
        live.started(&json!({ "task_id": "b1", "tool_use_id": "toolu_1",
            "task_type": "local_bash", "is_backgrounded": true }));
        assert_eq!(live.answer(&answer(7, "tick 1")), None);

        let other = json!({ "type": "control_response", "response": {
            "subtype": "success", "request_id": "hoshi-commands", "response": {} } });
        assert_eq!(live.answer(&other), None);
    }
}
