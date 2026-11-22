use crate::details::agents::{AgentState, subagent_state};
use crate::fold::{self, After, Ending, Fold};
use crate::format::format_duration;
use crate::m;
use crate::request_rows::{self, CallApproval, approvals_by_call};
use crate::state::ThreadState;
use crate::task_runs::{TaskOutcome, fold_task_runs};
use crate::tool_calls::{is_agent, is_delegation, is_html_render, is_request};
use crate::turn_changes::{answers_with_changes, with_card};
use crate::turn_status;
use crate::work::{self, WorkContext};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};
use gpui_kit::component::text::TextView;
use gpui_kit::*;
use hoshi_core::{
    HandoffDelivery, HandoffSource, ItemContent, StreamKind, ThreadBackgroundTasks, ThreadItem,
    TurnRetry,
};
use hoshi_ui::prelude::*;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Item(usize),
    /// A run of tool calls and reasoning, by their index in the thread's items.
    Work(Vec<usize>),
    Streaming(usize),
    /// A finished turn's work, behind one line.
    Fold(Fold),
    /// How long the running turn has worked, since the item at the index, where its work
    /// will fold.
    Status(Option<usize>),
    /// The commands the agent left running in the background.
    Background,
    /// Subagents and delegated tasks started one after another, as one card.
    Agents(Vec<usize>),
    /// The delegated tasks still running once the agent rests.
    TaskWaits,
    /// The agent's reply to a task's answer, while the work on its tasks goes on.
    Note(usize, TaskOutcome),
    /// Such a reply while it is still being written, by its index in the thread's streams.
    StreamingNote(usize, TaskOutcome),
    /// Above the history inherited from a thread, or the thread's own items: by that thread.
    ForkedFrom(String),
    /// After the turn a fork goes on from: by the fork.
    ForkedInto(String),
}

impl Row {
    /// The same row once `by` items went before the thread's first.
    fn shifted(&self, by: usize) -> Row {
        match self {
            Self::Item(index) => Self::Item(index + by),
            Self::Work(indices) => Self::Work(indices.iter().map(|index| index + by).collect()),
            Self::Streaming(index) => Self::Streaming(*index),
            Self::Fold(fold) => Self::Fold(Fold {
                start: fold.start + by,
                rows: fold.rows.iter().map(|row| row.shifted(by)).collect(),
                ..fold.clone()
            }),
            Self::Status(index) => Self::Status(index.map(|index| index + by)),
            Self::Background => Self::Background,
            Self::Agents(indices) => Self::Agents(indices.iter().map(|index| index + by).collect()),
            Self::TaskWaits => Self::TaskWaits,
            Self::Note(index, outcome) => Self::Note(index + by, *outcome),
            Self::StreamingNote(index, outcome) => Self::StreamingNote(*index, *outcome),
            Self::ForkedFrom(_) | Self::ForkedInto(_) => self.clone(),
        }
    }
}

pub struct TranscriptView {
    thread: Entity<ThreadState>,
    scroller: Entity<MessageScrollerState>,
    rows: Rc<Vec<Row>>,
    approvals: Rc<HashMap<String, CallApproval>>,
    turn_changes: Rc<HashMap<String, usize>>,
    /// The rows, runs and folds the user opened, by item id, `group_key` or fold key.
    open: Rc<HashSet<String>>,
    /// The first item shown, to tell a page put above from items added below.
    first_item: Option<String>,
    /// The thread's agent is at work, so its latest turn stays open.
    working: bool,
    /// The commands the agent left running, shown at the end.
    background: Option<ThreadBackgroundTasks>,
    /// How the running turn's provider retries to reach the model, beside its status.
    retry: Option<TurnRetry>,
    /// When the user stopped the running turn, which has yet to end.
    stopping: Option<std::time::Instant>,
    /// How the tasks the thread delegated stand, to redraw their cards as that changes.
    tasks: Vec<(String, AgentState)>,
    /// Redraws every second while a clock shows.
    _ticker: Option<Task<()>>,
    /// Redraws what reads the time of day, as a usage limit's reset passing.
    _clock: Task<()>,
    _workspace: Option<Subscription>,
    minimap: crate::minimap::MinimapState,
    /// The forks and sources the transcript links to.
    fork_marks: Rc<crate::fork::ForkMarks>,
    /// The answers that offer a fork.
    forkable: crate::fork::Forkable,
    _subscriptions: [Subscription; 3],
}

const CLOCK_EVERY: Duration = Duration::from_secs(30);

/// Rows left above the viewport when the page before them starts loading.
const LOAD_OLDER_WITHIN: usize = 5;

/// The transcript as the window shows it: drawn again only when it changes, so frames for the
/// rest of the window skip its rows. Drawn afresh in the frame the lanes' room changes.
pub fn element(transcript: Entity<TranscriptView>, room_changed: bool) -> AnyElement {
    if room_changed {
        return transcript.into_any_element();
    }
    AnyView::from(transcript)
        .cached(StyleRefinement::default().flex_1().min_h_0().size_full())
        .into_any_element()
}

