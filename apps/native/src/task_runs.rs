//! Between two messages of the user, the work on the tasks the agent delegated: the tasks'
//! answers stay out of view, the agent's replies to them read as notes, and what is settled
//! folds behind one line.

use crate::delegations::delegation_of;
use crate::fold::{After, Ending, Fold};
use crate::tool_calls::is_delegation;
use crate::transcript::Row;
use hoshi_core::{ItemContent, ThreadItem};
use std::collections::{HashSet, VecDeque};

/// How a task's answer says it ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskOutcome {
    Completed,
    Failed,
    Stopped,
}

/// How a task's answer says it ended, from the line hoshi puts first.
pub fn task_outcome(text: &str) -> TaskOutcome {
    let head = text.lines().next().unwrap_or_default();
    if head.ends_with("[failed]:") {
        TaskOutcome::Failed
    } else if head.ends_with("[stopped]:") {
        TaskOutcome::Stopped
    } else {
        TaskOutcome::Completed
    }
}

fn item<'a>(row: &Row, items: &'a [ThreadItem]) -> Option<(usize, &'a ThreadItem)> {
    match row {
        Row::Item(index) => items.get(*index).map(|item| (*index, item)),
        _ => None,
    }
}

/// A message the user sent, which starts what the agent works on next.
fn starts_work(row: &Row, items: &[ThreadItem]) -> bool {
    matches!(
        item(row, items).map(|(_, item)| &item.content),
        Some(ItemContent::UserMessage {
            steer: false,
            task_result: None,
            ..
        })
    )
}

fn task_result_of<'a>(row: &Row, items: &'a [ThreadItem]) -> Option<&'a str> {
    match item(row, items).map(|(_, item)| &item.content) {
        Some(ItemContent::UserMessage { task_result, .. }) => task_result.as_deref(),
        _ => None,
    }
}

/// An answer of the agent, saved or still being written.
fn is_answer(row: &Row, items: &[ThreadItem]) -> bool {
    matches!(row, Row::Streaming(_))
        || matches!(
            item(row, items).map(|(_, item)| &item.content),
            Some(ItemContent::AssistantMessage { .. })
        )
}

fn delegates(row: &Row, items: &[ThreadItem]) -> bool {
    matches!(
        item(row, items).map(|(_, item)| &item.content),
        Some(ItemContent::ToolCall { name, .. }) if is_delegation(name)
    )
}

fn flatten(rows: Vec<Row>) -> Vec<Row> {
    rows.into_iter()
        .flat_map(|row| match row {
            Row::Fold(fold) => flatten(fold.rows),
            row => vec![row],
        })
        .collect()
}

fn time_of(item: &ThreadItem) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(&item.created_at)
        .ok()
        .map(|time| time.timestamp_millis())
}

fn row_time(row: Option<&Row>, items: &[ThreadItem]) -> Option<i64> {
    row.and_then(|row| item(row, items))
        .and_then(|(_, item)| time_of(item))
}

