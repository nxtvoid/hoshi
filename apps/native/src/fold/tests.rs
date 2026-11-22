use super::{After, Ending, fold_turns, stays_in_view};
use crate::transcript::{Row, group};
use hoshi_core::{
    HandoffDelivery, HandoffSource, ItemContent, Resumption, ThreadItem, TurnFailure,
};

fn item(id: &str, created_at: &str, content: ItemContent) -> ThreadItem {
    ThreadItem {
        id: id.into(),
        thread_id: "t".into(),
        created_at: created_at.into(),
        content,
    }
}

fn at(seconds: u32) -> String {
    format!("2026-11-08T10:{:02}:{:02}Z", seconds / 60, seconds % 60)
}

fn user(id: &str, seconds: u32) -> ThreadItem {
    message(id, seconds, false)
}

fn steer(id: &str, seconds: u32) -> ThreadItem {
    message(id, seconds, true)
}

fn message(id: &str, seconds: u32, steer: bool) -> ThreadItem {
    item(
        id,
        &at(seconds),
        ItemContent::UserMessage {
            text: id.into(),
            attachments: Vec::new(),
            steer,
            task_result: None,
            pull_request_update: None,
            resumed: None,
            rewind: None,
            dropped: false,
            model_switch: None,
        },
    )
}

fn answer(id: &str, seconds: u32) -> ThreadItem {
    item(
        id,
        &at(seconds),
        ItemContent::AssistantMessage { text: id.into() },
    )
}

fn text(id: &str) -> ThreadItem {
    item(id, "", ItemContent::AssistantMessage { text: id.into() })
}

fn tool(id: &str, name: &str) -> ThreadItem {
    item(
        id,
        "",
        ItemContent::ToolCall {
            name: name.into(),
            input: "{}".into(),
            output: Some(String::new()),
            is_error: false,
            detail: None,
            output_omitted: false,
            interrupted: false,
        },
    )
}

fn failed_tool(id: &str, name: &str) -> ThreadItem {
    let mut call = tool(id, name);
    if let ItemContent::ToolCall { is_error, .. } = &mut call.content {
        *is_error = true;
    }
    call
}

fn running_tool(id: &str, name: &str) -> ThreadItem {
    let mut call = tool(id, name);
    if let ItemContent::ToolCall { output, .. } = &mut call.content {
        *output = None;
    }
    call
}

fn run_ended(id: &str, seconds: u32) -> ThreadItem {
    item(id, &at(seconds), ItemContent::RunEnded)
}

fn task_ended(id: &str, seconds: u32) -> ThreadItem {
    item(
        id,
        &at(seconds),
        ItemContent::BackgroundTaskEnded {
            tool_call_id: Some("dev".into()),
            failed: false,
        },
    )
}

fn stopped(id: &str, seconds: u32) -> ThreadItem {
    item(id, &at(seconds), ItemContent::TurnInterrupted)
}

fn fold(items: &[ThreadItem], working: bool) -> Vec<Row> {
    fold_turns(group(items), items, working, &|_| false)
}

fn shape(rows: &[Row], items: &[ThreadItem]) -> Vec<String> {
    rows.iter()
        .map(|row| match row {
            Row::Item(index) | Row::Note(index, _) => items[*index].id.clone(),
            Row::Work(_) => "work".into(),
            Row::Streaming(_) | Row::ReplyBlock { .. } | Row::StreamingNote(..) => {
                "streaming".into()
            }
            Row::Status(_) => "status".into(),
            Row::Background => "background".into(),
            Row::Agents(_) => "agents".into(),
            Row::TaskWaits => "tasks".into(),
            Row::ForkedFrom(_) | Row::ForkedInto(_) => "fork".into(),
            Row::Fold(fold) => format!(
                "{}:{}:{}",
                match fold.ending {
                    Ending::Answered => "answered",
                    Ending::Steered => "steered",
                    Ending::Stopped => "stopped",
                    Ending::Restarted => "restarted",
                },
                match fold.after {
                    After::Start => "start",
                    After::Steer => "steer",
                },
                fold.rows.len()
            ),
        })
        .collect()
}

fn durations(rows: &[Row]) -> Vec<Option<i64>> {
    rows.iter()
        .map(|row| match row {
            Row::Fold(fold) => fold.duration,
            _ => None,
        })
        .collect()
}

#[test]
fn a_finished_turn_folds_its_work_behind_its_answer() {
    let items = [
        user("u1", 0),
        tool("r", "Read"),
        text("note"),
        tool("e", "Edit"),
        answer("a1", 72),
    ];
    let rows = fold(&items, false);
    assert_eq!(shape(&rows, &items), ["u1", "answered:start:3", "a1"]);
    assert_eq!(durations(&rows), [None, Some(72_000), None]);
}