impl TranscriptView {
    pub fn new(thread: Entity<ThreadState>, working: bool, cx: &mut Context<Self>) -> Self {
        let fork_marks = crate::fork::marks(thread.read(cx), cx);
        let rows = Rc::new(crate::fork::with_dividers(
            build_rows(thread.read(cx), working, false, false, &HashSet::new()),
            thread.read(cx),
            &fork_marks,
        ));
        let mut forkable = crate::fork::Forkable::default();
        forkable.refresh(thread.read(cx), working, cx);
        let scroller = cx.new(|cx| MessageScrollerState::new(rows.len(), cx));
        let observe_thread = cx.observe(&thread, |this, _, cx| this.rebuild(cx));
        let observe_scroll = cx.observe(&scroller, |this, scroller, cx| {
            let scroller = scroller.read(cx);
            if scroller.is_scrolled_up()
                && scroller.logical_scroll_top().item_ix < LOAD_OLDER_WITHIN
            {
                this.thread.update(cx, |thread, cx| thread.load_older(cx));
            }
            cx.notify();
        });
        let observe_room = cx.observe_global::<hoshi_ui::LaneRoom>(|_, cx| cx.notify());
        let first_item = thread.read(cx).items.first().map(|item| item.id.clone());
        let approvals = Rc::new(approvals_by_call(&thread.read(cx).items));
        let turn_changes = Rc::new(answers_with_changes(&thread.read(cx).items));
        let observe_workspace = crate::markdown::workspace(cx).map(|workspace| {
            cx.observe(&workspace, |this, _, cx| {
                this.follow_forks(cx);
                this.follow_tasks(cx);
                cx.notify();
            })
        });
        Self {
            thread,
            scroller,
            rows,
            approvals,
            turn_changes,
            open: Rc::default(),
            first_item,
            working,
            background: None,
            retry: None,
            stopping: None,
            tasks: Vec::new(),
            _ticker: None,
            _clock: cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(CLOCK_EVERY).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }),
            _workspace: observe_workspace,
            minimap: Default::default(),
            fork_marks: Rc::new(fork_marks),
            forkable,
            _subscriptions: [observe_thread, observe_scroll, observe_room],
        }
    }

    pub(crate) fn scroller(&self) -> &Entity<MessageScrollerState> {
        &self.scroller
    }

    pub(crate) fn minimap_state(&self) -> &crate::minimap::MinimapState {
        &self.minimap
    }

    pub(crate) fn minimap_state_mut(&mut self) -> &mut crate::minimap::MinimapState {
        &mut self.minimap
    }

    pub(crate) fn minimap_origin_y(&self) -> Pixels {
        self.minimap.origin.y
    }

    pub fn thread_id(&self, cx: &App) -> String {
        self.thread.read(cx).thread_id.clone()
    }

    pub fn thread_state(&self) -> &Entity<ThreadState> {
        &self.thread
    }

    pub fn set_working(&mut self, working: bool, cx: &mut Context<Self>) {
        if self.working != working {
            self.working = working;
            self.rebuild(cx);
            self.tick(cx);
        }
    }

    /// The commands the agent left running, shown at the end while there are any.
    pub fn set_background(
        &mut self,
        background: Option<ThreadBackgroundTasks>,
        cx: &mut Context<Self>,
    ) {
        let background = background.filter(|background| !background.tasks.is_empty());
        if self.background != background {
            self.background = background;
            self.rebuild(cx);
            self.tick(cx);
        }
    }

    /// How the running turn's provider retries, shown beside its status while it does.
    pub fn set_retry(&mut self, retry: Option<TurnRetry>, cx: &mut Context<Self>) {
        if self.retry != retry {
            self.retry = retry;
            cx.notify();
        }
    }

    /// Says the running turn is stopping, in place of how long it worked, until it ends.
    pub fn set_stopping(&mut self, stopping: Option<std::time::Instant>, cx: &mut Context<Self>) {
        if self.stopping != stopping {
            self.stopping = stopping;
            cx.notify();
        }
    }

    /// Redraws the cards of the tasks the thread delegated as they change, and shows those
    /// still running at the end while the agent rests.
    fn follow_tasks(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = crate::markdown::workspace(cx) else {
            return;
        };
        let thread_id = self.thread.read(cx).thread_id.clone();
        let workspace = workspace.read(cx);
        let board = crate::delegations::TaskBoard::new(workspace);
        let mut tasks: Vec<(String, AgentState)> = workspace
            .model
            .threads()
            .filter(|thread| thread.parent_thread_id.as_deref() == Some(thread_id.as_str()))
            .map(|thread| (thread.id.clone(), board.state(thread)))
            .collect();
        tasks.sort_by(|a, b| a.0.cmp(&b.0));
        if tasks == self.tasks {
            return;
        }
        let before = self.active_tasks();
        self.tasks = tasks;
        let rebuild = self.active_tasks() != before;
        if rebuild {
            self.rebuild(cx);
        }
        self.tick(cx);
        cx.notify();
    }

    /// Redraws the dividers that link forks as forks come and go or get renamed.
    fn follow_forks(&mut self, cx: &mut Context<Self>) {
        let marks = crate::fork::marks(self.thread.read(cx), cx);
        if marks != *self.fork_marks {
            self.fork_marks = Rc::new(marks);
            self.rebuild(cx);
        }
    }

    fn task_waits(&self) -> bool {
        self.tasks.iter().any(|(_, state)| state.is_active())
    }

    fn active_tasks(&self) -> HashSet<String> {
        self.tasks
            .iter()
            .filter(|(_, state)| state.is_active())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Keeps the clocks of the status, the background commands and the tasks going.
    fn tick(&mut self, cx: &mut Context<Self>) {
        if !self.working && self.background.is_none() && !self.task_waits() {
            self._ticker = None;
            return;
        }
        if self._ticker.is_some() {
            return;
        }
        self._ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        }));
    }

    /// Opens or closes a row, a run or a fold, by item id, `group_key` or fold key. A call that
    /// its page left part of is read whole as it opens.
    pub fn toggle(&mut self, id: &str, cx: &mut Context<Self>) {
        let open = Rc::make_mut(&mut self.open);
        if !open.remove(id) {
            open.insert(id.to_owned());
            self.thread
                .update(cx, |thread, cx| thread.read_whole(id, cx));
        }
        cx.notify();
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let thread = self.thread.read(cx);
        let next = crate::fork::with_dividers(
            build_rows(
                thread,
                self.working,
                self.background.is_some(),
                self.task_waits(),
                &self.active_tasks(),
            ),
            thread,
            &self.fork_marks,
        );
        self.forkable.refresh(thread, self.working, cx);
        let approvals = Rc::new(approvals_by_call(&thread.items));
        let turn_changes = Rc::new(answers_with_changes(&thread.items));
        let first = thread.items.first().map(|item| item.id.clone());
        let pages = crate::html_page::pages_to_read(thread);
        let prepended = self
            .first_item
            .as_ref()
            .filter(|old| Some(*old) != first.as_ref())
            .and_then(|old| thread.items.iter().position(|item| &item.id == old));
        let (before, after) = (self.rows.len(), next.len());
        let (changed, count) = changed_rows(&self.rows, &next, prepended.unwrap_or(0));
        self.scroller.update(cx, |scroller, cx| {
            if before == 0 {
                scroller.reset(after, cx);
                return;
            }
            if !changed.is_empty() || count > 0 {
                scroller.splice(changed, count, cx);
            }
            if prepended.is_none() && after > 0 {
                scroller.remeasure_items(after.saturating_sub(2)..after, cx);
            }
        });
        self.thread.update(cx, |thread, cx| {
            for id in &pages {
                thread.read_whole(id, cx);
            }
        });
        self.first_item = first;
        self.rows = Rc::new(next);
        self.approvals = approvals;
        self.turn_changes = turn_changes;
        cx.notify();
    }

    /// Scrolls to the row that shows `item_id`, opening the fold it is in.
    pub fn reveal(&mut self, item_id: &str, cx: &mut Context<Self>) {
        let items = &self.thread.read(cx).items;
        let Some(index) = items.iter().position(|item| item.id == item_id) else {
            return;
        };
        let Some((row, fold)) = row_of(&self.rows, index, items) else {
            return;
        };
        if let Some(key) = fold {
            Rc::make_mut(&mut self.open).insert(key);
        }
        self.scroller
            .update(cx, |scroller, cx| scroller.scroll_to_item(row, cx));
        cx.notify();
    }
}

