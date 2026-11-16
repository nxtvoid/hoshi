//! Which terminals a thread's drawer holds and which one shows.

pub const MIN_HEIGHT: f32 = 120.;
pub const MAX_HEIGHT: f32 = 800.;
pub const DEFAULT_HEIGHT: f32 = 280.;

/// The terminals docked below a thread's chat, in tab order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drawer {
    pub open: bool,
    pub terminal_ids: Vec<String>,
    pub active_id: Option<String>,
}

impl Drawer {
    /// Opening a drawer with no terminal starts one.
    pub fn toggle(&self, make_id: impl FnOnce() -> String) -> Self {
        if self.open {
            return Self {
                open: false,
                ..self.clone()
            };
        }
        if !self.terminal_ids.is_empty() {
            return Self {
                open: true,
                ..self.clone()
            };
        }
        self.add(make_id)
    }

    pub fn add(&self, make_id: impl FnOnce() -> String) -> Self {
        let id = make_id();
        let mut terminal_ids = self.terminal_ids.clone();
        terminal_ids.push(id.clone());
        Self {
            open: true,
            terminal_ids,
            active_id: Some(id),
        }
    }

    /// Opens the drawer on terminal `id`, adding its tab if it has none.
    pub fn show(&self, id: &str) -> Self {
        let mut terminal_ids = self.terminal_ids.clone();
        if !terminal_ids.iter().any(|other| other == id) {
            terminal_ids.push(id.to_owned());
        }
        Self {
            open: true,
            terminal_ids,
            active_id: Some(id.to_owned()),
        }
    }

    pub fn activate(&self, id: &str) -> Self {
        if !self.terminal_ids.iter().any(|other| other == id) {
            return self.clone();
        }
        Self {
            active_id: Some(id.to_owned()),
            ..self.clone()
        }
    }

    /// Closing the active terminal focuses its right neighbour, or the left one at the end;
    /// closing the last one hides the drawer.
    pub fn close(&self, id: &str) -> Self {
        let Some(index) = self.terminal_ids.iter().position(|other| other == id) else {
            return self.clone();
        };
        let mut terminal_ids = self.terminal_ids.clone();
        terminal_ids.remove(index);
        if terminal_ids.is_empty() {
            return Self::default();
        }
        if self.active_id.as_deref() != Some(id) {
            return Self {
                terminal_ids,
                ..self.clone()
            };
        }
        let next = terminal_ids
            .get(index)
            .or_else(|| index.checked_sub(1).and_then(|left| terminal_ids.get(left)))
            .cloned();
        Self {
            open: self.open,
            terminal_ids,
            active_id: next,
        }
    }
}

pub fn clamp_height(height: f32) -> f32 {
    height.clamp(MIN_HEIGHT, MAX_HEIGHT).round()
}

#[cfg(test)]
mod tests {
    use super::{Drawer, clamp_height};

    fn drawer(ids: &[&str], active: &str) -> Drawer {
        Drawer {
            open: true,
            terminal_ids: ids.iter().map(|id| (*id).to_owned()).collect(),
            active_id: Some(active.to_owned()),
        }
    }

    #[test]
    fn opening_an_empty_drawer_starts_a_terminal() {
        let opened = Drawer::default().toggle(|| "a".into());
        assert_eq!(opened, drawer(&["a"], "a"));
        let hidden = opened.toggle(|| unreachable!());
        assert!(!hidden.open);
        assert_eq!(hidden.terminal_ids, vec!["a"]);
        assert_eq!(hidden.toggle(|| unreachable!()), opened);
    }

    #[test]
    fn showing_a_terminal_adds_its_tab_once() {
        let shown = drawer(&["a"], "a").show("setup");
        assert_eq!(shown, drawer(&["a", "setup"], "setup"));
        assert_eq!(shown.show("setup"), shown);
        assert_eq!(Drawer::default().show("setup"), drawer(&["setup"], "setup"));
    }

    #[test]
    fn closing_the_active_terminal_moves_to_a_neighbour() {
        let tabs = drawer(&["a", "b", "c"], "b");
        assert_eq!(tabs.close("b"), drawer(&["a", "c"], "c"));
        assert_eq!(drawer(&["a", "b"], "b").close("b"), drawer(&["a"], "a"));
        assert_eq!(tabs.close("a"), drawer(&["b", "c"], "b"));
        assert_eq!(drawer(&["a"], "a").close("a"), Drawer::default());
        assert_eq!(tabs.close("missing"), tabs);
    }

    #[test]
    fn activating_an_unknown_terminal_changes_nothing() {
        let tabs = drawer(&["a", "b"], "a");
        assert_eq!(tabs.activate("b"), drawer(&["a", "b"], "b"));
        assert_eq!(tabs.activate("z"), tabs);
    }

    #[test]
    fn the_height_stays_in_bounds() {
        assert_eq!(clamp_height(10.), 120.);
        assert_eq!(clamp_height(2000.), 800.);
        assert_eq!(clamp_height(300.4), 300.);
    }
}
