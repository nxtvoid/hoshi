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
mod tests;