#[test]
fn the_running_turn_and_turns_without_work_or_answer_stay_open() {
    let items = [
        user("u1", 0),
        answer("a1", 5),
        user("u2", 60),
        tool("r", "Read"),
        user("u3", 120),
        tool("s", "Grep"),
        answer("a3", 150),
    ];
    assert_eq!(
        shape(&fold(&items, false), &items),
        ["u1", "a1", "u2", "work", "u3", "answered:start:1", "a3"]
    );
    assert_eq!(
        shape(&fold(&items, true), &items)[5..],
        ["work".to_owned(), "a3".to_owned()]
    );
}

#[test]
fn each_run_of_a_turn_that_went_on_folds_behind_its_own_answer() {
    let items = [
        user("u1", 0),
        tool("agent", "Read"),
        answer("a1", 10),
        run_ended("run1", 11),
        tool("r", "Grep"),
        answer("a2", 31),
        run_ended("run2", 32),
        answer("a3", 40),
    ];
    let rows = fold(&items, false);
    assert_eq!(
        shape(&rows, &items),
        [
            "u1",
            "answered:start:1",
            "a1",
            "answered:start:1",
            "a2",
            "a3"
        ]
    );
    assert_eq!(durations(&rows)[3], Some(20_000));
}

#[test]
fn a_turn_whose_only_work_was_agents_folds_once_they_are_done() {
    let items = [
        user("u1", 0),
        tool("one", "Agent"),
        tool("two", "Agent"),
        answer("a1", 10),
    ];
    assert_eq!(
        shape(&fold(&items, false), &items),
        ["u1", "answered:start:2", "a1"]
    );
    let every_item = |row: &Row| matches!(row, Row::Item(_));
    let rows = fold_turns(group(&items), &items, false, &every_item);
    assert_eq!(shape(&rows, &items), ["u1", "one", "two", "a1"]);
}

#[test]
fn a_delegated_task_stays_in_view_after_its_turn_folds() {
    let items = [
        user("u1", 0),
        tool("r", "Read"),
        tool("task", "mcp__hoshi__delegate_task"),
        tool("done", "Agent"),
        running_tool("busy", "Agent"),
        answer("a1", 10),
    ];
    let live = |row: &Row| stays_in_view(row, &items);
    let rows = fold_turns(group(&items), &items, false, &live);
    assert_eq!(
        shape(&rows, &items),
        ["u1", "answered:start:2", "task", "busy", "a1"]
    );
}

#[test]
fn a_background_task_that_ended_starts_a_run() {
    let items = [
        user("u1", 0),
        tool("r", "Read"),
        answer("a1", 10),
        run_ended("run1", 11),
        task_ended("ended", 20),
        tool("g", "Grep"),
        answer("a2", 30),
    ];
    let rows = fold(&items, false);
    assert_eq!(
        shape(&rows, &items),
        [
            "u1",
            "answered:start:1",
            "a1",
            "ended",
            "answered:start:1",
            "a2"
        ]
    );
    assert_eq!(durations(&rows)[4], Some(10_000));
}

#[test]
fn a_background_task_that_ended_while_its_turn_ran_folds_with_that_turn() {
    let items = [
        user("u1", 0),
        tool("dev", "Bash"),
        tool("install", "Bash"),
        task_ended("u1:ended:b1", 12),
        tool("dev2", "Bash"),
        answer("a1", 28),
    ];
    let rows = fold(&items, false);
    assert_eq!(shape(&rows, &items), ["u1", "answered:start:3", "a1"]);
    assert_eq!(durations(&rows)[1], Some(28_000));
    assert_eq!(
        shape(&fold(&items[..5], true), &items),
        ["u1", "work", "u1:ended:b1", "work"]
    );
}

#[test]
fn a_background_task_that_ended_after_a_run_of_its_turn_starts_the_next_run() {
    let items = [
        user("u1", 0),
        tool("r", "Read"),
        answer("a1", 10),
        run_ended("u1:run1", 11),
        task_ended("u1:ended:b1", 20),
        tool("g", "Grep"),
        answer("a2", 30),
        task_ended("woke:ended:b2", 300),
        tool("h", "Grep"),
        answer("a3", 308),
    ];
    assert_eq!(
        shape(&fold(&items, false), &items),
        [
            "u1",
            "answered:start:1",
            "a1",
            "u1:ended:b1",
            "answered:start:1",
            "a2",
            "woke:ended:b2",
            "answered:start:1",
            "a3"
        ]
    );
}

#[test]
fn a_steer_cuts_the_run_and_both_parts_fold_once_the_turn_is_over() {
    let items = [
        user("u1", 0),
        tool("r", "Read"),
        text("partial"),
        steer("s", 20),
        tool("e", "Edit"),
        answer("a1", 50),
    ];
    let rows = fold(&items, false);
    assert_eq!(
        shape(&rows, &items),
        ["u1", "steered:start:2", "s", "answered:steer:1", "a1"]
    );
    assert_eq!(
        durations(&rows),
        [None, Some(50_000), None, Some(30_000), None]
    );
    assert_eq!(
        shape(&fold(&items[..5], true), &items),
        ["u1", "work", "partial", "s", "work"]
    );
}

