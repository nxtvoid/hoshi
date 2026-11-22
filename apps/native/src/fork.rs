//! Forks: a new thread that goes on from one of the agent's answers, the dividers that link
//! a fork and the thread it came from, and merging a fork back.

use crate::m;
use crate::markdown::CopyButton;
use crate::state::{ThreadState, Workspace};
use crate::transcript::Row;
use crate::work::WorkContext;
use gpui_kit::*;
use hoshi_core::{CommandError, Event, ForkPoint, ItemContent, ReadModel, Thread, ThreadItem};
use hoshi_ui::prelude::*;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Each fork up the chain from `thread_id` with the thread it came from, `thread_id` first.
pub fn lineage(model: &ReadModel, thread_id: &str) -> Vec<(String, String)> {
    let mut lineage: Vec<(String, String)> = Vec::new();
    let mut at = thread_id.to_owned();
    while let Some(origin) = model
        .thread(&at)
        .and_then(|thread| thread.forked_from.as_ref())
    {
        if lineage.iter().any(|(fork, _)| *fork == origin.thread_id) {
            break;
        }
        lineage.push((at, origin.thread_id.clone()));
        at = origin.thread_id.clone();
    }
    lineage
}

/// Tells the open thread where its inherited history comes from when the fork it shows was
/// opened before its creation reached the model.
pub fn follow_created(workspace: &Workspace, event: &Event, cx: &mut App) {
    let Event::ThreadCreated { thread } = event else {
        return;
    };
    let Some(open) = workspace
        .open
        .clone()
        .filter(|open| open.read(cx).thread_id == thread.id)
    else {
        return;
    };
    let lineage = lineage(&workspace.model, &thread.id);
    open.update(cx, |state, cx| state.set_lineage(lineage, cx));
}

fn title(thread: &Thread) -> String {
    if thread.title.is_empty() {
        m::untitled_thread()
    } else {
        thread.title.clone()
    }
}

/// What the transcript links to: the thread's forks and the threads its history came from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ForkMarks {
    /// Each fork, oldest first, with the item it goes on from.
    pub forks: Vec<(String, String)>,
    /// The title of each thread linked, by id.
    pub titles: HashMap<String, String>,
}

pub fn marks(thread: &ThreadState, cx: &App) -> ForkMarks {
    let Some(workspace) = crate::markdown::workspace(cx) else {
        return ForkMarks::default();
    };
    let model = &workspace.read(cx).model;
    let mut titles = HashMap::new();
    let forks = model
        .forks_of(&thread.thread_id)
        .into_iter()
        .filter_map(|fork| {
            let origin = fork.forked_from.as_ref()?;
            titles.insert(fork.id.clone(), title(fork));
            Some((fork.id.clone(), origin.turn.through_item_id.clone()))
        })
        .collect();
    for (_, source) in &thread.lineage {
        if let Some(source) = model.thread(source) {
            titles.insert(source.id.clone(), title(source));
        }
    }
    ForkMarks { forks, titles }
}

/// The first item a row shows, by its index in the thread's items.
pub fn first_index(row: &Row) -> Option<usize> {
    match row {
        Row::Item(index) | Row::Note(index, _) => Some(*index),
        Row::Work(indices) | Row::Agents(indices) => indices.first().copied(),
        Row::Fold(fold) => fold
            .rows
            .iter()
            .filter_map(first_index)
            .chain([fold.start])
            .min(),
        _ => None,
    }
}

fn is_divider(row: &Row) -> bool {
    matches!(row, Row::ForkedFrom(_) | Row::ForkedInto(_))
}

/// `rows` with "Forked from" above the thread's own items and each fork's inherited history,
/// and "Forked into" after the turn each fork goes on from.
pub fn with_dividers(mut rows: Vec<Row>, thread: &ThreadState, marks: &ForkMarks) -> Vec<Row> {
    let mut dividers: Vec<(usize, Row)> = thread
        .fork_boundaries()
        .into_iter()
        .filter(|(_, source)| marks.titles.contains_key(*source))
        .map(|(at, source)| (at, Row::ForkedFrom(source.to_owned())))
        .collect();
    for (fork, through) in &marks.forks {
        let through = thread.items[thread.inherited.min(thread.items.len())..]
            .iter()
            .position(|item| item.id == *through)
            .map(|at| at + thread.inherited);
        if let Some(through) = through {
            dividers.push((through + 1, Row::ForkedInto(fork.clone())));
        }
    }
    dividers.sort_by_key(|(at, _)| *at);
    for (at, divider) in dividers {
        let position = rows
            .iter()
            .position(|row| !is_divider(row) && first_index(row).is_none_or(|index| index >= at))
            .unwrap_or(rows.len());
        rows.insert(position, divider);
    }
    rows
}