/// The top row that shows the item at `index`, and the key of the fold that holds it.
fn row_of(rows: &[Row], index: usize, items: &[ThreadItem]) -> Option<(usize, Option<String>)> {
    fn holds(row: &Row, index: usize) -> bool {
        match row {
            Row::Item(item) | Row::Note(item, _) => *item == index,
            Row::Work(indices) => indices.contains(&index),
            Row::Fold(fold) => fold.rows.iter().any(|row| holds(row, index)),
            Row::Agents(indices) => indices.contains(&index),
            Row::Streaming(_)
            | Row::StreamingNote(..)
            | Row::Status(_)
            | Row::Background
            | Row::TaskWaits
            | Row::ForkedFrom(_)
            | Row::ForkedInto(_) => false,
        }
    }
    let at = rows.iter().position(|row| holds(row, index))?;
    let fold = match &rows[at] {
        Row::Fold(fold) => Some(fold.key(items)),
        Row::Agents(indices) => indices
            .first()
            .and_then(|first| items.get(*first))
            .map(|first| agents_key(&first.id)),
        _ => None,
    };
    Some((at, fold))
}

/// The rows of `old` that `new` replaces, and how many rows replace them: what lies between
/// the rows both start and end with. `shift` is how many items went before the first of `old`.
fn changed_rows(old: &[Row], new: &[Row], shift: usize) -> (Range<usize>, usize) {
    let same = |old: &Row, new: &Row| old.shifted(shift) == *new;
    let start = old
        .iter()
        .zip(new)
        .take_while(|(old, new)| same(old, new))
        .count();
    let end = old[start..]
        .iter()
        .rev()
        .zip(new[start..].iter().rev())
        .take_while(|(old, new)| same(old, new))
        .count();
    (start..old.len() - end, new.len() - start - end)
}

/// The key a group of agents opens under: its first call's.
fn agents_key(first_item: &str) -> String {
    format!("agents:{first_item}")
}

