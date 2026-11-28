mod browser;
mod changes;
mod diff_surface;
mod file_surface;
mod pdf_surface;
mod persist;
mod pull_request;
mod sheet_grid;
pub(crate) use pull_request::{check_look, format_review_threads};
mod surfaces;

use crate::m;
use crate::state::Workspace;
use browser::BrowserSurface;
use diff_surface::{DiffSurface, DiffSurfaceEvent};
use file_surface::FileSurface;
use gpui_kit::component::Icon;
use gpui_kit::component::Selectable as _;
use gpui_kit::*;
use hoshi_ui::assets::file_icon;
use hoshi_ui::prelude::*;
use pdf_surface::PdfSurface;
use pull_request::PullRequestSurface;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;
pub use surfaces::{
    DiffShown, SURFACE_KINDS, Surface, SurfaceContext, SurfaceKind, SurfaceState, ThreadSurfaces,
    kind_for_shortcut,
};

pub const DEFAULT_WIDTH: f32 = 440.;
pub const MIN_WIDTH: f32 = 320.;
pub const MAX_WIDTH: f32 = 760.;

/// A drag or a burst of changes writes `panel.json` once.
const SAVE_DELAY: Duration = Duration::from_millis(300);

/// Draws and ends the panel's terminal surfaces; the panel offers terminals once one is set.
pub trait TerminalHost: 'static {
    fn render(
        &self,
        thread_id: &str,
        terminal_id: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement;

    /// The surface `terminal_id` closed.
    fn close(&self, terminal_id: &str, cx: &mut App);
}

enum SurfaceView {
    Diff {
        view: Entity<DiffSurface>,
        _events: Subscription,
    },
    PullRequest(Entity<PullRequestSurface>),
    Browser(Entity<BrowserSurface>),
    File(Entity<FileSurface>),
    Pdf(Entity<PdfSurface>),
}

/// The thread's side panel on the right: tabs of open surfaces (changes, pull request,
/// browser, terminals, files), each thread with its own, kept across restarts in `panel.json`.
pub struct RightPanel {
    /// Where `panel.json` lives; `None` keeps the panel in memory only.
    home: Option<PathBuf>,
    saving: Option<Task<()>>,
    workspace: Entity<Workspace>,
    open: HashSet<String>,
    threads: HashMap<String, ThreadSurfaces>,
    /// The views of the shown thread's surfaces, by surface id.
    views: HashMap<String, SurfaceView>,
    views_for: Option<String>,
    width: f32,
    /// The panel takes the chat's place.
    maximized: bool,
    add_menu_open: bool,
    focus: FocusHandle,
    focus_requested: bool,
    terminal: Option<Rc<dyn TerminalHost>>,
    /// `HOSHI_NATIVE_PANEL`: opens the first thread shown on that surface.
    dev_surface: Option<Option<SurfaceKind>>,
    _subscriptions: Vec<Subscription>,
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The text a panel shows in place of what it can't show.
pub fn panel_message(text: String, colors: &Colors) -> Div {
    div()
        .px_3()
        .py_4()
        .text_center()
        .text_xs()
        .text_color(colors.muted_foreground)
        .child(text)
}

/// The last part of a path, as a tab names the file.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(path)
}

fn dev_surface() -> Option<Option<SurfaceKind>> {
    let name = std::env::var("HOSHI_NATIVE_PANEL").ok()?;
    Some(match name.as_str() {
        "diff" => Some(SurfaceKind::Diff),
        "pr" => Some(SurfaceKind::PullRequest),
        "browser" => Some(SurfaceKind::Browser),
        _ => None,
    })
}

