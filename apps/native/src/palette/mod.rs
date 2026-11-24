//! The command palette (Ctrl+K): search over threads, projects and app actions, and the
//! palettes it opens to start a thread, add a project or start from a pull request.

pub(crate) mod add_project;
mod commands;
mod folder_path;
mod frame;
mod pull_requests;
mod search;
#[cfg(test)]
mod ui_tests;

use crate::m;
use crate::sections::View;
use crate::settings::Section;
use crate::state::Workspace;
use add_project::AddProject;
use frame::{Entry, Frame, Mark, Row, Target};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::*;
use hoshi_ui::prelude::*;
use pull_requests::PullRequestSearch;
use std::collections::HashMap;
use std::path::PathBuf;

actions!(palette, [TogglePalette]);

pub fn init(cx: &mut App) {
    let key = if cfg!(target_os = "macos") {
        "cmd-k"
    } else {
        "ctrl-k"
    };
    cx.bind_keys([KeyBinding::new(key, TogglePalette, None)]);
}

/// Everything, the project for a new thread, the ways to add a project, or the repository's open
/// pull requests to start a thread from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Commands,
    NewThread,
    AddProject,
    PullRequests,
}

/// What the sidebar shows, which decides where a new thread goes, as the web reads its store.
#[derive(Clone, Debug, Default)]
pub struct SidebarScope {
    pub view: View,
    pub project_id: Option<String>,
    /// Project ids in the rail's order.
    pub project_order: Vec<String>,
}

pub enum PaletteEvent {
    Close,
    /// The sidebar should show this view, on this project for `View::Project`.
    ShowView {
        view: View,
        project_id: Option<String>,
    },
    /// Closes the palette and opens the settings on this section.
    OpenSettings(Section),
    /// Closes the palette and opens the usage limits.
    OpenUsage,
}

pub struct Palette {
    workspace: Entity<Workspace>,
    mode: Mode,
    input: Entity<InputState>,
    query: String,
    /// The highlighted entry; `None` until the arrows move where nothing starts highlighted.
    active: Option<usize>,
    /// The entry whose work is running.
    busy: Option<String>,
    scroll: ScrollHandle,
    icons: HashMap<String, Option<PathBuf>>,
    add: AddProject,
    pulls: PullRequestSearch,
    sidebar: Option<SidebarScope>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PaletteEvent> for Palette {}

impl Palette {
    pub fn set_sidebar(&mut self, scope: SidebarScope) {
        self.sidebar = Some(scope);
    }

    pub fn new(
        workspace: Entity<Workspace>,
        mode: Mode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        workspace.update(cx, |workspace, cx| workspace.reload_logins(cx));
        let input = cx.new(|cx| InputState::new(window, cx));
        let this = cx.entity().downgrade();
        let keys = cx.intercept_keystrokes(move |event, window, cx| {
            let handled = this
                .update(cx, |palette, cx| palette.key(&event.keystroke, window, cx))
                .unwrap_or(false);
            if handled {
                cx.stop_propagation();
            }
        });
        let subscriptions = vec![
            cx.subscribe_in(&input, window, |this, _, event, window, cx| {
                if let InputEvent::Change = event {
                    this.query_changed(window, cx);
                }
            }),
            cx.observe(&workspace, |_, _, cx| cx.notify()),
            keys,
        ];
        let mut palette = Self {
            workspace,
            mode,
            input,
            query: String::new(),
            active: None,
            busy: None,
            scroll: ScrollHandle::new(),
            icons: HashMap::new(),
            add: AddProject::default(),
            pulls: PullRequestSearch::default(),
            sidebar: None,
            _subscriptions: subscriptions,
        };
        palette.set_mode(mode, window, cx);
        palette
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = mode;
        self.busy = None;
        self.add = AddProject::default();
        self.pulls = PullRequestSearch::default();
        self.set_query(String::new(), window, cx);
        if mode == Mode::PullRequests {
            self.search_pull_requests(cx);
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(PaletteEvent::Close);
    }

    /// Puts `query` in the field as if typed.
    fn set_query(&mut self, query: String, window: &mut Window, cx: &mut Context<Self>) {
        self.input
            .update(cx, |input, cx| input.set_value(query, window, cx));
        self.sync_placeholder(window, cx);
        self.query_changed(window, cx);
    }

    fn query_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut query = self.input.read(cx).value().to_string();
        if self.mode == Mode::AddProject && self.add.browsing() {
            let expanded = folder_path::expand_home(
                &query,
                add_project::home().as_deref(),
                add_project::SEPARATOR,
            );
            if expanded != query {
                self.input.update(cx, |input, cx| {
                    input.set_value(expanded.clone(), window, cx)
                });
                query = expanded;
            }
        }
        let changed = query != self.query;
        self.query = query;
        self.add.error = None;
        self.reset_active();
        match self.mode {
            Mode::AddProject => self.schedule_listing(cx),
            Mode::PullRequests if changed => self.search_pull_requests(cx),
            _ => {}
        }
        cx.notify();
    }

