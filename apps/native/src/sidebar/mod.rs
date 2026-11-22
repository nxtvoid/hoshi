pub(crate) mod cards;
mod prefs;
mod project_settings;
mod snooze;
#[cfg(test)]
mod ui_tests;
mod worktree_setup;

mod actions;
mod heading;
mod menus;
mod rail_view;
mod thread_list;
mod thread_row;

use crate::drafts;
use crate::m;
use crate::palette;
use crate::sections::{
    self, Filter, Query, Section, SectionId, Signal, Sort, View, inbox_sections, project_signal,
    relative_time, sidebar_sections,
};
use crate::state::Workspace;
use crate::updates::UpdatesButton;
use cards::{CardTarget, Cards, anchor};
use chrono::{DateTime, Utc};
use gpui_kit::component::Selectable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_core::{
    ChecksState, Command, CommandError, Project, PullRequest, PullRequestState, Thread,
    ThreadStatus, is_snoozed, shows_woke, snooze_left,
};
use hoshi_ui::assets::brand;
use hoshi_ui::prelude::*;
use hoshi_ui::theme::Colors;
use prefs::Prefs;
use snooze::{CustomSnoozeForm, snooze_menu};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;
use worktree_setup::{WorktreeSetupEvent, WorktreeSetupForm};

const SAVE_DELAY: Duration = Duration::from_millis(300);

actions!(
    sidebar,
    [NextThread, PreviousThread, NextAttention, PreviousAttention]
);

/// Alt+↑/↓ moves through the threads the sidebar shows; with Shift it jumps between the threads
/// that need the user, in every project.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("alt-down", NextThread, None),
        KeyBinding::new("alt-up", PreviousThread, None),
        KeyBinding::new("alt-shift-down", NextAttention, None),
        KeyBinding::new("alt-shift-up", PreviousAttention, None),
    ]);
}

/// Handles the sidebar's shortcuts on `element`, the window's root.
pub fn shortcuts(element: Div, sidebar: &Entity<Sidebar>) -> Div {
    let step = |sidebar: &Entity<Sidebar>, step: isize, attention: bool| {
        let sidebar = sidebar.clone();
        move |cx: &mut App| sidebar.update(cx, |this, cx| this.step(step, attention, cx))
    };
    let (next, previous) = (step(sidebar, 1, false), step(sidebar, -1, false));
    let (next_attention, previous_attention) = (step(sidebar, 1, true), step(sidebar, -1, true));
    element
        .on_action(move |_: &NextThread, _, cx| next(cx))
        .on_action(move |_: &PreviousThread, _, cx| previous(cx))
        .on_action(move |_: &NextAttention, _, cx| next_attention(cx))
        .on_action(move |_: &PreviousAttention, _, cx| previous_attention(cx))
}

pub enum SidebarEvent {
    Toggle,
    OpenSettings,
    OpenUsage,
    OpenPalette(palette::Mode),
    /// Open the thread's pull request in the right panel.
    OpenPullRequest(String),
}

#[derive(Clone, PartialEq)]
enum MenuTarget {
    Thread(String),
    Project(String),
    /// A new thread's draft, by its key.
    Draft(String),
    /// The snooze choices of a thread, from its row's clock.
    Snooze(String),
}

/// A context menu, open at `position` in the window.
struct OpenMenu {
    /// Tells this opening apart from the next one, whose close it must not take.
    serial: usize,
    target: MenuTarget,
    position: Point<Pixels>,
}

#[derive(Clone, Copy)]
enum ThreadAction {
    Pin(bool),
    Wake,
    Archive(bool),
    RegenerateTitle,
    Fork,
    CleanUp,
    Release,
    DiscardDraft,
    Delete,
}