#[test]
fn a_run_a_steer_ended_folds_nothing_while_the_turn_still_works() {
    let mut items = vec![
        user("u1", 0),
        tool("r", "Read"),
        steer("s", 20),
        run_ended("run1", 21),
        tool("e", "Edit"),
    ];
    assert_eq!(
        shape(&fold(&items, true), &items),
        ["u1", "work", "s", "work"]
    );
    items.push(answer("a1", 50));
    assert_eq!(
        shape(&fold(&items, false), &items),
        ["u1", "steered:start:1", "s", "answered:steer:1", "a1"]
    );
}

#[test]
fn a_run_that_ended_before_the_latest_response_folds_while_working() {
    let items = [
        user("u1", 0),
        tool("r", "Read"),
        answer("a1", 10),
        run_ended("run1", 11),
        task_ended("ended", 20),
        tool("g", "Grep"),
    ];
    assert_eq!(
        shape(&fold(&items, true), &items),
        ["u1", "answered:start:1", "a1", "ended", "work"]
    );
}

#[test]
fn a_stopped_turn_folds_whole_with_what_the_agent_said_last_in_view() {
    let items = [
        user("u1", 0),
        tool("r", "Read"),
        answer("said", 5),
        stopped("stop", 12),
    ];
    let rows = fold(&items, false);
    assert_eq!(shape(&rows, &items), ["u1", "stopped:start:1", "said"]);
    assert_eq!(durations(&rows)[1], Some(12_000));
}

#[test]
fn settled_work_after_the_answer_folds_and_a_failed_turn_stays_open() {
    let start = [user("u1", 0), tool("r", "Read"), answer("a1", 5)];
    let items = [start.as_slice(), &[tool("w", "Write")]].concat();
    assert_eq!(
        shape(&fold(&items, false), &items),
        ["u1", "answered:start:2", "a1"]
    );
    let items = [start.as_slice(), &[failed_tool("x", "Bash")]].concat();
    assert_eq!(
        shape(&fold(&items, false), &items),
        ["u1", "answered:start:1", "a1", "work"]
    );
    let failure = item(
        "fail",
        &at(6),
        ItemContent::TurnFailed {
            reason: TurnFailure::ProviderError { detail: "x".into() },
        },
    );
    let items = [start.as_slice(), &[failure]].concat();
    assert_eq!(
        shape(&fold(&items, false), &items),
        ["u1", "work", "a1", "fail"]
    );
}

#[test]
fn a_run_a_restart_cut_folds_as_cut_by_it_whether_or_not_it_resumed() {
    let start = [user("u1", 0), tool("r", "Bash")];
    let cut = item("cut", &at(12), ItemContent::TurnCut);
    let items = [start.as_slice(), std::slice::from_ref(&cut)].concat();
    assert_eq!(
        shape(&fold(&items, false), &items),
        ["u1", "restarted:start:1"]
    );

    let mut resumed = user("resumed", 20);
    if let ItemContent::UserMessage { resumed: at, .. } = &mut resumed.content {
        *at = Some(Resumption::Restart);
    }
    let rest = [resumed, tool("again", "Bash"), answer("a1", 30)];
    let items = [start.as_slice(), &[cut], &rest].concat();
    assert_eq!(
        shape(&fold(&items, false), &items),
        [
            "u1",
            "restarted:start:1",
            "resumed",
            "answered:start:1",
            "a1"
        ]
    );
}

#[test]
fn a_context_handoff_stays_in_view_before_the_work_of_its_turn() {
    let handoff = item(
        "handoff",
        &at(1),
        ItemContent::ContextHandoff {
            sources: vec![HandoffSource::MergeBack {
                fork_thread_id: "fork".into(),
            }],
            delivery: HandoffDelivery::Inline,
            included_items: 2,
            omitted_items: 0,
        },
    );
    let items = [user("u1", 0), handoff, tool("r", "Read"), answer("a1", 30)];
    assert_eq!(
        shape(&fold(&items, false), &items),
        ["u1", "handoff", "answered:start:1", "a1"]
    );
}

#[test]
fn live_rows_stay_out_of_the_fold_after_it() {
    let items = [
        user("u1", 0),
        tool("r", "Read"),
        tool("agent", "Agent"),
        tool("e", "Edit"),
        answer("a1", 60),
    ];
    let live = |row: &Row| *row == Row::Item(2);
    let rows = fold_turns(group(&items), &items, false, &live);
    assert_eq!(
        shape(&rows, &items),
        ["u1", "answered:start:2", "agent", "a1"]
    );
}