impl RightPanel {
    pub fn new(workspace: Entity<Workspace>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let this = cx.weak_entity();
        let keys = cx.intercept_keystrokes(move |event, _, cx| {
            let keystroke = &event.keystroke;
            let modifiers = keystroke.modifiers;
            if !(modifiers.control
                && modifiers.alt
                && !modifiers.shift
                && keystroke.key.eq_ignore_ascii_case("b"))
            {
                return;
            }
            let toggled = this
                .update(cx, |panel, cx| {
                    let Some(id) = panel.workspace.read(cx).open_thread_id(cx) else {
                        return false;
                    };
                    let open = !panel.is_open(&id);
                    panel.set_open(&id, open, cx);
                    true
                })
                .unwrap_or(false);
            if toggled {
                cx.stop_propagation();
            }
        });
        let browsers = Self::watch_browsers(&workspace, cx);
        let observe = cx.observe(&workspace, |this, _, cx| {
            this.forget_deleted(cx);
            this.apply_dev_surface(cx);
            cx.notify();
        });
        let home = workspace
            .read(cx)
            .settings_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let saved = home.as_deref().map(persist::load).unwrap_or_default();
        let mut this = Self {
            home,
            saving: None,
            workspace,
            open: saved.open,
            threads: saved.threads,
            views: HashMap::new(),
            views_for: None,
            width: saved.width,
            maximized: saved.maximized,
            add_menu_open: false,
            focus: cx.focus_handle(),
            focus_requested: false,
            terminal: None,
            dev_surface: dev_surface(),
            _subscriptions: [keys, observe].into_iter().chain(browsers).collect(),
        };
        this.forget_deleted(cx);
        this
    }

    fn snapshot(&self) -> persist::Saved {
        persist::Saved {
            open: self.open.clone(),
            maximized: self.maximized,
            width: self.width,
            threads: self.threads.clone(),
        }
    }