/// Subagents and delegated tasks started one after another become one row, in folds too.
fn agents_together(rows: Vec<Row>, items: &[ThreadItem]) -> Vec<Row> {
    let subagent = |row: &Row| match row {
        Row::Item(index) => matches!(
            items.get(*index).map(|item| &item.content),
            Some(ItemContent::ToolCall { name, .. }) if is_agent(name)
        ),
        _ => false,
    };
    let mut out: Vec<Row> = Vec::with_capacity(rows.len());
    for row in rows {
        let row = match row {
            Row::Fold(fold) => Row::Fold(Fold {
                rows: agents_together(fold.rows, items),
                ..fold
            }),
            row => row,
        };
        if let Row::Item(index) = row
            && subagent(&row)
        {
            match out.last_mut() {
                Some(Row::Agents(indices)) => {
                    indices.push(index);
                    continue;
                }
                Some(last) if subagent(last) => {
                    let Row::Item(first) = *last else {
                        unreachable!()
                    };
                    *last = Row::Agents(vec![first, index]);
                    continue;
                }
                _ => {}
            }
        }
        out.push(row);
    }
    out
}

/// The run of work a running turn is adding to: the last row, when it is one, past the
/// background commands that follow everything.
fn live_work_row(rows: &[Row]) -> Option<usize> {
    let at = rows
        .iter()
        .rposition(|row| !matches!(row, Row::Background | Row::TaskWaits))?;
    matches!(rows[at], Row::Work(_)).then_some(at)
}

/// Where a running turn's status goes: after the last fold, message from the user or ended
/// background task; at the top when there is none. The index in `rows` and the item it follows.
fn status_anchor(rows: &[Row], items: &[ThreadItem]) -> (usize, Option<usize>) {
    for (at, row) in rows.iter().enumerate().rev() {
        match row {
            Row::Fold(fold) => return (at + 1, Some(fold.start)),
            Row::Item(index) => {
                let starts = matches!(
                    items.get(*index).map(|item| &item.content),
                    Some(ItemContent::UserMessage { .. } | ItemContent::BackgroundTaskEnded { .. })
                );
                if starts {
                    return (at + 1, Some(*index));
                }
            }
            _ => {}
        }
    }
    (0, None)
}

fn build_rows(
    thread: &ThreadState,
    working: bool,
    background: bool,
    task_waits: bool,
    active_tasks: &HashSet<String>,
) -> Vec<Row> {
    let items = &thread.items;
    let rows = group(items);
    let live_work = rows
        .last()
        .filter(|row| match row {
            Row::Work(indices) => indices
                .iter()
                .filter_map(|index| items.get(*index))
                .any(work::active_entry),
            _ => false,
        })
        .cloned();
    let live = |row: &Row| Some(row) == live_work.as_ref() || fold::stays_in_view(row, items);
    let streams = |text: bool| {
        thread
            .streaming
            .iter()
            .enumerate()
            .filter(move |(_, streaming)| match streaming.kind {
                StreamKind::ToolOutput => false,
                StreamKind::Reasoning => !text,
                StreamKind::Text => text,
            })
            .map(|(index, _)| Row::Streaming(index))
    };
    let mut rows = fold::fold_turns(rows, items, working, &live);
    rows.extend(streams(true));
    let rows = fold_task_runs(rows, items, &|task| active_tasks.contains(task));
    let mut rows = shown_rows(rows, items);
    rows.extend(streams(false));
    let mut rows = agents_together(rows, items);
    if working {
        let (at, since) = status_anchor(&rows, items);
        rows.insert(at, Row::Status(since));
    }
    if background {
        rows.push(Row::Background);
    }
    if task_waits && !working {
        rows.push(Row::TaskWaits);
    }
    rows
}

/// The thread's items as rows, with the marks of where runs end and start that turns fold by.
pub fn group(items: &[ThreadItem]) -> Vec<Row> {
    let mut rows = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if !shown(item) && !marks_a_run(item) {
            continue;
        }
        if is_work(item) {
            if let Some(Row::Work(indices)) = rows.last_mut()
                && indices
                    .last()
                    .is_some_and(|last| items[*last].thread_id == item.thread_id)
            {
                indices.push(index);
            } else {
                rows.push(Row::Work(vec![index]));
            }
        } else {
            rows.push(Row::Item(index));
        }
    }
    rows
}

fn shown_rows(rows: Vec<Row>, items: &[ThreadItem]) -> Vec<Row> {
    rows.into_iter()
        .filter_map(|row| match row {
            Row::Item(index) => items
                .get(index)
                .is_some_and(shown)
                .then_some(Row::Item(index)),
            Row::Fold(fold) => Some(Row::Fold(Fold {
                rows: shown_rows(fold.rows, items),
                ..fold
            })),
            row => Some(row),
        })
        .collect()
}

fn marks_a_run(item: &ThreadItem) -> bool {
    matches!(
        item.content,
        ItemContent::RunEnded
            | ItemContent::BackgroundTaskEnded { .. }
            | ItemContent::UserMessage { .. }
    )
}

/// A message hoshi sent for the user, which shows as what it stands for or not at all.
fn sent_for_the_user(item: &ThreadItem) -> bool {
    match &item.content {
        ItemContent::UserMessage {
            text, task_result, ..
        } => task_result.is_some() || crate::compaction::is_compact_command(text),
        _ => false,
    }
}

