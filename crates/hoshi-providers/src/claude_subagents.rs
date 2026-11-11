//! The model and effort each subagent of a turn runs at, and how the agent is told to choose
//! them.

use crate::claude_models::{manifest, parse_effort};
use hoshi_core::{Effort, ProviderOutput, SubagentEffort, ToolDetail};
use serde_json::Value;
use std::collections::HashMap;

/// Subagents of a turn, by the id of the call that started each.
#[derive(Default)]
pub(crate) struct Subagents {
    /// What the turn's own agent runs on, which its subagents inherit.
    turn: Option<Run>,
    started: HashMap<String, Subagent>,
}

#[derive(Clone)]
struct Run {
    model: String,
    effort: Option<Effort>,
}

struct Subagent {
    /// The subagent whose call started it; `None` for the turn's own agent.
    owner: Option<String>,
    run: Run,
    /// What it was asked for or inherited, which the model it turns out to run on may not
    /// support.
    asked_effort: Option<Effort>,
    /// One of its own messages told the model it runs on.
    seen: bool,
}

impl Run {
    fn resolve(model: Option<&str>, effort: Option<Effort>) -> Self {
        let (model, effort) = manifest().run(model, effort);
        Self { model, effort }
    }

    fn detail(&self) -> ToolDetail {
        ToolDetail::Subagent {
            model: self.model.clone(),
            effort: self.effort,
        }
    }
}

impl Subagents {
    /// For a turn on `model`, `None` for the default one, at `effort`.
    pub(crate) fn for_turn(model: Option<&str>, effort: Option<Effort>) -> Self {
        Self {
            turn: Some(Run::resolve(model, effort)),
            started: HashMap::new(),
        }
    }

    /// The `Agent` call `id`, made by the turn's agent or by the subagent `owner`, started a
    /// subagent: what it runs on, as its input asks or as it inherits.
    pub(crate) fn started(
        &mut self,
        owner: Option<&str>,
        id: &str,
        input: &Value,
    ) -> Option<ToolDetail> {
        let inherited = match owner {
            Some(owner) => self.started.get(owner).map(|subagent| subagent.run.clone()),
            None => self.turn.clone(),
        }?;
        let fork = input["subagent_type"] == "fork";
        let model = input["model"]
            .as_str()
            .filter(|model| !fork && *model != "inherit")
            .unwrap_or(&inherited.model);
        let asked_effort = input["effort"]
            .as_str()
            .filter(|_| !fork)
            .and_then(parse_effort)
            .or(inherited.effort);
        let run = Run::resolve(Some(model), asked_effort);
        let detail = run.detail();
        self.started.insert(
            id.to_owned(),
            Subagent {
                owner: owner.map(str::to_owned),
                run,
                asked_effort,
                seen: false,
            },
        );
        Some(detail)
    }

    /// The subagent of call `id` answered on `model`, which corrects what it was thought to
    /// run on. Says so on the call, in its owner's transcript, when that changed.
    pub(crate) fn model_seen(&mut self, id: &str, model: Option<&str>) -> Option<ProviderOutput> {
        let model = model.filter(|model| !model.is_empty() && *model != "<synthetic>")?;
        let subagent = self.started.get_mut(id).filter(|subagent| !subagent.seen)?;
        subagent.seen = true;
        let run = Run::resolve(Some(model), subagent.asked_effort);
        if run.model == subagent.run.model && run.effort == subagent.run.effort {
            return None;
        }
        subagent.run = run;
        let output = ProviderOutput::ToolCallDetail {
            item_id: id.to_owned(),
            detail: subagent.run.detail(),
        };
        Some(match &subagent.owner {
            Some(owner) => ProviderOutput::Subagent {
                tool_use_id: owner.clone(),
                output: Box::new(output),
            },
            None => output,
        })
    }

    /// What the subagent of call `id` runs on, when it started one.
    pub(crate) fn detail(&self, id: &str) -> Option<ToolDetail> {
        self.started.get(id).map(|subagent| subagent.run.detail())
    }
}

