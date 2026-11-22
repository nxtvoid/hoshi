use super::{ForkMarks, fork_failure, forkable, merge_back_failure, with_dividers};
use crate::state::ThreadState;
use crate::transcript::{Row, group};
use gpui_kit::{AppContext as _, TestAppContext};
use hoshi_core::{CommandError, ItemContent, ThreadItem, TranscriptPage};

fn item(thread_id: &str, id: &str, content: ItemContent) -> ThreadItem {
    ThreadItem {
        id: id.into(),
        thread_id: thread_id.into(),
        created_at: "2026-11-15T10:00:00Z".into(),
        content,
    }
}

fn user(thread_id: &str, id: &str) -> ThreadItem {
    item(
        thread_id,
        id,
        ItemContent::UserMessage {
            text: id.into(),
            attachments: Vec::new(),
            steer: false,
            task_result: None,
            pull_request_update: None,
            resumed: None,
            rewind: None,
            dropped: false,
            model_switch: None,
        },
    )
}

fn answer(thread_id: &str, id: &str) -> ThreadItem {
    item(
        thread_id,
        id,
        ItemContent::AssistantMessage { text: id.into() },
    )
}

fn page(items: Vec<ThreadItem>, has_older: bool) -> TranscriptPage {
    TranscriptPage { items, has_older }
}

fn shape(rows: &[Row], items: &[ThreadItem]) -> Vec<String> {
    rows.iter()
        .map(|row| match row {
            Row::Item(index) => items[*index].id.clone(),
            Row::ForkedFrom(id) => format!("from:{id}"),
            Row::ForkedInto(id) => format!("into:{id}"),
            other => format!("{other:?}"),
        })
        .collect()
}

fn marks(forks: &[(&str, &str)], titled: &[&str]) -> ForkMarks {
    ForkMarks {
        forks: forks
            .iter()
            .map(|(fork, through)| (fork.to_string(), through.to_string()))
            .collect(),
        titles: titled
            .iter()
            .map(|id| (id.to_string(), format!("{id} title")))
            .collect(),
    }
}

#[test]
fn only_finished_answers_of_the_thread_itself_offer_a_fork() {
    let items = vec![
        answer("s", "inherited"),
        user("t", "u1"),
        answer("t", "a1"),
        user("t", "u2"),
        answer("t", "a2"),
    ];
    let idle = forkable(&items, 1, false);
    assert_eq!(idle.len(), 2);
    assert!(idle.contains("a1") && idle.contains("a2"));
    let working = forkable(&items, 1, true);
    assert_eq!(working.into_iter().collect::<Vec<_>>(), ["a1"]);
}

#[test]
fn a_message_the_agent_wrote_before_more_work_is_no_answer() {
    let call = |id: &str| {
        item(
            "t",
            id,
            ItemContent::ToolCall {
                name: "Bash".into(),
                input: "{}".into(),
                output: None,
                is_error: false,
                interrupted: true,
                detail: None,
                output_omitted: false,
            },
        )
    };
    let items = vec![
        user("t", "u1"),
        answer("t", "narration"),
        call("c1"),
        user("t", "u2"),
        answer("t", "step"),
        call("c2"),
        answer("t", "a2"),
    ];
    let answers = forkable(&items, 0, false);
    assert_eq!(answers.into_iter().collect::<Vec<_>>(), ["a2"]);
}

#[gpui_kit::test]
fn a_fork_of_a_fork_shows_each_source_above_its_history(cx: &mut TestAppContext) {
    let thread = cx.new(|_| ThreadState::new("f", vec![user("f", "u3"), answer("f", "a3")]));
    thread.update(cx, |thread, cx| {
        thread.lineage = vec![("f".into(), "s".into()), ("s".into(), "g".into())];
        thread.add_history(page(vec![user("s", "u2"), answer("s", "a2")], true), cx);
    });
    let rows = |thread: &ThreadState| {
        with_dividers(group(&thread.items), thread, &marks(&[], &["s", "g"]))
    };
    thread.read_with(cx, |thread, _| {
        assert_eq!(
            shape(&rows(thread), &thread.items),
            ["u2", "a2", "from:s", "u3", "a3"],
            "the source's history is not whole yet"
        );
    });

    thread.update(cx, |thread, cx| {
        thread.add_history(page(vec![user("s", "u1")], false), cx);
        thread.add_history(page(vec![user("g", "u0"), answer("g", "a0")], false), cx);
    });
    thread.read_with(cx, |thread, _| {
        assert_eq!(thread.inherited, 5);
        assert_eq!(
            shape(&rows(thread), &thread.items),
            ["u0", "a0", "from:g", "u1", "u2", "a2", "from:s", "u3", "a3"]
        );
    });
}

#[test]
fn a_fork_divider_follows_the_turn_it_goes_on_from() {
    let items = vec![
        user("t", "u1"),
        answer("t", "a1"),
        user("t", "u2"),
        answer("t", "a2"),
    ];
    let thread = ThreadState::new("t", items);
    let rows = with_dividers(
        group(&thread.items),
        &thread,
        &marks(
            &[("f1", "a1"), ("f2", "a1"), ("f3", "a2")],
            &["f1", "f2", "f3"],
        ),
    );
    assert_eq!(
        shape(&rows, &thread.items),
        ["u1", "a1", "into:f1", "into:f2", "u2", "a2", "into:f3"]
    );
    let unknown = with_dividers(
        group(&thread.items),
        &thread,
        &marks(&[("f1", "gone")], &["f1"]),
    );
    assert_eq!(unknown.len(), 4);
}

#[test]
fn work_from_two_threads_never_shares_a_row() {
    let call = |thread_id: &str, id: &str| {
        item(
            thread_id,
            id,
            ItemContent::Reasoning {
                text: "thinking".into(),
            },
        )
    };
    let rows = group(&[call("s", "r1"), call("f", "r2")]);
    assert_eq!(rows, [Row::Work(vec![0]), Row::Work(vec![1])]);
}

#[test]
fn refused_forks_and_merges_say_why() {
    let (thread_id, item_id, turn_id) = ("t".to_string(), "a".to_string(), "u".to_string());
    assert_eq!(
        fork_failure(&CommandError::NoStableTurn {
            thread_id: thread_id.clone()
        }),
        crate::m::fork_no_stable_turn()
    );
    assert_eq!(
        fork_failure(&CommandError::TurnNotFound {
            thread_id: thread_id.clone(),
            item_id,
        }),
        crate::m::fork_turn_not_found()
    );
    assert_eq!(
        fork_failure(&CommandError::TurnNotForkable {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
        }),
        crate::m::fork_turn_running()
    );
    let cases = [
        (
            CommandError::NotAFork {
                thread_id: thread_id.clone(),
            },
            crate::m::merge_back_not_a_fork(),
        ),
        (
            CommandError::MergeBackTurnNotFinished {
                thread_id: thread_id.clone(),
                turn_id,
            },
            crate::m::merge_back_turn_not_finished(),
        ),
        (
            CommandError::MergeBackTargetBusy {
                thread_id: thread_id.clone(),
            },
            "Finish or stop the target thread's active work and clear its queue before \
                 merging it back."
                .into(),
        ),
        (
            CommandError::MergeBackPending {
                thread_id: thread_id.clone(),
                fork_thread_id: "g".into(),
            },
            crate::m::merge_back_pending(),
        ),
        (
            CommandError::WorktreeShared {
                thread_id,
                other_thread_id: "g".into(),
            },
            crate::m::merge_back_worktree_shared(),
        ),
    ];
    for (error, text) in cases {
        assert_eq!(merge_back_failure(&error), text, "{error:?}");
    }
}