fn shown(item: &ThreadItem) -> bool {
    match &item.content {
        ItemContent::ToolCall { name, .. } => !is_request(name),
        ItemContent::Approval { tool_call_id, .. } => tool_call_id.is_none(),
        ItemContent::UserMessage { .. } => !sent_for_the_user(item),
        ItemContent::AssistantMessage { .. }
        | ItemContent::TaskList { .. }
        | ItemContent::Compaction { .. }
        | ItemContent::Reasoning { .. }
        | ItemContent::TurnFailed { .. }
        | ItemContent::TurnInterrupted
        | ItemContent::TurnCut
        | ItemContent::Questions { .. }
        | ItemContent::ProposedPlan { .. }
        | ItemContent::SecretRequest { .. }
        | ItemContent::ModelRerouted { .. }
        | ItemContent::ProviderRecovered { .. }
        | ItemContent::McpServerFailed { .. }
        | ItemContent::HookStopped { .. }
        | ItemContent::BackgroundTaskEnded { .. }
        | ItemContent::ModelSwitched { .. } => true,
        ItemContent::ContextHandoff {
            sources, delivery, ..
        } => {
            *delivery == HandoffDelivery::Failed
                || sources
                    .iter()
                    .any(|source| !matches!(source, HandoffSource::Fork { .. }))
        }
        _ => false,
    }
}

fn is_work(item: &ThreadItem) -> bool {
    match &item.content {
        ItemContent::ToolCall { name, .. } => !is_agent(name) && !is_html_render(name),
        ItemContent::Reasoning { .. } => true,
        _ => false,
    }
}

impl Render for TranscriptView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let thread = self.thread.clone();
        let rows = self.rows.clone();
        let approvals = self.approvals.clone();
        let turn_changes = self.turn_changes.clone();
        let background = self.background.clone();
        let retry = self.retry.clone();
        let stopping = self.stopping;
        let live_work = self.working.then(|| live_work_row(&self.rows)).flatten();
        let working = self.working;
        let live_tasks = working
            .then(|| live_task_list(&self.thread.read(cx).items))
            .flatten();
        let open = self.open.clone();
        let view = cx.entity().downgrade();
        let loading = self.thread.read(cx).loading;
        let fork_marks = self.fork_marks.clone();
        let forkable = self.forkable.ids.clone();
        let inherited = self.thread.read(cx).inherited;
        let fade = colors.background;
        let minimap = self.minimap(
            crate::minimap::marks(&self.rows, &self.thread.read(cx).items),
            cx,
        );

        div()
            .relative()
            .flex_1()
            .min_h_0()
            .size_full()
            .capture_action(|_: &gpui_kit::base::input::Copy, window, cx| {
                if crate::markdown::copy_selection(window, cx) {
                    cx.stop_propagation();
                }
            })
            .child(hoshi_ui::selection_area())
            .when(loading, |this| {
                this.flex()
                    .items_center()
                    .justify_center()
                    .text_color(colors.muted_foreground)
                    .child("…")
            })
            .when(!loading, |this| {
                this.child(
                    MessageScroller::new(
                        "transcript",
                        self.scroller.clone(),
                        move |index, _, cx| {
                            let read_only = rows
                                .get(index)
                                .is_some_and(|row| crate::fork::inherited_row(row, inherited));
                            let ctx = WorkContext {
                                thread: thread.read(cx),
                                approvals: &approvals,
                                open: &open,
                                view: &view,
                                colors: &colors,
                                turn_changes: &turn_changes,
                                working,
                                read_only,
                                forkable: &forkable,
                                live_tasks: live_tasks.as_deref(),
                            };
                            let row = match rows.get(index) {
                                Some(Row::Status(since)) => turn_status::status_row(
                                    since
                                        .and_then(|since| ctx.thread.items.get(since))
                                        .map(|item| item.created_at.as_str()),
                                    stopping,
                                    retry.as_ref(),
                                    ctx.colors,
                                ),
                                Some(Row::Background) => match &background {
                                    Some(background) => turn_status::background_rows(
                                        background,
                                        ctx.thread.engine.clone(),
                                        ctx.colors,
                                    ),
                                    None => div().into_any_element(),
                                },
                                Some(Row::TaskWaits) => crate::delegations::task_waits(
                                    &ctx.thread.thread_id,
                                    ctx.colors,
                                    cx,
                                ),
                                Some(row @ (Row::ForkedFrom(_) | Row::ForkedInto(_))) => {
                                    crate::fork::divider(row, &fork_marks, ctx.colors)
                                }
                                Some(row) => render_row(row, live_work == Some(index), &ctx, cx),
                                None => div().into_any_element(),
                            };
                            div()
                                .when(read_only, |row| row.opacity(0.6))
                                .child(Lane::new().child(row))
                        },
                    )
                    .with_row_style(StyleRefinement::default().pb_4())
                    .with_bottom_fade(fade)
                    .with_jump_button_label(m::scroll_to_end())
                    .with_jump_button_renderer(|button| {
                        button
                            .icon(gpui_kit::component::IconName::ChevronDown)
                            .label(m::scroll_to_end())
                            .xsmall()
                            .shadow_sm()
                    })
                    .size_full(),
                )
                .children(minimap)
                .child(crate::delegations::report_shown_trays())
            })
    }
}

