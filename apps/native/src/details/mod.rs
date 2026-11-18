pub(crate) mod agents;
mod commit;
mod git;
pub(crate) mod prefs;
mod rows;
mod servers;
#[cfg(test)]
mod tests;
mod usage;

use crate::m;
use crate::sections::relative_time;
use crate::state::Workspace;
use crate::usage::limits::{SectionState, section_state};
pub use agents::track_pending_tasks;
use agents::{AgentKind, AgentState, ThreadAgent, busy_threads, subagent_calls, thread_agents};
use chrono::Utc;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::*;
use hoshi_core::{
    ChangesSummary, CommandError, Editor, GitAction, GitActionRequest, GitActionResult, GitStatus,
    LocalServer, PullRequestState, Thread, ThreadItem, ThreadStatus, UsageLimits,
};
use hoshi_ui::prelude::*;
use prefs::{Prefs, editor_label, open_shortcut, open_target};
use rows::{
    CARD_CLEARANCE, CARD_GAP, CARD_WIDTH, INLINE_MIN_WIDTH, diff_count, editor_icon, error_line,
    icon, row, row_static, section_label, separator, split_arrow, split_primary, split_row,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long a finished action's result stays in place of the button.
const DONE_FOR: Duration = Duration::from_secs(10);
/// While a turn adds items, the checkout is read again at most this often.
const ACTIVITY_EVERY: Duration = Duration::from_secs(1);

/// What a row of the card asks the window to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Diff,
    PullRequest,
}