/// Whether a row shows history the thread inherited.
pub fn inherited_row(row: &Row, inherited: usize) -> bool {
    first_index(row).is_some_and(|index| index < inherited)
}

/// The thread's own answers a fork can start from, all but the running turn's: each turn's
/// last message, when no work followed it. Messages between the agent's steps are not answers.
pub fn forkable(items: &[ThreadItem], inherited: usize, working: bool) -> HashSet<String> {
    let own = inherited.min(items.len());
    let end = if working {
        items[own..]
            .iter()
            .rposition(|item| matches!(item.content, ItemContent::UserMessage { steer: false, .. }))
            .map_or(own, |at| own + at)
    } else {
        items.len()
    };
    items[own..end]
        .split(|item| matches!(item.content, ItemContent::UserMessage { steer: false, .. }))
        .filter_map(|turn| {
            let at = turn
                .iter()
                .rposition(|item| matches!(item.content, ItemContent::AssistantMessage { .. }))?;
            let worked_after = turn[at + 1..].iter().any(|item| {
                matches!(
                    item.content,
                    ItemContent::ToolCall { .. } | ItemContent::Reasoning { .. }
                )
            });
            (!worked_after).then(|| turn[at].id.clone())
        })
        .collect()
}

/// [`forkable`], read again only as items come or the agent starts or stops.
#[derive(Default)]
pub struct Forkable {
    read_at: Option<(usize, usize, bool, bool)>,
    pub ids: Rc<HashSet<String>>,
}

impl Forkable {
    /// None for a transcript that is no listed thread, such as a subagent's.
    pub fn refresh(&mut self, thread: &ThreadState, working: bool, cx: &App) {
        let listed = crate::markdown::workspace(cx)
            .is_none_or(|workspace| workspace.read(cx).model.thread(&thread.thread_id).is_some());
        let at = (thread.items.len(), thread.inherited, working, listed);
        if self.read_at == Some(at) {
            return;
        }
        self.read_at = Some(at);
        self.ids = Rc::new(if listed {
            forkable(&thread.items, thread.inherited, working)
        } else {
            HashSet::new()
        });
    }
}

/// A divider that names a thread and opens it.
pub fn divider(row: &Row, marks: &ForkMarks, colors: &Colors) -> AnyElement {
    let (kind, label, target) = match row {
        Row::ForkedFrom(source) => ("forked-from", m::forked_from(), source),
        Row::ForkedInto(fork) => ("forked-into", m::forked_into(), fork),
        _ => return div().into_any_element(),
    };
    let title = marks.titles.get(target).cloned().unwrap_or_default();
    let selector = format!("{kind}-{target}");
    let link = format!("{kind}-link-{target}");
    let target = target.clone();
    let rule = || div().h_px().flex_1().bg(colors.border);
    div()
        .debug_selector(move || selector.clone())
        .flex()
        .items_center()
        .gap_3()
        .text_xs()
        .text_color(colors.muted_foreground)
        .child(rule())
        .child(
            div()
                .flex()
                .min_w_0()
                .items_center()
                .gap_1()
                .child(div().flex_none().child(label))
                .child(
                    div()
                        .id(SharedString::from(link.clone()))
                        .debug_selector(move || link.clone())
                        .min_w_0()
                        .truncate()
                        .text_color(colors.foreground)
                        .cursor_pointer()
                        .hover(|style| style.underline())
                        .press_selects_nothing()
                        .on_click(move |_, _, cx| open_thread(&target, cx))
                        .child(title),
                ),
        )
        .child(rule())
        .into_any_element()
}

fn open_thread(thread_id: &str, cx: &mut App) {
    if let Some(workspace) = crate::markdown::workspace(cx) {
        workspace.update(cx, |workspace, cx| workspace.open_thread(thread_id, cx));
    }
}

