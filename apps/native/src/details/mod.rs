pub(crate) mod agents;
mod commit;
mod git;
pub(crate) mod prefs;
mod rows;
mod servers;
#[cfg(test)]
mod tests;
mod usage;

mod actions;
mod agents_view;
mod content;
mod git_view;
mod place_rows;
mod servers_view;
mod view_state;

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
    /// Grows with each engine call the card runs, so only the latest one tells its failure.
    runs: u64,
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
            runs: 0,
            previous_agents_open: false,
            subagent_calls: None,
            servers_expanded: false,
            stopping: None,
            error: None,
            server_error: None,
            _subscriptions: vec![keys, observe],
        }
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