/// The web app's sidebar: search and actions on top, the project rail, the threads of what the
/// rail picked in sections, and the footer.
pub struct Sidebar {
    workspace: Entity<Workspace>,
    search: Entity<InputState>,
    prefs: Prefs,
    home: Option<PathBuf>,
    project_id: Option<String>,
    sort_open: bool,
    /// Found once per project; `None` when it has none.
    icons: HashMap<String, Option<PathBuf>>,
    shown_thread: Option<String>,
    menu: Option<OpenMenu>,
    menu_serial: usize,
    cards: Cards,
    setup: Option<(Entity<WorktreeSetupForm>, Subscription)>,
    custom_snooze: Option<(Entity<CustomSnoozeForm>, Subscription)>,
    /// The project whose settings dialog is open.
    project_settings: Option<String>,
    /// When the list sorts again for a snooze that ends.
    refresh: Option<(DateTime<Utc>, Task<()>)>,
    cleaning_merged: bool,
    saving: Option<Task<()>>,
    thread_list: ListState,
    list_view: Option<std::rc::Rc<thread_list::ListView>>,
    list_keys: Vec<thread_list::EntryKey>,
    rail_scroll: ScrollHandle,
    updates: Entity<UpdatesButton>,
    /// The rail and the sections of threads, views of their own so that frames for the rest of
    /// the window, or of the rail's signal rings, draw what didn't change from the cache.
    rail_part: Entity<Part>,
    list_part: Entity<Part>,
    /// The rail's signal rings, by element id.
    rings: HashMap<SharedString, Entity<SignalRing>>,
    /// Redraws the rows' relative times.
    _clock: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SidebarEvent> for Sidebar {}

const CLOCK_EVERY: Duration = Duration::from_secs(30);

impl Sidebar {
    pub fn new(workspace: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(m::search_threads()));
        let subscriptions = vec![
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    cx.notify();
                }
            }),
            cx.observe(&workspace, |this, workspace, cx| {
                let open = workspace.read(cx).open_thread_id(cx);
                if open != this.shown_thread {
                    this.shown_thread.clone_from(&open);
                    let project = open
                        .and_then(|id| workspace.read(cx).model.thread(&id)?.project_id.clone());
                    if project.is_some() {
                        this.project_id = project;
                    }
                }
                let model = &workspace.read(cx).model;
                let gone = match &this.project_id {
                    Some(id) => model.project(id).is_none(),
                    None => model.projects().next().is_some(),
                };
                if gone {
                    this.project_id = latest_project(model.projects());
                }
                this.schedule_refresh(cx);
                cx.notify();
            }),
            cx.observe_self(|this, cx| {
                this.rail_part.update(cx, |_, cx| cx.notify());
                this.list_part.update(cx, |_, cx| cx.notify());
            }),
        ];
        let home = workspace
            .read(cx)
            .settings_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let prefs = home.as_deref().map(prefs::load).unwrap_or_default();
        let project_id = latest_project(workspace.read(cx).model.projects());
        let updates = cx.new(|cx| UpdatesButton::new(workspace.clone(), window, cx));
        let part = |draw: Draw, cx: &mut Context<Self>| {
            let sidebar = cx.weak_entity();
            cx.new(|_| Part { sidebar, draw })
        };
        let rail_part = part(Self::rail, cx);
        let list_part = part(Self::list, cx);
        let mut sidebar = Self {
            workspace,
            search,
            prefs,
            home,
            project_id,
            sort_open: false,
            icons: HashMap::new(),
            shown_thread: None,
            menu: None,
            menu_serial: 0,
            cards: Cards::default(),
            setup: None,
            custom_snooze: None,
            project_settings: None,
            refresh: None,
            cleaning_merged: false,
            saving: None,
            thread_list: ListState::new(0, ListAlignment::Top, px(200.)),
            list_view: None,
            list_keys: Vec::new(),
            rail_scroll: ScrollHandle::new(),
            updates,
            rail_part,
            list_part,
            rings: HashMap::new(),
            _clock: cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(CLOCK_EVERY).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }),
            _subscriptions: subscriptions,
        };
        sidebar.follow_scope(cx);
        sidebar.schedule_refresh(cx);
        sidebar
    }
}

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let weak = cx.weak_entity();
        div()
            .id("sidebar")
            .relative()
            .flex()
            .flex_col()
            .w(px(self.prefs.width))
            .h_full()
            .flex_none()
            .bg(colors.sidebar)
            .text_color(colors.sidebar_foreground)
            .border_r_1()
            .border_color(colors.sidebar_border)
            .child(self.header(&colors, cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .when(self.shows_rail(cx), |this| {
                        this.child(
                            AnyView::from(self.rail_part.clone())
                                .cached(StyleRefinement::default().w_12().flex_none().h_full()),
                        )
                    })
                    .child(
                        AnyView::from(self.list_part.clone())
                            .cached(StyleRefinement::default().flex_1().min_w_0().h_full()),
                    ),
            )
            .child(self.footer(&colors, cx))
            .child(
                ResizeHandle::new("resize-sidebar", ResizeSide::Right, self.prefs.width)
                    .range(prefs::MIN_WIDTH, prefs::MAX_WIDTH)
                    .default_value(prefs::DEFAULT_WIDTH)
                    .on_change(move |width, _, cx| {
                        weak.update(cx, |this, cx| this.set_width(width, cx)).ok();
                    }),
            )
            .children(self.context_menu(cx))
            .children(self.card(cx))
            .children(self.setup_dialog(cx))
            .children(self.custom_snooze_dialog(cx))
            .children(self.project_settings_dialog(cx))
    }
}