/// An answer with, on hover, a button that copies it and one that forks from it.
pub fn answer_actions(
    answer: AnyElement,
    item: &ThreadItem,
    text: &str,
    ctx: &WorkContext,
) -> AnyElement {
    if ctx.read_only || !ctx.forkable.contains(&item.id) {
        return answer;
    }
    let group = SharedString::from(format!("answer-{}", item.id));
    let fork = {
        let selector = format!("fork-{}", item.id);
        let (thread_id, item_id) = (ctx.thread.thread_id.clone(), item.id.clone());
        div().debug_selector(move || selector.clone()).child(
            Button::new(SharedString::from(format!("fork-{}", item.id)))
                .icon(IconName::GitFork)
                .size(ButtonSize::IconXs)
                .variant(ButtonVariant::Ghost)
                .tooltip(Tooltip::new(m::fork_from_response()))
                .on_click(move |_, window, cx| {
                    if let Some(workspace) = crate::markdown::workspace(cx) {
                        let point = ForkPoint::Turn {
                            item_id: item_id.clone(),
                        };
                        fork_thread(&workspace, thread_id.clone(), point, window, cx);
                    }
                }),
        )
    };
    div()
        .group(group.clone())
        .flex()
        .flex_col()
        .child(answer)
        .child(
            div()
                .flex()
                .items_center()
                .gap_0p5()
                .h_6()
                .mt_1()
                .ml(px(-6.))
                .opacity(0.)
                .group_hover(group, |actions| actions.opacity(1.))
                .child(CopyButton::new(
                    SharedString::from(format!("copy-{}", item.id)),
                    text.to_owned().into(),
                    m::copy_message().into(),
                ))
                .child(fork),
        )
        .into_any_element()
}

pub fn fork_failure(error: &CommandError) -> String {
    match error {
        CommandError::NoStableTurn { .. } => m::fork_no_stable_turn(),
        CommandError::TurnNotFound { .. } => m::fork_turn_not_found(),
        CommandError::TurnNotForkable { .. } => m::fork_turn_running(),
        _ => m::error_generic(),
    }
}

pub fn merge_back_failure(error: &CommandError) -> String {
    match error {
        CommandError::NotAFork { .. } => m::merge_back_not_a_fork(),
        CommandError::NoStableTurn { .. } => m::merge_back_needs_turn(),
        CommandError::MergeBackTurnNotFinished { .. } => m::merge_back_turn_not_finished(),
        CommandError::MergeBackTargetBusy { .. } => m::merge_back_target_busy(),
        CommandError::MergeBackPending { .. } => m::merge_back_pending(),
        CommandError::WorktreeShared { .. } => m::merge_back_worktree_shared(),
        _ => m::error_generic(),
    }
}

/// Forks `thread_id` from `point` into a new thread and opens it.
pub fn fork_thread(
    workspace: &Entity<Workspace>,
    thread_id: String,
    point: ForkPoint,
    window: &mut Window,
    cx: &mut App,
) {
    let engine = workspace.read(cx).engine.clone();
    let workspace = workspace.downgrade();
    let fork_id = uuid::Uuid::new_v4().to_string();
    let from_answer = matches!(point, ForkPoint::Turn { .. });
    window
        .spawn(cx, async move |cx| {
            let id = fork_id.clone();
            let forked = cx
                .background_spawn(async move { engine.fork_thread(&thread_id, &id, &point) })
                .await;
            cx.update(|window, cx| match forked {
                Ok(()) => {
                    workspace
                        .update(cx, |workspace, cx| workspace.open_thread(&fork_id, cx))
                        .ok();
                }
                Err(error) => {
                    let title = if from_answer {
                        m::fork_response_failed()
                    } else {
                        m::fork_thread_failed()
                    };
                    let notice = ConfirmRequest::new(title, m::dialog_ok())
                        .description(fork_failure(&error))
                        .tone(ConfirmTone::Warning);
                    DialogLayer::confirm(notice, window, cx).detach();
                }
            })
            .ok();
        })
        .detach();
}

/// The open fork, as the merge-back button shows it.
struct Shown {
    fork_id: String,
    parent_id: String,
    parent_title: String,
    /// What the fork was checked at; a new check runs when it changes.
    key: String,
    /// A turn of the fork completed, so it can merge back.
    ready: bool,
}

