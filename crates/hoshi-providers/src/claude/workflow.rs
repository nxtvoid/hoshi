//! A workflow's progress: the agents it runs, by phase, as Claude Code reports them while it
//! runs. Each report repeats every agent, which a stale report must not take back.

use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// How much of an agent's result its line shows.
const RESULT_MAX_CHARS: usize = 120;

/// Workflows that reported progress, by task id.
#[derive(Default)]
pub(crate) struct Workflows(HashMap<String, Progress>);

#[derive(Default)]
struct Progress {
    phases: BTreeMap<u64, String>,
    agents: BTreeMap<u64, Agent>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Queued,
    Running,
    Done,
    Failed,
}

impl State {
    fn ended(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }

    fn mark(self) -> &'static str {
        match self {
            Self::Queued => "○",
            Self::Running => "●",
            Self::Done => "✓",
            Self::Failed => "✗",
        }
    }
}

#[derive(Clone)]
struct Agent {
    label: String,
    phase: Option<u64>,
    state: State,
    attempt: u64,
    result: Option<String>,
}

impl Agent {
    fn parse(entry: &Value) -> Option<(u64, Self)> {
        let index = entry["index"].as_u64()?;
        let label = entry["label"].as_str().filter(|label| !label.is_empty())?;
        // Claude Code marks an agent `start` once scheduled; it runs once it has `startedAt`.
        let state = match entry["state"].as_str() {
            Some("queued") => State::Queued,
            Some("start") if entry["startedAt"].is_null() => State::Queued,
            Some("done") => State::Done,
            Some("error") => State::Failed,
            _ => State::Running,
        };
        let result = entry["resultPreview"]
            .as_str()
            .and_then(|result| result.lines().find(|line| !line.trim().is_empty()))
            .map(|line| crate::truncate(line.trim(), RESULT_MAX_CHARS));
        Some((
            index,
            Self {
                label: label.to_owned(),
                phase: entry["phaseIndex"].as_u64(),
                state,
                attempt: entry["attempt"].as_u64().unwrap_or(1),
                result,
            },
        ))
    }

    fn line(&self) -> String {
        match &self.result {
            Some(result) if self.state.ended() => {
                format!("{} {}: {result}", self.state.mark(), self.label)
            }
            _ => format!("{} {}", self.state.mark(), self.label),
        }
    }
}

impl Progress {
    fn merge(&mut self, entries: &[Value]) {
        for entry in entries {
            match entry["type"].as_str() {
                Some("workflow_phase") => {
                    if let (Some(index), Some(title)) =
                        (entry["index"].as_u64(), entry["title"].as_str())
                    {
                        self.phases.insert(index, title.to_owned());
                    }
                }
                Some("workflow_agent") => {
                    let Some((index, agent)) = Agent::parse(entry) else {
                        continue;
                    };
                    let stale = self.agents.get(&index).is_some_and(|known| {
                        agent.attempt < known.attempt
                            || (agent.attempt == known.attempt
                                && known.state.ended()
                                && !agent.state.ended())
                    });
                    if !stale {
                        self.agents.insert(index, agent);
                    }
                }
                _ => {}
            }
        }
    }

    /// Each phase's title over its agents, then the agents of no phase.
    fn lines(&self) -> String {
        let mut lines = Vec::new();
        for (index, title) in &self.phases {
            lines.push(title.clone());
            lines.extend(
                self.agents
                    .values()
                    .filter(|agent| agent.phase == Some(*index))
                    .map(|agent| format!("  {}", agent.line())),
            );
        }
        lines.extend(
            self.agents
                .values()
                .filter(|agent| {
                    agent
                        .phase
                        .is_none_or(|phase| !self.phases.contains_key(&phase))
                })
                .map(Agent::line),
        );
        lines.join("\n")
    }
}