/// What tells the agent how to choose its subagents' model and effort; `None` when they
/// run at the thread's own. `delegates` when it has hoshi's `delegate_task`.
pub(crate) fn instructions(
    policy: SubagentEffort,
    model: Option<&str>,
    effort: Option<Effort>,
    delegates: bool,
) -> Option<String> {
    if policy == SubagentEffort::Thread {
        return None;
    }
    let (slug, effort) = manifest().run(model, effort);
    let name = manifest().name(&slug).unwrap_or(&slug);
    let tools = if delegates {
        "each Agent call and delegate_task"
    } else {
        "each Agent call"
    };
    let yours = match effort {
        Some(effort) => format!("{name} ({slug}) at {} effort", effort.as_str()),
        None => format!("{name} ({slug})"),
    };
    Some(format!(
        "Subagents and delegated tasks: you run on {yours}, their ceiling. These instructions ask you to set `model` and `effort` on {tools} by the work:\n\
         - Searching, reading or summarizing: model \"haiku\", effort \"low\".\n\
         - A focused, well-defined change: model \"sonnet\", effort \"medium\".\n\
         - Hard design, debugging, review or anything ambiguous: omit both to inherit yours.\n\
         Never above yours; the user's choice wins. Forks inherit yours."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn turn() -> Subagents {
        Subagents::for_turn(Some("opus"), Some(Effort::Max))
    }

    fn subagent(model: &str, effort: Option<Effort>) -> Option<ToolDetail> {
        Some(ToolDetail::Subagent {
            model: model.into(),
            effort,
        })
    }

    #[test]
    fn a_subagent_runs_on_what_its_call_asks_for() {
        let mut subagents = turn();
        let input = json!({ "description": "Find it", "model": "haiku", "effort": "low" });
        assert_eq!(
            subagents.started(None, "toolu_1", &input),
            subagent("claude-haiku-5-5", Some(Effort::Low))
        );
        assert_eq!(
            subagents.detail("toolu_1"),
            subagent("claude-haiku-5-5", Some(Effort::Low))
        );
    }

    #[test]
    fn a_subagent_inherits_what_its_call_leaves_out() {
        let mut subagents = turn();
        assert_eq!(
            subagents.started(None, "toolu_1", &json!({ "model": "inherit" })),
            subagent("claude-opus-5-5", Some(Effort::Max))
        );
        assert_eq!(
            subagents.started(None, "toolu_2", &json!({ "model": "sonnet" })),
            subagent("claude-sonnet-5-5", Some(Effort::Max))
        );
        let nested = json!({ "effort": "medium" });
        assert_eq!(
            subagents.started(Some("toolu_2"), "toolu_3", &nested),
            subagent("claude-sonnet-5-5", Some(Effort::Medium))
        );
    }

    #[test]
    fn a_fork_always_runs_on_what_its_parent_runs_on() {
        let mut subagents = turn();
        let input = json!({ "subagent_type": "fork", "model": "haiku", "effort": "low" });
        assert_eq!(
            subagents.started(None, "toolu_1", &input),
            subagent("claude-opus-5-5", Some(Effort::Max))
        );
    }

    #[test]
    fn a_model_without_effort_levels_runs_at_none() {
        let mut subagents = turn();
        assert_eq!(
            subagents.started(None, "toolu_1", &json!({ "model": "haiku-4.5" })),
            subagent("claude-haiku-4-5", None)
        );
    }

    #[test]
    fn its_own_messages_correct_the_model_once() {
        let mut subagents = turn();
        subagents.started(
            None,
            "toolu_1",
            &json!({ "subagent_type": "Explore", "effort": "low" }),
        );
        assert_eq!(subagents.model_seen("toolu_1", Some("<synthetic>")), None);
        assert_eq!(
            subagents.model_seen("toolu_1", Some("claude-haiku-4-5-20251001")),
            Some(ProviderOutput::ToolCallDetail {
                item_id: "toolu_1".into(),
                detail: ToolDetail::Subagent {
                    model: "claude-haiku-4-5".into(),
                    effort: None,
                },
            })
        );
        assert_eq!(
            subagents.model_seen("toolu_1", Some("claude-opus-5-5")),
            None
        );
        assert_eq!(
            subagents.detail("toolu_1"),
            subagent("claude-haiku-4-5", None)
        );
    }

    #[test]
    fn a_nested_subagent_s_correction_goes_to_its_owner_s_transcript() {
        let mut subagents = turn();
        subagents.started(None, "toolu_1", &json!({}));
        subagents.started(Some("toolu_1"), "toolu_2", &json!({}));
        assert_eq!(
            subagents.model_seen("toolu_1", Some("claude-opus-5-5")),
            None
        );
        assert_eq!(
            subagents.model_seen("toolu_2", Some("claude-sonnet-5-5")),
            Some(ProviderOutput::Subagent {
                tool_use_id: "toolu_1".into(),
                output: Box::new(ProviderOutput::ToolCallDetail {
                    item_id: "toolu_2".into(),
                    detail: ToolDetail::Subagent {
                        model: "claude-sonnet-5-5".into(),
                        effort: Some(Effort::Max),
                    },
                }),
            })
        );
    }

    #[test]
    fn without_a_turn_model_nothing_is_known() {
        let mut subagents = Subagents::default();
        assert_eq!(subagents.started(None, "toolu_1", &json!({})), None);
    }

    #[test]
    fn the_subagent_instructions_match_their_budget() {
        let told = instructions(
            SubagentEffort::AgentDecides,
            Some("opus"),
            Some(Effort::Medium),
            true,
        )
        .unwrap();
        let budget: serde_json::Value =
            serde_json::from_str(include_str!("../../../agent-context-budget.json")).unwrap();
        assert_eq!(
            told.len() as u64,
            budget["subagentInstructionsBytes"],
            "set agent-context-budget.json to the measured value"
        );
    }

    #[test]
    fn the_agent_is_told_to_choose_only_when_it_decides() {
        assert_eq!(
            instructions(
                SubagentEffort::Thread,
                Some("opus"),
                Some(Effort::Max),
                true
            ),
            None
        );
        let told = instructions(
            SubagentEffort::AgentDecides,
            Some("opus"),
            Some(Effort::Max),
            true,
        )
        .unwrap();
        assert!(
            told.contains("Claude Opus 5.5 (claude-opus-5-5) at max effort"),
            "{told}"
        );
        assert!(told.contains("delegate_task"));
        let alone = instructions(SubagentEffort::AgentDecides, None, None, false).unwrap();
        assert!(alone.contains("at medium effort"), "{alone}");
        assert!(!alone.contains("delegate_task"));
        let no_levels =
            instructions(SubagentEffort::AgentDecides, Some("haiku-4.5"), None, false).unwrap();
        assert!(
            no_levels.contains("Claude Haiku 4.5 (claude-haiku-4-5),"),
            "{no_levels}"
        );
    }
}
