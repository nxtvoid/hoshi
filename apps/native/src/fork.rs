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
mod tests {
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
}

#[cfg(test)]
mod ui_tests {
    use crate::HoshiWindow;
    use crate::state::{self, Workspace};
    use gpui_kit::component::theme::ThemeMode;
    use gpui_kit::{
        AppContext as _, Bounds, Entity, Modifiers, MouseButton, Point, TestAppContext,
        VisualTestContext, WindowBounds, WindowOptions, px, size,
    };
    use hoshi_core::{
        AccessMode, Command, Event, ForkPoint, ItemContent, PendingHandoff, Rewind, ThreadItem,
        ThreadWorkspace,
    };
    use hoshi_engine::Engine;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    struct Harness {
        cx: VisualTestContext,
        window: Entity<HoshiWindow>,
        workspace: Entity<Workspace>,
        engine: Arc<Engine>,
        home: PathBuf,
    }

    fn user(thread_id: &str, id: &str) -> ThreadItem {
        ThreadItem {
            id: id.into(),
            thread_id: thread_id.into(),
            created_at: "2026-11-15T10:00:00Z".into(),
            content: ItemContent::UserMessage {
                text: format!("Question {id}"),
                attachments: Vec::new(),
                steer: false,
                task_result: None,
                pull_request_update: None,
                resumed: None,
                rewind: Some(Rewind::Start),
                dropped: false,
                model_switch: None,
            },
        }
    }

    fn answer(thread_id: &str, id: &str) -> ThreadItem {
        ThreadItem {
            id: id.into(),
            thread_id: thread_id.into(),
            created_at: "2026-11-15T10:00:01Z".into(),
            content: ItemContent::AssistantMessage {
                text: format!("Answer {id}"),
            },
        }
    }

    /// Writes items as a finished turn would, through a second connection to the store.
    fn record(home: &Path, items: Vec<ThreadItem>) {
        let mut store = hoshi_store::Store::open(&home.join("state.sqlite")).unwrap();
        let events: Vec<Event> = items
            .into_iter()
            .map(|item| Event::ItemAdded { item })
            .collect();
        store.append(&events, "2026-11-15T10:00:02Z").unwrap();
    }

