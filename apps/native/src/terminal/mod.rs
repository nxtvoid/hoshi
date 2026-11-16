//! The terminals docked under a thread's chat: tabs per thread, opened from the header or
//! with Ctrl+J, kept across restarts. The right panel shows terminals through `panel`.

mod colors;
mod drawer;
mod emulator;
mod frame;
mod keys;
mod links;
mod mouse;
mod panel;
mod setup_bar;
mod store;
mod view;

use crate::m;
use crate::state::Workspace;
use drawer::Drawer;
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_ui::prelude::*;
pub use panel::PanelTerminals;
pub use setup_bar::WorktreeSetupBar;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;
pub use view::{ShellExited, TerminalView};

actions!(terminal, [ToggleTerminal]);

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("ctrl-j", ToggleTerminal, None)]);
}

/// The terminal drawer under the chat, one per thread, opened from the header.
pub struct TerminalDrawer {
    workspace: Entity<Workspace>,
    drawers: HashMap<String, Drawer>,
    height: f32,
    /// The attached views, all of the thread on screen.
    views: HashMap<String, Entity<TerminalView>>,
    views_thread: Option<String>,
    pending_focus: Option<String>,
    /// Where `terminals.json` lives; nothing is kept without one.
    home: Option<PathBuf>,
    saving: Option<Task<()>>,
    _forget: Subscription,
}

impl TerminalDrawer {
    pub fn new(workspace: Entity<Workspace>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = workspace
            .read(cx)
            .settings_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let saved = home.as_deref().map(store::load).unwrap_or_default();
        let forget = cx.observe(&workspace, |this, _, cx| this.forget_deleted(cx));
        let mut this = Self {
            workspace,
            drawers: saved.drawers,
            height: saved.height,
            views: HashMap::new(),
            views_thread: None,
            pending_focus: None,
            home,
            saving: None,
            _forget: forget,
        };
        this.forget_deleted(cx);
        this
    }

    /// Drops the drawers of threads that were deleted.
    fn forget_deleted(&mut self, cx: &mut Context<Self>) {
        let model = &self.workspace.read(cx).model;
        let before = self.drawers.len();
        self.drawers
            .retain(|thread_id, _| model.thread(thread_id).is_some());
        if self.drawers.len() != before {
            self.save(cx);
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        let saved = store::Saved {
            height: self.height,
            drawers: self.drawers.clone(),
        };
        self.saving = Some(cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            cx.background_spawn(async move { store::save(&home, &saved) })
                .await
                .ok();
        }));
    }

    pub fn is_open(&self, thread_id: &str) -> bool {
        self.drawers
            .get(thread_id)
            .is_some_and(|drawer| drawer.open)
    }

    pub fn toggle(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        self.change(thread_id, |drawer| drawer.toggle(new_id), cx);
    }

    pub fn add_terminal(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        self.change(thread_id, |drawer| drawer.add(new_id), cx);
    }

    pub fn activate(&mut self, thread_id: &str, terminal_id: &str, cx: &mut Context<Self>) {
        self.change(thread_id, |drawer| drawer.activate(terminal_id), cx);
    }

    /// Opens the drawer on a terminal the engine started, such as a worktree's setup.
    pub fn show_terminal(&mut self, thread_id: &str, terminal_id: &str, cx: &mut Context<Self>) {
        self.change(thread_id, |drawer| drawer.show(terminal_id), cx);
    }

    /// Stops the terminal's shell too.
    pub fn close_terminal(&mut self, thread_id: &str, terminal_id: &str, cx: &mut Context<Self>) {
        if let Some(view) = self.views.remove(terminal_id) {
            view.update(cx, |view, _| view.detach());
        }
        self.workspace.read(cx).engine.close_terminal(terminal_id);
        self.change(thread_id, |drawer| drawer.close(terminal_id), cx);
    }

    fn change(
        &mut self,
        thread_id: &str,
        change: impl FnOnce(&Drawer) -> Drawer,
        cx: &mut Context<Self>,
    ) {
        let drawer = change(&self.drawers.get(thread_id).cloned().unwrap_or_default());
        if drawer.open {
            self.pending_focus = drawer.active_id.clone();
        }
        self.drawers.insert(thread_id.to_owned(), drawer);
        self.save(cx);
        cx.notify();
    }

    pub fn set_height(&mut self, height: f32, cx: &mut Context<Self>) {
        self.height = drawer::clamp_height(height);
        self.save(cx);
        cx.notify();
    }

    /// Detaches the views no longer on screen: another thread's, a hidden drawer's or a
    /// closed tab's.
    fn keep_views(&mut self, thread_id: Option<&str>, drawer: Option<&Drawer>, cx: &mut App) {
        let same_thread = self.views_thread.as_deref() == thread_id;
        self.views_thread = thread_id.map(str::to_owned);
        let gone: Vec<String> = self
            .views
            .keys()
            .filter(|id| {
                !same_thread || !drawer.is_some_and(|drawer| drawer.terminal_ids.contains(id))
            })
            .cloned()
            .collect();
        for id in gone {
            if let Some(view) = self.views.remove(&id) {
                view.update(cx, |view, _| view.detach());
            }
        }
    }

    fn view(
        &mut self,
        thread_id: &str,
        terminal_id: &str,
        cx: &mut Context<Self>,
    ) -> Entity<TerminalView> {
        if let Some(view) = self.views.get(terminal_id) {
            return view.clone();
        }
        let workspace = self.workspace.clone();
        let open_here: view::OpenHere = {
            let drawer = cx.entity().downgrade();
            let thread_id = thread_id.to_owned();
            Rc::new(move |_, cx| {
                drawer
                    .update(cx, |drawer, cx| drawer.add_terminal(&thread_id, cx))
                    .ok();
            })
        };
        let view = cx.new(|cx| {
            TerminalView::new(
                workspace,
                thread_id.to_owned(),
                terminal_id.to_owned(),
                open_here,
                cx,
            )
        });
        self.views.insert(terminal_id.to_owned(), view.clone());
        view
    }

    fn tab(
        thread_id: &str,
        terminal_id: &str,
        number: usize,
        active: bool,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = m::terminal_tab(number);
        let activate = {
            let (thread_id, terminal_id) = (thread_id.to_owned(), terminal_id.to_owned());
            cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.activate(&thread_id, &terminal_id, cx)
            })
        };
        let middle_close = {
            let (thread_id, terminal_id) = (thread_id.to_owned(), terminal_id.to_owned());
            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                this.close_terminal(&thread_id, &terminal_id, cx)
            })
        };
        let close = {
            let (thread_id, terminal_id) = (thread_id.to_owned(), terminal_id.to_owned());
            cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.close_terminal(&thread_id, &terminal_id, cx)
            })
        };
        let group = SharedString::from(format!("terminal-tab-{terminal_id}"));
        div()
            .id(SharedString::from(format!("terminal-tab-{terminal_id}")))
            .group(group.clone())
            .flex_none()
            .h_6()
            .flex()
            .items_center()
            .rounded_md()
            .text_xs()
            .map(|this| {
                if active {
                    this.bg(colors.secondary)
                        .text_color(colors.secondary_foreground)
                } else {
                    let hover = colors.accent.opacity(0.6);
                    this.text_color(colors.muted_foreground)
                        .hover(move |style| style.bg(hover))
                }
            })
            .on_click(activate)
            .on_mouse_down(MouseButton::Middle, middle_close)
            .child(
                div()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .pl_2()
                    .child(Icon::new(IconName::SquareTerminal).size_3p5())
                    .child(label.clone()),
            )
            .child(
                div()
                    .id(SharedString::from(format!("terminal-close-{terminal_id}")))
                    .mx_0p5()
                    .size_4()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .opacity(0.6)
                    .group_hover(group, |style| style.opacity(1.))
                    .hover(|style| style.bg(colors.accent))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        gpui_kit::base::GlobalState::suppress_text_selection(cx)
                    })
                    .on_click(close)
                    .child(Icon::new(IconName::X).size_3())
                    .with_tooltip(m::close_surface(&label)),
            )
            .into_any_element()
    }
}