/// A project being dragged along the rail; drawn under the pointer as its button.
#[derive(Clone)]
struct ProjectDrag {
    id: String,
    initials: String,
    icon: Option<PathBuf>,
}

impl Render for ProjectDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi();
        div()
            .size_8()
            .flex()
            .items_center()
            .justify_center()
            .rounded_lg()
            .border_1()
            .border_color(colors.sidebar_ring)
            .bg(colors.sidebar_accent)
            .text_color(colors.sidebar_accent_foreground)
            .text_size(rems(0.6875))
            .font_weight(FontWeight::SEMIBOLD)
            .shadow_md()
            .child(project_mark(
                self.icon.clone(),
                self.initials.clone(),
                px(20.),
                px(5.),
            ))
    }
}

/// The project's menu, from the rail or from the "…" by its name.
fn project_menu(menu: PopoverMenu, this: WeakEntity<Sidebar>, project_id: String) -> PopoverMenu {
    let (settings, editing, removing) = (this.clone(), this.clone(), this);
    let (settings_id, edit_id, remove_id) = (project_id.clone(), project_id.clone(), project_id);
    menu.item(
        MenuOption::new("project-settings", m::project_settings_menu()).on_select(move |_, cx| {
            settings
                .update(cx, |this, cx| {
                    this.open_project_settings(settings_id.clone(), cx)
                })
                .ok();
        }),
    )
    .item(
        MenuOption::new("project-setup", m::worktree_setup_menu()).on_select(move |window, cx| {
            editing
                .update(cx, |this, cx| this.edit_setup(&edit_id, window, cx))
                .ok();
        }),
    )
    .item(
        MenuOption::new("project-remove", m::remove_project())
            .destructive()
            .on_select(move |window, cx| {
                removing
                    .update(cx, |this, cx| {
                        this.remove_project(remove_id.clone(), window, cx)
                    })
                    .ok();
            }),
    )
}

/// What every row of the list needs to know besides its thread.
struct RowContext {
    open: Option<String>,
    show_project: bool,
    project_titles: HashMap<String, String>,
    pull_requests: HashMap<String, PullRequest>,
    unseen: HashSet<String>,
    /// The branch each project's root is on.
    root_branches: HashMap<String, Option<String>>,
    /// Keys of the drafts kept.
    drafts: HashSet<String>,
    /// When each snoozed thread wakes.
    snoozed: HashMap<String, String>,
    /// Threads that woke from a snooze since the user last saw them.
    woke: HashSet<String>,
}

fn latest_project<'a>(projects: impl Iterator<Item = &'a Project>) -> Option<String> {
    projects
        .max_by(|a, b| a.updated_at.cmp(&b.updated_at))
        .map(|project| project.id.clone())
}

pub(crate) fn initials(title: &str) -> String {
    title.chars().take(2).collect::<String>().to_uppercase()
}

pub(crate) fn project_mark(
    icon: Option<PathBuf>,
    initials: String,
    size: Pixels,
    radius: Pixels,
) -> AnyElement {
    match icon {
        Some(path) => img(path)
            .size(size)
            .rounded(radius)
            .object_fit(ObjectFit::Contain)
            .with_fallback(move || div().child(initials.clone()).into_any_element())
            .into_any_element(),
        None => div().child(initials).into_any_element(),
    }
}