fn render_row(row: &Row, last: bool, ctx: &WorkContext, cx: &App) -> AnyElement {
    let thread = ctx.thread;
    match row {
        Row::Item(item) => thread
            .items
            .get(*item)
            .map(|item| render_item(item, ctx, cx))
            .unwrap_or_else(|| div().into_any_element()),
        Row::Work(indices) => {
            let entries: Vec<&ThreadItem> = indices
                .iter()
                .filter_map(|index| thread.items.get(*index))
                .collect();
            let live = last;
            work::work_group(&entries, live, ctx, cx)
        }
        Row::Streaming(streaming) => thread
            .streaming
            .get(*streaming)
            .map(|streaming| match streaming.kind {
                StreamKind::Reasoning => {
                    work::reasoning(&streaming.item_id, &streaming.text, true, ctx)
                }
                _ => render_text(&streaming.item_id, &streaming.text, false, ctx.colors),
            })
            .unwrap_or_else(|| div().into_any_element()),
        Row::StreamingNote(streaming, outcome) => thread
            .streaming
            .get(*streaming)
            .map(|streaming| {
                crate::delegations::task_note(
                    &streaming.item_id,
                    markdown(&streaming.item_id, &streaming.text, ctx.colors),
                    *outcome,
                    ctx.colors,
                )
            })
            .unwrap_or_else(|| div().into_any_element()),
        Row::Fold(fold) => render_fold(fold, ctx, cx),
        Row::Agents(indices) => render_agents(indices, ctx, cx),
        Row::Note(index, outcome) => match thread.items.get(*index) {
            Some(
                item @ ThreadItem {
                    content: ItemContent::AssistantMessage { text },
                    ..
                },
            ) => with_card(
                crate::delegations::task_note(
                    &item.id,
                    markdown(&item.id, text, ctx.colors),
                    *outcome,
                    ctx.colors,
                ),
                item,
                ctx,
                cx,
            ),
            _ => div().into_any_element(),
        },
        Row::Status(_)
        | Row::Background
        | Row::TaskWaits
        | Row::ForkedFrom(_)
        | Row::ForkedInto(_) => div().into_any_element(),
    }
}

fn render_agents(indices: &[usize], ctx: &WorkContext, cx: &App) -> AnyElement {
    let thread = ctx.thread;
    let items: Vec<&ThreadItem> = indices
        .iter()
        .filter_map(|index| thread.items.get(*index))
        .collect();
    let Some(first) = items.first() else {
        return div().into_any_element();
    };
    let key = agents_key(&first.id);
    let open = ctx.open.contains(&key);
    let workspace = crate::markdown::workspace(cx);
    let board = workspace
        .as_ref()
        .map(|workspace| crate::delegations::TaskBoard::new(workspace.read(cx)));
    let agents = items
        .iter()
        .map(|item| match &item.content {
            ItemContent::ToolCall { name, .. } if is_delegation(name) => (
                crate::delegations::delegation_state(item, board.as_ref()),
                crate::delegations::delegation_started(item, board.as_ref()).to_owned(),
            ),
            ItemContent::ToolCall {
                output,
                is_error,
                interrupted,
                ..
            } => (
                Some(subagent_state(output.is_some(), *is_error, *interrupted)),
                item.created_at.clone(),
            ),
            _ => (None, item.created_at.clone()),
        })
        .collect();
    let lines = open.then(|| {
        items
            .iter()
            .map(|item| agent_row(item, true, ctx, cx))
            .collect()
    });
    let provider = items
        .first()
        .map(|item| crate::subagents::thread_provider(&item.thread_id, cx))
        .unwrap_or_else(hoshi_core::default_provider);
    crate::subagents::agent_group(&key, &provider, agents, lines, ctx.toggle(&key), ctx.colors)
}

fn agent_row(item: &ThreadItem, grouped: bool, ctx: &WorkContext, cx: &App) -> AnyElement {
    match &item.content {
        ItemContent::ToolCall { name, .. } if is_delegation(name) => {
            crate::delegations::delegation_row(item, grouped, ctx.colors, cx)
        }
        _ => crate::subagents::subagent_row(
            item,
            grouped,
            &ctx.thread.streaming,
            ctx.thread.engine.clone(),
            ctx.view.clone(),
            ctx.colors,
            cx,
        ),
    }
}

/// The task list a running turn works through: the latest.
fn live_task_list(items: &[ThreadItem]) -> Option<String> {
    items
        .iter()
        .rfind(|item| matches!(item.content, ItemContent::TaskList { .. }))
        .map(|item| item.id.clone())
}

fn render_fold(fold: &Fold, ctx: &WorkContext, cx: &App) -> AnyElement {
    let key = fold.key(&ctx.thread.items);
    let open = ctx.open.contains(&key);
    FoldRow::new(SharedString::from(key.clone()), fold_label(fold))
        .open(open)
        .on_toggle(ctx.toggle(&key))
        .when(open, |row| {
            row.children(
                fold.rows
                    .iter()
                    .map(|inner| render_row(inner, false, ctx, cx)),
            )
        })
        .into_any_element()
}

fn fold_label(fold: &Fold) -> String {
    let duration = fold.duration.map(format_duration);
    match (fold.ending, fold.after, duration) {
        (Ending::Stopped, _, Some(duration)) => m::stopped_after(duration),
        (Ending::Stopped, _, None) => m::stopped_response(),
        (Ending::Restarted, _, Some(duration)) => m::cut_by_restart_after(duration),
        (Ending::Restarted, _, None) => m::cut_by_restart(),
        (_, After::Steer, Some(duration)) => m::continued_for(duration),
        (_, After::Steer, None) => m::continued(),
        (_, After::Start, Some(duration)) => m::worked_for(duration),
        (_, After::Start, None) => m::worked(),
    }
}

