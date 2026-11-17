//! Finished turns fold their work behind one line, with the agent's answer kept in view.

use crate::tool_calls::{is_agent, is_delegation, is_html_render, is_subagent};
use crate::transcript::Row;
use hoshi_core::{ItemContent, ThreadItem};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    Answered,
    /// Cut by the user's steer.
    Steered,
    Stopped,
    /// Cut by a restart of hoshi, which then resumed the turn.
    Restarted,
}

/// What the folded work follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum After {
    Start,
    Steer,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fold {
    /// What started the work, by its index in the thread's items.
    pub start: usize,
    pub rows: Vec<Row>,
    pub ending: Ending,
    pub after: After,
    /// From what started the work to its end, in milliseconds; `None` when unknown.
    pub duration: Option<i64>,
    /// Settled work on delegated tasks, folded apart from the turn's.
    pub tasks: bool,
}

impl Fold {
    /// The key it opens under.
    pub fn key(&self, items: &[ThreadItem]) -> String {
        let start = items.get(self.start).map_or("", |item| item.id.as_str());
        let kind = if self.tasks { "tasks" } else { "fold" };
        format!("{kind}:{start}")
    }
}

/// Splits the id of a background task's end into its turn and the task; mirrors hoshi-core.
const TASK_ENDED: &str = ":ended:";

/// Parts of a turn a steer cut, folded only once the turn is over.
struct Held {
    open: Vec<Row>,
    settled: Vec<Row>,
}

struct Folder<'a> {
    items: &'a [ThreadItem],
    live: &'a dyn Fn(&Row) -> bool,
    started: Option<usize>,
    after: After,
    turn: Vec<Row>,
    held: Vec<Held>,
    turn_start: Option<i64>,
    folded: Vec<Row>,
}

/// The work of each finished run folds into one row: between what started it (the user's
/// message, a background task that ended, or the run before) and the agent's answer, which
/// stays in view, with the finished work after it. A steer cuts the run: what came before
/// folds whole, and what follows folds on its own. A stopped run folds whole. The running
/// turn, and a failed one, stay open, as do `live` rows, after the fold, and context handoffs,
/// before it. A run that a steer
/// ended before any answer does not cut the turn, and while working neither does any run
/// that ended since the latest response started.
pub fn fold_turns(
    rows: Vec<Row>,
    items: &[ThreadItem],
    working: bool,
    live: &dyn Fn(&Row) -> bool,
) -> Vec<Row> {
    let starts = response_starts(&rows, items);
    let response_at = starts.iter().max().copied();
    let past_response = |index: usize| working && response_at.is_none_or(|at| index > at);
    let mut folder = Folder {
        items,
        live,
        started: None,
        after: After::Start,
        turn: Vec::new(),
        held: Vec::new(),
        turn_start: None,
        folded: Vec::new(),
    };
    for (index, row) in rows.into_iter().enumerate() {
        let Some((at, item)) = indexed_item(&row, items) else {
            folder.turn.push(row);
            continue;
        };
        match &item.content {
            ItemContent::UserMessage { steer: true, .. } => {
                if folder.after == After::Start {
                    folder.turn_start = folder.started.and_then(|start| time_of(&items[start]));
                }
                let turn = std::mem::take(&mut folder.turn);
                let mut open = turn.clone();
                open.push(row.clone());
                let mut settled = folder.settle(turn, Some(item));
                settled.push(row);
                folder.held.push(Held { open, settled });
                folder.started = Some(at);
                folder.after = After::Steer;
            }
            _ if starts.contains(&index) => {
                folder.close(false);
                folder.folded.push(row);
                folder.started = Some(at);
                folder.after = After::Start;
            }
            ItemContent::RunEnded => {
                if past_response(index)
                    || (folder.after == After::Steer && !folder.answered(&folder.turn))
                {
                    continue;
                }
                folder.close(false);
                folder.started = Some(at);
                folder.after = After::Start;
            }
            _ => folder.turn.push(row),
        }
    }
    folder.close(working);
    folder.folded
}