/// What cleaning up a merged thread would do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MergedCleanup {
    /// Remove its worktree.
    Worktree,
    /// Switch the project root back to the base branch.
    Root,
}

/// `None` when nothing: the pull request isn't merged, or the worktree isn't hoshi's.
fn merged_cleanup(thread: &Thread, pull_request: Option<&PullRequest>) -> Option<MergedCleanup> {
    let pull_request = pull_request?;
    if pull_request.state != PullRequestState::Merged || thread.worktree_adopted {
        return None;
    }
    if thread.worktree_path.is_some() {
        return Some(MergedCleanup::Worktree);
    }
    let on_head = thread.branch.as_deref() == Some(pull_request.head_branch.as_str())
        && pull_request.head_branch != pull_request.base_branch;
    on_head.then_some(MergedCleanup::Root)
}

/// A thread in the project root that worked on another branch than the one the root is on now.
pub(crate) fn branch_mismatch(thread: &Thread, root_branch: Option<&str>) -> bool {
    if thread.worktree_path.is_some() || thread.project_id.is_none() {
        return false;
    }
    match (thread.branch.as_deref(), root_branch) {
        (Some(branch), Some(root)) => branch != root,
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Problem {
    Conflicts,
    ChecksFailing,
    ChangesRequested,
}

/// The icon of a pull request's state, in the color of its state or of what holds it back.
pub(crate) fn pull_request_mark(pull_request: &PullRequest, colors: &Colors) -> (IconName, Hsla) {
    let (icon, state_color) = match pull_request.state {
        PullRequestState::Open => (IconName::GitPullRequest, colors.pr_open),
        PullRequestState::Draft => (IconName::GitPullRequestDraft, colors.muted_foreground),
        PullRequestState::Merged => (IconName::GitMerge, colors.pr_merged),
        PullRequestState::Closed => (IconName::GitPullRequestClosed, colors.status_error),
    };
    let color = match pull_request_problem(pull_request) {
        Some(Problem::ChangesRequested) => colors.warning,
        Some(_) => colors.status_error,
        None => state_color,
    };
    (icon, color)
}

/// What holds an open pull request back, the worst first.
fn pull_request_problem(pull_request: &PullRequest) -> Option<Problem> {
    if pull_request.state != PullRequestState::Open {
        return None;
    }
    if pull_request.conflicts {
        Some(Problem::Conflicts)
    } else if pull_request.checks == Some(ChecksState::Failing) {
        Some(Problem::ChecksFailing)
    } else if pull_request.changes_requested {
        Some(Problem::ChangesRequested)
    } else {
        None
    }
}

/// The tasks a thread delegated, and theirs, which are deleted with it.
fn delegated_tasks(thread_id: &str, threads: &[Thread]) -> usize {
    threads
        .iter()
        .filter(|thread| thread.parent_thread_id.as_deref() == Some(thread_id))
        .map(|task| 1 + delegated_tasks(&task.id, threads))
        .sum()
}

fn cleanup_error_message(error: &CommandError) -> String {
    match error {
        CommandError::WorktreeDirty { .. } => m::cleanup_error_dirty(),
        CommandError::TerminalsOpen { .. } => m::cleanup_error_terminals(),
        CommandError::PullRequestNotMerged { .. } => m::cleanup_error_not_merged(),
        CommandError::UnpushedCommits { .. } => m::cleanup_error_unpushed(),
        CommandError::WorktreeLocked { reason, .. } if !reason.is_empty() => {
            m::cleanup_error_locked_reason(reason)
        }
        CommandError::WorktreeLocked { .. } => m::cleanup_error_locked(),
        CommandError::NothingToClean { .. } => m::cleanup_error_nothing(),
        CommandError::WorktreeAdopted { .. } => m::cleanup_error_adopted(),
        CommandError::ThreadBusy { .. } => m::error_thread_busy(),
        CommandError::GitFailed { detail } => detail.clone(),
        _ => m::error_generic(),
    }
}

fn palette_shortcut() -> String {
    if cfg!(target_os = "macos") {
        m::palette_shortcut_cmd()
    } else {
        m::palette_shortcut_ctrl()
    }
}

fn rail_button(id: impl Into<ElementId>, colors: &Colors) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex_none()
        .size_8()
        .flex()
        .items_center()
        .justify_center()
        .rounded_lg()
        .border_1()
        .border_color(colors.sidebar_border)
        .text_color(colors.sidebar_foreground.opacity(0.8))
        .cursor_pointer()
        .hover(|this| this.bg(colors.sidebar_accent))
}

fn rail_tooltip(tooltip: Tooltip) -> Tooltip {
    tooltip.side(Side::Right).side_offset(px(12.))
}

fn selected(button: Stateful<Div>, colors: &Colors) -> Stateful<Div> {
    button
        .border_color(colors.sidebar_ring)
        .bg(colors.sidebar_accent)
        .text_color(colors.sidebar_accent_foreground)
}

/// The header's backdrop: a few stars and a glow from the top right, as the web's
/// `bg-starfield`.
fn starfield(colors: &Colors) -> impl IntoElement {
    const STARS: [(f32, f32, f32); 6] = [
        (0.12, 0.30, 1.),
        (0.31, 0.74, 1.),
        (0.47, 0.22, 1.5),
        (0.64, 0.66, 1.),
        (0.22, 0.52, 1.),
        (0.79, 0.38, 1.5),
    ];
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .bg(linear_gradient(
            225.,
            linear_color_stop(colors.header_glow, 0.),
            linear_color_stop(colors.header_glow.opacity(0.), 0.7),
        ))
        .children(STARS.map(|(x, y, size)| {
            div()
                .absolute()
                .left(relative(x))
                .top(relative(y))
                .size(px(size))
                .rounded_full()
                .bg(colors.header_star)
        }))
}

type Draw = fn(&mut Sidebar, &Colors, &mut Context<Sidebar>) -> AnyElement;

/// A part of the sidebar drawn as a view of its own.
struct Part {
    sidebar: WeakEntity<Sidebar>,
    draw: Draw,
}

impl Render for Part {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let draw = self.draw;
        let drawn = self
            .sidebar
            .update(cx, |sidebar, cx| draw(sidebar, &colors, cx))
            .ok();
        div().size_full().flex().children(drawn)
    }
}