    fn sync_placeholder(&self, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = match self.mode {
            Mode::Commands => m::palette_placeholder(),
            Mode::NewThread => m::new_thread_in_placeholder(),
            Mode::PullRequests => m::palette_pull_request_placeholder(),
            Mode::AddProject => self.add.placeholder(),
        };
        self.input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx)
        });
    }

    fn auto_highlight(&self) -> bool {
        self.mode != Mode::AddProject || self.add.on_sources()
    }

    fn reset_active(&mut self) {
        self.active = self.auto_highlight().then_some(0);
        self.scroll.set_offset(Point::default());
    }

    fn current(&self, len: usize) -> Option<usize> {
        self.active
            .filter(|_| len > 0)
            .map(|active| active.min(len - 1))
    }

    fn frame(&self, cx: &App) -> Frame {
        match self.mode {
            Mode::Commands => self.commands_frame(cx),
            Mode::NewThread => self.new_thread_frame(cx),
            Mode::AddProject => self.add_project_frame(cx),
            Mode::PullRequests => self.pull_requests_frame(cx),
        }
    }

    fn can_go_back(&self) -> bool {
        matches!(self.mode, Mode::NewThread | Mode::AddProject)
    }

    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.mode {
            Mode::AddProject => self.back_step(window, cx),
            Mode::NewThread => self.set_mode(Mode::Commands, window, cx),
            _ => {}
        }
    }

    fn key(&mut self, keystroke: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.input.read(cx).focus_handle(cx).is_focused(window) {
            return false;
        }
        let modifiers = &keystroke.modifiers;
        let held = if cfg!(target_os = "macos") {
            modifiers.platform
        } else {
            modifiers.control
        };
        let plain = !modifiers.modified();
        match keystroke.key.as_str() {
            "k" if held && !modifiers.alt && !modifiers.shift => self.close(cx),
            "down" if plain => self.step(1, cx),
            "up" if plain => self.step(-1, cx),
            "enter" if !modifiers.alt && !modifiers.shift => self.enter(held, window, cx),
            "tab" if plain && self.frame(cx).tab => self.tab(window, cx),
            "backspace" if plain && self.query.is_empty() && self.can_go_back() => {
                self.back(window, cx)
            }
            _ => return false,
        }
        true
    }

    fn step(&mut self, step: isize, cx: &mut Context<Self>) {
        let frame = self.frame(cx);
        let len = frame.len();
        if len == 0 {
            return;
        }
        let next = match self.current(len) {
            None if step > 0 => 0,
            None => len - 1,
            Some(current) => (current as isize + step).rem_euclid(len as isize) as usize,
        };
        self.active = Some(next);
        self.scroll.scroll_to_item(frame.child_index(next));
        cx.notify();
    }

    fn highlighted(&self, cx: &App) -> Option<Entry> {
        let mut frame = self.frame(cx);
        let index = self.current(frame.len())?;
        let mut seen = 0;
        for group in &mut frame.groups {
            if index < seen + group.entries.len() {
                return Some(group.entries.swap_remove(index - seen));
            }
            seen += group.entries.len();
        }
        None
    }

    fn enter(&mut self, held: bool, window: &mut Window, cx: &mut Context<Self>) {
        let submits = self.frame(cx).submits;
        let entry = self.highlighted(cx);
        if submits && (entry.is_none() || held) {
            self.submit(window, cx);
        } else if let Some(entry) = entry {
            self.run(entry, window, cx);
        }
    }

    fn tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.highlighted(cx) {
            Some(entry) => self.run(entry, window, cx),
            None => self.complete_tab(window, cx),
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == Mode::AddProject {
            self.submit_step(window, cx);
        }
    }

    fn run(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        self.run_target(entry.key, entry.target, entry.disabled, window, cx);
    }

    fn run_target(
        &mut self,
        key: String,
        target: Target,
        disabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if disabled || self.busy.is_some() {
            return;
        }
        match target {
            Target::Thread(id) => self.open_thread(&id, cx),
            Target::Project(id) => self.select_project(id, cx),
            Target::Action(action) => self.run_action(action, window, cx),
            Target::StartIn(project) => self.start_in(project, cx),
            Target::Source(source) => self.pick_source(source, window, cx),
            Target::Browse(query) => self.browse_to(query, window, cx),
            Target::PullRequest(pull_request) => {
                self.start_from_pull_request(key, *pull_request, cx)
            }
        }
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

    fn title(&self) -> String {
        match self.mode {
            Mode::Commands => m::palette_label(),
            Mode::NewThread => m::new_thread(),
            Mode::AddProject => m::add_project(),
            Mode::PullRequests => m::new_thread_from_pull_request(),
        }
    }

    fn header(
        &self,
        frame: &Frame,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let submit = frame.submit.as_ref().map(|submit| {
            let keys = if self.highlighted(cx).is_some() {
                format!("{} ↵", frame::modifier_label())
            } else {
                "↵".to_owned()
            };
            Button::new("palette-submit")
                .label(submit.label.clone())
                .size(ButtonSize::Xs)
                .variant(ButtonVariant::Secondary)
                .disabled(submit.disabled)
                .when(!submit.pending, |this| this.suffix(Kbd::new(keys)))
                .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx)))
        });
        div()
            .h_12()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.))
            .px_4()
            .border_b_1()
            .border_color(colors.border)
            .map(|this| {
                if frame.back {
                    this.child(
                        div()
                            .id("palette-back")
                            .ml(px(-4.))
                            .size_6()
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .cursor_pointer()
                            .text_color(colors.muted_foreground)
                            .hover(|this| this.text_color(colors.foreground))
                            .on_click(cx.listener(|this, _, window, cx| this.back(window, cx)))
                            .child(Icon::new(IconName::ArrowLeft).size_4())
                            .with_tooltip(Tooltip::new(m::palette_back())),
                    )
                } else {
                    this.child(frame::mark(&Mark::Icon(IconName::Search), None, colors))
                }
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(text_input(&self.input).appearance(false)),
            )
            .children(submit)
    }

    fn list(
        &mut self,
        frame: &Frame,
        colors: &Colors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if frame.len() == 0 {
            return match &frame.empty {
                Some(empty) => div()
                    .px_3()
                    .py_6()
                    .text_center()
                    .text_color(colors.muted_foreground)
                    .child(empty.clone())
                    .into_any_element(),
                None => Empty.into_any_element(),
            };
        }
        let mono = cx.theme().mono_font_family.clone();
        let current = self.current(frame.len());
        let mut children = Vec::new();
        let mut index = 0;
        for group in &frame.groups {
            if !group.label.is_empty() {
                children.push(frame::heading(&group.label, colors));
            }
            for entry in &group.entries {
                let icon = match &entry.row {
                    Row::Project { id, .. }
                    | Row::TwoLine {
                        mark: Mark::Project(id),
                        ..
                    } => self.icon(id, cx),
                    _ => None,
                };
                let active = current == Some(index);
                let busy = self.busy.as_ref();
                let waiting = busy.is_some_and(|busy| *busy != entry.key);
                let running = busy == Some(&entry.key);
                let key = entry.key.clone();
                let target = entry.target.clone();
                let disabled = entry.disabled;
                let row = frame::row(
                    &entry.row,
                    icon,
                    colors,
                    &mono,
                    cx.listener(|_, _, _, cx| cx.emit(PaletteEvent::OpenSettings(Section::Forges))),
                );
                let selector = format!("palette-{}", entry.key);
                children.push(
                    div()
                        .id(SharedString::from(format!("palette-entry-{index}")))
                        .debug_selector(move || selector)
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .min_h_9()
                        .px(px(10.))
                        .py_1p5()
                        .rounded_lg()
                        .when(!disabled, |this| this.cursor_pointer())
                        .when(active, |this| {
                            this.bg(colors.accent).text_color(colors.accent_foreground)
                        })
                        .when(waiting, |this| this.opacity(0.5))
                        .on_mouse_move(cx.listener(move |this, _, _, cx| {
                            if this.active != Some(index) {
                                this.active = Some(index);
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.run_target(key.clone(), target.clone(), disabled, window, cx)
                        }))
                        .children(row)
                        .map(|row| {
                            if running && !cx.reduce_motion() {
                                row.with_loop(
                                    hoshi_ui::looping(std::time::Duration::from_secs(2)),
                                    |row, delta| {
                                        row.opacity(1. - 0.5 * (delta * std::f32::consts::PI).sin())
                                    },
                                )
                                .into_any_element()
                            } else {
                                row.into_any_element()
                            }
                        }),
                );
                index += 1;
            }
            if group.divider {
                children.push(frame::divider(colors));
            }
        }
        let max_height = px(416.).min(window.viewport_size().height * 0.6);
        div()
            .id("palette-results")
            .track_scroll(&self.scroll)
            .overflow_y_scroll()
            .max_h(max_height)
            .p_1p5()
            .flex()
            .flex_col()
            .children(children)
            .into_any_element()
    }

    fn footer(
        &self,
        frame: &Frame,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap_4()
            .px_4()
            .py_2()
            .border_t_1()
            .border_color(colors.border)
            .text_xs()
            .text_color(colors.muted_foreground)
            .child(frame::hint(&["↑", "↓"], m::palette_navigate()))
            .child(frame::hint(&["↵"], frame.enter_label.clone()))
            .when(frame.back, |this| {
                this.child(frame::hint(&["⌫"], m::palette_back()))
            })
            .child(frame::hint(&["Esc"], m::palette_close()))
            .when_some(frame.footer.clone(), |this, label| {
                let pending = self.add.pending;
                this.child(
                    div()
                        .id("palette-file-manager")
                        .ml_auto()
                        .when(pending, |this| this.opacity(0.5))
                        .when(!pending, |this| {
                            this.cursor_pointer()
                                .hover(|this| this.text_color(colors.foreground))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.pick_with_system_dialog(window, cx)
                                }))
                        })
                        .child(label),
                )
            })
    }
}

impl Render for Palette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let frame = self.frame(cx);
        let header = self.header(&frame, &colors, cx);
        let list = self.list(&frame, &colors, window, cx);
        let footer = self.footer(&frame, &colors, cx);
        let this = cx.entity().downgrade();
        Dialog::new("palette")
            .palette()
            .size(DialogSize::Xl)
            .open(true)
            .close_button(false)
            .title(self.title())
            .initial_focus(InitialFocus::Handle(self.input.focus_handle(cx)))
            .on_open_change(move |change, _, cx| {
                if !change.open {
                    this.update(cx, |this, cx| this.close(cx)).ok();
                }
            })
            .child(header)
            .when_some(frame.status.clone(), |this, status| {
                this.child(
                    div()
                        .px_4()
                        .py_2()
                        .border_b_1()
                        .border_color(colors.border)
                        .text_xs()
                        .text_color(colors.destructive)
                        .child(status),
                )
            })
            .when_some(frame.summary.as_ref(), |this, summary| {
                this.child(frame::summary(summary, &colors))
            })
            .child(list)
            .child(footer)
    }
}