/// `message` under the line that says its turn runs on another model or effort, when it does.
fn with_model_switch(
    item: &ThreadItem,
    switch: Option<&hoshi_core::ModelSwitch>,
    colors: &Colors,
    cx: &App,
    message: AnyElement,
) -> AnyElement {
    let workspace = crate::markdown::workspace(cx);
    let text = switch.and_then(|switch| {
        request_rows::model_switch_text(
            switch,
            workspace.as_ref().map(|workspace| workspace.read(cx)),
        )
    });
    match text {
        Some(text) => div()
            .flex()
            .flex_col()
            .gap_4()
            .child(request_rows::model_switch_divider(item, text, colors))
            .child(message)
            .into_any_element(),
        None => message,
    }
}

fn render_item(item: &ThreadItem, ctx: &WorkContext, cx: &App) -> AnyElement {
    let colors = ctx.colors;
    match &item.content {
        ItemContent::UserMessage {
            resumed: Some(resumed),
            ..
        } => request_rows::resumed_turn(item, *resumed, colors),
        ItemContent::UserMessage {
            text,
            pull_request_update: Some(update),
            ..
        } => crate::pull_request_update::pull_request_update_row(
            &item.id,
            update,
            text,
            ctx.open.contains(&item.id),
            ctx.toggle(&item.id),
            colors,
        ),
        ItemContent::UserMessage {
            text,
            attachments,
            steer,
            rewind,
            dropped,
            model_switch,
            ..
        } => with_model_switch(
            item,
            model_switch.as_deref(),
            colors,
            cx,
            crate::user_message::user_message(
                &item.id,
                text,
                attachments,
                *steer,
                &item.created_at,
                rewind.as_ref().filter(|_| !ctx.read_only).map(|_| {
                    crate::rewind::rewind_button(
                        crate::rewind::RewindTarget {
                            thread_id: item.thread_id.clone(),
                            message_id: item.id.clone(),
                            text: text.clone(),
                            attachments: attachments.clone(),
                        },
                        ctx.working,
                        ctx.thread.engine.clone(),
                        ctx.view.clone(),
                        colors,
                    )
                }),
                (*dropped && !ctx.read_only).then(|| {
                    let view = ctx.view.clone();
                    let draft = crate::rewind::message_draft(text, attachments);
                    crate::user_message::not_sent_note(
                        &item.id,
                        move |cx| {
                            let draft = draft.clone();
                            view.update(cx, |_, cx| cx.emit(crate::rewind::Rewound(draft)))
                                .ok();
                        },
                        cx,
                    )
                }),
                cx,
            ),
        ),
        ItemContent::TaskList { tasks } => crate::task_list::task_list_row(
            &item.id,
            tasks,
            ctx.live_tasks == Some(item.id.as_str()),
            colors,
        ),
        ItemContent::Compaction {
            status,
            before_tokens,
            after_tokens,
            error,
        } => crate::compaction::compaction_row(
            &item.id,
            *status,
            crate::compaction::compaction_label(
                *status,
                *before_tokens,
                *after_tokens,
                error.as_deref(),
            ),
            colors,
        ),
        ItemContent::ToolCall { name, .. } if is_delegation(name) => {
            crate::delegations::delegation_row(item, false, colors, cx)
        }
        ItemContent::ToolCall {
            name,
            output: Some(_),
            is_error: false,
            ..
        } if is_html_render(name) => {
            crate::html_page::html_page_row(&item.id, &item.thread_id, ctx.thread.content(item))
        }
        ItemContent::AssistantMessage { text } => with_card(
            crate::fork::answer_actions(
                div()
                    .debug_selector({
                        let selector = format!("answer-{}", item.id);
                        move || selector.clone()
                    })
                    .child(render_text(&item.id, text, false, colors))
                    .into_any_element(),
                item,
                text,
                ctx,
            ),
            item,
            ctx,
            cx,
        ),
        ItemContent::ToolCall { name, .. } if crate::tool_calls::is_subagent(name) => {
            crate::subagents::subagent_row(
                item,
                false,
                &ctx.thread.streaming,
                ctx.thread.engine.clone(),
                ctx.view.clone(),
                colors,
                cx,
            )
        }
        ItemContent::Reasoning { .. } | ItemContent::ToolCall { .. } => work::entry(item, ctx, cx),
        ItemContent::TurnFailed { reason } => request_rows::failure_row(item, reason, ctx, cx),
        ItemContent::ModelRerouted { model, reason } => {
            request_rows::model_rerouted(item, model, reason.as_deref(), colors, cx)
        }
        ItemContent::ProviderRecovered {
            attempt,
            max_attempts,
        } => request_rows::provider_recovered(item, *attempt, *max_attempts, colors, cx),
        ItemContent::McpServerFailed { name, error } => {
            request_rows::mcp_server_failed(item, name, error, colors, cx)
        }
        ItemContent::HookStopped {
            event,
            source,
            reason,
        } => request_rows::hook_stopped(item, event, source.as_deref(), reason, colors),
        ItemContent::TurnInterrupted => request_rows::turn_interrupted(false, colors),
        ItemContent::TurnCut => request_rows::turn_interrupted(true, colors),
        ItemContent::ContextHandoff {
            sources, delivery, ..
        } => request_rows::context_handoff(item, sources, *delivery, colors, cx),
        ItemContent::ModelSwitched { from, to } => {
            let workspace = crate::markdown::workspace(cx);
            let switch = hoshi_core::ModelSwitch {
                from: from.clone(),
                to: to.clone(),
            };
            match request_rows::model_switch_text(
                &switch,
                workspace.as_ref().map(|workspace| workspace.read(cx)),
            ) {
                Some(text) => request_rows::model_switch_divider(item, text, colors),
                None => div().into_any_element(),
            }
        }
        ItemContent::BackgroundTaskEnded {
            tool_call_id,
            failed,
        } => turn_status::task_ended_row(
            tool_call_id.as_deref(),
            *failed,
            &ctx.thread.items,
            ctx.thread.cwd.as_deref(),
            ctx.view.clone(),
            colors,
        ),
        ItemContent::Approval { .. }
        | ItemContent::Questions { .. }
        | ItemContent::ProposedPlan { .. }
        | ItemContent::SecretRequest { .. } => request_rows::request_row(item, ctx, cx),
        _ => div().into_any_element(),
    }
}