impl Workflows {
    /// The workflow's progress once `frame` is taken in, `None` when it tells none.
    pub(crate) fn update(&mut self, frame: &Value) -> Option<String> {
        let entries = frame["workflow_progress"]
            .as_array()
            .filter(|entries| !entries.is_empty())?;
        let task_id = frame["task_id"].as_str()?;
        let progress = self.0.entry(task_id.to_owned()).or_default();
        progress.merge(entries);
        Some(progress.lines()).filter(|lines| !lines.is_empty())
    }

    /// What the workflow that ended with `frame` did, which it forgets.
    pub(crate) fn ended(&mut self, frame: &Value) -> Option<String> {
        let task_id = frame["task_id"].as_str()?;
        self.update(frame);
        let lines = self.0.remove(task_id)?.lines();
        Some(lines).filter(|lines| !lines.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn progress(agents: Value) -> Value {
        let mut entries = vec![json!({ "type": "workflow_phase", "index": 1, "title": "Reply" })];
        entries.extend(agents.as_array().unwrap().iter().cloned());
        json!({ "type": "system", "subtype": "task_progress", "task_id": "w1",
            "tool_use_id": "toolu_workflow", "description": "Reply: reply-one",
            "workflow_progress": entries })
    }

    fn agent(index: u64, state: &str, extra: Value) -> Value {
        let mut agent = json!({ "type": "workflow_agent", "index": index,
            "label": format!("reply-{index}"), "phaseIndex": 1, "phaseTitle": "Reply",
            "state": state, "queuedAt": 1, "promptPreview": "Reply" });
        agent
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        agent
    }

    #[test]
    fn a_workflow_shows_each_agent_under_its_phase() {
        let mut workflows = Workflows::default();
        let scheduled = progress(json!([
            agent(1, "start", json!({ "startedAt": 2, "attempt": 1 })),
            agent(2, "start", json!({})),
        ]));
        assert_eq!(
            workflows.update(&scheduled).as_deref(),
            Some("Reply\n  ● reply-1\n  ○ reply-2")
        );

        let one_done = progress(json!([
            agent(
                1,
                "done",
                json!({ "startedAt": 2, "attempt": 1,
                "resultPreview": "\nONE\nmore" })
            ),
            agent(2, "error", json!({ "startedAt": 2, "attempt": 1 })),
        ]));
        assert_eq!(
            workflows.update(&one_done).as_deref(),
            Some("Reply\n  ✓ reply-1: ONE\n  ✗ reply-2")
        );
    }

    #[test]
    fn a_stale_report_takes_back_no_agent_s_end_but_a_retry_does() {
        let mut workflows = Workflows::default();
        workflows.update(&progress(json!([agent(
            1,
            "done",
            json!({ "startedAt": 2 })
        )])));

        let stale = progress(json!([agent(1, "start", json!({ "startedAt": 2 }))]));
        assert_eq!(
            workflows.update(&stale).as_deref(),
            Some("Reply\n  ✓ reply-1")
        );

        let retry = progress(json!([agent(
            1,
            "start",
            json!({ "startedAt": 3, "attempt": 2 })
        )]));
        assert_eq!(
            workflows.update(&retry).as_deref(),
            Some("Reply\n  ● reply-1")
        );
    }

    #[test]
    fn a_frame_without_progress_tells_nothing_and_an_ended_workflow_keeps_its_last_progress() {
        let mut workflows = Workflows::default();
        assert_eq!(
            workflows.update(&json!({ "task_id": "a1", "description": "Running ls" })),
            None
        );
        workflows.update(&progress(json!([agent(
            1,
            "start",
            json!({ "startedAt": 2 })
        )])));

        let ended = json!({ "task_id": "w1", "status": "completed" });
        assert_eq!(
            workflows.ended(&ended).as_deref(),
            Some("Reply\n  ● reply-1")
        );
        assert!(workflows.0.is_empty());
        assert_eq!(workflows.ended(&ended), None);
    }
}