/// A rail button's signal ring, a view of its own so that its animation redraws only itself
/// and what holds it.
struct SignalRing {
    signal: Signal,
}

impl Render for SignalRing {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        signal_ring(self.signal, &colors, cx)
    }
}

/// A ring in the signal's color that grows from a rail button and fades, again and again, as the
/// web's `animate-signal`. Still when the system asks for less motion.
fn signal_ring(signal: Signal, colors: &Colors, cx: &App) -> AnyElement {
    let ring = div()
        .absolute()
        .top(px(-1.))
        .left(px(-1.))
        .right(px(-1.))
        .bottom(px(-1.))
        .rounded_lg()
        .border_2()
        .border_color(signal_color(signal, colors));
    if cx.reduce_motion() {
        return ring.opacity(0.).into_any_element();
    }
    ring.with_loop(
        hoshi_ui::looping(Duration::from_millis(2400)),
        |ring, delta| {
            let progress = (delta / 0.7).min(1.);
            let eased = 1. - (1. - progress).powi(2);
            let grow = px(-1. - 4.8 * eased);
            ring.top(grow)
                .left(grow)
                .right(grow)
                .bottom(grow)
                .opacity(0.6 * (1. - eased))
        },
    )
    .into_any_element()
}

fn count_badge(count: usize, colors: &Colors) -> impl IntoElement {
    div()
        .absolute()
        .top(px(-6.))
        .right(px(-6.))
        .h(px(14.))
        .min_w(px(14.))
        .px_1()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(colors.foreground)
        .text_color(colors.background)
        .text_size(rems(0.5625))
        .font_weight(FontWeight::SEMIBOLD)
        .child(if count > 9 {
            "9+".to_owned()
        } else {
            count.to_string()
        })
}

fn signal_color(signal: Signal, colors: &Colors) -> Hsla {
    match signal {
        Signal::Error => colors.status_error,
        Signal::Waiting => colors.status_waiting,
        Signal::Done => colors.status_done,
        Signal::Working => colors.status_working,
    }
}