fn markdown(id: &str, text: &str, colors: &Colors) -> TextView {
    crate::markdown::markdown(SharedString::from(id.to_owned()), text.to_owned(), colors)
}

fn render_text(id: &str, text: &str, muted: bool, colors: &Colors) -> AnyElement {
    div()
        .text_sm()
        .when(muted, |this| this.text_color(colors.muted_foreground))
        .when(!muted, |this| this.text_color(colors.foreground))
        .child(markdown(id, text, colors))
        .into_any_element()
}

#[cfg(test)]
mod ui_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod real_data_tests {
    use super::TranscriptView;
    use crate::state::ThreadState;
    use gpui_kit::component::theme::ThemeMode;
    use gpui_kit::{
        AppContext as _, Bounds, Point, TestAppContext, VisualTestContext, WindowBounds,
        WindowOptions, px, size,
    };
    use hoshi_core::ItemContent;
    use hoshi_engine::Engine;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// Reads a copy of the home's database; set `HOSHI_NATIVE_SAMPLE_THREAD` to the
    /// thread to open.
    #[gpui_kit::test]
    #[ignore]
    fn a_real_thread_opens_whole_with_every_call_open(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let thread_id = std::env::var("HOSHI_NATIVE_SAMPLE_THREAD").expect("a thread to open");
        let source = crate::home::resolve().unwrap();
        let home =
            std::env::temp_dir().join(format!("hoshi-native-sample-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        for file in ["state.sqlite", "state.sqlite-wal", "state.sqlite-shm"] {
            if source.join(file).exists() {
                std::fs::copy(source.join(file), home.join(file)).unwrap();
            }
        }
        let engine = Arc::new(Engine::open(&home, Arc::new(|_| {})).unwrap());

        let started = Instant::now();
        let page = engine.transcript(&thread_id, None).unwrap();
        let read = started.elapsed();
        cx.update(|cx| {
            gpui_kit::init(cx);
            hoshi_ui::theme::apply(ThemeMode::Dark, cx);
        });
        let (window, view, thread) = cx.update(|cx| {
            let thread = cx.new(|_| {
                let mut thread = ThreadState::new(&thread_id, page.items);
                thread.engine = Some(engine.clone());
                thread
            });
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1000.), px(800.)),
                })),
                ..Default::default()
            };
            let shown = thread.clone();
            let (window, view) = gpui_kit::open_window(options, cx, |_, cx| {
                cx.new(|cx| TranscriptView::new(shown, false, cx))
            })
            .unwrap();
            (window, view, thread)
        });
        let mut cx = VisualTestContext::from_window(window, cx);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let drawn = started.elapsed();
        eprintln!("page read in {read:?}, first frame at {drawn:?}");

        let ids: Vec<String> = thread.read_with(&cx, |thread, _| {
            thread.items.iter().map(|item| item.id.clone()).collect()
        });
        view.update(&mut cx, |view, cx| {
            for id in &ids {
                view.toggle(&format!("work:{id}"), cx);
                view.toggle(id, cx);
            }
        });
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            cx.run_until_parked();
            let left = thread.read_with(&cx, |thread, _| {
                thread
                    .items
                    .iter()
                    .filter(|item| crate::state::left_out(&item.content))
                    .filter(|item| !thread.whole.contains_key(&item.id))
                    .count()
            });
            if left == 0 || Instant::now() > deadline {
                eprintln!("{left} calls still unread");
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        for _ in 0..4 {
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }

        let calls: Vec<String> = thread.read_with(&cx, |thread, _| {
            thread
                .items
                .iter()
                .rev()
                .filter(|item| matches!(item.content, ItemContent::ToolCall { .. }))
                .take(3)
                .map(|item| item.id.clone())
                .collect()
        });
        for id in calls {
            let header: &'static str = Box::leak(format!("disclosure-{id}").into_boxed_str());
            let input: &'static str = Box::leak(format!("code-block-{id}:input").into_boxed_str());
            eprintln!(
                "{id}: header {:?}, input {:?}",
                cx.debug_bounds(header),
                cx.debug_bounds(input)
            );
        }
        engine.close_lingering_agents();
    }
}
