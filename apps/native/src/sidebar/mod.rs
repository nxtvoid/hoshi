pub(crate) mod cards;
mod prefs;
mod project_settings;
mod snooze;
#[cfg(test)]
mod ui_tests;
mod worktree_setup;

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
    list_scroll: ScrollHandle,
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
            list_scroll: ScrollHandle::new(),
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

    /// Wakes the list when the soonest snooze ends, so the thread leaves Snoozed on time.
    fn schedule_refresh(&mut self, cx: &mut Context<Self>) {
        let now = Utc::now();
        let Some(at) = sections::next_refresh(self.workspace.read(cx).model.threads(), now) else {
            return;
        };
        if self
            .refresh
            .as_ref()
            .is_some_and(|(scheduled, _)| *scheduled <= at)
        {
            return;
        }
        let wait = (at - now).to_std().unwrap_or_default();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                this.refresh = None;
                this.workspace.update(cx, |_, cx| cx.notify());
            })
            .ok();
        });
        self.refresh = Some((at, task));
    }

    /// Writes the prefs a moment after the last change, so a drag writes once.
    fn save_prefs(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        let prefs = self.prefs.clone();
        self.saving = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            cx.background_spawn(async move { prefs::save(&home, &prefs) })
                .await
                .ok();
        }));
    }

    fn set_view(&mut self, view: View, cx: &mut Context<Self>) {
        self.prefs.view = view;
        self.save_prefs(cx);
        self.follow_scope(cx);
        cx.notify();
    }

    /// Picks a project on the rail: its threads, and an empty composer in it, as the web's
    /// `selectProject` leaves the open thread.
    fn select_project(&mut self, project_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.project_id = Some(project_id.clone());
        self.search
            .update(cx, |search, cx| search.set_value("", window, cx));
        self.set_view(View::Project, cx);
        self.workspace.update(cx, |workspace, cx| {
            workspace.start_draft(Some(project_id), cx)
        });
    }

    /// With no thread open, the main view is a new thread in what the rail picked.
    fn follow_scope(&self, cx: &mut Context<Self>) {
        let project = match self.prefs.view {
            View::Chats => None,
            _ => match &self.project_id {
                Some(id) => Some(id.clone()),
                None => return,
            },
        };
        self.workspace.update(cx, |workspace, cx| {
            let current = workspace.draft.as_ref().map(|draft| &draft.project_id);
            if workspace.open.is_none() && current != Some(&project) {
                workspace.start_draft(project, cx);
            }
        });
    }

    fn toggle_section(&mut self, section: SectionId, cx: &mut Context<Self>) {
        if !self.prefs.collapsed.remove(&section) {
            self.prefs.collapsed.insert(section);
        }
        self.save_prefs(cx);
        cx.notify();
    }

    fn ring(&mut self, id: impl Into<SharedString>, signal: Signal, cx: &mut App) -> AnyElement {
        let ring = self
            .rings
            .entry(id.into())
            .or_insert_with(|| {
                cx.new(|_| SignalRing {
                    signal,
                    started: std::time::Instant::now(),
                    next_frame: None,
                })
            })
            .clone();
        if ring.read(cx).signal != signal {
            ring.update(cx, |ring, cx| {
                ring.signal = signal;
                cx.notify();
            });
        }
        ring.into_any_element()
    }

    fn set_width(&mut self, width: f32, cx: &mut Context<Self>) {
        self.prefs.width = prefs::clamp_width(width);
        self.save_prefs(cx);
        cx.notify();
    }

    fn move_project(&mut self, moved: &str, over: &str, cx: &mut Context<Self>) {
        let ids: Vec<String> = self
            .ordered_projects(cx)
            .into_iter()
            .map(|project| project.id)
            .collect();
        self.prefs.project_order = prefs::move_id(&ids, moved, over);
        self.save_prefs(cx);
        cx.notify();
    }

    fn ordered_projects(&self, cx: &App) -> Vec<Project> {
        let mut projects: Vec<Project> =
            self.workspace.read(cx).model.projects().cloned().collect();
        projects.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        prefs::order_by(projects, &self.prefs.project_order, |project| &project.id)
    }

    fn icon(&mut self, project_id: &str, cx: &mut Context<Self>) -> Option<PathBuf> {
        if let Some(found) = self.icons.get(project_id) {
            return found.clone();
        }
        self.icons.insert(project_id.to_owned(), None);
        let engine = self.workspace.read(cx).engine.clone();
        let id = project_id.to_owned();
        cx.spawn(async move |this, cx| {
            let lookup = id.clone();
            let found = cx
                .background_spawn(async move { engine.project_icon_path(&lookup) })
                .await;
            if found.is_some() {
                this.update(cx, |this, cx| {
                    this.icons.insert(id, found);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
        None
    }

    fn new_thread(&mut self, cx: &mut Context<Self>) {
        let no_projects = self.workspace.read(cx).model.projects().next().is_none();
        match (self.prefs.view, &self.project_id) {
            (View::Project, Some(project_id)) => {
                let project_id = Some(project_id.clone());
                self.workspace
                    .update(cx, |workspace, cx| workspace.start_draft(project_id, cx));
            }
            (View::Chats, _) => self.open_new_draft(None, cx),
            _ if no_projects => self.open_new_draft(None, cx),
            _ => cx.emit(SidebarEvent::OpenPalette(palette::Mode::NewThread)),
        }
    }

    /// What the sidebar shows, for the palette to start threads where the web would.
    pub fn scope(&self, cx: &App) -> palette::SidebarScope {
        palette::SidebarScope {
            view: self.prefs.view,
            project_id: self.project_id.clone(),
            project_order: self
                .ordered_projects(cx)
                .into_iter()
                .map(|project| project.id)
                .collect(),
        }
    }

    pub fn view(&self) -> View {
        self.prefs.view
    }

    /// The project's icon, once the rail has looked it up.
    pub fn project_icon(&self, project_id: &str) -> Option<PathBuf> {
        self.icons.get(project_id).cloned().flatten()
    }

    /// Shows `view`, on `project_id` when one is given.
    pub fn show(&mut self, view: View, project_id: Option<String>, cx: &mut Context<Self>) {
        if project_id.is_some() {
            self.project_id = project_id;
        }
        self.set_view(view, cx);
    }

    fn open_thread(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |workspace, cx| workspace.open_thread(thread_id, cx));
    }

    /// The sections the list shows now.
    fn current_sections(&self, cx: &App) -> Vec<Section> {
        let search = self.search.read(cx).value().to_string();
        let workspace = self.workspace.read(cx);
        let threads: Vec<Thread> = workspace.model.threads().cloned().collect();
        sidebar_sections(
            &threads,
            &workspace.pull_requests,
            &Query {
                view: self.prefs.view,
                project_id: self.project_id.as_deref(),
                search: &search,
                filter: self.prefs.filter,
                sort: self.prefs.sort,
                inactive_after: sections::inactive_after(
                    workspace.settings.general.inactive_after_days,
                ),
                logins: &workspace.logins,
                model: &workspace.model,
            },
            Utc::now(),
            &workspace.unseen,
        )
    }

    fn step(&mut self, step: isize, attention: bool, cx: &mut Context<Self>) {
        let order: Vec<String> = if attention {
            let workspace = self.workspace.read(cx);
            let threads: Vec<Thread> = workspace.model.threads().cloned().collect();
            inbox_sections(
                sections::with_delegated_work(&threads),
                &workspace.pull_requests,
                &workspace.model,
                Utc::now(),
                &workspace.unseen,
                &workspace.logins,
            )
            .into_iter()
            .find(|section| section.id == SectionId::Attention)
            .map(|section| {
                section
                    .threads
                    .into_iter()
                    .map(|thread| thread.id)
                    .collect()
            })
            .unwrap_or_default()
        } else {
            let searching = !self.search.read(cx).value().is_empty();
            prefs::visible_order(
                &self.current_sections(cx),
                |section| searching || !self.prefs.collapsed.contains(&section),
                self.shown_thread.as_deref(),
            )
        };
        if let Some(next) = prefs::step_thread(&order, self.shown_thread.as_deref(), step) {
            self.open_thread(&next, cx);
        }
    }

    fn dispatch(&self, command: Command, cx: &mut Context<Self>) {
        let engine = self.workspace.read(cx).engine.clone();
        cx.background_spawn(async move {
            if let Err(error) = engine.dispatch(command) {
                eprintln!("hoshi: sidebar command failed: {error:?}");
            }
        })
        .detach();
    }

    fn open_menu(&mut self, target: MenuTarget, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.menu_serial += 1;
        self.cards.close();
        self.menu = Some(OpenMenu {
            serial: self.menu_serial,
            target,
            position,
        });
        cx.notify();
    }

    fn thread_action(
        &mut self,
        thread_id: String,
        action: ThreadAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            ThreadAction::Pin(pinned) => {
                self.dispatch(Command::ThreadPin { thread_id, pinned }, cx)
            }
            ThreadAction::Wake => self.wake_thread(thread_id, window, cx),
            ThreadAction::Archive(archived) => self.dispatch(
                Command::ThreadArchive {
                    thread_id,
                    archived,
                },
                cx,
            ),
            ThreadAction::RegenerateTitle => {
                let engine = self.workspace.read(cx).engine.clone();
                cx.background_spawn(async move { engine.regenerate_title(&thread_id) })
                    .detach();
            }
            ThreadAction::Fork => crate::fork::fork_thread(
                &self.workspace,
                thread_id,
                hoshi_core::ForkPoint::LatestStable,
                window,
                cx,
            ),
            ThreadAction::CleanUp | ThreadAction::Release => {
                let release = matches!(action, ThreadAction::Release);
                let engine = self.workspace.read(cx).engine.clone();
                cx.spawn_in(window, async move |_, cx| {
                    let result = cx
                        .background_spawn(async move {
                            if release {
                                engine.release_worktree(&thread_id)
                            } else {
                                engine.clean_up_thread(&thread_id)
                            }
                        })
                        .await;
                    if let Err(error) = result {
                        let title = if release {
                            m::release_worktree()
                        } else {
                            m::clean_up_thread()
                        };
                        cx.update(|window, cx| {
                            DialogLayer::confirm(
                                ConfirmRequest::new(title, m::dialog_ok())
                                    .description(cleanup_error_message(&error))
                                    .tone(ConfirmTone::Warning),
                                window,
                                cx,
                            )
                        })
                        .map(|answer| answer.detach())
                        .ok();
                    }
                })
                .detach();
            }
            ThreadAction::DiscardDraft => self.workspace.update(cx, |workspace, cx| {
                workspace.forget_drafts(&[thread_id], cx)
            }),
            ThreadAction::Delete => {
                let workspace = self.workspace.read(cx);
                let Some(thread) = workspace.model.thread(&thread_id) else {
                    return;
                };
                let title = if thread.title.is_empty() {
                    m::untitled_thread()
                } else {
                    thread.title.clone()
                };
                let threads: Vec<Thread> = workspace.model.threads().cloned().collect();
                let tasks = delegated_tasks(&thread_id, &threads);
                let description = if tasks > 0 {
                    m::delete_thread_with_tasks_confirm(tasks, &title)
                } else {
                    m::delete_thread_confirm(&title)
                };
                let answer = DialogLayer::confirm(
                    ConfirmRequest::new(m::delete_thread(), m::delete_thread())
                        .description(description)
                        .cancel_label(m::cancel())
                        .destructive(),
                    window,
                    cx,
                );
                cx.spawn(async move |this, cx| {
                    if answer.await {
                        this.update(cx, |this, cx| {
                            this.dispatch(Command::ThreadDelete { thread_id }, cx)
                        })
                        .ok();
                    }
                })
                .detach();
            }
        }
    }

    fn remove_project(&mut self, project_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.workspace.read(cx).model.project(&project_id) else {
            return;
        };
        let answer = DialogLayer::confirm(
            ConfirmRequest::new(m::remove_project(), m::remove_project())
                .description(m::remove_project_confirm(&project.title))
                .cancel_label(m::cancel())
                .destructive(),
            window,
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await {
                this.update(cx, |this, cx| {
                    this.dispatch(Command::ProjectRemove { project_id }, cx)
                })
                .ok();
            }
        })
        .detach();
    }

    fn edit_setup(&mut self, project_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.workspace.read(cx);
        let Some(project) = workspace.model.project(project_id).cloned() else {
            return;
        };
        let engine = workspace.engine.clone();
        let form = cx.new(|cx| WorktreeSetupForm::new(project, engine, window, cx));
        let subscription = cx.subscribe(
            &form,
            |this, _, event: &WorktreeSetupEvent, cx| match event {
                WorktreeSetupEvent::Close => {
                    this.setup = None;
                    cx.notify();
                }
            },
        );
        self.setup = Some((form, subscription));
        cx.notify();
    }

    /// Cleans up every merged thread of a section that still has a worktree, one after another.
    /// Each one is checked on its own; those that would lose work are kept and listed after.
    fn clean_up_merged(
        &mut self,
        threads: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = threads.len();
        let label = m::cleanup_merged(count);
        let answer = DialogLayer::confirm(
            ConfirmRequest::new(label.clone(), m::clean_up_thread())
                .description(m::cleanup_merged_confirm(count))
                .cancel_label(m::cancel())
                .destructive(),
            window,
            cx,
        );
        let engine = self.workspace.read(cx).engine.clone();
        cx.spawn_in(window, async move |this, cx| {
            if !answer.await {
                return;
            }
            this.update(cx, |this, cx| {
                this.cleaning_merged = true;
                cx.notify();
            })
            .ok();
            let failures = cx
                .background_spawn(async move {
                    threads
                        .iter()
                        .filter_map(|id| engine.clean_up_thread(id).err())
                        .map(|error| cleanup_error_message(&error))
                        .collect::<Vec<_>>()
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.cleaning_merged = false;
                cx.notify();
                if !failures.is_empty() {
                    let reasons: BTreeSet<String> = failures.iter().cloned().collect();
                    DialogLayer::confirm(
                        ConfirmRequest::new(label, m::dialog_ok())
                            .description(m::cleanup_merged_failed(
                                failures.len(),
                                reasons.into_iter().collect::<Vec<_>>().join(" "),
                            ))
                            .tone(ConfirmTone::Warning),
                        window,
                        cx,
                    )
                    .detach();
                }
            })
            .ok();
        })
        .detach();
    }

    fn header(&mut self, colors: &Colors, cx: &mut Context<Self>) -> impl IntoElement {
        let has_projects = self.workspace.read(cx).model.projects().next().is_some();
        let searching = !self.search.read(cx).value().is_empty();
        div()
            .flex()
            .flex_col()
            .flex_none()
            .relative()
            .px_2()
            .pb(px(10.))
            .border_b_1()
            .border_color(colors.sidebar_border)
            .bg(colors.header)
            .child(starfield(colors))
            .child(
                div()
                    .h_10()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("toggle-sidebar")
                            .icon(IconName::PanelLeft)
                            .size(ButtonSize::IconSm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip(Tooltip::new(m::toggle_sidebar()).side(Side::Bottom))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::Toggle))),
                    )
                    .child(
                        div()
                            .id("sidebar-drag")
                            .drag_region()
                            .flex_1()
                            .h_full()
                            .flex()
                            .items_center()
                            .px_1()
                            .child(
                                svg()
                                    .path(brand::WORDMARK)
                                    .h(px(12.))
                                    .w(px(39.))
                                    .text_color(colors.sidebar_foreground),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .id("search-threads")
                            .flex_1()
                            .min_w_0()
                            .h_8()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .rounded_lg()
                            .text_sm()
                            .text_color(colors.sidebar_foreground.opacity(0.7))
                            .hover(|this| this.bg(colors.sidebar_accent.opacity(0.6)))
                            .child(Icon::new(IconName::Search).size_4().flex_none())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&self.search).appearance(false)),
                            )
                            .when(!searching, |this| {
                                this.child(
                                    div()
                                        .id("palette-shortcut")
                                        .flex_none()
                                        .cursor_pointer()
                                        .on_click(cx.listener(|_, _, _, cx| {
                                            cx.emit(SidebarEvent::OpenPalette(
                                                palette::Mode::Commands,
                                            ))
                                        }))
                                        .child(Kbd::new(palette_shortcut()))
                                        .with_tooltip(
                                            Tooltip::new(m::palette_label()).side(Side::Bottom),
                                        ),
                                )
                            }),
                    )
                    .child(self.sort_menu(cx))
                    .child(
                        Button::new("add-project")
                            .icon(IconName::FolderPlus)
                            .size(ButtonSize::IconSm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip(Tooltip::new(m::add_project()).side(Side::Bottom))
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(SidebarEvent::OpenPalette(palette::Mode::AddProject))
                            })),
                    )
                    .child(
                        Button::new("new-thread")
                            .icon(IconName::SquarePen)
                            .size(ButtonSize::IconSm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip(Tooltip::new(m::new_thread()).side(Side::Bottom))
                            .disabled(!has_projects)
                            .on_click(cx.listener(|this, _, _, cx| this.new_thread(cx))),
                    ),
            )
    }

    fn sort_menu(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        let (sort, filter) = (self.prefs.sort, self.prefs.filter);
        let narrowing = (filter != Filter::All).then_some(cx.hoshi().sidebar_primary);
        Popover::new("sort-menu")
            .align(Align::End)
            .w(px(176.))
            .open(self.sort_open)
            .on_open_change(cx.listener(|this, change: &PopoverChange, _, cx| {
                this.sort_open = change.open;
                cx.notify();
            }))
            .trigger(move |open| {
                Button::new("sort-trigger")
                    .icon(IconName::SlidersHorizontal)
                    .size(ButtonSize::IconSm)
                    .variant(ButtonVariant::Ghost)
                    .selected(open)
                    .when_some(narrowing, |button, color| button.text_color(color))
                    .tooltip(Tooltip::new(m::filter_threads()).side(Side::Bottom))
            })
            .menu(move |menu, _, _| {
                let sorts = [
                    (Sort::Activity, m::sort_by_activity()),
                    (Sort::Created, m::sort_by_created()),
                ]
                .into_iter()
                .enumerate()
                .map(|(index, (option, label))| {
                    let this = this.clone();
                    MenuOption::radio(("sort-option", index), label, option == sort).on_select(
                        move |_, cx| {
                            this.update(cx, |this, cx| {
                                this.prefs.sort = option;
                                this.save_prefs(cx);
                                cx.notify();
                            })
                            .ok();
                        },
                    )
                });
                let filters = [
                    (Filter::All, m::filter_all_threads()),
                    (Filter::Worktrees, m::filter_worktrees()),
                    (Filter::PullRequests, m::filter_with_pull_request()),
                ]
                .into_iter()
                .enumerate()
                .map(|(index, (option, label))| {
                    let this = this.clone();
                    MenuOption::radio(("filter-option", index), label, option == filter).on_select(
                        move |_, cx| {
                            this.update(cx, |this, cx| {
                                this.prefs.filter = option;
                                this.save_prefs(cx);
                                cx.notify();
                            })
                            .ok();
                        },
                    )
                });
                menu.heading(m::sort_threads())
                    .items(sorts)
                    .separator()
                    .heading(m::show_threads())
                    .items(filters)
            })
    }

    fn shows_rail(&self, cx: &App) -> bool {
        let model = &self.workspace.read(cx).model;
        model.projects().next().is_some()
            || model.threads().any(|thread| thread.project_id.is_none())
    }

    fn rail(&mut self, colors: &Colors, cx: &mut Context<Self>) -> AnyElement {
        let projects = self.ordered_projects(cx);
        let workspace = self.workspace.read(cx);
        let threads: Vec<Thread> = workspace.model.threads().cloned().collect();
        let has_chats = threads.iter().any(|thread| thread.project_id.is_none());
        if projects.is_empty() && !has_chats {
            return div().into_any_element();
        }
        let unseen = workspace.unseen.clone();
        let inbox = inbox_sections(
            sections::with_delegated_work(&threads),
            &workspace.pull_requests,
            &workspace.model,
            Utc::now(),
            &unseen,
            &workspace.logins,
        );
        let inbox_count = sections::inbox_count(&inbox);
        let inbox_waiting = inbox
            .iter()
            .any(|section| section.id == SectionId::Attention);
        let open_project = self
            .shown_thread
            .as_deref()
            .and_then(|id| workspace.model.thread(id)?.project_id.clone());
        let view = self.prefs.view;
        let now = Utc::now();
        let awake: Vec<Thread> = threads
            .iter()
            .filter(|thread| !is_snoozed(&workspace.model, thread, now))
            .cloned()
            .collect();

        let inbox_label = if inbox_count > 0 {
            m::inbox_waiting(inbox_count)
        } else {
            m::inbox()
        };
        let inbox_signal = (inbox_count > 0).then_some(if inbox_waiting {
            Signal::Waiting
        } else {
            Signal::Done
        });
        let inbox_ring = inbox_signal.map(|signal| self.ring("signal-inbox", signal, cx));
        let inbox_button = rail_button("rail-inbox", colors)
            .when(view == View::Inbox, |this| selected(this, colors))
            .when_some(inbox_signal, |this, signal| {
                this.border_color(signal_color(signal, colors))
            })
            .on_click(cx.listener(|this, _, _, cx| {
                let view = if this.prefs.view == View::Inbox {
                    View::Project
                } else {
                    View::Inbox
                };
                this.set_view(view, cx)
            }))
            .children(inbox_ring)
            .child(Icon::new(IconName::Inbox).size_4())
            .when(inbox_count > 0, |this| {
                this.child(count_badge(inbox_count, colors))
            })
            .with_tooltip(rail_tooltip(Tooltip::new(inbox_label)));

        let chats_signal = project_signal(&awake, None, &unseen);
        let chats_ring = chats_signal.map(|signal| self.ring("signal-chats", signal, cx));
        let chats_button = rail_button("rail-chats", colors)
            .when(view == View::Chats, |this| selected(this, colors))
            .when_some(chats_signal, |this, signal| {
                this.border_color(signal_color(signal, colors))
            })
            .on_click(cx.listener(|this, _, _, cx| {
                if this.prefs.view == View::Chats {
                    this.set_view(View::Project, cx)
                } else {
                    this.open_new_draft(None, cx)
                }
            }))
            .children(chats_ring)
            .child(Icon::new(IconName::MessageCircle).size_4())
            .with_tooltip(rail_tooltip(Tooltip::new(m::chats())));

        let in_project = view == View::Project;
        let buttons: Vec<AnyElement> = projects
            .into_iter()
            .map(|project| {
                let signal = project_signal(&awake, Some(&project.id), &unseen);
                let ring =
                    signal.map(|signal| self.ring(format!("signal-{}", project.id), signal, cx));
                let icon = self.icon(&project.id, cx);
                let chosen = in_project && self.project_id.as_deref() == Some(project.id.as_str());
                let holds_open =
                    !in_project && open_project.as_deref() == Some(project.id.as_str());
                let initials = initials(&project.title);
                let (title, root) = (project.title.clone(), project.workspace_root.clone());
                let status = signal.map(signal_label);
                let id = project.id.clone();
                let drag = ProjectDrag {
                    id: project.id.clone(),
                    initials: initials.clone(),
                    icon: icon.clone(),
                };
                let (dropped_on, menu_for) = (project.id.clone(), project.id.clone());
                let selector = format!("rail-{}", project.id);
                rail_button(SharedString::from(selector.clone()), colors)
                    .debug_selector(move || selector.clone())
                    .text_size(rems(0.6875))
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(chosen, |this| selected(this, colors))
                    .when(holds_open, |this| {
                        this.bg(colors.sidebar_accent.opacity(0.6))
                    })
                    .when_some(signal, |this, signal| {
                        this.border_color(signal_color(signal, colors))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_project(id.clone(), window, cx)
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.open_menu(
                                MenuTarget::Project(menu_for.clone()),
                                event.position,
                                cx,
                            )
                        }),
                    )
                    .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
                    .drag_over::<ProjectDrag>({
                        let ring = colors.sidebar_ring;
                        move |style, _, _, _| style.border_color(ring)
                    })
                    .on_drop(cx.listener(move |this, drag: &ProjectDrag, _, cx| {
                        this.move_project(&drag.id, &dropped_on, cx)
                    }))
                    .children(ring)
                    .child(project_mark(icon, initials, px(20.), px(5.)))
                    .with_tooltip(rail_tooltip(Tooltip::element(move |_, _| {
                        div()
                            .flex()
                            .flex_col()
                            .child(div().font_weight(FontWeight::MEDIUM).child(title.clone()))
                            .child(div().opacity(0.7).child(root.clone()))
                            .children(status.clone())
                    })))
                    .into_any_element()
            })
            .collect();
        let has_projects = !buttons.is_empty();

        div()
            .id("project-rail")
            .track_scroll(&self.rail_scroll)
            .w_12()
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.))
            .py_2()
            .border_r_1()
            .border_color(colors.sidebar_border)
            .overflow_y_scroll()
            .child(inbox_button)
            .child(chats_button)
            .when(has_projects, |this| {
                this.child(
                    div()
                        .flex_none()
                        .my(px(2.))
                        .h_px()
                        .w_6()
                        .bg(colors.sidebar_border),
                )
            })
            .children(buttons)
            .into_any_element()
    }

    fn list(&mut self, colors: &Colors, cx: &mut Context<Self>) -> AnyElement {
        let search = self.search.read(cx).value().to_string();
        let view = self.prefs.view;
        let workspace = self.workspace.read(cx);
        let has_projects = workspace.model.projects().next().is_some();
        if !has_projects && view != View::Chats {
            return div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .gap_3()
                .p_4()
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(m::no_projects()),
                )
                .child(
                    Button::new("add-first-project")
                        .label(m::add_project())
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(SidebarEvent::OpenPalette(palette::Mode::AddProject))
                        })),
                )
                .into_any_element();
        }
        let project = self
            .project_id
            .as_deref()
            .and_then(|id| workspace.model.project(id))
            .cloned();
        if project.is_none() && view == View::Project {
            return empty_message(m::no_project_selected(), colors);
        }

        let project_titles: HashMap<String, String> = workspace
            .model
            .projects()
            .map(|project| (project.id.clone(), project.title.clone()))
            .collect();
        let now = Utc::now();
        let snoozed: HashMap<String, String> = workspace
            .model
            .threads()
            .filter(|thread| is_snoozed(&workspace.model, thread, now))
            .filter_map(|thread| Some((thread.id.clone(), thread.snoozed_until.clone()?)))
            .collect();
        let woke: HashSet<String> = workspace
            .model
            .threads()
            .filter(|thread| shows_woke(&workspace.model, thread, now))
            .map(|thread| thread.id.clone())
            .collect();
        let rows = RowContext {
            open: self.shown_thread.clone(),
            snoozed,
            woke,
            show_project: view == View::Inbox,
            project_titles,
            pull_requests: workspace.pull_requests.clone(),
            unseen: workspace.unseen.clone(),
            root_branches: workspace.project_branches.clone(),
            drafts: workspace.drafts.keys().cloned().collect(),
        };
        let lone_project = (rows.project_titles.len() == 1)
            .then(|| {
                rows.project_titles
                    .iter()
                    .next()
                    .map(|(id, title)| (id.clone(), title.clone()))
            })
            .flatten();
        let sections = self.current_sections(cx);

        let heading = match view {
            View::Inbox => self.view_heading(IconName::Inbox, m::inbox(), lone_project, colors, cx),
            View::Chats => self.view_heading(
                IconName::MessageCircle,
                m::chats(),
                lone_project,
                colors,
                cx,
            ),
            View::Project => self.project_heading(project.as_ref(), colors, cx),
        };

        let inbox = view == View::Inbox;
        let narrowed = !search.is_empty() || self.prefs.filter != Filter::All;
        let only_active = !inbox && sections.len() == 1 && sections[0].id == SectionId::Active;
        let body = if sections.is_empty() {
            empty_message(
                if narrowed {
                    m::no_matching_threads()
                } else if inbox {
                    m::inbox_empty()
                } else if view == View::Chats {
                    m::no_chats()
                } else {
                    m::no_threads()
                },
                colors,
            )
        } else {
            let force_open = !search.is_empty();
            div()
                .id("thread-sections")
                .track_scroll(&self.list_scroll)
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .gap_1()
                .px(px(6.))
                .pb_2()
                .overflow_y_scroll()
                .children(sections.into_iter().map(|section| {
                    self.section(section, !only_active, force_open, &rows, colors, cx)
                }))
                .into_any_element()
        };

        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .pt(px(6.))
            .child(heading)
            .children(self.new_thread_drafts(colors, cx))
            .child(body)
            .into_any_element()
    }

    /// New threads with unsent text: this project's or the chats' one, or every one in the inbox.
    /// The draft on screen is left out.
    fn new_thread_drafts(&mut self, colors: &Colors, cx: &mut Context<Self>) -> Option<AnyElement> {
        let workspace = self.workspace.read(cx);
        let shown = workspace
            .open
            .is_none()
            .then(|| workspace.draft_key(cx))
            .flatten();
        let mut keys: Vec<String> = match self.prefs.view {
            View::Inbox => workspace
                .drafts
                .keys()
                .filter(|key| drafts::new_thread_project(key).is_some())
                .cloned()
                .collect(),
            View::Chats => vec![drafts::key(None, None)],
            View::Project => vec![drafts::key(None, self.project_id.as_deref())],
        };
        keys.sort();
        let rows: Vec<(String, Option<Project>)> = keys
            .into_iter()
            .filter(|key| workspace.drafts.contains_key(key) && Some(key) != shown.as_ref())
            .filter_map(|key| match drafts::new_thread_project(&key)? {
                None => Some((key, None)),
                Some(project_id) => {
                    let project = workspace.model.project(project_id)?.clone();
                    Some((key, Some(project)))
                }
            })
            .collect();
        if rows.is_empty() {
            return None;
        }
        let inbox = self.prefs.view == View::Inbox;
        let rows: Vec<AnyElement> = rows
            .into_iter()
            .map(|(key, project)| {
                let label = match &project {
                    None => m::new_chat(),
                    Some(project) if inbox => project.title.clone(),
                    Some(_) => m::new_thread(),
                };
                let group = SharedString::from(format!("draft-{key}"));
                let (opening, menu_for, discarding) = (project.clone(), key.clone(), key.clone());
                div()
                    .id(SharedString::from(format!("draft-row-{key}")))
                    .debug_selector({
                        let key = key.clone();
                        move || format!("draft-row-{key}")
                    })
                    .group(group.clone())
                    .relative()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py(px(6.))
                    .rounded_md()
                    .cursor_pointer()
                    .bg(colors.draft_surface)
                    .hover(|this| this.bg(colors.draft.opacity(0.1)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_new_draft(opening.as_ref().map(|project| project.id.clone()), cx)
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.open_menu(MenuTarget::Draft(menu_for.clone()), event.position, cx)
                        }),
                    )
                    .child(
                        Icon::new(IconName::SquarePen)
                            .size(px(14.))
                            .flex_none()
                            .text_color(colors.draft),
                    )
                    .child(div().flex_1().min_w_0().truncate().text_sm().child(label))
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .italic()
                            .text_color(colors.draft)
                            .group_hover(group.clone(), |style| style.opacity(0.))
                            .child(m::draft()),
                    )
                    .child(
                        div()
                            .absolute()
                            .right_1()
                            .opacity(0.)
                            .group_hover(group, |style| style.opacity(1.))
                            .child(
                                Button::new(SharedString::from(format!("discard-{key}")))
                                    .icon(IconName::X)
                                    .size(ButtonSize::IconXs)
                                    .variant(ButtonVariant::Ghost)
                                    .tooltip(
                                        Tooltip::new(m::discard_draft())
                                            .side(Side::Right)
                                            .side_offset(px(15.)),
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let key = discarding.clone();
                                        this.workspace.update(cx, |workspace, cx| {
                                            workspace.forget_drafts(&[key], cx)
                                        });
                                    })),
                            ),
                    )
                    .into_any_element()
            })
            .collect();
        Some(
            div()
                .flex()
                .flex_col()
                .gap_px()
                .px(px(6.))
                .children(rows)
                .child(div().mx_2().my(px(6.)).h_px().bg(colors.sidebar_border))
                .into_any_element(),
        )
    }

    /// Shows the draft of a new thread in `project_id`, or of a new chat.
    fn open_new_draft(&mut self, project_id: Option<String>, cx: &mut Context<Self>) {
        match &project_id {
            Some(id) => {
                self.project_id = Some(id.clone());
                self.prefs.view = View::Project;
            }
            None => self.prefs.view = View::Chats,
        }
        self.save_prefs(cx);
        self.workspace
            .update(cx, |workspace, cx| workspace.start_draft(project_id, cx));
        cx.notify();
    }

    fn view_heading(
        &mut self,
        icon: IconName,
        title: String,
        lone_project: Option<(String, String)>,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let has_projects = self.workspace.read(cx).model.projects().next().is_some();
        div()
            .h_8()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl_2()
            .pr_1()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .when(has_projects, |this| {
                let back = lone_project
                    .as_ref()
                    .map(|(_, title)| title.clone())
                    .unwrap_or_else(m::projects);
                let lone = lone_project.as_ref().map(|(id, _)| id.clone());
                this.child(
                    div()
                        .id("back-to-project")
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .min_w_0()
                        .max_w(relative(0.5))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(colors.muted_foreground)
                        .cursor_pointer()
                        .hover(|this| this.text_color(colors.foreground))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(lone) = &lone {
                                this.project_id = Some(lone.clone());
                            }
                            this.set_view(View::Project, cx)
                        }))
                        .child(Icon::new(IconName::ArrowLeft).size(px(14.)).flex_none())
                        .child(div().truncate().child(back)),
                )
                .child(
                    div()
                        .font_weight(FontWeight::NORMAL)
                        .text_color(colors.muted_foreground.opacity(0.6))
                        .child("/"),
                )
            })
            .when(!has_projects, |this| {
                this.child(
                    Icon::new(icon)
                        .size(px(14.))
                        .text_color(colors.muted_foreground),
                )
            })
            .child(div().truncate().child(title))
            .into_any_element()
    }

    fn project_heading(
        &mut self,
        project: Option<&Project>,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(project) = project else {
            return div().into_any_element();
        };
        let icon = self.icon(&project.id, cx);
        let this = cx.weak_entity();
        let id = project.id.clone();
        div()
            .h_8()
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .pl_2()
            .pr_1()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(match icon {
                        Some(path) => img(path)
                            .size(px(14.))
                            .rounded(px(3.))
                            .object_fit(ObjectFit::Contain)
                            .into_any_element(),
                        None => Icon::new(IconName::Folder)
                            .size(px(14.))
                            .text_color(colors.muted_foreground)
                            .into_any_element(),
                    })
                    .child(div().truncate().child(project.title.clone())),
            )
            .child(
                Popover::new("project-actions")
                    .align(Align::End)
                    .w(px(192.))
                    .trigger(|open| {
                        Button::new("project-actions-trigger")
                            .icon(IconName::Ellipsis)
                            .size(ButtonSize::IconXs)
                            .variant(ButtonVariant::Ghost)
                            .selected(open)
                            .tooltip(m::project_actions())
                    })
                    .menu(move |menu, _, _| project_menu(menu, this, id)),
            )
            .into_any_element()
    }

    fn section(
        &mut self,
        section: Section,
        show_header: bool,
        force_open: bool,
        rows: &RowContext,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = section.id;
        let open = force_open || !self.prefs.collapsed.contains(&id);
        let count = section.threads.len();
        let merged: Vec<String> = if id == SectionId::Done {
            section
                .threads
                .iter()
                .filter(|thread| {
                    thread.status != ThreadStatus::Working
                        && merged_cleanup(thread, rows.pull_requests.get(&thread.id)).is_some()
                })
                .map(|thread| thread.id.clone())
                .collect()
        } else {
            Vec::new()
        };
        let shown: Vec<Thread> = section
            .threads
            .into_iter()
            .filter(|thread| open || rows.open.as_deref() == Some(thread.id.as_str()))
            .collect();
        div()
            .flex()
            .flex_col()
            .when(show_header, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .id(SharedString::from(format!("section-{id:?}")))
                                .debug_selector(move || format!("section-{id:?}"))
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap_1()
                                .px_2()
                                .pt_2()
                                .pb_1()
                                .rounded_md()
                                .text_size(rems(0.6875))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors.muted_foreground)
                                .when(!force_open, |this| {
                                    this.cursor_pointer()
                                        .hover(|this| this.text_color(colors.sidebar_foreground))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.toggle_section(id, cx)
                                        }))
                                })
                                .child(
                                    Icon::new(if open {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronRight
                                    })
                                    .size_3(),
                                )
                                .child(section_label(id).to_uppercase())
                                .child(
                                    div()
                                        .ml_auto()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(count.to_string()),
                                ),
                        )
                        .when(!merged.is_empty(), |this| {
                            let label = m::cleanup_merged(merged.len());
                            let running = self.cleaning_merged;
                            this.child(
                                div()
                                    .id("clean-up-merged")
                                    .mt_1()
                                    .mr_1()
                                    .size_5()
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.))
                                    .text_color(colors.muted_foreground)
                                    .when(running, |this| this.opacity(0.5))
                                    .when(!running, |this| {
                                        this.cursor_pointer()
                                            .hover(|this| {
                                                this.bg(colors.sidebar_accent)
                                                    .text_color(colors.sidebar_foreground)
                                            })
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.clean_up_merged(merged.clone(), window, cx)
                                            }))
                                    })
                                    .child(Icon::new(IconName::Brush).size(px(14.)))
                                    .with_tooltip(Tooltip::new(label).side(Side::Right)),
                            )
                        }),
                )
            })
            .child(
                div().flex().flex_col().gap_px().children(
                    shown
                        .into_iter()
                        .map(|thread| self.row(thread, rows, colors, cx)),
                ),
            )
            .into_any_element()
    }

    fn row(
        &mut self,
        thread: Thread,
        rows: &RowContext,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = rows.open.as_deref() == Some(thread.id.as_str());
        let status = match thread.status {
            ThreadStatus::Working => Some((m::status_working(), colors.status_working)),
            ThreadStatus::WaitingForInput => {
                Some((m::status_waiting_for_input(), colors.status_waiting))
            }
            ThreadStatus::Error => Some((m::status_error(), colors.status_error)),
            ThreadStatus::Idle if rows.unseen.contains(&thread.id) => {
                Some((m::status_completed(), colors.status_done))
            }
            ThreadStatus::Idle => None,
        };
        let project = rows.show_project.then(|| match &thread.project_id {
            None => m::chat(),
            Some(id) => rows.project_titles.get(id).cloned().unwrap_or_default(),
        });
        let pull_request = rows.pull_requests.get(&thread.id).cloned();
        let root_branch = thread
            .project_id
            .as_ref()
            .and_then(|id| rows.root_branches.get(id).cloned().flatten());
        let mismatch = branch_mismatch(&thread, root_branch.as_deref());
        let title = if thread.title.is_empty() {
            m::untitled_thread()
        } else {
            thread.title.clone()
        };
        let second_line = thread.branch.is_some() || project.is_some() || pull_request.is_some();
        let mono = cx.theme().mono_font_family.clone();
        let (clicked, menu_for, hovered) =
            (thread.id.clone(), thread.id.clone(), thread.id.clone());
        let draft = !selected && rows.drafts.contains(&thread.id);
        let snoozed = rows
            .snoozed
            .get(&thread.id)
            .map(|until| crate::snooze::left_text(snooze_left(until, Utc::now())));
        let woke = snoozed.is_none() && status.is_none() && rows.woke.contains(&thread.id);
        let group = SharedString::from(format!("thread-row-{}", thread.id));
        let hover_action = (!thread.archived && !woke)
            .then(|| self.row_snooze_button(&thread.id, snoozed.is_some(), colors, cx));
        let trailing = match (snoozed, status) {
            (Some(left), _) => div()
                .flex_none()
                .flex()
                .items_center()
                .gap_1()
                .text_xs()
                .text_color(colors.status_working)
                .child(Icon::new(IconName::AlarmClock).size_3())
                .child(left)
                .into_any_element(),
            (None, Some((label, color))) => div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_xs()
                .text_color(color)
                .child(div().size(px(6.)).rounded_full().bg(color))
                .child(label)
                .into_any_element(),
            (None, None) if woke => self.woke_chip(&thread.id, colors, cx),
            (None, None) if draft => div()
                .flex_none()
                .text_xs()
                .italic()
                .text_color(colors.draft)
                .child(m::draft())
                .into_any_element(),
            (None, None) => div()
                .flex_none()
                .text_xs()
                .text_color(colors.muted_foreground)
                .child(relative_time(&thread.updated_at, Utc::now()))
                .into_any_element(),
        };
        let trailing = div()
            .flex_none()
            .when(hover_action.is_some(), |this| {
                this.group_hover(group.clone(), |style| style.opacity(0.))
            })
            .child(trailing);

        let selector = format!("thread-{}", thread.id);
        let row = div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .group(group.clone())
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px_2()
            .py(px(6.))
            .rounded_md()
            .cursor_pointer()
            .text_color(colors.sidebar_foreground)
            .hover(|this| this.bg(colors.sidebar_accent.opacity(0.6)))
            .when(selected, |this| {
                this.bg(colors.sidebar_accent)
                    .text_color(colors.sidebar_accent_foreground)
            })
            .on_hover(
                cx.listener(move |this, over: &bool, _, cx| this.hover_row(&hovered, *over, cx)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.quiet_card(&clicked, cx);
                this.open_thread(&clicked, cx)
            }))
            .child(anchor(&self.cards, CardTarget::Thread(thread.id.clone())))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.open_menu(MenuTarget::Thread(menu_for.clone()), event.position, cx)
                }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .child(
                        svg()
                            .path(crate::composer::provider_mark(&thread.provider))
                            .size(px(14.))
                            .flex_none()
                            .text_color(crate::composer::provider_tint(&thread.provider, colors)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .when(thread.archived, |this| {
                                this.text_color(colors.muted_foreground)
                            })
                            .child(title),
                    )
                    .when(thread.forked_from.is_some(), |this| {
                        let selector = format!("fork-mark-{}", thread.id);
                        this.child(
                            div()
                                .debug_selector(move || selector.clone())
                                .flex_none()
                                .text_color(colors.muted_foreground)
                                .child(Icon::new(IconName::GitFork).size_3()),
                        )
                    })
                    .child(trailing),
            )
            .when_some(hover_action, |this, action| {
                this.child(
                    div()
                        .absolute()
                        .top(px(4.))
                        .right_1()
                        .opacity(0.)
                        .group_hover(group, |style| style.opacity(1.))
                        .child(action),
                )
            })
            .when(second_line, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .min_w_0()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .when_some(project, |this, project| {
                            this.child(
                                div()
                                    .flex_none()
                                    .max_w(relative(0.45))
                                    .truncate()
                                    .text_color(colors.sidebar_foreground.opacity(0.8))
                                    .child(project),
                            )
                        })
                        .when_some(thread.branch.clone(), |this, branch| {
                            this.child(
                                Icon::new(if thread.worktree_path.is_some() {
                                    IconName::GitFork
                                } else {
                                    IconName::GitBranch
                                })
                                .size_3()
                                .flex_none()
                                .when(thread.worktree_released, |icon| icon.opacity(0.4)),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(rems(0.6875))
                                    .font_family(mono.clone())
                                    .child(branch),
                            )
                        })
                        .when_some(pull_request, |this, pull_request| {
                            this.child(self.pull_request_pill(
                                &thread.id,
                                &pull_request,
                                colors,
                                cx,
                            ))
                        }),
                )
            })
            .when(mismatch, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .min_w_0()
                        .text_xs()
                        .text_color(colors.warning)
                        .child(Icon::new(IconName::CircleAlert).size_3().flex_none())
                        .child(div().truncate().child(m::root_on_other_branch())),
                )
            });

        row.into_any_element()
    }

    /// On a hovered row: wake it when snoozed, or open its snooze choices.
    fn row_snooze_button(
        &self,
        thread_id: &str,
        snoozed: bool,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = thread_id.to_owned();
        let button = Button::new(SharedString::from(format!("snooze-row-{thread_id}")))
            .size(ButtonSize::IconXs)
            .variant(ButtonVariant::Ghost);
        let button =
            if snoozed {
                button
                    .icon(IconName::AlarmClockOff)
                    .text_color(colors.status_working)
                    .tooltip(
                        Tooltip::new(m::wake_thread_now())
                            .side(Side::Right)
                            .side_offset(px(15.)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.wake_thread(id.clone(), window, cx)
                    }))
            } else {
                button
                    .icon(IconName::Clock)
                    .tooltip(
                        Tooltip::new(m::snooze_thread())
                            .side(Side::Right)
                            .side_offset(px(15.)),
                    )
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        this.open_menu(MenuTarget::Snooze(id.clone()), event.position(), cx)
                    }))
            };
        let selector = format!("snooze-row-{thread_id}");
        div()
            .debug_selector(move || selector.clone())
            .child(button)
            .into_any_element()
    }

    /// "Woke": the thread woke from its snooze since the user saw it; a click puts it away.
    fn woke_chip(&self, thread_id: &str, colors: &Colors, cx: &mut Context<Self>) -> AnyElement {
        let id = thread_id.to_owned();
        let selector = format!("woke-{thread_id}");
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .flex_none()
            .px_1p5()
            .rounded(px(4.))
            .bg(colors.warning.opacity(0.12))
            .text_xs()
            .text_color(colors.warning)
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.dismiss_woke(id.clone(), cx)
            }))
            .child(m::woke_chip())
            .with_tooltip(Tooltip::new(m::dismiss_woke()).side(Side::Right))
            .into_any_element()
    }

    fn pull_request_pill(
        &self,
        thread_id: &str,
        pull_request: &PullRequest,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let (icon, color) = pull_request_mark(pull_request, colors);
        let failing = pull_request.checks == Some(ChecksState::Failing);
        let comments = pull_request.unresolved_comments;
        let (id, hovered) = (thread_id.to_owned(), thread_id.to_owned());
        div()
            .id(SharedString::from(format!("pull-request-{thread_id}")))
            .relative()
            .ml_auto()
            .mr(px(-4.))
            .my(px(-2.))
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .px_1()
            .py(px(2.))
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|this| this.bg(colors.sidebar_accent).text_color(colors.foreground))
            .on_hover(
                cx.listener(move |this, over: &bool, _, cx| this.hover_pill(&hovered, *over, cx)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.quiet_card(&id, cx);
                this.open_thread(&id, cx);
                cx.emit(SidebarEvent::OpenPullRequest(id.clone()));
            }))
            .child(anchor(
                &self.cards,
                CardTarget::PullRequest(thread_id.to_owned()),
            ))
            .child(Icon::new(icon).size_3().text_color(color))
            .child(format!("#{}", pull_request.number))
            .when(failing, |this| {
                this.child(div().size(px(6.)).rounded_full().bg(colors.status_error))
            })
            .when(comments > 0, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .child(Icon::new(IconName::MessageSquare).size_3())
                        .child(comments.to_string()),
                )
            })
    }

    fn context_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let open = self.menu.as_ref()?;
        let serial = open.serial;
        let this = cx.weak_entity();
        let closing = this.clone();
        let on_close = move |_: &mut Window, cx: &mut App| {
            closing
                .update(cx, |this, cx| {
                    if this.menu.as_ref().map(|menu| menu.serial) == Some(serial) {
                        this.menu = None;
                        cx.notify();
                    }
                })
                .ok();
        };
        let id = SharedString::from(format!("context-menu-{serial}"));
        match &open.target {
            MenuTarget::Project(project_id) => {
                let project_id = project_id.clone();
                Some(
                    context_menu(id, open.position, on_close, move |menu, _, _| {
                        project_menu(menu, this, project_id)
                    })
                    .into_any_element(),
                )
            }
            MenuTarget::Snooze(thread_id) => {
                let thread_id = thread_id.clone();
                let presets = self
                    .workspace
                    .read(cx)
                    .settings
                    .general
                    .snooze_presets
                    .clone();
                Some(
                    context_menu(id, open.position, on_close, move |menu, _, _| {
                        snooze_menu(menu, this, thread_id, &presets)
                    })
                    .into_any_element(),
                )
            }
            MenuTarget::Draft(key) => {
                let (workspace, key) = (self.workspace.downgrade(), key.clone());
                Some(
                    context_menu(id, open.position, on_close, move |menu, _, _| {
                        menu.item(
                            MenuOption::new("draft-discard", m::discard_draft())
                                .destructive()
                                .on_select(move |_, cx| {
                                    workspace
                                        .update(cx, |workspace, cx| {
                                            workspace.forget_drafts(std::slice::from_ref(&key), cx)
                                        })
                                        .ok();
                                }),
                        )
                    })
                    .into_any_element(),
                )
            }
            MenuTarget::Thread(thread_id) => {
                let workspace = self.workspace.read(cx);
                let thread = workspace.model.thread(thread_id)?.clone();
                let has_draft = workspace.drafts.contains_key(thread_id)
                    && self.shown_thread.as_deref() != Some(thread_id.as_str());
                let pull_request = workspace.pull_requests.get(thread_id).cloned();
                let can_clean = merged_cleanup(&thread, pull_request.as_ref()).is_some();
                let can_release = thread.worktree_path.is_some()
                    && !thread.worktree_released
                    && !thread.worktree_adopted;
                let snoozed = is_snoozed(&workspace.model, &thread, Utc::now());
                let presets = workspace.settings.general.snooze_presets.clone();
                let others =
                    crate::provider_switch::other_models(&workspace.models, &thread.provider);
                let switch = (!others.is_empty()).then(|| self.workspace.clone());
                Some(
                    context_menu(id, open.position, on_close, move |menu, _, _| {
                        let item = |key: &'static str, label: String, action: ThreadAction| {
                            let (this, id) = (this.clone(), thread.id.clone());
                            MenuOption::new(key, label).on_select(move |window, cx| {
                                this.update(cx, |this, cx| {
                                    this.thread_action(id.clone(), action, window, cx)
                                })
                                .ok();
                            })
                        };
                        let menu = menu.item(item(
                            "thread-pin",
                            if thread.pinned {
                                m::unpin_thread()
                            } else {
                                m::pin_thread()
                            },
                            ThreadAction::Pin(!thread.pinned),
                        ));
                        let menu = if thread.archived {
                            menu
                        } else if snoozed {
                            menu.item(item("thread-wake", m::wake_thread(), ThreadAction::Wake))
                        } else {
                            let (this, id) = (this.clone(), thread.id.clone());
                            menu.submenu(
                                "thread-snooze",
                                m::snooze_menu(),
                                None,
                                move |menu, _, _| snooze_menu(menu, this, id, &presets),
                            )
                        };
                        let menu = menu
                            .item(item("thread-fork", m::fork_thread(), ThreadAction::Fork))
                            .item(item(
                                "thread-archive",
                                if thread.archived {
                                    m::unarchive_thread()
                                } else {
                                    m::archive_thread()
                                },
                                ThreadAction::Archive(!thread.archived),
                            ))
                            .item(item(
                                "thread-title",
                                m::regenerate_title(),
                                ThreadAction::RegenerateTitle,
                            ));
                        let menu = match switch {
                            Some(workspace) => {
                                let id = thread.id.clone();
                                menu.submenu(
                                    "thread-continue-with",
                                    m::continue_with(),
                                    Some(IconName::ArrowRightLeft),
                                    move |menu, _, cx| {
                                        crate::provider_switch::continue_with_menu(
                                            menu, workspace, &id, cx,
                                        )
                                    },
                                )
                            }
                            None => menu,
                        };
                        let menu = if has_draft {
                            menu.item(item(
                                "thread-discard-draft",
                                m::discard_draft(),
                                ThreadAction::DiscardDraft,
                            ))
                        } else {
                            menu
                        };
                        let menu = if can_clean || can_release {
                            let menu = menu.separator();
                            let menu = if can_clean {
                                menu.item(item(
                                    "thread-clean-up",
                                    m::clean_up_thread(),
                                    ThreadAction::CleanUp,
                                ))
                            } else {
                                menu
                            };
                            if can_release {
                                menu.item(item(
                                    "thread-release",
                                    m::release_worktree(),
                                    ThreadAction::Release,
                                ))
                            } else {
                                menu
                            }
                        } else {
                            menu
                        };
                        menu.separator().item(
                            item("thread-delete", m::delete_thread(), ThreadAction::Delete)
                                .destructive(),
                        )
                    })
                    .into_any_element(),
                )
            }
        }
    }

    fn setup_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (form, _) = self.setup.as_ref()?;
        let state = form.read(cx);
        let importing = state.importing();
        let project = state.project_title().to_owned();
        let weak = form.downgrade();
        let act =
            |run: fn(&mut WorktreeSetupForm, &mut Window, &mut Context<WorktreeSetupForm>)| {
                let weak = weak.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    weak.update(cx, |form, cx| run(form, window, cx)).ok();
                }
            };
        let dialog = Dialog::new("worktree-setup")
            .open(true)
            .size(DialogSize::Custom(px(512.)))
            .close_button(false)
            .on_open_change(cx.listener(|this, change: &OpenChange, _, cx| {
                if !change.open {
                    this.setup = None;
                    cx.notify();
                }
            }))
            .title(if importing {
                m::worktree_setup_import_title()
            } else {
                m::worktree_setup_title()
            })
            .child(form.clone());
        let dialog = if importing {
            dialog
                .footer(
                    DialogButton::new("worktree-setup-back")
                        .label(m::back())
                        .variant(DialogButtonVariant::Outline)
                        .on_click(act(|form, _, cx| form.back(cx))),
                )
                .footer(
                    DialogButton::new("worktree-setup-import")
                        .label(m::worktree_setup_import())
                        .disabled(!state.can_import(cx))
                        .on_click(act(|form, window, cx| form.import(window, cx))),
                )
        } else {
            let share = form.update(cx, |form, cx| form.share_menu(cx));
            dialog
                .description(m::worktree_setup_description(project))
                .footer(share)
                .footer(
                    DialogButton::new("worktree-setup-cancel")
                        .label(m::cancel())
                        .variant(DialogButtonVariant::Outline)
                        .close(),
                )
                .footer(
                    DialogButton::new("worktree-setup-save")
                        .label(m::save())
                        .disabled(!form.read(cx).can_save(cx))
                        .on_click(act(|form, _, cx| form.save(cx))),
                )
        };
        Some(dialog.into_any_element())
    }

    fn footer(&self, colors: &Colors, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .px_2()
            .py(px(6.))
            .border_t_1()
            .border_color(colors.sidebar_border)
            .child(
                Button::new("settings")
                    .icon(IconName::Settings)
                    .size(ButtonSize::IconSm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip(m::settings())
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::OpenSettings))),
            )
            .child(
                Button::new("usage")
                    .icon(IconName::ChartColumn)
                    .size(ButtonSize::IconSm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip(m::usage())
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::OpenUsage))),
            )
            .child(div().ml_auto().child(self.updates.clone()))
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