fn signal_label(signal: Signal) -> String {
    match signal {
        Signal::Error => m::status_error(),
        Signal::Waiting => m::status_waiting_for_input(),
        Signal::Done => m::status_completed(),
        Signal::Working => m::status_working(),
    }
}

fn section_label(id: SectionId) -> String {
    match id {
        SectionId::Pinned => m::section_pinned(),
        SectionId::Attention => m::section_attention(),
        SectionId::Active => m::section_active(),
        SectionId::Inactive => m::section_inactive(),
        SectionId::Snoozed => m::section_snoozed(),
        SectionId::Review => m::section_review(),
        SectionId::Reviewing => m::section_reviewing(),
        SectionId::Done => m::section_done(),
        SectionId::Completed => m::status_completed(),
    }
}

fn empty_message(text: String, colors: &Colors) -> AnyElement {
    div()
        .flex_1()
        .p_4()
        .text_center()
        .text_xs()
        .text_color(colors.muted_foreground)
        .child(text)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::prefs::sample_thread;
    use super::{
        MergedCleanup, Problem, branch_mismatch, delegated_tasks, merged_cleanup,
        pull_request_problem,
    };
    use hoshi_core::{PullRequest, PullRequestState};

    fn pull_request(state: PullRequestState) -> PullRequest {
        serde_json::from_value(serde_json::json!({
            "forge": "github",
            "number": 7,
            "title": "Rail",
            "url": "https://example.com/7",
            "headBranch": "feature",
            "baseBranch": "main",
            "state": state,
            "unresolvedComments": 0,
            "openTasks": 0,
            "changesRequested": false,
            "conflicts": false,
            "additions": 0,
            "deletions": 0,
            "updatedAt": "2026-11-13T10:00:00Z"
        }))
        .unwrap()
    }

    #[test]
    fn merged_threads_clean_up_their_worktree_or_their_root() {
        let merged = pull_request(PullRequestState::Merged);
        let mut thread = sample_thread("a");
        thread.worktree_path = Some("/tmp/a".into());
        assert_eq!(
            merged_cleanup(&thread, Some(&merged)),
            Some(MergedCleanup::Worktree)
        );
        thread.worktree_adopted = true;
        assert_eq!(merged_cleanup(&thread, Some(&merged)), None);

        let mut root = sample_thread("b");
        root.branch = Some("feature".into());
        assert_eq!(
            merged_cleanup(&root, Some(&merged)),
            Some(MergedCleanup::Root)
        );
        root.branch = Some("main".into());
        assert_eq!(merged_cleanup(&root, Some(&merged)), None);
        let open = pull_request(PullRequestState::Open);
        assert_eq!(merged_cleanup(&thread, Some(&open)), None);
    }

    #[test]
    fn only_root_threads_on_another_branch_mismatch() {
        let mut thread = sample_thread("a");
        thread.branch = Some("feature".into());
        assert!(branch_mismatch(&thread, Some("main")));
        assert!(!branch_mismatch(&thread, Some("feature")));
        assert!(!branch_mismatch(&thread, None));
        thread.worktree_path = Some("/tmp/a".into());
        assert!(!branch_mismatch(&thread, Some("main")));
    }

    #[test]
    fn delegated_tasks_count_their_own_tasks() {
        let mut task = sample_thread("task");
        task.parent_thread_id = Some("root".into());
        let mut nested = sample_thread("nested");
        nested.parent_thread_id = Some("task".into());
        let threads = vec![sample_thread("root"), task, nested];
        assert_eq!(delegated_tasks("root", &threads), 2);
        assert_eq!(delegated_tasks("nested", &threads), 0);
    }

    #[test]
    fn a_problem_only_holds_back_an_open_pull_request() {
        let mut open = pull_request(PullRequestState::Open);
        open.changes_requested = true;
        assert_eq!(pull_request_problem(&open), Some(Problem::ChangesRequested));
        open.conflicts = true;
        assert_eq!(pull_request_problem(&open), Some(Problem::Conflicts));
        let mut merged = pull_request(PullRequestState::Merged);
        merged.conflicts = true;
        assert_eq!(pull_request_problem(&merged), None);
    }
}