/// Whether a row stays in view when its turn folds: a delegated task, which runs on its own,
/// a page the agent showed, or a subagent without its answer.
pub fn stays_in_view(row: &Row, items: &[ThreadItem]) -> bool {
    match indexed_item(row, items).map(|(_, item)| &item.content) {
        Some(ItemContent::ToolCall {
            name,
            output,
            interrupted,
            is_error,
            ..
        }) => {
            is_delegation(name)
                || (is_html_render(name) && output.is_some() && !is_error)
                || (is_subagent(name) && output.is_none() && !interrupted)
        }
        _ => false,
    }
}

fn indexed_item<'a>(row: &Row, items: &'a [ThreadItem]) -> Option<(usize, &'a ThreadItem)> {
    match row {
        Row::Item(index) => items.get(*index).map(|item| (*index, item)),
        _ => None,
    }
}

impl<'a> Folder<'a> {
    fn item(&self, row: &Row) -> Option<&'a ThreadItem> {
        indexed_item(row, self.items).map(|(_, item)| item)
    }

    fn is_live(&self, row: &Row) -> bool {
        (self.live)(row)
    }

    /// A line that opens the turn rather than being its work: it stays out of the fold.
    fn is_handoff(&self, row: &Row) -> bool {
        self.item(row).is_some_and(|item| {
            matches!(
                item.content,
                ItemContent::ContextHandoff { .. } | ItemContent::ModelSwitched { .. }
            )
        })
    }

    fn is_answer(&self, row: &Row) -> bool {
        self.item(row)
            .is_some_and(|item| matches!(item.content, ItemContent::AssistantMessage { .. }))
    }

    fn answered(&self, part: &[Row]) -> bool {
        part.iter().any(|row| self.is_answer(row))
    }

    fn has_work(&self, part: &[Row]) -> bool {
        part.iter().any(|row| {
            !self.is_live(row)
                && match row {
                    Row::Work(_) => true,
                    _ => self.item(row).is_some_and(|item| {
                        matches!(&item.content, ItemContent::ToolCall { name, .. } if is_agent(name))
                    }),
                }
        })
    }

    /// Work done with: no longer running and without a failed call.
    fn settled(&self, row: &Row) -> bool {
        let Row::Work(indices) = row else {
            return false;
        };
        !self.is_live(row)
            && !indices
                .iter()
                .filter_map(|index| self.items.get(*index))
                .any(|item| matches!(item.content, ItemContent::ToolCall { is_error: true, .. }))
    }

    fn end_of(&self, part: &[Row]) -> Option<i64> {
        part.iter().rev().find_map(|row| {
            let item = match row {
                Row::Item(index) => self.items.get(*index),
                Row::Work(indices) => indices.last().and_then(|index| self.items.get(*index)),
                _ => None,
            };
            item.and_then(time_of)
        })
    }

    fn fold(&self, part: Vec<Row>, ending: Ending, ended_at: &str) -> Vec<Row> {
        let Some(start) = self.started else {
            return part;
        };
        let duration = self
            .items
            .get(start)
            .and_then(time_of)
            .zip(parse_time(ended_at))
            .map(|(started, ended)| ended - started);
        let (mut folded, part): (Vec<Row>, Vec<Row>) =
            part.into_iter().partition(|row| self.is_handoff(row));
        let (live, rows): (Vec<Row>, Vec<Row>) =
            part.into_iter().partition(|row| self.is_live(row));
        folded.push(Row::Fold(Fold {
            start,
            rows,
            ending,
            after: self.after,
            duration,
            tasks: false,
        }));
        folded.extend(live);
        folded
    }

    fn settle(&self, part: Vec<Row>, steer: Option<&ThreadItem>) -> Vec<Row> {
        let failed = part.iter().any(|row| {
            self.item(row)
                .is_some_and(|item| matches!(item.content, ItemContent::TurnFailed { .. }))
        });
        if self.started.is_none() || failed || !self.has_work(&part) {
            return part;
        }
        if let Some(steer) = steer {
            return self.fold(part, Ending::Steered, &steer.created_at);
        }
        let stop = part.last().and_then(|row| self.item(row)).filter(|item| {
            matches!(
                item.content,
                ItemContent::TurnInterrupted | ItemContent::TurnCut
            )
        });
        if let Some(stop) = stop {
            let before = &part[..part.len() - 1];
            let answer = before.last().filter(|row| self.is_answer(row));
            let work = &before[..before.len() - usize::from(answer.is_some())];
            if !self.has_work(work) {
                return part;
            }
            let ending = if matches!(stop.content, ItemContent::TurnCut) {
                Ending::Restarted
            } else {
                Ending::Stopped
            };
            let mut rows = self.fold(work.to_vec(), ending, &stop.created_at);
            rows.extend(answer.cloned());
            return rows;
        }
        let Some(answer_at) = part.iter().rposition(|row| self.is_answer(row)) else {
            return part;
        };
        let answer = &part[answer_at];
        let trailing = &part[answer_at + 1..];
        let mut work = part[..answer_at].to_vec();
        work.extend(trailing.iter().filter(|row| self.settled(row)).cloned());
        if !self.has_work(&work) {
            return part;
        }
        let answered_at = self
            .item(answer)
            .map_or("", |item| item.created_at.as_str());
        let mut rows = self.fold(work, Ending::Answered, answered_at);
        rows.push(answer.clone());
        rows.extend(trailing.iter().filter(|row| !self.settled(row)).cloned());
        rows
    }

    fn close(&mut self, running: bool) {
        let end = self.end_of(&self.turn);
        let turn = std::mem::take(&mut self.turn);
        for held in std::mem::take(&mut self.held) {
            if running {
                self.folded.extend(held.open);
            } else {
                for row in held.settled {
                    let row = self.whole_turn(row, end);
                    self.folded.push(row);
                }
            }
        }
        let rows = if running {
            turn
        } else {
            self.settle(turn, None)
        };
        self.folded.extend(rows);
    }

    /// The first part of a turn a steer cut lasts as long as the whole turn.
    fn whole_turn(&self, row: Row, end: Option<i64>) -> Row {
        match row {
            Row::Fold(mut fold) if fold.after == After::Start => {
                if let Some((start, end)) = self.turn_start.zip(end) {
                    fold.duration = Some(end - start);
                }
                Row::Fold(fold)
            }
            row => row,
        }
    }
}