/// What the card is about: the open thread, or the project a new thread will start in.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Subject {
    Thread(String),
    Draft(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetailsEvent {
    ShowSurface {
        thread_id: String,
        surface: Surface,
    },
    /// Scroll the transcript to the item, as a subagent's row asks.
    ShowItem {
        thread_id: String,
        item_id: String,
    },
}

/// When the open thread's checkout was last read, and as what it stood.
struct Freshness {
    thread_id: String,
    checkout: String,
    activity: String,
    at: Instant,
}

/// Where the open thread's subagent calls are in its transcript, while the transcript holds
/// the same items.
struct SubagentCalls {
    key: (String, usize, Option<String>, Option<String>),
    calls: Vec<usize>,
}

/// The thread details: a card floating over the top-right corner of the chat while it is wide
/// enough, else a popover from the header button.
pub struct ThreadDetails {
    workspace: Entity<Workspace>,
    open: bool,
    inline: bool,
    popover_open: bool,
    branch_menu_open: bool,
    editor_anchor: PopoverAnchor,
    home: Option<PathBuf>,
    prefs: Prefs,
    /// Editors on this machine; `None` until found.
    editors: Option<Vec<Editor>>,
    /// Each thread's uncommitted changes and checkout, as last read.
    changes: HashMap<String, ChangesSummary>,
    statuses: HashMap<String, GitStatus>,
    chat_folders: HashMap<String, String>,
    /// The local branches, read when the branch menu opens.
    branches: Option<Vec<String>>,
    running: HashMap<String, Instant>,
    done: HashMap<String, GitActionResult>,
    fresh: Option<Freshness>,
    refresh_pending: bool,
    next_tick: Option<Instant>,
    shown_thread: Option<String>,
    shown_subject: Option<Subject>,
    /// Grows each time the card turns to another subject, so a late result of the last one
    /// is dropped.
    subject_changes: u64,
    previous_agents_open: bool,
    subagent_calls: Option<SubagentCalls>,
    servers_expanded: bool,
    stopping: Option<(u32, u16)>,
    error: Option<String>,
    server_error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DetailsEvent> for ThreadDetails {}

impl ThreadDetails {
    pub fn new(workspace: Entity<Workspace>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = workspace
            .read(cx)
            .settings_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let prefs = home.as_deref().map(prefs::load).unwrap_or_default();
        let engine = workspace.read(cx).engine.clone();
        cx.spawn(async move |this, cx| {
            let editors = cx.background_spawn(async move { engine.editors() }).await;
            this.update(cx, |this, cx| {
                this.editors = Some(editors);
                cx.notify();
            })
            .ok();
        })
        .detach();
        let this = cx.weak_entity();
        let keys = cx.intercept_keystrokes(move |event, _, cx| {
            let keystroke = &event.keystroke;
            let modifiers = keystroke.modifiers;
            if keystroke.key != "o" || !modifiers.secondary(cx) || modifiers.shift || modifiers.alt
            {
                return;
            }
            let handled = this
                .update(cx, |details, cx| details.open_in_target(cx))
                .unwrap_or(false);
            if handled {
                cx.stop_propagation();
            }
        });
        let observe = cx.observe(&workspace, |this, workspace, cx| {
            let thread = workspace.read(cx).open_thread_id(cx);
            if thread != this.shown_thread {
                this.show_thread(thread, cx);
            }
            let subject = this.subject(cx);
            if subject != this.shown_subject {
                this.shown_subject = subject;
                this.subject_changes += 1;
                this.popover_open = false;
                this.error = None;
            }
            cx.notify();
        });
        Self {
            workspace,
            open: false,
            inline: true,
            editor_anchor: PopoverAnchor::new(),
            popover_open: false,
            branch_menu_open: false,
            home,
            prefs,
            editors: None,
            changes: HashMap::new(),
            statuses: HashMap::new(),
            chat_folders: HashMap::new(),
            branches: None,
            running: HashMap::new(),
            done: HashMap::new(),
            fresh: None,
            refresh_pending: false,
            next_tick: None,
            shown_thread: None,
            shown_subject: None,
            subject_changes: 0,
            previous_agents_open: false,
            subagent_calls: None,
            servers_expanded: false,
            stopping: None,
            error: None,
            server_error: None,
            _subscriptions: vec![keys, observe],
        }
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        self.open = !self.open;
        cx.notify();
    }

    /// Whether the chat leaves room for the card; without it the details open as a popover.
    #[cfg(test)]
    pub fn is_inline(&self) -> bool {
        self.inline
    }

    fn set_inline(&mut self, inline: bool, cx: &mut Context<Self>) {
        if self.inline != inline {
            self.inline = inline;
            self.popover_open = false;
            cx.notify();
        }
    }

    /// What the floating card takes from the chat's lanes while it shows.
    pub fn lane_room(&self, cx: &App) -> LaneRoom {
        let card = self.open && self.inline && self.subject(cx).is_some();
        if card {
            LaneRoom {
                right: px(CARD_WIDTH + CARD_GAP + CARD_CLEARANCE),
                gutter: px(CARD_CLEARANCE),
            }
        } else {
            LaneRoom::default()
        }
    }

    /// Whether the details show now, as the card or the popover.
    fn shown(&self) -> bool {
        if self.inline {
            self.open
        } else {
            self.popover_open
        }
    }

    fn subject(&self, cx: &App) -> Option<Subject> {
        let workspace = self.workspace.read(cx);
        if let Some(thread_id) = workspace.open_thread_id(cx) {
            return Some(Subject::Thread(thread_id));
        }
        workspace
            .draft
            .as_ref()
            .and_then(|draft| draft.project_id.clone())
            .filter(|project_id| workspace.model.project(project_id).is_some())
            .map(Subject::Draft)
    }

    fn engine(&self, cx: &App) -> std::sync::Arc<hoshi_engine::Engine> {
        self.workspace.read(cx).engine.clone()
    }

    fn show_thread(&mut self, thread: Option<String>, cx: &mut Context<Self>) {
        let engine = self.engine(cx);
        if let Some(old) = self.shown_thread.take() {
            let engine = engine.clone();
            cx.background_spawn(async move { engine.unwatch_servers(&old) })
                .detach();
        }
        self.shown_thread = thread.clone();
        self.popover_open = false;
        self.branch_menu_open = false;
        self.branches = None;
        self.previous_agents_open = false;
        self.servers_expanded = false;
        self.error = None;
        self.server_error = None;
        let Some(thread_id) = thread else {
            return;
        };
        let workspace = self.workspace.downgrade();
        cx.spawn(async move |_, cx| {
            let id = thread_id.clone();
            let servers = cx
                .background_spawn(async move { engine.watch_servers(&id) })
                .await;
            workspace
                .update(cx, |workspace, cx| {
                    workspace.servers.insert(thread_id, servers);
                    cx.notify();
                })
                .ok();
        })
        .detach();
    }

    /// Reads the thread's checkout again when it may have changed: at once when its branch,
    /// worktree or status changes, and at most once a second while its agent works.
    fn ensure_fresh(&mut self, thread: &Thread, cx: &mut Context<Self>) {
        let checkout = format!(
            "{:?}|{:?}|{:?}",
            thread.branch, thread.worktree_path, thread.status
        );
        if let Some(fresh) = &self.fresh
            && fresh.thread_id == thread.id
            && fresh.checkout == checkout
        {
            if fresh.activity == thread.updated_at {
                return;
            }
            let wait = ACTIVITY_EVERY.saturating_sub(fresh.at.elapsed());
            if !wait.is_zero() {
                if !self.refresh_pending {
                    self.refresh_pending = true;
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(wait).await;
                        this.update(cx, |this, cx| {
                            this.refresh_pending = false;
                            cx.notify();
                        })
                        .ok();
                    })
                    .detach();
                }
                return;
            }
        }
        self.fresh = Some(Freshness {
            thread_id: thread.id.clone(),
            checkout,
            activity: thread.updated_at.clone(),
            at: Instant::now(),
        });
        self.load_checkout(thread.id.clone(), thread.project_id.is_some(), cx);
    }

    fn load_checkout(&mut self, thread_id: String, project: bool, cx: &mut Context<Self>) {
        let engine = self.engine(cx);
        let read_folder = !project && !self.chat_folders.contains_key(&thread_id);
        cx.spawn(async move |this, cx| {
            let id = thread_id.clone();
            let (changes, status, folder) = cx
                .background_spawn(async move {
                    let changes = engine.thread_changes(&id).ok();
                    let status = project.then(|| engine.git_status(&id).ok()).flatten();
                    let folder = read_folder.then(|| engine.thread_folder(&id)).flatten();
                    (changes, status, folder)
                })
                .await;
            this.update(cx, |this, cx| {
                match changes {
                    Some(changes) => this.changes.insert(thread_id.clone(), changes),
                    None => this.changes.remove(&thread_id),
                };
                match status {
                    Some(status) => this.statuses.insert(thread_id.clone(), status),
                    None => this.statuses.remove(&thread_id),
                };
                if let Some(folder) = folder {
                    this.chat_folders.insert(thread_id, folder);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Draws again in `wait`, unless an earlier draw is already due.
    fn tick_in(&mut self, wait: Duration, cx: &mut Context<Self>) {
        let at = Instant::now() + wait;
        if self.next_tick.is_some_and(|due| due <= at) {
            return;
        }
        self.next_tick = Some(at);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                if this.next_tick == Some(at) {
                    this.next_tick = None;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn save_prefs(&self, cx: &mut Context<Self>) {
        if let Some(home) = self.home.clone() {
            let prefs = self.prefs.clone();
            cx.background_spawn(async move { prefs::save(&home, &prefs) })
                .detach_and_log_err(cx);
        }
    }

    fn open_target(&self) -> Option<Editor> {
        open_target(self.editors.as_deref()?, self.prefs.preferred_editor)
    }

    /// Ctrl+O: opens the card's folder where "Open in" would.
    fn open_in_target(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(editor) = self.open_target() else {
            return false;
        };
        if self.subject(cx).is_none() {
            return false;
        }
        self.open_in(editor, cx);
        true
    }

    fn open_in(&mut self, editor: Editor, cx: &mut Context<Self>) {
        let Some(subject) = self.subject(cx) else {
            return;
        };
        if self.prefs.preferred_editor != Some(editor) {
            self.prefs.preferred_editor = Some(editor);
            self.save_prefs(cx);
        }
        let engine = self.engine(cx);
        self.run(
            move || match subject {
                Subject::Thread(thread_id) => engine.open_in_editor(&thread_id, editor),
                Subject::Draft(project_id) => engine.open_project_in_editor(&project_id, editor),
            },
            cx,
        );
    }

    fn switch_branch(&mut self, branch: String, cx: &mut Context<Self>) {
        let Some(thread_id) = self.shown_thread.clone() else {
            return;
        };
        let engine = self.engine(cx);
        self.run(move || engine.switch_branch(&thread_id, &branch), cx);
    }

    /// Runs an engine call in the background, telling its failure under the card.
    fn run(
        &mut self,
        call: impl FnOnce() -> Result<(), CommandError> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        self.error = None;
        cx.notify();
        let subject = self.subject_changes;
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { call() }).await;
            if let Err(error) = result {
                this.update(cx, |this, cx| {
                    if this.subject_changes != subject {
                        return;
                    }
                    this.error = Some(error_message(&error));
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn set_branch_menu(&mut self, open: bool, cx: &mut Context<Self>) {
        self.branch_menu_open = open;
        cx.notify();
        let Some(thread_id) = self.shown_thread.clone().filter(|_| open) else {
            return;
        };
        let engine = self.engine(cx);
        cx.spawn(async move |this, cx| {
            let branches = cx
                .background_spawn(async move { engine.thread_branches(&thread_id) })
                .await
                .unwrap_or_default();
            this.update(cx, |this, cx| {
                this.branches = Some(branches);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn show(&mut self, surface: Surface, cx: &mut Context<Self>) {
        let Some(thread_id) = self.shown_thread.clone() else {
            return;
        };
        self.popover_open = false;
        cx.emit(DetailsEvent::ShowSurface { thread_id, surface });
        cx.notify();
    }

    /// The thread's agents, its subagents read from where their calls were last found.
    fn agents(&mut self, thread_id: &str, cx: &App) -> Vec<ThreadAgent> {
        let workspace = self.workspace.read(cx);
        let items: &[ThreadItem] = workspace
            .open
            .as_ref()
            .map(|open| open.read(cx))
            .filter(|open| open.thread_id == thread_id)
            .map_or(&[], |open| &open.items);
        let key = (
            thread_id.to_owned(),
            items.len(),
            items.first().map(|item| item.id.clone()),
            items.last().map(|item| item.id.clone()),
        );
        if self
            .subagent_calls
            .as_ref()
            .is_none_or(|cached| cached.key != key)
        {
            self.subagent_calls = Some(SubagentCalls {
                calls: subagent_calls(thread_id, items),
                key,
            });
        }
        let calls = self
            .subagent_calls
            .as_ref()
            .map_or(&[][..], |cached| &cached.calls);
        let threads: Vec<&Thread> = workspace.model.threads().collect();
        let pending = &workspace.pending_tasks;
        let busy = busy_threads(&threads, pending, workspace.background_tasks.keys());
        thread_agents(
            thread_id,
            threads,
            calls.iter().filter_map(|index| items.get(*index)),
            pending,
            &busy,
        )
    }

    fn show_item(&mut self, item_id: String, cx: &mut Context<Self>) {
        if let Some(thread_id) = self.shown_thread.clone() {
            cx.emit(DetailsEvent::ShowItem { thread_id, item_id });
        }
    }

    fn open_thread(&mut self, thread_id: String, cx: &mut Context<Self>) {
        self.popover_open = false;
        self.workspace
            .update(cx, |workspace, cx| workspace.open_thread(&thread_id, cx));
    }

    fn stop_server(&mut self, server: &LocalServer, cx: &mut Context<Self>) {
        let Some(thread_id) = self.shown_thread.clone() else {
            return;
        };
        if self.stopping.is_some() {
            return;
        }
        let (pid, port) = (server.pid, server.port);
        self.stopping = Some((pid, port));
        self.server_error = None;
        cx.notify();
        let engine = self.engine(cx);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { engine.stop_server(&thread_id, pid, port) })
                .await;
            this.update(cx, |this, cx| {
                this.stopping = None;
                if let Err(error) = result {
                    this.server_error = Some(stop_error(&error));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn set_draft(&mut self, draft: bool, cx: &mut Context<Self>) {
        self.prefs.draft_pull_requests = draft;
        self.save_prefs(cx);
        cx.notify();
    }

    fn quick_request(&self, action: GitAction) -> GitActionRequest {
        GitActionRequest {
            action,
            message: None,
            paths: None,
            new_branch: false,
            draft: self.prefs.draft_pull_requests,
        }
    }

    /// Runs a git action, asking first when it would push straight to the default branch.
    fn start_git(
        &mut self,
        request: GitActionRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(thread_id) = self.shown_thread.clone() else {
            return;
        };
        let Some(status) = self.statuses.get(&thread_id) else {
            return;
        };
        if !git::pushes_to_default_branch(status, request.action) {
            self.run_git(thread_id, request, window, cx);
            return;
        }
        let branch = status.branch.clone().unwrap_or_default();
        let this = cx.weak_entity();
        DialogLayer::open(window, cx, move |dialog, _, _| {
            let go = |new_branch: bool| {
                let this = this.clone();
                let thread_id = thread_id.clone();
                let request = GitActionRequest {
                    new_branch,
                    ..request.clone()
                };
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    let (thread_id, request) = (thread_id.clone(), request.clone());
                    this.update(cx, |this, cx| this.run_git(thread_id, request, window, cx))
                        .ok();
                }
            };
            dialog
                .alert()
                .title(m::git_default_branch_title(&branch))
                .description(m::git_default_branch_description(&branch))
                .footer(
                    DialogButton::new("git-default-cancel")
                        .label(m::cancel())
                        .variant(DialogButtonVariant::Outline)
                        .close(),
                )
                .footer(
                    DialogButton::new("git-default-new-branch")
                        .label(m::git_default_branch_new())
                        .variant(DialogButtonVariant::Outline)
                        .on_click(go(true))
                        .close(),
                )
                .footer(
                    DialogButton::new("git-default-confirm")
                        .label(m::git_default_branch_confirm(&branch))
                        .on_click(go(false))
                        .close(),
                )
        });
    }

    fn run_git(
        &mut self,
        thread_id: String,
        request: GitActionRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.running.contains_key(&thread_id) {
            return;
        }
        self.running.insert(thread_id.clone(), Instant::now());
        self.done.remove(&thread_id);
        self.workspace.update(cx, |workspace, _| {
            workspace.git_phases.remove(&thread_id);
        });
        cx.notify();
        let engine = self.engine(cx);
        cx.spawn_in(window, async move |this, cx| {
            let id = thread_id.clone();
            let result = cx
                .background_spawn(async move { engine.run_git_action(&id, &request) })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.running.remove(&thread_id);
                this.load_checkout(thread_id.clone(), true, cx);
                match result {
                    Ok(result) => this.finished(thread_id, result, cx),
                    Err(error) => DialogLayer::confirm(
                        ConfirmRequest::new(m::git_action_failed(), m::dialog_ok())
                            .description(git::error_text(&error))
                            .tone(ConfirmTone::Warning),
                        window,
                        cx,
                    )
                    .detach(),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn finished(&mut self, thread_id: String, result: GitActionResult, cx: &mut Context<Self>) {
        self.done.insert(thread_id.clone(), result.clone());
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DONE_FOR).await;
            this.update(cx, |this, cx| {
                if this.done.get(&thread_id) == Some(&result) {
                    this.done.remove(&thread_id);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn open_commit_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread_id) = self.shown_thread.clone() else {
            return;
        };
        let Some(status) = self.statuses.get(&thread_id).cloned() else {
            return;
        };
        commit::open(
            cx.weak_entity(),
            thread_id,
            status,
            self.engine(cx),
            window,
            cx,
        );
    }

    /// The header's part: how many servers run while the details are out of sight, and the
    /// button that shows them.
    pub fn header_toggle(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(subject) = self.subject(cx) else {
            return div().into_any_element();
        };
        let colors = cx.hoshi().clone();
        let count = match &subject {
            Subject::Thread(thread_id) => self
                .workspace
                .read(cx)
                .servers
                .get(thread_id)
                .map_or(0, Vec::len),
            Subject::Draft(_) => 0,
        };
        let notice = (!self.shown() && count > 0).then(|| {
            servers_notice(count, &colors).on_click(cx.listener(|this, _, _, cx| {
                if this.inline {
                    this.open = true;
                } else {
                    this.popover_open = true;
                }
                cx.notify();
            }))
        });
        let toggle = |open: bool| {
            Button::new("toggle-details")
                .icon(IconName::List)
                .size(ButtonSize::IconSm)
                .variant(if open {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .tooltip(Tooltip::new(m::thread_details()).side(Side::Bottom))
        };
        let button = if self.inline {
            toggle(self.open)
                .on_click(cx.listener(|this, _, _, cx| this.toggle(cx)))
                .into_any_element()
        } else {
            let this = cx.weak_entity();
            Popover::new("details-popover")
                .align(Align::End)
                .w(px(CARD_WIDTH))
                .p(px(6.))
                .gap_0()
                .open(self.popover_open)
                .on_open_change(cx.listener(|this, change: &PopoverChange, _, cx| {
                    this.popover_open = change.open;
                    cx.notify();
                }))
                .trigger(toggle)
                .content(move |_, window, cx| {
                    this.update(cx, |this, cx| this.subject_content(&subject, window, cx))
                        .unwrap_or_else(|_| div().into_any_element())
                })
                .into_any_element()
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .children(notice)
            .child(button)
            .into_any_element()
    }

    fn subject_content(
        &mut self,
        subject: &Subject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match subject {
            Subject::Thread(thread_id) => self.content(thread_id, window, cx),
            Subject::Draft(project_id) => self.draft_content(project_id, cx),
        }
    }

    fn draft_content(&mut self, project_id: &str, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.hoshi().clone();
        let workspace = self.workspace.read(cx);
        let Some(root) = workspace
            .model
            .project(project_id)
            .map(|project| project.workspace_root.clone())
        else {
            return div().into_any_element();
        };
        let branch = workspace
            .project_branches
            .get(project_id)
            .cloned()
            .flatten();
        let provider = workspace.choices.model.provider.clone();
        let mut column = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(self.folder_row(&root, None, &colors));
        if let Some(editor) = self.open_target() {
            column = column.child(self.editor_row(editor, &colors, cx));
        }
        if let Some(branch) = branch {
            let mono = cx.theme().mono_font_family.clone();
            column = column.child(separator(&colors)).child(
                row_static("details-branch", &colors)
                    .debug_selector(|| "details-draft-branch".into())
                    .child(icon(IconName::GitBranch, &colors))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(mono)
                            .text_xs()
                            .child(branch),
                    ),
            );
        }
        if let Some(limits) = self.usage_limits(&provider, cx) {
            column = column
                .child(separator(&colors))
                .child(usage::section(&limits, &colors));
        }
        column
            .when_some(self.error.clone(), |this, error| {
                this.child(error_line(error, &colors))
            })
            .into_any_element()
    }

    fn content(&mut self, thread_id: &str, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(thread) = self.workspace.read(cx).model.thread(thread_id).cloned() else {
            return div().into_any_element();
        };
        self.ensure_fresh(&thread, cx);
        let colors = cx.hoshi().clone();
        let workspace = self.workspace.read(cx);
        let root = thread
            .project_id
            .as_deref()
            .and_then(|project| workspace.model.project(project))
            .map(|project| project.workspace_root.clone());
        let pull_request = workspace.pull_requests.get(thread_id).cloned();
        let servers = workspace
            .servers
            .get(thread_id)
            .cloned()
            .unwrap_or_default();
        let commands_running = workspace
            .background_tasks
            .get(thread_id)
            .copied()
            .unwrap_or(0);
        let phase = workspace.git_phases.get(thread_id).copied();
        let parent = thread
            .parent_thread_id
            .as_deref()
            .and_then(|parent| workspace.model.thread(parent))
            .map(|parent| (parent.id.clone(), parent.title.clone()));
        let path = thread
            .worktree_path
            .clone()
            .or(root)
            .or_else(|| self.chat_folders.get(thread_id).cloned())
            .unwrap_or_default();
        let agents = self.agents(thread_id, cx);
        let changes = self.changes.get(thread_id).cloned();
        let running = self.running.get(thread_id).copied();
        if running.is_some() {
            self.tick_in(Duration::from_secs(1), cx);
        }
        if !servers.is_empty() {
            self.tick_in(servers::age_tick(&servers, servers::now_millis()), cx);
        }

        let mut column = div().flex().flex_col().gap(px(2.)).child(self.folder_row(
            &path,
            worktree_badge(&thread),
            &colors,
        ));
        if let Some(editor) = self.open_target() {
            column = column.child(self.editor_row(editor, &colors, cx));
        }
        if thread.branch.is_some() || pull_request.is_some() || changes.is_some() {
            column = column.child(separator(&colors));
        }
        if let Some(branch) = thread.branch.clone() {
            column = column.child(self.branch_row(branch, &thread, &colors, cx));
        }
        if let Some(pull_request) = &pull_request {
            column = column.child(
                pull_request_row(pull_request, thread.watching_pull_request, &colors)
                    .on_click(cx.listener(|this, _, _, cx| this.show(Surface::PullRequest, cx))),
            );
        }
        if thread.project_id.is_some()
            && let Some(status) = self.statuses.get(thread_id).cloned()
        {
            let open = pull_request.as_ref().is_some_and(|pull_request| {
                matches!(
                    pull_request.state,
                    PullRequestState::Open | PullRequestState::Draft
                )
            });
            let working = thread.active_turn_id.is_some();
            column = column.child(self.git_row(
                thread_id, &status, open, working, running, phase, &colors, cx,
            ));
        }
        if let Some(changes) = changes {
            column = column.child(
                row("details-diff", &colors)
                    .child(icon(IconName::FileDiff, &colors))
                    .child(div().flex_none().child(m::surface_diff()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(colors.muted_foreground)
                            .child(m::diff_scope_uncommitted()),
                    )
                    .child(diff_count(changes.additions, changes.deletions, &colors))
                    .on_click(cx.listener(|this, _, _, cx| this.show(Surface::Diff, cx))),
            );
        }
        column = self.agents_section(column, agents, parent, &colors, cx);
        column = self.servers_section(column, servers, commands_running, &colors, cx);
        if let Some(limits) = self.usage_limits(usage::provider_of(&thread), cx) {
            column = column
                .child(separator(&colors))
                .child(usage::section(&limits, &colors));
        }
        column
            .when_some(self.error.clone(), |this, error| {
                this.child(error_line(error, &colors))
            })
            .into_any_element()
    }

    /// The limits of `provider` when the card shows them, as last read.
    fn usage_limits(&self, provider: &str, cx: &App) -> Option<UsageLimits> {
        let workspace = self.workspace.read(cx);
        if !workspace.settings.usage_limits_in_details {
            return None;
        }
        let known = crate::providers::known(provider)?;
        if !known.enabled(&workspace.settings.providers) {
            return None;
        }
        let limits = workspace.usage_limits(provider);
        let state = section_state(
            workspace.providers.accounts.get(provider).copied(),
            limits,
            false,
            false,
        );
        limits
            .filter(|limits| state == SectionState::Limits && !limits.windows.is_empty())
            .cloned()
    }

    fn folder_row(&self, path: &str, badge: Option<String>, colors: &Colors) -> impl IntoElement {
        let folder = path
            .split(['\\', '/'])
            .rfind(|part| !part.is_empty())
            .unwrap_or(path)
            .to_owned();
        let hover = colors.foreground.opacity(0.05);
        let reveal = PathBuf::from(path);
        div()
            .id("details-folder")
            .flex()
            .flex_none()
            .h_8()
            .min_w_0()
            .items_center()
            .gap(px(10.))
            .rounded_lg()
            .pr_1()
            .pl(px(10.))
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(colors.foreground.opacity(0.8))
            .child(icon(IconName::Folder, colors))
            .child(
                div()
                    .id("details-folder-name")
                    .min_w_0()
                    .truncate()
                    .child(folder)
                    .with_tooltip(Tooltip::new(path.to_owned())),
            )
            .when_some(badge, |this, badge| {
                this.child(
                    div()
                        .flex_none()
                        .rounded_full()
                        .border_1()
                        .border_color(colors.border)
                        .px(px(6.))
                        .text_size(px(10.))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(colors.muted_foreground)
                        .child(badge),
                )
            })
            .child(
                div()
                    .id("details-reveal")
                    .ml_auto()
                    .flex()
                    .flex_none()
                    .size_6()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .child(icon(IconName::FolderOpen, colors))
                    .on_click(move |_, _, cx| cx.reveal_path(&reveal))
                    .with_tooltip(Tooltip::new(m::show_in_folder())),
            )
    }

    fn editor_row(
        &self,
        editor: Editor,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let installed = self.editors.clone().unwrap_or_default();
        let this = cx.weak_entity();
        let arrow_colors = colors.clone();
        let menu_colors = colors.clone();
        let menu = Popover::new("details-editor-menu")
            .anchor(self.editor_anchor.clone())
            .align(Align::End)
            .match_trigger_width()
            .trigger(move |open| {
                split_arrow("details-editor-arrow", open, &arrow_colors)
                    .debug_selector(|| "details-editor-arrow".into())
            })
            .menu(move |menu, _, _| {
                menu.items(installed.into_iter().enumerate().map(|(index, choice)| {
                    let this = this.clone();
                    MenuOption::new(("details-editor", index), editor_label(choice))
                        .leading(div().mr_1().child(editor_icon(choice, &menu_colors)))
                        .when(choice == editor, |option| option.shortcut(open_shortcut()))
                        .on_select(move |_, cx| {
                            this.update(cx, |this, cx| this.open_in(choice, cx)).ok();
                        })
                }))
            });
        split_row(
            "details-editor",
            split_primary("details-open-editor", colors)
                .child(editor_icon(editor, colors))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(m::open_in_editor(editor_label(editor))),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.open_in(editor, cx))),
            Some(menu.into_any_element()),
            colors,
        )
        .debug_selector(|| "details-editor-row".into())
        .child(self.editor_anchor.area())
    }

    fn branch_row(
        &self,
        branch: String,
        thread: &Thread,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mono = cx.theme().mono_font_family.clone();
        let worktree = thread.worktree_path.is_some();
        let build = {
            let colors = colors.clone();
            let branch = branch.clone();
            move |open: bool, enabled: bool| {
                let base = if enabled {
                    row("details-branch", &colors)
                } else {
                    row_static("details-branch", &colors).opacity(0.6)
                };
                base.when(open, |this| this.bg(colors.foreground.opacity(0.05)))
                    .child(icon(
                        if worktree {
                            IconName::GitFork
                        } else {
                            IconName::GitBranch
                        },
                        &colors,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(mono)
                            .text_xs()
                            .child(branch),
                    )
                    .child(icon(IconName::ChevronDown, &colors))
            }
        };
        if thread.status == ThreadStatus::Working {
            return build(false, false)
                .with_tooltip(Tooltip::new(m::error_thread_busy()))
                .into_any_element();
        }
        let others: Option<Vec<String>> = self.branches.as_ref().map(|branches| {
            branches
                .iter()
                .filter(|name| **name != branch)
                .cloned()
                .collect()
        });
        let this = cx.weak_entity();
        let muted = colors.muted_foreground;
        div()
            .flex()
            .flex_col()
            .child(
                Popover::new("details-branch-menu")
                    .align(Align::End)
                    .open(self.branch_menu_open)
                    .on_open_change(cx.listener(|this, change: &PopoverChange, _, cx| {
                        this.set_branch_menu(change.open, cx)
                    }))
                    .trigger(move |open| build(open, true))
                    .menu(move |menu, _, _| {
                        let empty = others.as_ref().is_some_and(Vec::is_empty);
                        menu.max_list_height(px(288.))
                            .heading(m::switch_branch())
                            .when(empty, |menu| {
                                menu.child(
                                    div()
                                        .px_2()
                                        .py_1p5()
                                        .text_xs()
                                        .text_color(muted)
                                        .child(m::no_other_branches()),
                                )
                            })
                            .items(
                                others
                                    .into_iter()
                                    .flatten()
                                    .enumerate()
                                    .map(|(index, name)| {
                                        let this = this.clone();
                                        MenuOption::new(
                                            ("details-branch-option", index),
                                            name.clone(),
                                        )
                                        .mono()
                                        .on_select(
                                            move |_, cx| {
                                                let name = name.clone();
                                                this.update(cx, |this, cx| {
                                                    this.switch_branch(name, cx)
                                                })
                                                .ok();
                                            },
                                        )
                                    }),
                            )
                    }),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn git_row(
        &self,
        thread_id: &str,
        status: &GitStatus,
        pull_request_open: bool,
        working: bool,
        running: Option<Instant>,
        phase: Option<hoshi_core::GitActionPhase>,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let draft = self.prefs.draft_pull_requests;
        let primary = if let Some(started) = running {
            div()
                .id("details-git-progress")
                .flex()
                .flex_1()
                .h_8()
                .min_w_0()
                .items_center()
                .gap(px(10.))
                .px(px(10.))
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors.foreground.opacity(0.8))
                .child(
                    Spinner::new()
                        .icon(IconName::LoaderCircle)
                        .color(colors.muted_foreground)
                        .with_size(px(16.)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(git::phase_text(phase)),
                )
                .child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(crate::format::format_duration(
                            started.elapsed().as_millis() as i64,
                        )),
                )
                .into_any_element()
        } else if let Some(done) = self.done.get(thread_id) {
            let (title, description) = git::done_text(done);
            let url = done
                .pull_request
                .as_ref()
                .map(|pull_request| pull_request.url.clone());
            split_primary("details-git-done", colors)
                .when(url.is_none(), |this| this.cursor_default())
                .child(
                    Icon::new(IconName::CircleCheck)
                        .size_4()
                        .flex_none()
                        .text_color(colors.status_done),
                )
                .child(div().truncate().child(title))
                .when_some(description, |this, description| {
                    this.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(colors.muted_foreground)
                            .child(description),
                    )
                })
                .when_some(url, |this, url| {
                    this.on_click(move |_, _, cx| cx.open_url(&url))
                })
                .into_any_element()
        } else {
            let quick = git::quick_action(status, pull_request_open);
            let blocker = if working {
                Some(m::git_error_busy())
            } else {
                quick.blocker.map(git::blocker_text)
            };
            let button = split_primary("details-git-quick", colors)
                .child(icon(git::label_icon(quick.label), colors))
                .child(div().truncate().child(git::label_text(quick.label, draft)));
            match blocker {
                Some(blocker) => button
                    .opacity(0.6)
                    .cursor_default()
                    .with_tooltip(Tooltip::new(blocker).side(Side::Left).max_width(px(240.)))
                    .into_any_element(),
                None => {
                    let action = quick.action;
                    button
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let request = this.quick_request(action);
                            this.start_git(request, window, cx)
                        }))
                        .into_any_element()
                }
            }
        };
        let menu = running.is_none().then(|| {
            let blockers = git::menu_blockers(status);
            let this = cx.weak_entity();
            let arrow_colors = colors.clone();
            Popover::new("details-git-menu")
                .align(Align::End)
                .trigger(move |open| {
                    split_arrow("details-git-arrow", open, &arrow_colors)
                        .debug_selector(|| "details-git-arrow".into())
                })
                .menu(move |menu, _, _| {
                    let action = |action: GitAction| {
                        let this = this.clone();
                        move |window: &mut Window, cx: &mut App| {
                            this.update(cx, |this, cx| {
                                let request = this.quick_request(action);
                                this.start_git(request, window, cx)
                            })
                            .ok();
                        }
                    };
                    let commit = this.clone();
                    let toggle = this.clone();
                    menu.item(
                        MenuOption::new("details-git-commit", m::git_commit_menu())
                            .icon(IconName::GitCommitHorizontal)
                            .disabled(working || blockers.commit.is_some())
                            .when_some(blockers.commit, |option, blocker| {
                                option.tooltip(blocker_tooltip(blocker))
                            })
                            .on_select(move |window, cx| {
                                commit
                                    .update(cx, |this, cx| this.open_commit_dialog(window, cx))
                                    .ok();
                            }),
                    )
                    .item(
                        MenuOption::new("details-git-push", m::git_push())
                            .icon(IconName::CloudUpload)
                            .disabled(working || blockers.push.is_some())
                            .when_some(blockers.push, |option, blocker| {
                                option.tooltip(blocker_tooltip(blocker))
                            })
                            .on_select(action(GitAction::Push)),
                    )
                    .when(!pull_request_open, |menu| {
                        menu.item(
                            MenuOption::new(
                                "details-git-pr",
                                if draft {
                                    m::git_create_draft_pr()
                                } else {
                                    m::git_create_pr()
                                },
                            )
                            .icon(IconName::GitPullRequest)
                            .disabled(working || blockers.create_pull_request.is_some())
                            .when_some(blockers.create_pull_request, |option, blocker| {
                                option.tooltip(blocker_tooltip(blocker))
                            })
                            .on_select(action(GitAction::CreatePullRequest)),
                        )
                    })
                    .separator()
                    .item(
                        MenuOption::checkbox(
                            "details-git-draft",
                            m::git_draft_pull_requests(),
                            draft,
                        )
                        .on_select(move |_, cx| {
                            toggle
                                .update(cx, |this, cx| this.set_draft(!draft, cx))
                                .ok();
                        }),
                    )
                })
                .into_any_element()
        });
        split_row("details-git", primary, menu, colors)
    }

    fn agents_section(
        &self,
        column: Div,
        agents: Vec<ThreadAgent>,
        parent: Option<(String, String)>,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> Div {
        if agents.is_empty() && parent.is_none() {
            return column;
        }
        let (active, previous): (Vec<_>, Vec<_>) = agents
            .into_iter()
            .partition(|agent| agent.state.is_active());
        let now = Utc::now();
        let agent_row = |agent: ThreadAgent, cx: &mut Context<Self>| {
            let base = match agent.kind {
                AgentKind::Task => {
                    let id = agent.id.clone();
                    row(
                        SharedString::from(format!("details-agent-{}", agent.id)),
                        colors,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.open_thread(id.clone(), cx)))
                }
                AgentKind::Subagent => {
                    let id = agent.id.clone();
                    row(
                        SharedString::from(format!("details-agent-{}", agent.id)),
                        colors,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.show_item(id.clone(), cx)))
                }
            };
            let selector = format!("details-agent-{}", agent.id);
            base.debug_selector(move || selector)
                .child(
                    div()
                        .relative()
                        .flex()
                        .flex_none()
                        .child(icon(
                            match agent.kind {
                                AgentKind::Task => IconName::GitFork,
                                AgentKind::Subagent => IconName::Bot,
                            },
                            colors,
                        ))
                        .child(
                            div()
                                .absolute()
                                .right(px(-2.))
                                .bottom(px(-2.))
                                .size(px(6.))
                                .rounded_full()
                                .border_2()
                                .border_color(colors.popover)
                                .bg(agent_dot(agent.state, colors)),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(if agent.title.is_empty() {
                            m::subagent()
                        } else {
                            agent.title.clone()
                        }),
                )
                .child(
                    div()
                        .flex_none()
                        .text_xs()
                        .font_weight(FontWeight::NORMAL)
                        .text_color(colors.muted_foreground)
                        .child(relative_time(&agent.at, now)),
                )
        };
        let mut column = column
            .child(separator(colors))
            .child(section_label(m::agents(), colors));
        if let Some((id, title)) = parent {
            column = column.child(
                row("details-delegated-by", colors)
                    .child(icon(IconName::GitFork, colors))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(m::delegated_by(title)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.open_thread(id.clone(), cx))),
            );
        }
        if !active.is_empty() {
            let rows: Vec<_> = active
                .into_iter()
                .map(|agent| agent_row(agent, cx))
                .collect();
            column = column.child(
                div()
                    .id("details-agents-active")
                    .flex()
                    .flex_col()
                    .max_h(px(240.))
                    .overflow_y_scroll()
                    .children(rows),
            );
        }
        if !previous.is_empty() {
            let open = self.previous_agents_open;
            column = column.child(
                row("details-agents-previous", colors)
                    .font_weight(FontWeight::NORMAL)
                    .text_color(colors.muted_foreground)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(m::agents_previous()),
                    )
                    .child(div().text_xs().child(previous.len().to_string()))
                    .child(
                        Icon::new(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size_3p5()
                        .flex_none(),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.previous_agents_open = !this.previous_agents_open;
                        cx.notify();
                    })),
            );
            if open {
                let rows: Vec<_> = previous
                    .into_iter()
                    .map(|agent| agent_row(agent, cx))
                    .collect();
                column = column.child(
                    div()
                        .id("details-agents-previous-list")
                        .flex()
                        .flex_col()
                        .max_h(px(192.))
                        .overflow_y_scroll()
                        .children(rows),
                );
            }
        }
        column
    }

    fn servers_section(
        &self,
        column: Div,
        servers: Vec<LocalServer>,
        commands_running: usize,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> Div {
        if servers.is_empty() {
            return column;
        }
        let now = servers::now_millis();
        let hidden = servers.len().saturating_sub(servers::SERVERS_SHOWN);
        let shown = if self.servers_expanded {
            servers.len()
        } else {
            servers::SERVERS_SHOWN
        };
        let mono = colors.foreground.opacity(0.8);
        let rows: Vec<_> =
            servers
                .into_iter()
                .take(shown)
                .map(|server| {
                    let key = format!("{}:{}", server.pid, server.port);
                    let stoppable = servers::stops_from_row(&server, commands_running);
                    let stopping = self.stopping == Some((server.pid, server.port));
                    let label = m::server_stop_label(server.port);
                    let destructive = colors.destructive;
                    div()
                        .id(SharedString::from(format!("details-server-{key}")))
                        .relative()
                        .flex()
                        .min_h_10()
                        .min_w_0()
                        .items_center()
                        .gap(px(10.))
                        .px(px(10.))
                        .py_1()
                        .when(stoppable, |this| this.pr_10())
                        .child(
                            div()
                                .flex_none()
                                .size(px(7.))
                                .rounded_full()
                                .bg(colors.status_done),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(mono)
                                        .child(format!("localhost:{}", server.port)),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_xs()
                                        .text_color(colors.muted_foreground)
                                        .child(servers::server_detail(&server, now)),
                                ),
                        )
                        .when(stoppable, |this| {
                            this.child(
                                div()
                                    .id(SharedString::from(format!("details-server-stop-{key}")))
                                    .absolute()
                                    .right(px(6.))
                                    .top(px(8.))
                                    .flex()
                                    .size_6()
                                    .items_center()
                                    .justify_center()
                                    .rounded_md()
                                    .text_color(destructive)
                                    .when(self.stopping.is_none(), |this| {
                                        this.cursor_pointer()
                                            .hover(move |style| style.bg(destructive.opacity(0.1)))
                                    })
                                    .child(if stopping {
                                        Spinner::new()
                                            .icon(IconName::LoaderCircle)
                                            .color(destructive)
                                            .with_size(px(14.))
                                            .into_any_element()
                                    } else {
                                        Icon::new(IconName::Square).size_3p5().into_any_element()
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.stop_server(&server, cx)
                                    }))
                                    .with_tooltip(Tooltip::new(label)),
                            )
                        })
                })
                .collect();
        let hover = colors.foreground.opacity(0.05);
        column
            .child(separator(colors))
            .child(section_label(m::servers(), colors))
            .children(rows)
            .when(hidden > 0, |this| {
                this.child(
                    div()
                        .id("details-servers-more")
                        .flex()
                        .h_8()
                        .items_center()
                        .rounded_lg()
                        .px(px(10.))
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover))
                        .child(if self.servers_expanded {
                            m::servers_show_less()
                        } else {
                            m::servers_show_more(hidden)
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.servers_expanded = !this.servers_expanded;
                            cx.notify();
                        })),
                )
            })
            .when_some(self.server_error.clone(), |this, error| {
                this.child(error_line(error, colors))
            })
    }
}

impl Render for ThreadDetails {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        let inline = self.inline;
        let measure = canvas(
            move |bounds, _, cx| {
                let wide = bounds.size.width >= px(INLINE_MIN_WIDTH);
                if wide != inline {
                    cx.defer(move |cx| {
                        this.update(cx, |this, cx| this.set_inline(wide, cx)).ok();
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let subject = self.subject(cx);
        let card = subject.filter(|_| self.open && self.inline).map(|subject| {
            let colors = cx.hoshi().clone();
            let content = self.subject_content(&subject, window, cx);
            div()
                .absolute()
                .size_full()
                .p(px(CARD_GAP))
                .flex()
                .flex_col()
                .items_end()
                .child(
                    div()
                        .id("thread-details")
                        .debug_selector(|| "thread-details-card".into())
                        .occlude()
                        .w(px(CARD_WIDTH))
                        .max_h_full()
                        .overflow_y_scroll()
                        .p(px(6.))
                        .rounded_2xl()
                        .border_1()
                        .border_color(colors.border.opacity(0.6))
                        .bg(colors.popover.opacity(0.85))
                        .shadow_lg()
                        .child(crate::browser::obstacle(crate::browser::Obstacle::Details))
                        .child(content),
                )
        });
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(measure)
            .children(card)
    }
}

fn worktree_badge(thread: &Thread) -> Option<String> {
    thread.worktree_path.as_ref()?;
    Some(if thread.worktree_released {
        m::worktree_released_badge()
    } else {
        m::worktree_badge()
    })
}

fn servers_notice(count: usize, colors: &Colors) -> Stateful<Div> {
    let hover = colors.foreground.opacity(0.05);
    let foreground = colors.foreground;
    div()
        .id("details-servers-notice")
        .flex()
        .flex_none()
        .h(px(26.))
        .items_center()
        .gap(px(6.))
        .rounded_md()
        .border_1()
        .border_color(colors.border)
        .px_2()
        .text_xs()
        .whitespace_nowrap()
        .text_color(colors.muted_foreground)
        .cursor_pointer()
        .hover(move |style| style.bg(hover).text_color(foreground))
        .child(
            div()
                .flex_none()
                .size(px(7.))
                .rounded_full()
                .bg(colors.status_done),
        )
        .child(if count == 1 {
            m::servers_running_one()
        } else {
            m::servers_running(count)
        })
}

fn pull_request_row(
    pull_request: &hoshi_core::PullRequest,
    watching: bool,
    colors: &Colors,
) -> Stateful<Div> {
    let (name, color) = match pull_request.state {
        PullRequestState::Open => (IconName::GitPullRequest, colors.pr_open),
        PullRequestState::Draft => (IconName::GitPullRequestDraft, colors.muted_foreground),
        PullRequestState::Merged => (IconName::GitMerge, colors.pr_merged),
        PullRequestState::Closed => (IconName::GitPullRequestClosed, colors.status_error),
    };
    row("details-pull-request", colors)
        .child(Icon::new(name).size_4().flex_none().text_color(color))
        .child(div().flex_none().child(format!("#{}", pull_request.number)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::NORMAL)
                .text_color(colors.muted_foreground)
                .child(pull_request.title.clone()),
        )
        .when(watching, |this| {
            this.child(
                div()
                    .id("details-pr-watching")
                    .flex_none()
                    .child(icon(IconName::Eye, colors))
                    .with_tooltip(Tooltip::new(m::pr_watching_tooltip())),
            )
        })
}

/// Why a git action of the menu can't run, beside it on hover.
fn blocker_tooltip(blocker: git::Blocker) -> Tooltip {
    Tooltip::new(git::blocker_text(blocker))
        .side(Side::Left)
        .max_width(px(240.))
}

fn agent_dot(state: AgentState, colors: &Colors) -> Hsla {
    match state {
        AgentState::Running => colors.status_working,
        AgentState::Waiting => colors.status_waiting,
        AgentState::Done => colors.status_done,
        AgentState::Stopped => colors.muted_foreground.opacity(0.5),
        AgentState::Failed => colors.status_error,
    }
}

fn error_message(error: &CommandError) -> String {
    match error {
        CommandError::GitFailed { detail } => detail.clone(),
        CommandError::ThreadBusy { .. } => m::error_thread_busy(),
        _ => m::error_generic(),
    }
}

fn stop_error(error: &CommandError) -> String {
    match error {
        CommandError::ServerNotFound { .. } => m::error_server_gone(),
        CommandError::ServerStopRefused { .. } => m::error_server_stop_refused(),
        CommandError::ServerStopFailed { detail } => detail.clone(),
        _ => m::error_generic(),
    }
}