    /// Writes `panel.json` a moment after the last change.
    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        let saved = self.snapshot();
        self.saving = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            cx.background_spawn(async move { persist::save(&home, &saved) })
                .await
                .ok();
        }));
    }

    /// Forgets the panels of threads that were deleted.
    fn forget_deleted(&mut self, cx: &mut Context<Self>) {
        let workspace = self.workspace.read(cx);
        let mut saved = persist::Saved {
            open: std::mem::take(&mut self.open),
            threads: std::mem::take(&mut self.threads),
            ..persist::Saved::default()
        };
        let forgot = saved.forget(|id| workspace.model.thread(id).is_some());
        self.open = saved.open;
        self.threads = saved.threads;
        if forgot {
            self.save(cx);
        }
    }

    fn apply_dev_surface(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.workspace.read(cx).open_thread_id(cx) else {
            return;
        };
        match self.dev_surface.take() {
            Some(Some(kind)) => self.open_surface(&id, kind, cx),
            Some(None) => self.set_open(&id, true, cx),
            None => {}
        }
    }

    pub fn set_terminal_host(&mut self, host: Rc<dyn TerminalHost>, cx: &mut Context<Self>) {
        self.terminal = Some(host);
        cx.notify();
    }

    pub fn is_open(&self, thread_id: &str) -> bool {
        self.open.contains(thread_id)
    }

    /// The thread's panel is open and takes the chat's place.
    pub fn is_maximized(&self, thread_id: &str) -> bool {
        self.maximized && self.is_open(thread_id)
    }

    #[cfg(test)]
    pub fn width(&self) -> f32 {
        self.width
    }

    #[cfg(test)]
    pub fn surfaces(&self, thread_id: &str) -> Option<&ThreadSurfaces> {
        self.threads.get(thread_id)
    }

    /// From the header's button: an empty panel opens ready for its one-key shortcuts.
    pub fn toggle(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        let open = !self.is_open(thread_id);
        self.set_open(thread_id, open, cx);
        let empty = self
            .threads
            .get(thread_id)
            .is_none_or(|panel| panel.surfaces.is_empty());
        self.focus_requested = open && empty;
    }

    fn set_open(&mut self, thread_id: &str, open: bool, cx: &mut Context<Self>) {
        if open {
            self.open.insert(thread_id.to_owned());
        } else if self.open.remove(thread_id) {
            self.maximized = false;
            self.add_menu_open = false;
        }
        self.save(cx);
        cx.notify();
    }

    pub fn close(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        self.set_open(thread_id, false, cx);
    }

    pub fn toggle_maximized(&mut self, cx: &mut Context<Self>) {
        self.maximized = !self.maximized;
        self.save(cx);
        cx.notify();
    }

    pub fn set_width(&mut self, width: f32, cx: &mut Context<Self>) {
        self.width = width.clamp(MIN_WIDTH, MAX_WIDTH).round();
        self.save(cx);
        cx.notify();
    }

    fn context(&self, thread_id: &str, cx: &App) -> SurfaceContext {
        SurfaceContext {
            has_pull_request: self
                .workspace
                .read(cx)
                .pull_requests
                .contains_key(thread_id),
            has_terminal: self.terminal.is_some(),
        }
    }

    /// Opens the panel on a surface of that kind, reusing a single-instance one.
    pub fn open_surface(&mut self, thread_id: &str, kind: SurfaceKind, cx: &mut Context<Self>) {
        self.threads
            .entry(thread_id.to_owned())
            .or_default()
            .open(kind, new_id);
        self.set_open(thread_id, true, cx);
    }

    /// Opens the panel on the file at `path`, in the tab already showing it if there is one,
    /// read again if it changed since.
    pub fn open_file(&mut self, thread_id: &str, path: &str, cx: &mut Context<Self>) {
        let panel = self.threads.entry(thread_id.to_owned()).or_default();
        panel.open_file(path, new_id);
        match panel.active_id.as_ref().and_then(|id| self.views.get(id)) {
            Some(SurfaceView::File(view)) => view.update(cx, |view, cx| view.refresh(cx)),
            Some(SurfaceView::Pdf(view)) => view.update(cx, |view, cx| view.refresh(cx)),
            _ => {}
        }
        self.set_open(thread_id, true, cx);
    }

    /// Opens the panel on what a turn changed, at `focus_path` when given.
    pub fn open_turn_diff(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        focus_path: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let panel = self.threads.entry(thread_id.to_owned()).or_default();
        panel.show_turn_diff(turn_id, focus_path.clone(), new_id);
        if let Some(id) = panel.active_id.clone()
            && let Some(SurfaceView::Diff { view, .. }) = self.views.get(&id)
        {
            view.update(cx, |view, cx| {
                view.show(DiffShown::Turn, Some(turn_id.to_owned()), focus_path, cx)
            });
        }
        self.set_open(thread_id, true, cx);
    }

    pub fn activate_surface(&mut self, thread_id: &str, surface_id: &str, cx: &mut Context<Self>) {
        if let Some(panel) = self.threads.get_mut(thread_id) {
            panel.activate(surface_id);
        }
        self.save(cx);
        cx.notify();
    }

    /// Closing the last surface closes the panel too.
    pub fn close_surface(&mut self, thread_id: &str, surface_id: &str, cx: &mut Context<Self>) {
        let Some(panel) = self.threads.get_mut(thread_id) else {
            return;
        };
        let closed = panel.close(surface_id);
        let empty = panel.surfaces.is_empty();
        self.views.remove(surface_id);
        match closed.map(|surface| surface.kind()) {
            Some(SurfaceKind::Terminal) => {
                if let Some(host) = self.terminal.clone() {
                    host.close(surface_id, cx);
                }
            }
            Some(SurfaceKind::Browser) => {
                let browsers = self.workspace.read(cx).browsers.clone();
                browsers.update(cx, |browsers, cx| browsers.close(thread_id, cx));
            }
            _ => {}
        }
        if empty {
            self.set_open(thread_id, false, cx);
        }
        self.save(cx);
        cx.notify();
    }

    fn diff_shown(
        &mut self,
        thread_id: &str,
        surface_id: &str,
        picked: DiffShown,
        cx: &mut Context<Self>,
    ) {
        let surface = self
            .threads
            .get_mut(thread_id)
            .and_then(|panel| panel.get_mut(surface_id));
        if let Some(Surface {
            state: SurfaceState::Diff {
                shown, focus_path, ..
            },
            ..
        }) = surface
        {
            *shown = picked;
            *focus_path = None;
        }
        self.save(cx);
    }

    fn surface_view(
        &mut self,
        thread_id: &str,
        surface: &Surface,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(view) = self.views.get(&surface.id) {
            return match view {
                SurfaceView::Diff { view, .. } => view.clone().into_any_element(),
                SurfaceView::PullRequest(view) => view.clone().into_any_element(),
                SurfaceView::Browser(view) => view.clone().into_any_element(),
                SurfaceView::File(view) => view.clone().into_any_element(),
                SurfaceView::Pdf(view) => view.clone().into_any_element(),
            };
        }
        let workspace = self.workspace.clone();
        let view = match &surface.state {
            SurfaceState::Diff {
                shown,
                turn_id,
                focus_path,
            } => {
                let (shown, turn_id, focus_path) = (*shown, turn_id.clone(), focus_path.clone());
                let thread = thread_id.to_owned();
                let view = cx
                    .new(|cx| DiffSurface::new(workspace, thread, shown, turn_id, focus_path, cx));
                let (thread, id) = (thread_id.to_owned(), surface.id.clone());
                let events = cx.subscribe(&view, move |this, _, event, cx| match event {
                    DiffSurfaceEvent::Shown(shown) => this.diff_shown(&thread, &id, *shown, cx),
                });
                SurfaceView::Diff {
                    view,
                    _events: events,
                }
            }
            SurfaceState::PullRequest => {
                let thread = thread_id.to_owned();
                SurfaceView::PullRequest(
                    cx.new(|cx| PullRequestSurface::new(workspace, thread, window, cx)),
                )
            }
            SurfaceState::Browser => {
                let thread = thread_id.to_owned();
                SurfaceView::Browser(
                    cx.new(|cx| BrowserSurface::new(workspace, thread, window, cx)),
                )
            }
            SurfaceState::File { path } if hoshi_engine::pdf::is_pdf(path) => {
                let (path, home) = (path.clone(), self.home.clone());
                SurfaceView::Pdf(cx.new(|cx| PdfSurface::new(path, home, window, cx)))
            }
            SurfaceState::File { path } => {
                let path = path.clone();
                SurfaceView::File(cx.new(|cx| FileSurface::new(path, cx)))
            }
            SurfaceState::Terminal => {
                let colors = cx.hoshi().clone();
                return match self.terminal.clone() {
                    Some(host) => host.render(thread_id, &surface.id, window, cx),
                    None => panel_message(m::surface_terminal(), &colors).into_any_element(),
                };
            }
        };
        self.views.insert(surface.id.clone(), view);
        self.surface_view(thread_id, surface, window, cx)
    }

    fn tab(
        &self,
        index: usize,
        thread_id: &str,
        surface: &Surface,
        active: bool,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let kind = surface.kind();
        let title = match kind {
            SurfaceKind::PullRequest => self
                .workspace
                .read(cx)
                .pull_requests
                .get(thread_id)
                .map(|pull_request| format!("#{}", pull_request.number))
                .unwrap_or_else(|| kind.label()),
            SurfaceKind::File => match &surface.state {
                SurfaceState::File { path } => file_name(path).to_owned(),
                _ => kind.label(),
            },
            _ => kind.label(),
        };
        let icon = match &surface.state {
            SurfaceState::File { path } => {
                let (icon, tint) = file_path_icon(path);
                svg()
                    .path(file_icon(icon))
                    .size_3p5()
                    .flex_none()
                    .text_color(tint(colors))
                    .into_any_element()
            }
            _ => Icon::new(kind.icon()).size_3p5().into_any_element(),
        };
        let group: SharedString = format!("surface-tab-{index}").into();
        let (thread, id) = (thread_id.to_owned(), surface.id.clone());
        let (close_thread, close_id) = (thread.clone(), id.clone());
        let (middle_thread, middle_id) = (thread.clone(), id.clone());
        let hover = colors.accent.opacity(0.6);
        div()
            .id(("surface-tab", index))
            .debug_selector(move || format!("surface-tab-{index}"))
            .group(group.clone())
            .flex_none()
            .flex()
            .items_center()
            .h_6()
            .rounded_md()
            .text_xs()
            .map(|tab| {
                if active {
                    tab.bg(colors.secondary)
                        .text_color(colors.secondary_foreground)
                } else {
                    tab.text_color(colors.muted_foreground)
                        .hover(move |tab| tab.bg(hover))
                }
            })
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _, _, cx| {
                    this.close_surface(&middle_thread, &middle_id, cx)
                }),
            )
            .child(
                div()
                    .id(("surface-tab-select", index))
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .h_full()
                    .pl_2()
                    .cursor_pointer()
                    .child(icon)
                    .child(div().max_w(px(180.)).truncate().child(title))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.activate_surface(&thread, &id, cx)),
                    ),
            )
            .child(
                div()
                    .id(("surface-tab-close", index))
                    .debug_selector(move || format!("surface-tab-close-{index}"))
                    .mx_0p5()
                    .size_4()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .cursor_pointer()
                    .opacity(0.6)
                    .group_hover(group, |close| close.opacity(1.))
                    .hover(|close| close.bg(colors.accent))
                    .child(Icon::new(IconName::X).size_3())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.close_surface(&close_thread, &close_id, cx)
                    }))
                    .with_tooltip(m::close_surface(kind.label())),
            )
    }

    fn add_menu(&self, thread_id: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let this = cx.weak_entity();
        let thread = thread_id.to_owned();
        let context = self.context(thread_id, cx);
        Popover::new("add-surface-menu")
            .align(Align::End)
            .w(px(192.))
            .open(self.add_menu_open)
            .on_open_change(cx.listener(|this, change: &PopoverChange, _, cx| {
                this.add_menu_open = change.open;
                cx.notify();
            }))
            .trigger(|open| {
                Button::new("add-surface")
                    .icon(IconName::Plus)
                    .size(ButtonSize::IconSm)
                    .variant(ButtonVariant::Ghost)
                    .selected(open)
                    .tooltip(Tooltip::new(m::open_surface()).side(Side::Bottom))
            })
            .menu(move |menu, _, _| {
                menu.items(SURFACE_KINDS.into_iter().map(|kind| {
                    let (this, thread) = (this.clone(), thread.clone());
                    MenuOption::new(("add-surface", kind as usize), kind.label())
                        .icon(kind.icon())
                        .trailing(Kbd::new(kind.shortcut()))
                        .when_some(kind.shortcut().chars().next(), MenuOption::accelerator)
                        .disabled(!kind.available(context))
                        .on_select(move |_, cx| {
                            this.update(cx, |this, cx| this.open_surface(&thread, kind, cx))
                                .ok();
                        })
                }))
            })
    }

    fn launcher(
        &self,
        thread_id: &str,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let context = self.context(thread_id, cx);
        let hover = colors.accent;
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .p_4()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(m::open_surface()),
            )
            .child(div().flex().flex_col().w_full().max_w(px(240.)).children(
                SURFACE_KINDS.into_iter().map(|kind| {
                    let available = kind.available(context);
                    let thread = thread_id.to_owned();
                    div()
                        .id(("launch-surface", kind as usize))
                        .debug_selector(move || format!("launch-{}", kind.shortcut()))
                        .flex()
                        .items_center()
                        .gap_2p5()
                        .px_2()
                        .py_1p5()
                        .rounded_md()
                        .text_sm()
                        .map(|row| {
                            if available {
                                row.cursor_pointer()
                                    .hover(move |row| row.bg(hover))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.open_surface(&thread, kind, cx)
                                    }))
                            } else {
                                row.opacity(0.45)
                            }
                        })
                        .child(Icon::new(kind.icon()).size_4())
                        .child(div().flex_1().child(kind.label()))
                        .child(Kbd::new(kind.shortcut()))
                }),
            ))
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform {
            return;
        }
        let Some(thread_id) = self.workspace.read(cx).open_thread_id(cx) else {
            return;
        };
        let empty = self
            .threads
            .get(&thread_id)
            .is_none_or(|panel| panel.surfaces.is_empty());
        if !empty {
            return;
        }
        if let Some(kind) = kind_for_shortcut(&event.keystroke.key, self.context(&thread_id, cx)) {
            self.open_surface(&thread_id, kind, cx);
            cx.stop_propagation();
        }
    }
}