/// Where each response starts: at a message from the user, or at a background task that
/// ended and gets a run of its own. A task that ended while a run of its own turn went on is
/// taken in by that run and starts nothing.
fn response_starts(rows: &[Row], items: &[ThreadItem]) -> HashSet<usize> {
    let mut starts = HashSet::new();
    let mut turn: Option<&str> = None;
    let mut between_runs = false;
    for (index, row) in rows.iter().enumerate() {
        let Some((_, item)) = indexed_item(row, items) else {
            between_runs = false;
            continue;
        };
        match &item.content {
            ItemContent::UserMessage { steer: true, .. } => {}
            ItemContent::UserMessage { .. } => {
                starts.insert(index);
                turn = Some(&item.id);
                between_runs = false;
            }
            ItemContent::BackgroundTaskEnded { .. } => {
                let ended_in = item.id.find(TASK_ENDED).map(|at| &item.id[..at]);
                if ended_in.is_some() && ended_in == turn && !between_runs {
                    continue;
                }
                starts.insert(index);
                turn = ended_in;
                between_runs = false;
            }
            ItemContent::RunEnded => between_runs = true,
            ItemContent::TurnInterrupted
            | ItemContent::TurnCut
            | ItemContent::TurnFailed { .. } => turn = None,
            _ => between_runs = false,
        }
    }
    starts
}

fn time_of(item: &ThreadItem) -> Option<i64> {
    parse_time(&item.created_at)
}

fn parse_time(time: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(time)
        .ok()
        .map(|time| time.timestamp_millis())
}

#[cfg(test)]
mod tests {
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
                Row::Streaming(_) | Row::StreamingNote(..) => "streaming".into(),
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
}
