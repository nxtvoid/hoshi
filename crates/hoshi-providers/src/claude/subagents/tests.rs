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
fn a_resumed_call_starts_again_in_its_owner_s_transcript_with_what_it_runs_on() {
    let mut subagents = turn();
    subagents.started(None, "toolu_1", &json!({}));
    subagents.launched(None, "toolu_1", "Agent", "{}".into());
    subagents.launched(Some("toolu_1"), "toolu_2", "Agent", "{\"a\":1}".into());

    assert_eq!(subagents.owner("toolu_2"), Some(Some("toolu_1")));
    assert_eq!(subagents.owner("toolu_3"), None);
    assert_eq!(
        subagents.reopened("toolu_1"),
        [
            ProviderOutput::ToolCallStarted {
                item_id: "toolu_1".into(),
                name: "Agent".into(),
                input: "{}".into(),
            },
            ProviderOutput::ToolCallDetail {
                item_id: "toolu_1".into(),
                detail: ToolDetail::Subagent {
                    model: "claude-opus-5-5".into(),
                    effort: Some(Effort::Max),
                },
            },
        ]
    );
    assert_eq!(
        subagents.reopened("toolu_2"),
        [ProviderOutput::Subagent {
            tool_use_id: "toolu_1".into(),
            output: Box::new(ProviderOutput::ToolCallStarted {
                item_id: "toolu_2".into(),
                name: "Agent".into(),
                input: "{\"a\":1}".into(),
            }),
        }]
    );
}

#[test]
fn the_launch_of_a_subagent_from_an_earlier_process_is_read_from_its_session() {
    let projects = std::env::temp_dir().join(format!("hoshi-launch-{}", std::process::id()));
    let folder = projects.join("-repo");
    let subagents = folder.join("s1").join("subagents");
    std::fs::create_dir_all(&subagents).unwrap();
    std::fs::create_dir_all(projects.join("-other")).unwrap();
    std::fs::write(
        subagents.join("agent-a1.meta.json"),
        r#"{"agentType":"general-purpose","toolUseId":"toolu_agent","spawnDepth":1}"#,
    )
    .unwrap();
    let call = json!({ "type": "assistant", "message": { "content": [
            { "type": "tool_use", "id": "toolu_agent", "name": "Agent",
              "input": { "description": "Audit", "prompt": "Look" } }] } });
    std::fs::write(
        folder.join("s1.jsonl"),
        format!(
            "{}\n{call}\n",
            json!({ "type": "user", "message": { "content": "hi" } })
        ),
    )
    .unwrap();

    let found = launch_on_disk(&projects, "s1", "a1");
    let missing = launch_on_disk(&projects, "s1", "a2");
    let escaping = launch_on_disk(&projects, "s1", "../a1");
    std::fs::remove_dir_all(&projects).unwrap();

    assert_eq!(
        found,
        Some((
            "toolu_agent".into(),
            Some((
                "Agent".into(),
                json!({ "description": "Audit", "prompt": "Look" })
            ))
        ))
    );
    assert_eq!(missing, None);
    assert_eq!(escaping, None);
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
        serde_json::from_str(include_str!("../../../../../agent-context-budget.json")).unwrap();
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
