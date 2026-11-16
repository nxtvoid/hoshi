//! Terminals as surfaces of the right panel: the panel keeps their tabs, this draws and ends
//! their shells. Only the shown one streams; a hidden one detaches and redraws from its
//! history when it shows again.

use super::view::{OpenHere, TerminalView};
use crate::panel::{RightPanel, SurfaceKind, TerminalHost};
use crate::state::Workspace;
use gpui_kit::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

struct Shown {
    thread_id: String,
    view: Entity<TerminalView>,
}

struct State {
    workspace: Entity<Workspace>,
    panel: WeakEntity<RightPanel>,
    views: HashMap<String, Shown>,
    /// Terminals the panel drew since the app started: a new one takes the keyboard.
    seen: HashSet<String>,
    _subscriptions: Vec<Subscription>,
}

pub struct PanelTerminals {
    state: Rc<RefCell<State>>,
}

impl PanelTerminals {
    /// Lets the panel open terminals.
    pub fn register(
        workspace: &Entity<Workspace>,
        panel: &Entity<RightPanel>,
        cx: &mut App,
    ) -> Rc<PanelTerminals> {
        let state = Rc::new(RefCell::new(State {
            workspace: workspace.clone(),
            panel: panel.downgrade(),
            views: HashMap::new(),
            seen: HashSet::new(),
            _subscriptions: Vec::new(),
        }));
        let subscriptions = vec![
            {
                let state = Rc::downgrade(&state);
                cx.observe(panel, move |_, cx| {
                    if let Some(state) = state.upgrade() {
                        detach_hidden(&state, cx);
                    }
                })
            },
            {
                let state = Rc::downgrade(&state);
                cx.observe(workspace, move |_, cx| {
                    if let Some(state) = state.upgrade() {
                        detach_hidden(&state, cx);
                    }
                })
            },
        ];
        state.borrow_mut()._subscriptions = subscriptions;
        let host = Rc::new(PanelTerminals { state });
        panel.update(cx, |panel, cx| panel.set_terminal_host(host.clone(), cx));
        host
    }
}

fn detach(views: Vec<Shown>, cx: &mut App) {
    for shown in views {
        shown.view.update(cx, |view, _| view.detach());
    }
}

/// Detaches the views of a closed panel or of a thread no longer on screen.
fn detach_hidden(state: &Rc<RefCell<State>>, cx: &mut App) {
    let gone = {
        let mut state = state.borrow_mut();
        let open = state.workspace.read(cx).open_thread_id(cx);
        let panel_open = open.as_deref().is_some_and(|thread_id| {
            state
                .panel
                .upgrade()
                .is_some_and(|panel| panel.read(cx).is_open(thread_id))
        });
        let ids: Vec<String> = state
            .views
            .iter()
            .filter(|(_, shown)| !panel_open || open.as_deref() != Some(shown.thread_id.as_str()))
            .map(|(id, _)| id.clone())
            .collect();
        ids.iter()
            .filter_map(|id| state.views.remove(id))
            .collect::<Vec<_>>()
    };
    detach(gone, cx);
}

impl TerminalHost for PanelTerminals {
    fn render(
        &self,
        thread_id: &str,
        terminal_id: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let others = {
            let mut state = self.state.borrow_mut();
            let ids: Vec<String> = state
                .views
                .keys()
                .filter(|id| id.as_str() != terminal_id)
                .cloned()
                .collect();
            ids.iter()
                .filter_map(|id| state.views.remove(id))
                .collect::<Vec<_>>()
        };
        detach(others, cx);
        let existing = self
            .state
            .borrow()
            .views
            .get(terminal_id)
            .map(|shown| shown.view.clone());
        let view = match existing {
            Some(view) => view,
            None => {
                let (workspace, panel) = {
                    let state = self.state.borrow();
                    (state.workspace.clone(), state.panel.clone())
                };
                let open_here: OpenHere = {
                    let thread_id = thread_id.to_owned();
                    Rc::new(move |_, cx| {
                        panel
                            .update(cx, |panel, cx| {
                                panel.open_surface(&thread_id, SurfaceKind::Terminal, cx)
                            })
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
                let mut state = self.state.borrow_mut();
                state.views.insert(
                    terminal_id.to_owned(),
                    Shown {
                        thread_id: thread_id.to_owned(),
                        view: view.clone(),
                    },
                );
                if state.seen.insert(terminal_id.to_owned()) {
                    let focus = view.read(cx).focus_handle(cx);
                    window.defer(cx, move |window, cx| window.focus(&focus, cx));
                }
                view
            }
        };
        view.into_any_element()
    }

    fn close(&self, terminal_id: &str, cx: &mut App) {
        let closed = {
            let mut state = self.state.borrow_mut();
            state.seen.remove(terminal_id);
            state.views.remove(terminal_id)
        };
        detach(closed.into_iter().collect(), cx);
        let engine = self.state.borrow().workspace.read(cx).engine.clone();
        engine.close_terminal(terminal_id);
    }
}

#[cfg(test)]
impl PanelTerminals {
    pub fn view(&self, terminal_id: &str) -> Option<Entity<TerminalView>> {
        self.state
            .borrow()
            .views
            .get(terminal_id)
            .map(|shown| shown.view.clone())
    }
}
