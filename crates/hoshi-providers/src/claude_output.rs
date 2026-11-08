//! A shell command's output while it runs. The CLI only sends it with the tool result, but
//! answers `get_task_output` for a running task, so the turn asks for it now and then.

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

#[derive(Debug)]
struct Running {
    tool_use_id: String,
    /// Bytes the command had written at the last answer, to report only changes.
    seen: Option<u64>,
}

/// The turn's shell commands still running, by task id.
#[derive(Debug, Clone, Default)]
pub(crate) struct LiveOutput(Arc<Mutex<HashMap<String, Running>>>);

impl LiveOutput {
    fn tasks(&self) -> MutexGuard<'_, HashMap<String, Running>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Follows a command once its task starts; one sent to the background has no output to
    /// wait for in the turn.
    pub fn started(&self, value: &Value) {
        let (Some(task_id), Some(tool_use_id)) =
            (value["task_id"].as_str(), value["tool_use_id"].as_str())
        else {
            return;
        };
        if value["task_type"] != "local_bash" || value["is_backgrounded"] == true {
            return;
        }
        self.tasks().insert(
            task_id.to_owned(),
            Running {
                tool_use_id: tool_use_id.to_owned(),
                seen: None,
            },
        );
    }

    pub fn ended(&self, value: &Value) {
        if let Some(task_id) = value["task_id"].as_str() {
            self.tasks().remove(task_id);
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
        let running = tasks.get_mut(task_id)?;
        if running.seen == Some(total) {
            return None;
        }
        running.seen = Some(total);
        Some(ProviderOutput::ToolOutput {
            item_id: running.tool_use_id.clone(),
            output: strip_ansi(output["output"].as_str().unwrap_or_default()),
        })
    }

    /// Asks for the running commands' output until the turn's stdin closes.
    pub fn poll(&self, stdin: Stdin) {
        let live = self.clone();
        thread::spawn(move || {
            while stdin.is_open() {
                thread::sleep(POLL_EVERY);
                let tasks: Vec<String> = live.tasks().keys().cloned().collect();
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
