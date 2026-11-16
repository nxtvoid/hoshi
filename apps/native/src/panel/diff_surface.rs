use super::changes::{ChangedFiles, Files, Source, totals};
use super::surfaces::DiffShown;
use crate::m;
use crate::state::Workspace;
use gpui_kit::*;
use hoshi_core::DiffScope;
use hoshi_ui::prelude::*;
use std::time::Duration;

/// While a turn adds items, the checkout is read again at most this often.
const ACTIVITY: Duration = Duration::from_secs(1);

/// What the diff surface should reload on.
#[derive(Debug, PartialEq, Eq)]
pub enum Reload {
    Now,
    /// After [`ACTIVITY`], then [`ChangeKey::catch_up`].
    Later,
    No,
}

/// When a thread's checkout may have changed: at once when its branch, worktree or status
/// changes, as when a turn ends, and at most once a second while its agent works.
#[derive(Debug, Default)]
pub struct ChangeKey {
    checkout: Option<String>,
    activity: String,
    behind: bool,
}

impl ChangeKey {
    pub fn new(checkout: Option<String>, activity: &str) -> Self {
        Self {
            checkout,
            activity: activity.to_owned(),
            behind: false,
        }
    }

    pub fn observe(&mut self, checkout: Option<String>, activity: &str) -> Reload {
        if checkout != self.checkout {
            *self = Self::new(checkout, activity);
            return Reload::Now;
        }
        if activity != self.activity && !self.behind {
            self.behind = true;
            return Reload::Later;
        }
        Reload::No
    }

    /// The wait is over: whether the activity moved meanwhile, so it reloads.
    pub fn catch_up(&mut self, activity: &str) -> bool {
        self.behind = false;
        if activity == self.activity {
            return false;
        }
        activity.clone_into(&mut self.activity);
        true
    }
}

/// The checkout part and the activity part of a thread's [`ChangeKey`].
fn checkout_of(workspace: &Workspace, thread_id: &str) -> (Option<String>, String) {
    workspace
        .model
        .thread(thread_id)
        .map(|thread| {
            (
                Some(format!(
                    "{:?}|{:?}|{:?}",
                    thread.branch, thread.worktree_path, thread.status
                )),
                thread.updated_at.clone(),
            )
        })
        .unwrap_or_default()
}

pub enum DiffSurfaceEvent {
    /// The user picked what to compare.
    Shown(DiffShown),
}

impl EventEmitter<DiffSurfaceEvent> for DiffSurface {}

/// What the thread's working tree changed: uncommitted, since the branch began, or in a turn.
pub struct DiffSurface {
    workspace: Entity<Workspace>,
    thread_id: String,
    shown: DiffShown,
    turn_id: Option<String>,
    files: Entity<ChangedFiles>,
    key: ChangeKey,
    wait: Option<Task<()>>,
    _subscriptions: [Subscription; 2],
}

fn source(shown: DiffShown, turn_id: Option<&str>) -> Source {
    match (shown, turn_id) {
        (DiffShown::Turn, Some(turn_id)) => Source::Turn(turn_id.to_owned()),
        (DiffShown::Scope(scope), _) => Source::Thread(scope),
        (DiffShown::Turn, None) => Source::Thread(DiffScope::Uncommitted),
    }
}

impl DiffSurface {
    pub fn new(
        workspace: Entity<Workspace>,
        thread_id: String,
        shown: DiffShown,
        turn_id: Option<String>,
        focus_path: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let engine = workspace.read(cx).engine.clone();
        let (checkout, activity) = checkout_of(workspace.read(cx), &thread_id);
        let files = cx.new(|cx| {
            ChangedFiles::new(
                engine,
                thread_id.clone(),
                source(shown, turn_id.as_deref()),
                focus_path,
                cx,
            )
        });
        let subscriptions = [
            cx.observe(&workspace, Self::workspace_changed),
            cx.observe(&files, |_, _, cx| cx.notify()),
        ];
        Self {
            workspace,
            thread_id,
            shown,
            turn_id,
            files,
            key: ChangeKey::new(checkout, &activity),
            wait: None,
            _subscriptions: subscriptions,
        }
    }

    #[cfg(test)]
    pub fn files(&self) -> &Entity<ChangedFiles> {
        &self.files
    }