impl Render for TerminalDrawer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let thread_id = self.workspace.read(cx).open_thread_id(cx);
        let drawer = thread_id
            .as_deref()
            .and_then(|id| self.drawers.get(id))
            .filter(|drawer| drawer.open)
            .cloned();
        self.keep_views(thread_id.as_deref(), drawer.as_ref(), cx);
        let (Some(thread_id), Some(drawer)) = (thread_id, drawer) else {
            return div().id("terminal-drawer");
        };
        let active = drawer
            .active_id
            .clone()
            .map(|id| self.view(&thread_id, &id, cx));
        let focus_now = active
            .as_ref()
            .is_some_and(|view| self.pending_focus.as_deref() == Some(view.read(cx).terminal_id()));
        if let Some(view) = active.clone().filter(|_| focus_now) {
            self.pending_focus = None;
            cx.defer_in(window, move |_, window, cx| {
                let focus = view.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            });
        }
        let tabs: Vec<_> = drawer
            .terminal_ids
            .iter()
            .enumerate()
            .map(|(index, id)| {
                let active = drawer.active_id.as_deref() == Some(id.as_str());
                Self::tab(&thread_id, id, index + 1, active, &colors, cx)
            })
            .collect();
        let add = {
            let thread_id = thread_id.clone();
            cx.listener(move |this, _: &ClickEvent, _, cx| this.add_terminal(&thread_id, cx))
        };
        let hide = {
            let thread_id = thread_id.clone();
            cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(&thread_id, cx))
        };
        let handle = cx.entity().downgrade();

        div()
            .id("terminal-drawer")
            .relative()
            .flex_none()
            .h(px(self.height))
            .max_h(relative(0.7))
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(colors.border)
            .bg(colors.background)
            .child(
                ResizeHandle::new("terminal-resize", ResizeSide::Top, self.height)
                    .range(drawer::MIN_HEIGHT, drawer::MAX_HEIGHT)
                    .default_value(drawer::DEFAULT_HEIGHT)
                    .on_change(move |height, _, cx| {
                        handle
                            .update(cx, |drawer, cx| drawer.set_height(height, cx))
                            .ok();
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .h_8()
                    .flex()
                    .items_center()
                    .gap_1()
                    .pl_2()
                    .pr_1()
                    .child(
                        div()
                            .id("terminal-tabs")
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_1()
                            .overflow_x_scroll()
                            .children(tabs),
                    )
                    .child(
                        Button::new("terminal-new")
                            .icon(IconName::Plus)
                            .size(ButtonSize::IconSm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip(Tooltip::new(m::new_terminal()))
                            .on_click(add),
                    )
                    .child(
                        Button::new("terminal-hide")
                            .icon(IconName::ChevronDown)
                            .size(ButtonSize::IconSm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip(Tooltip::new(m::hide_terminal()))
                            .on_click(hide),
                    ),
            )
            .child(div().flex_1().min_h_0().children(active))
    }
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
impl TerminalDrawer {
    fn drawer(&self, thread_id: &str) -> Drawer {
        self.drawers.get(thread_id).cloned().unwrap_or_default()
    }

    fn attached(&self, terminal_id: &str) -> Option<Entity<TerminalView>> {
        self.views.get(terminal_id).cloned()
    }
}

#[cfg(test)]
mod tests;