fn palette_shortcut() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘K"
    } else {
        "Ctrl K"
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
        self.sidebar
            .update(cx, |sidebar, cx| draw(sidebar, &colors, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

/// A rail button's signal ring: a ring in the signal's color that grows from the button and
/// fades, again and again, as the web's `animate-signal`; still when the system asks for less
/// motion. A view of its own, so that its frames redraw only itself and what holds it, at
/// `RING_FRAME` rather than at the display's rate.
struct SignalRing {
    signal: Signal,
    started: std::time::Instant,
    next_frame: Option<Task<()>>,
}

const RING_PERIOD: Duration = Duration::from_millis(2400);
const RING_FRAME: Duration = Duration::from_millis(33);

impl Render for SignalRing {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let ring = div()
            .absolute()
            .top(px(-1.))
            .left(px(-1.))
            .right(px(-1.))
            .bottom(px(-1.))
            .rounded_lg()
            .border_2()
            .border_color(signal_color(self.signal, &colors));
        if cx.reduce_motion() {
            return ring.opacity(0.);
        }
        if self.next_frame.is_none() {
            self.next_frame = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(RING_FRAME).await;
                this.update(cx, |this, cx| {
                    this.next_frame = None;
                    cx.notify();
                })
                .ok();
            }));
        }
        let delta = (self.started.elapsed().as_secs_f32() % RING_PERIOD.as_secs_f32())
            / RING_PERIOD.as_secs_f32();
        let progress = (delta / 0.7).min(1.);
        let eased = 1. - (1. - progress).powi(2);
        let grow = px(-1. - 4.8 * eased);
        ring.top(grow)
            .left(grow)
            .right(grow)
            .bottom(grow)
            .opacity(0.6 * (1. - eased))
    }
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