/// The tasks a row delegates or hears back from.
fn tasks_of(row: &Row, items: &[ThreadItem]) -> Vec<String> {
    if let Row::Fold(fold) = row {
        return fold
            .rows
            .iter()
            .flat_map(|row| tasks_of(row, items))
            .collect();
    }
    if let Some(result) = task_result_of(row, items) {
        return vec![result.to_owned()];
    }
    match item(row, items).map(|(_, item)| &item.content) {
        Some(ItemContent::ToolCall {
            name,
            input,
            output,
            is_error,
            ..
        }) if is_delegation(name) => delegation_of(input, output.as_deref(), *is_error)
            .task_id
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

/// Between two messages of the user, the work on the tasks the agent delegated. The tasks'
/// answers stay out of view, as each task's row shows its own, and the agent's replies to
/// them read as notes, the work behind each reply left out. `Row::Streaming` rows, last,
/// are answers still being written.
///
/// What is settled folds behind one line: everything up to the last answer that left none
/// of the tasks so far `active`. What follows stays in view: the agent's last reply since,
/// the work still going on toward it, and the tasks it delegates next. What folded never
/// comes back into view, however the agent goes on.
pub fn fold_task_runs(
    rows: Vec<Row>,
    items: &[ThreadItem],
    active: &dyn Fn(&str) -> bool,
) -> Vec<Row> {
    let mut folded = Vec::with_capacity(rows.len());
    let mut start = None;
    let mut part = Vec::new();
    for row in rows {
        if starts_work(&row, items) {
            folded.extend(task_runs(start, std::mem::take(&mut part), items, active));
            start = item(&row, items).map(|(index, _)| index);
            folded.push(row);
        } else {
            part.push(row);
        }
    }
    folded.extend(task_runs(start, part, items, active));
    folded
}

fn task_runs(
    start: Option<usize>,
    part: Vec<Row>,
    items: &[ThreadItem],
    active: &dyn Fn(&str) -> bool,
) -> Vec<Row> {
    if !part.iter().any(|row| task_result_of(row, items).is_some()) {
        return part;
    }
    let mut tasks: Vec<String> = Vec::new();
    let mut settled = None;
    for (index, row) in part.iter().enumerate() {
        tasks.extend(tasks_of(row, items));
        if task_result_of(row, items).is_some() && !tasks.iter().any(|task| active(task)) {
            settled = Some(index);
        }
    }
    let cut = settled.map_or(0, |settled| reply_after(&part, settled, items));
    let ended = row_time(part.get(cut), items)
        .or_else(|| settled.and_then(|settled| row_time(part.get(settled), items)));
    let mut part = part;
    let after = part.split_off(cut);
    let mut to_user = HashSet::new();
    let before = with_notes(part, false, items, Some(&mut to_user));
    let after = with_notes(after, true, items, None);
    let Some(start) = start.filter(|_| cut > 0) else {
        return before.into_iter().chain(after).collect();
    };
    let mut shown = folded(start, before, &to_user, ended, items);
    shown.extend(after);
    shown
}

/// The settled rows behind fold lines. What the user said along the way and the agent's
/// answers to them stay in view, as they do when a turn folds, with a line of their own for
/// what came before and after each.
fn folded(
    start: usize,
    rows: Vec<Row>,
    to_user: &HashSet<usize>,
    ended: Option<i64>,
    items: &[ThreadItem],
) -> Vec<Row> {
    let mut shown = Vec::new();
    let mut since = start;
    let mut behind = Vec::new();
    let fold = |since: usize, behind: &mut Vec<Row>, until: Option<i64>, shown: &mut Vec<Row>| {
        if behind.is_empty() {
            return;
        }
        let began = items.get(since).and_then(time_of);
        shown.push(Row::Fold(Fold {
            start: since,
            rows: flatten(std::mem::take(behind)),
            ending: Ending::Answered,
            after: if since == start {
                After::Start
            } else {
                After::Steer
            },
            duration: began.zip(until).map(|(began, until)| until - began),
            tasks: true,
        }));
    };
    for row in rows {
        match row {
            Row::Item(index) if to_user.contains(&index) => {
                fold(
                    since,
                    &mut behind,
                    items.get(index).and_then(time_of),
                    &mut shown,
                );
                shown.push(row);
                since = index;
            }
            row => behind.push(row),
        }
    }
    fold(since, &mut behind, ended, &mut shown);
    shown
}

/// Where what stays in view begins, after the answer at `settled`: the agent's last reply
/// before it delegates again, else the task it delegates next, else what it works on.
fn reply_after(part: &[Row], settled: usize, items: &[ThreadItem]) -> usize {
    let mut reply = None;
    for (index, row) in part.iter().enumerate().skip(settled + 1) {
        if delegates(row, items) {
            return reply.unwrap_or(index);
        }
        if is_answer(row, items) {
            reply = Some(index);
        }
    }
    reply.unwrap_or(settled + 1)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Heard {
    Task(TaskOutcome),
    User,
}

/// The rows without the tasks' answers, the agent's replies to them as notes. The agent
/// answers what reaches it in the order it came, so each reply goes with the oldest answer
/// or message of the user still without one; `to_user` gets those messages and the replies
/// to them. While the tasks are `in_view`, the work behind each reply is left out too.
fn with_notes(
    rows: Vec<Row>,
    in_view: bool,
    items: &[ThreadItem],
    mut to_user: Option<&mut HashSet<usize>>,
) -> Vec<Row> {
    let mut unanswered: VecDeque<Heard> = VecDeque::new();
    let mut reply_to: Option<Heard> = None;
    let mut answered = false;
    let mut shown = Vec::with_capacity(rows.len());
    for row in rows {
        if let Row::Streaming(streaming) = row {
            reply_to = unanswered.pop_front().or(reply_to);
            match reply_to {
                Some(Heard::Task(outcome)) => shown.push(Row::StreamingNote(streaming, outcome)),
                _ => shown.push(row),
            }
            continue;
        }
        let Some((index, item)) = item(&row, items) else {
            if !(matches!(row, Row::Fold(_)) && in_view && answered) {
                shown.push(row);
            }
            continue;
        };
        match &item.content {
            ItemContent::UserMessage {
                text, task_result, ..
            } => {
                unanswered.push_back(match task_result {
                    Some(_) => Heard::Task(task_outcome(text)),
                    None => Heard::User,
                });
                answered |= task_result.is_some();
                if task_result.is_none() {
                    if let Some(to_user) = to_user.as_deref_mut() {
                        to_user.insert(index);
                    }
                    shown.push(row);
                }
            }
            ItemContent::AssistantMessage { .. } => {
                reply_to = unanswered.pop_front().or(reply_to);
                match reply_to {
                    Some(Heard::Task(outcome)) => shown.push(Row::Note(index, outcome)),
                    Some(Heard::User) => {
                        if let Some(to_user) = to_user.as_deref_mut() {
                            to_user.insert(index);
                        }
                        shown.push(row);
                    }
                    None => shown.push(row),
                }
            }
            _ => shown.push(row),
        }
    }
    shown
}

#[cfg(test)]
mod tests {
    use super::{TaskOutcome, fold_task_runs, task_outcome};
    use crate::fold::{After, Ending, Fold};
    use crate::transcript::Row;
    use hoshi_core::{ItemContent, ThreadItem};

    fn at(second: u32) -> String {
        format!("2026-11-10T10:{:02}:{:02}.000Z", second / 60, second % 60)
    }

    fn item(id: &str, second: u32, content: ItemContent) -> ThreadItem {
        ThreadItem {
            id: id.into(),
            thread_id: "t".into(),
            created_at: at(second),
            content,
        }
    }

    fn message(id: &str, second: u32, text: &str, steer: bool, task: Option<&str>) -> ThreadItem {
        item(
            id,
            second,
            ItemContent::UserMessage {
                text: text.into(),
                attachments: Vec::new(),
                steer,
                task_result: task.map(str::to_owned),
                pull_request_update: None,
                resumed: None,
                rewind: None,
                dropped: false,
                model_switch: None,
            },
        )
    }

    fn user(id: &str, second: u32) -> ThreadItem {
        message(id, second, id, false, None)
    }

    fn steer(id: &str, second: u32) -> ThreadItem {
        message(id, second, id, true, None)
    }

    fn result(id: &str, task: &str, second: u32, status: &str) -> ThreadItem {
        let text = format!("Delegated task {task} ({task}) [{status}]:\n{task} is done");
        message(id, second, &text, false, Some(task))
    }

    fn said(id: &str, second: u32) -> ThreadItem {
        item(
            id,
            second,
            ItemContent::AssistantMessage { text: id.into() },
        )
    }

    fn delegated(id: &str, task: &str) -> ThreadItem {
        item(
            id,
            1,
            ItemContent::ToolCall {
                name: "mcp__hoshi__delegate_task".into(),
                input: format!(r#"{{"title":"{task}"}}"#),
                output: Some(format!(r#"{{"taskId":"{task}"}}"#)),
                is_error: false,
                detail: None,
                output_omitted: false,
                interrupted: false,
            },
        )
    }

    /// The thread's items and its rows: each item its own row, except `d1` and `d2`, which
    /// the turn folded.
    fn thread(extra: Vec<ThreadItem>) -> (Vec<ThreadItem>, Vec<Row>) {
        let mut items = vec![
            user("u1", 0),
            delegated("d1", "a"),
            delegated("d2", "b"),
            said("started", 2),
            result("ra", "a", 30, "completed"),
            said("a answered", 31),
            result("rb", "b", 90, "failed"),
            said("summary", 92),
        ];
        items.extend(extra);
        let mut rows = vec![
            Row::Item(0),
            Row::Fold(Fold {
                start: 0,
                rows: vec![Row::Item(1), Row::Item(2)],
                ending: Ending::Answered,
                after: After::Start,
                duration: Some(2000),
                tasks: false,
            }),
        ];
        rows.extend((3..items.len()).map(Row::Item));
        (items, rows)
    }

    fn shape(rows: &[Row], items: &[ThreadItem]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Item(index) => items[*index].id.clone(),
                Row::Note(index, outcome) => format!("{} ({outcome:?})", items[*index].id),
                Row::Fold(fold) => format!(
                    "[{}] {:?}",
                    shape(&fold.rows, items).join(", "),
                    fold.duration
                ),
                row => format!("{row:?}"),
            })
            .collect()
    }

    fn none(_: &str) -> bool {
        false
    }

    fn still_b(task: &str) -> bool {
        task == "b"
    }

    #[test]
    fn a_task_answer_tells_how_the_task_ended() {
        assert_eq!(
            task_outcome("Delegated task a (A) [completed]:\nDone"),
            TaskOutcome::Completed
        );
        assert_eq!(
            task_outcome("Delegated task a (A) [failed]:\nNo"),
            TaskOutcome::Failed
        );
        assert_eq!(
            task_outcome("Delegated task a (A) [stopped]:\n"),
            TaskOutcome::Stopped
        );
    }

    #[test]
    fn while_a_task_still_runs_its_answers_hide_and_the_replies_read_as_notes() {
        let (items, mut rows) = thread(Vec::new());
        rows.truncate(5);
        assert_eq!(
            shape(&fold_task_runs(rows, &items, &still_b), &items),
            [
                "u1",
                "[d1, d2] Some(2000)",
                "started",
                "a answered (Completed)"
            ]
        );
    }

    #[test]
    fn once_every_task_answered_the_work_folds_behind_its_last_answer() {
        let (items, rows) = thread(Vec::new());
        assert_eq!(
            shape(&fold_task_runs(rows, &items, &none), &items),
            [
                "u1",
                "[d1, d2, started, a answered (Completed)] Some(92000)",
                "summary"
            ]
        );
    }

    #[test]
    fn the_agent_still_working_on_the_last_answer_keeps_it_all_open() {
        let (items, rows) = thread(Vec::new());
        let shown = fold_task_runs(rows, &items, &still_b);
        assert_eq!(shape(&shown, &items).last().unwrap(), "summary (Failed)");
        assert!(matches!(shown[1], Row::Fold(Fold { tasks: false, .. })));
    }

    #[test]
    fn of_the_replies_after_the_last_answer_only_the_latest_stays_in_view() {
        let (items, rows) = thread(vec![said("all of it", 95)]);
        let shown = fold_task_runs(rows, &items, &none);
        assert_eq!(
            shape(&shown, &items),
            [
                "u1",
                "[d1, d2, started, a answered (Completed), summary (Failed)] Some(95000)",
                "all of it"
            ]
        );
        assert!(matches!(shown[1], Row::Fold(Fold { tasks: true, .. })));
    }

    #[test]
    fn a_reply_to_a_task_still_being_written_reads_as_a_note() {
        let (items, mut rows) = thread(Vec::new());
        rows.truncate(4);
        rows.push(Row::Streaming(0));
        let shown = fold_task_runs(rows, &items, &still_b);
        assert_eq!(
            shown.last(),
            Some(&Row::StreamingNote(0, TaskOutcome::Completed))
        );
    }

    #[test]
    fn an_answer_being_written_after_the_last_task_answer_stays_in_view_as_text() {
        let (items, mut rows) = thread(Vec::new());
        rows.push(Row::Streaming(0));
        assert_eq!(
            shape(&fold_task_runs(rows, &items, &none), &items),
            [
                "u1",
                "[d1, d2, started, a answered (Completed), summary (Failed)] Some(90000)",
                "Streaming(0)"
            ]
        );
    }

    #[test]
    fn work_without_delegated_tasks_stays_as_it_is() {
        let items = vec![user("u1", 0), said("done", 3)];
        let rows = vec![Row::Item(0), Row::Item(1)];
        assert_eq!(fold_task_runs(rows.clone(), &items, &none), rows);
    }

    #[test]
    fn each_message_of_the_user_starts_its_own_stretch() {
        let (items, rows) = thread(vec![user("u2", 100), said("next", 101)]);
        let shown = shape(&fold_task_runs(rows, &items, &none), &items);
        assert_eq!(shown[shown.len() - 3..], ["summary", "u2", "next"]);
    }

    #[test]
    fn what_settled_stays_folded_while_the_agent_delegates_again() {
        let items = vec![
            user("u1", 0),
            delegated("d1", "a"),
            said("started", 2),
            result("ra", "a", 10, "completed"),
            delegated("d2", "b"),
            said("got a", 12),
        ];
        let first: Vec<Row> = (0..4).map(Row::Item).collect();
        assert_eq!(
            shape(&fold_task_runs(first, &items, &none), &items),
            ["u1", "[d1, started] Some(10000)"]
        );
        let second: Vec<Row> = (0..6).map(Row::Item).collect();
        assert_eq!(
            shape(&fold_task_runs(second, &items, &still_b), &items),
            ["u1", "[d1, started] Some(1000)", "d2", "got a"]
        );
    }

    #[test]
    fn what_the_agent_tells_the_user_between_task_answers_is_no_note() {
        let (mut items, _) = thread(Vec::new());
        items.truncate(6);
        items.extend([
            steer("steer", 32),
            said("going well", 33),
            result("rb", "b", 90, "completed"),
            said("summary", 92),
        ]);
        let rows: Vec<Row> = (0..items.len()).map(Row::Item).collect();
        let open = shape(
            &fold_task_runs(rows[..8].to_vec(), &items, &still_b),
            &items,
        );
        assert_eq!(
            open[open.len() - 3..],
            ["a answered (Completed)", "steer", "going well"]
        );
        assert_eq!(
            shape(&fold_task_runs(rows, &items, &none), &items),
            [
                "u1",
                "[d1, d2, started, a answered (Completed)] Some(32000)",
                "steer",
                "going well",
                "summary"
            ]
        );
    }

    #[test]
    fn each_reply_goes_with_the_oldest_answer_or_message_still_without_one() {
        let (mut items, _) = thread(Vec::new());
        items.truncate(5);
        items.extend([
            steer("steer", 31),
            said("a is done", 32),
            result("rc", "c", 33, "completed"),
            said("fine, thanks", 34),
            said("c is done", 35),
        ]);
        let rows: Vec<Row> = (0..items.len()).map(Row::Item).collect();
        let shown = shape(&fold_task_runs(rows, &items, &still_b), &items);
        assert_eq!(
            shown[shown.len() - 4..],
            [
                "steer",
                "a is done (Completed)",
                "fine, thanks",
                "c is done (Completed)"
            ]
        );
    }
}