    /// Shows what the panel's surface says, as when a turn's changes open it.
    pub fn show(
        &mut self,
        shown: DiffShown,
        turn_id: Option<String>,
        focus_path: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.shown = shown;
        self.turn_id = turn_id;
        let source = source(shown, self.turn_id.as_deref());
        self.files
            .update(cx, |files, cx| files.show(source, focus_path, cx));
        cx.notify();
    }

    fn workspace_changed(&mut self, workspace: Entity<Workspace>, cx: &mut Context<Self>) {
        let (checkout, activity) = checkout_of(workspace.read(cx), &self.thread_id);
        match self.key.observe(checkout, &activity) {
            Reload::Now => self.reload(cx),
            Reload::Later => {
                self.wait = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(ACTIVITY).await;
                    this.update(cx, |this, cx| {
                        let (_, activity) = checkout_of(this.workspace.read(cx), &this.thread_id);
                        if this.key.catch_up(&activity) {
                            this.reload(cx);
                        }
                    })
                    .ok();
                }));
            }
            Reload::No => {}
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.files.update(cx, |files, cx| files.reload(cx));
    }

    fn pick(&mut self, shown: DiffShown, cx: &mut Context<Self>) {
        if shown == self.shown {
            return;
        }
        cx.emit(DiffSurfaceEvent::Shown(shown));
        self.show(shown, self.turn_id.clone(), None, cx);
    }
}

impl Render for DiffSurface {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let mut scopes = vec![
            (
                DiffShown::Scope(DiffScope::Uncommitted),
                m::diff_scope_uncommitted(),
            ),
            (DiffShown::Scope(DiffScope::Branch), m::diff_scope_branch()),
        ];
        if self.turn_id.is_some() {
            scopes.push((DiffShown::Turn, m::diff_scope_turn()));
        }
        let summary = match self.files.read(cx).files() {
            Files::Ready(files) => Some((files.len(), totals(files))),
            _ => None,
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1p5()
                    .border_b_1()
                    .border_color(colors.border)
                    .children(
                        scopes
                            .into_iter()
                            .enumerate()
                            .map(|(index, (shown, label))| {
                                Button::new(("diff-scope", index))
                                    .label(label)
                                    .size(ButtonSize::Xs)
                                    .variant(if shown == self.shown {
                                        ButtonVariant::Outline
                                    } else {
                                        ButtonVariant::Ghost
                                    })
                                    .on_click(
                                        cx.listener(move |this, _, _, cx| this.pick(shown, cx)),
                                    )
                            }),
                    )
                    .child(div().flex_1())
                    .when_some(summary, |this, (count, (additions, deletions))| {
                        this.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_xs()
                                .text_color(colors.muted_foreground)
                                .child(m::diff_file_count(count))
                                .child(DiffStat::new(additions, deletions)),
                        )
                    })
                    .child(
                        Button::new("diff-refresh")
                            .icon(IconName::RefreshCw)
                            .size(ButtonSize::IconSm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip(m::refresh())
                            .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                    ),
            )
            .child(self.files.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::{ChangeKey, Reload};

    fn checkout(text: &str) -> Option<String> {
        Some(text.to_owned())
    }

    #[test]
    fn a_new_branch_or_status_reloads_at_once() {
        let mut key = ChangeKey::new(checkout("main|working"), "t1");
        assert_eq!(key.observe(checkout("main|working"), "t1"), Reload::No);
        assert_eq!(key.observe(checkout("main|idle"), "t2"), Reload::Now);
        assert_eq!(key.observe(checkout("main|idle"), "t2"), Reload::No);
    }

    #[test]
    fn activity_reloads_once_per_wait_with_the_latest() {
        let mut key = ChangeKey::new(checkout("main"), "t1");
        assert_eq!(key.observe(checkout("main"), "t2"), Reload::Later);
        assert_eq!(key.observe(checkout("main"), "t3"), Reload::No);
        assert!(key.catch_up("t3"));
        assert_eq!(key.observe(checkout("main"), "t3"), Reload::No);
        assert_eq!(key.observe(checkout("main"), "t4"), Reload::Later);
        assert!(key.catch_up("t4"));
        assert!(!key.catch_up("t4"));
    }
}