impl Render for RightPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let thread_id = self.workspace.read(cx).open_thread_id(cx);
        if self.views_for != thread_id {
            self.views.clear();
            self.views_for.clone_from(&thread_id);
        }
        let Some(thread_id) = thread_id.filter(|id| self.is_open(id)) else {
            return div().id("right-panel").into_any_element();
        };
        if std::mem::take(&mut self.focus_requested) {
            window.focus(&self.focus, cx);
        }
        let panel = self.threads.get(&thread_id).cloned().unwrap_or_default();
        let tabs: Vec<_> = panel
            .surfaces
            .iter()
            .enumerate()
            .map(|(index, surface)| {
                let active = panel.active_id.as_deref() == Some(surface.id.as_str());
                self.tab(index, &thread_id, surface, active, &colors, cx)
            })
            .collect();
        let body = match panel.active() {
            Some(surface) => self.surface_view(&thread_id, surface, window, cx),
            None => self.launcher(&thread_id, &colors, cx).into_any_element(),
        };
        let maximized = self.maximized;
        let close_thread = thread_id.clone();
        let weak = cx.weak_entity();
        div()
            .id("right-panel")
            .debug_selector(|| "right-panel".into())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    let empty = this
                        .views_for
                        .as_ref()
                        .and_then(|id| this.threads.get(id))
                        .is_none_or(|panel| panel.surfaces.is_empty());
                    if empty {
                        window.focus(&this.focus, cx);
                    }
                }),
            )
            .relative()
            .h_full()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(colors.background)
            .map(|panel| {
                if maximized {
                    panel.flex_1().min_w_0()
                } else {
                    panel
                        .flex_none()
                        .w(px(self.width))
                        .border_l_1()
                        .border_color(colors.border)
                }
            })
            .when(!maximized, |panel| {
                panel.child(
                    ResizeHandle::new("resize-panel", ResizeSide::Left, self.width)
                        .range(MIN_WIDTH, MAX_WIDTH)
                        .default_value(DEFAULT_WIDTH)
                        .on_change(move |width, _, cx| {
                            weak.update(cx, |this, cx| this.set_width(width, cx)).ok();
                        }),
                )
            })
            .child(
                div()
                    .flex_none()
                    .h_9()
                    .flex()
                    .items_center()
                    .gap_1()
                    .pl_2()
                    .pr_1()
                    .border_b_1()
                    .border_color(colors.border)
                    .child(
                        div()
                            .id("surface-tabs")
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap_1()
                            .overflow_x_scroll()
                            .children(tabs),
                    )
                    .child(self.add_menu(&thread_id, cx))
                    .child(
                        Button::new("maximize-panel")
                            .icon(if maximized {
                                IconName::Minimize2
                            } else {
                                IconName::Maximize2
                            })
                            .size(ButtonSize::IconSm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip(
                                Tooltip::new(if maximized {
                                    m::restore_panel()
                                } else {
                                    m::maximize_panel()
                                })
                                .side(Side::Bottom),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_maximized(cx))),
                    )
                    .child(
                        Button::new("close-panel")
                            .icon(IconName::X)
                            .size(ButtonSize::IconSm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip(Tooltip::new(m::close_panel()).side(Side::Bottom))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.close(&close_thread, cx)),
                            ),
                    ),
            )
            .child(div().flex().flex_col().flex_1().min_h_0().child(body))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