/// "Merge back" in the header of an open fork.
pub struct MergeBack {
    workspace: Entity<Workspace>,
    shown: Option<Shown>,
    merging: bool,
    _observe: Subscription,
}

impl MergeBack {
    pub fn new(workspace: Entity<Workspace>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&workspace, |this, _, cx| this.follow(cx));
        let mut this = Self {
            workspace,
            shown: None,
            merging: false,
            _observe: observe,
        };
        this.follow(cx);
        this
    }

    pub fn is_shown(&self) -> bool {
        self.shown.is_some()
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        let workspace = self.workspace.read(cx);
        let fork = workspace
            .open_thread_id(cx)
            .and_then(|id| workspace.model.thread(&id))
            .and_then(|fork| {
                let parent = workspace
                    .model
                    .thread(&fork.forked_from.as_ref()?.thread_id)?;
                Some((fork, parent))
            });
        let Some((fork, parent)) = fork else {
            if self.shown.take().is_some() {
                cx.notify();
            }
            return;
        };
        let key = format!(
            "{}|{:?}|{}|{:?}|{:?}",
            fork.id, fork.status, fork.updated_at, fork.active_turn_id, fork.last_turn_ended_at
        );
        let parent_title = title(parent);
        if let Some(shown) = self.shown.as_mut().filter(|shown| shown.key == key) {
            if shown.parent_title != parent_title {
                shown.parent_title = parent_title;
                cx.notify();
            }
            return;
        }
        let ready = self
            .shown
            .as_ref()
            .is_some_and(|shown| shown.fork_id == fork.id && shown.ready);
        let engine = workspace.engine.clone();
        let fork_id = fork.id.clone();
        self.shown = Some(Shown {
            fork_id: fork.id.clone(),
            parent_id: parent.id.clone(),
            parent_title,
            key: key.clone(),
            ready,
        });
        cx.spawn(async move |this, cx| {
            let ready = cx
                .background_spawn(async move {
                    engine
                        .turn_point(&fork_id, &ForkPoint::LatestStable)
                        .is_ok()
                })
                .await;
            this.update(cx, |this, cx| {
                if let Some(shown) = this.shown.as_mut().filter(|shown| shown.key == key) {
                    shown.ready = ready;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(shown) = self.shown.as_ref().filter(|shown| shown.ready) else {
            return;
        };
        if self.merging {
            return;
        }
        self.merging = true;
        let (fork_id, parent_id) = (shown.fork_id.clone(), shown.parent_id.clone());
        let engine = self.workspace.read(cx).engine.clone();
        let workspace = self.workspace.downgrade();
        cx.spawn_in(window, async move |this, cx| {
            let merged = cx
                .background_spawn(
                    async move { engine.merge_back(&fork_id, &ForkPoint::LatestStable) },
                )
                .await;
            this.update(cx, |this, cx| {
                this.merging = false;
                cx.notify();
            })
            .ok();
            match merged {
                Ok(()) => {
                    workspace
                        .update(cx, |workspace, cx| workspace.open_thread(&parent_id, cx))
                        .ok();
                }
                Err(error) => {
                    cx.update(|window, cx| {
                        let notice = ConfirmRequest::new(m::merge_back_failed(), m::dialog_ok())
                            .description(merge_back_failure(&error))
                            .tone(ConfirmTone::Warning);
                        DialogLayer::confirm(notice, window, cx).detach();
                    })
                    .ok();
                }
            }
        })
        .detach();
        cx.notify();
    }
}

impl Render for MergeBack {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(shown) = &self.shown else {
            return div().into_any_element();
        };
        let tooltip = if shown.ready {
            m::merge_back_into(&shown.parent_title)
        } else {
            m::merge_back_needs_turn()
        };
        div()
            .debug_selector(|| "merge-back".into())
            .child(
                Button::new("merge-back")
                    .icon(IconName::GitMerge)
                    .label(m::merge_back())
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Outline)
                    .disabled(!shown.ready)
                    .loading(self.merging)
                    .tooltip(Tooltip::new(tooltip).side(Side::Bottom))
                    .on_click(cx.listener(|this, _, window, cx| this.merge(window, cx))),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod ui_tests;