    fn redraw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        for _ in 0..3 {
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.run_until_parked();
        }
    }

    fn wait_for(h: &mut Harness, what: &str, done: impl Fn(&mut Harness) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            redraw(&mut h.cx);
            if done(h) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn shows(h: &mut Harness, selector: String) -> bool {
        let selector: &'static str = Box::leak(selector.into_boxed_str());
        h.cx.debug_bounds(selector).is_some()
    }

    fn bounds(h: &mut Harness, selector: String) -> Bounds<gpui_kit::Pixels> {
        let selector: &'static str = Box::leak(selector.into_boxed_str());
        h.cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn"))
    }

    fn click(h: &mut Harness, selector: String) {
        let at = bounds(h, selector).center();
        h.cx.simulate_click(at, Modifiers::none());
        redraw(&mut h.cx);
    }

    fn open_id(h: &mut Harness) -> Option<String> {
        h.workspace
            .read_with(&h.cx, |workspace, cx| workspace.open_thread_id(cx))
    }

    fn open(h: &mut Harness, thread_id: &str) {
        h.workspace.update(&mut h.cx, |workspace, cx| {
            workspace.open_thread(thread_id, cx)
        });
        redraw(&mut h.cx);
    }

    /// Thread `t`, "Parser quotes", with two answered turns, open in a window.
    fn harness(cx: &mut TestAppContext) -> Harness {
        cx.executor().allow_parking();
        let dir = std::env::temp_dir().join(format!("hoshi-fork-{}", uuid::Uuid::new_v4()));
        let home = dir.join("home");
        std::fs::create_dir_all(dir.join("project")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let (notify, notifications) = state::channel();
        let engine = Arc::new(Engine::open(&home, notify).unwrap());
        engine
            .dispatch(Command::ProjectCreate {
                project_id: "p".into(),
                title: "project".into(),
                workspace_root: dir.join("project").to_string_lossy().into_owned(),
            })
            .unwrap();
        engine
            .dispatch(Command::ThreadCreate {
                thread_id: "t".into(),
                project_id: Some("p".into()),
                title: "Parser quotes".into(),
                workspace: ThreadWorkspace::Local,
                provider: hoshi_core::default_provider(),
                model: None,
                effort: None,
                fast_mode: false,
                ultracode: false,
                access_mode: AccessMode::default(),
                plan_mode: false,
            })
            .unwrap();
        record(
            &home,
            vec![
                user("t", "u1"),
                answer("t", "a1"),
                user("t", "u2"),
                answer("t", "a2"),
            ],
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            hoshi_ui::DialogLayer::init(cx);
            crate::terminal::init(cx);
            crate::sidebar::init(cx);
            crate::palette::init(cx);
            crate::thread_header::init(cx);
            crate::markdown::init(engine.clone(), home.clone(), cx);
            hoshi_ui::theme::apply(ThemeMode::Light, cx);
        });
        let shared = engine.clone();
        let workspace = cx.update(|cx| cx.new(|cx| Workspace::new(shared, notifications, cx)));
        let (window, shown) = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1200.), px(800.)),
                })),
                ..Default::default()
            };
            let mut shown = None;
            let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
                let view = cx.new(|cx| HoshiWindow::new(workspace.clone(), window, cx));
                shown = Some(view.clone());
                view
            })
            .unwrap();
            (window, shown.unwrap())
        });
        let cx = VisualTestContext::from_window(window, cx);
        let mut h = Harness {
            cx,
            window: shown,
            workspace,
            engine,
            home,
        };
        open(&mut h, "t");
        wait_for(&mut h, "the transcript", |h| shows(h, "fork-a2".into()));
        h
    }

    fn fork_of(h: &mut Harness, thread_id: &str) -> Option<String> {
        h.workspace.read_with(&h.cx, |workspace, _| {
            workspace
                .model
                .forks_of(thread_id)
                .first()
                .map(|fork| fork.id.clone())
        })
    }

    #[gpui_kit::test]
    fn an_answer_forks_into_a_thread_that_shows_its_history_and_links_back(
        cx: &mut TestAppContext,
    ) {
        let mut h = harness(cx);
        assert!(shows(&mut h, "rewind-u1".into()));
        click(&mut h, "fork-a1".into());
        wait_for(&mut h, "the fork to open", |h| {
            open_id(h).is_some_and(|open| open != "t")
        });
        let fork = open_id(&mut h).unwrap();
        assert_eq!(fork_of(&mut h, "t").as_deref(), Some(fork.as_str()));
        let title = h.workspace.read_with(&h.cx, |workspace, _| {
            workspace.model.thread(&fork).unwrap().title.clone()
        });
        assert_eq!(title, "Parser quotes fork");

        wait_for(&mut h, "the inherited history", |h| {
            shows(h, "forked-from-t".into())
        });
        assert!(shows(&mut h, "collapse-u1".into()));
        assert!(!shows(&mut h, "collapse-u2".into()), "only up to the fork");
        assert!(
            !shows(&mut h, "rewind-u1".into()),
            "inherited history is read-only"
        );
        assert!(!shows(&mut h, "fork-a1".into()));
        let above = bounds(&mut h, "collapse-u1".into());
        let divider = bounds(&mut h, "forked-from-t".into());
        assert!(
            divider.top() > above.bottom(),
            "{divider:?} below {above:?}"
        );

        click(&mut h, "forked-from-link-t".into());
        wait_for(&mut h, "the source to open", |h| {
            open_id(h).as_deref() == Some("t")
        });
        wait_for(&mut h, "the fork's divider", |h| {
            shows(h, format!("forked-into-{fork}"))
        });
        let into = bounds(&mut h, format!("forked-into-{fork}"));
        let answer = bounds(&mut h, "fork-a1".into());
        let next = bounds(&mut h, "collapse-u2".into());
        assert!(
            answer.bottom() <= into.top() && into.bottom() <= next.top(),
            "{answer:?} / {into:?} / {next:?}"
        );

        click(&mut h, format!("forked-into-link-{fork}"));
        wait_for(&mut h, "the fork to open again", |h| {
            open_id(h).as_deref() == Some(fork.as_str())
        });
    }

    fn merge_back_ready(h: &mut Harness) -> bool {
        let merge_back = h
            .window
            .read_with(&h.cx, |window, _| window.merge_back.clone());
        merge_back.read_with(&h.cx, |merge_back, _| {
            merge_back.shown.as_ref().is_some_and(|shown| shown.ready)
        })
    }

    #[gpui_kit::test]
    fn merge_back_waits_for_a_turn_of_the_fork_then_opens_the_thread_it_came_from(
        cx: &mut TestAppContext,
    ) {
        let mut h = harness(cx);
        assert!(!shows(&mut h, "merge-back".into()), "not a fork");
        h.engine
            .fork_thread("t", "f", &ForkPoint::LatestStable)
            .unwrap();
        wait_for(&mut h, "the fork", |h| fork_of(h, "t").is_some());
        open(&mut h, "f");
        wait_for(&mut h, "the merge-back button", |h| {
            shows(h, "merge-back".into())
        });
        assert!(!merge_back_ready(&mut h));
        click(&mut h, "merge-back".into());
        assert_eq!(open_id(&mut h).as_deref(), Some("f"));

        record(&h.home, vec![user("f", "u3"), answer("f", "a3")]);
        let merge_back = h
            .window
            .read_with(&h.cx, |window, _| window.merge_back.clone());
        merge_back.update(&mut h.cx, |merge_back, cx| {
            merge_back.shown = None;
            merge_back.follow(cx);
        });
        wait_for(&mut h, "the fork's turn to count", merge_back_ready);
        click(&mut h, "merge-back".into());
        wait_for(&mut h, "the source to open", |h| {
            open_id(h).as_deref() == Some("t")
        });
        let pending = h.workspace.read_with(&h.cx, |workspace, _| {
            workspace
                .model
                .thread("t")
                .unwrap()
                .pending_handoffs
                .clone()
        });
        assert!(
            pending.iter().any(|handoff| matches!(
                handoff,
                PendingHandoff::MergeBack { fork_thread_id, .. } if fork_thread_id == "f"
            )),
            "{pending:?}"
        );

        h.engine
            .fork_thread("t", "g", &ForkPoint::LatestStable)
            .unwrap();
        record(&h.home, vec![user("g", "u4"), answer("g", "a4")]);
        open(&mut h, "g");
        wait_for(&mut h, "the second fork's turn to count", merge_back_ready);
        click(&mut h, "merge-back".into());
        wait_for(&mut h, "the refusal", |h| {
            shows(h, "hoshi-dialog-popup-0".into())
        });
        assert_eq!(open_id(&mut h).as_deref(), Some("g"));
    }

    #[gpui_kit::test]
    fn a_subagent_transcript_offers_no_fork(cx: &mut TestAppContext) {
        let mut h = harness(cx);
        h.cx.update(|_, cx| {
            let mut forkable = super::Forkable::default();
            let subagent = crate::state::ThreadState::new("call", vec![answer("call", "x")]);
            forkable.refresh(&subagent, false, cx);
            assert!(forkable.ids.is_empty());
            let thread = crate::state::ThreadState::new("t", vec![answer("t", "x")]);
            forkable.refresh(&thread, false, cx);
            assert!(forkable.ids.contains("x"));
        });
    }

    #[gpui_kit::test]
    fn the_thread_menu_forks_from_the_latest_finished_turn(cx: &mut TestAppContext) {
        let mut h = harness(cx);
        let row = bounds(&mut h, "thread-t".into()).center();
        h.cx.simulate_mouse_down(row, MouseButton::Right, Modifiers::none());
        h.cx.simulate_mouse_up(row, MouseButton::Right, Modifiers::none());
        redraw(&mut h.cx);
        click(&mut h, "menu-item-thread-fork".into());
        wait_for(&mut h, "the fork to open", |h| {
            open_id(h).is_some_and(|open| open != "t")
        });
        let fork = open_id(&mut h).unwrap();
        let through = h.workspace.read_with(&h.cx, |workspace, _| {
            let thread = workspace.model.thread(&fork).unwrap();
            thread
                .forked_from
                .as_ref()
                .unwrap()
                .turn
                .through_item_id
                .clone()
        });
        assert_eq!(through, "a2");
        wait_for(&mut h, "the whole inherited history", |h| {
            shows(h, "collapse-u2".into()) && shows(h, "forked-from-t".into())
        });
    }
}
