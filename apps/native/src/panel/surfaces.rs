use crate::m;
use hoshi_core::DiffScope;
use hoshi_ui::prelude::IconName;

/// A kind of view the side panel can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SurfaceKind {
    Terminal,
    Diff,
    PullRequest,
    Browser,
}

/// Launcher order.
pub const SURFACE_KINDS: [SurfaceKind; 4] = [
    SurfaceKind::Terminal,
    SurfaceKind::Diff,
    SurfaceKind::PullRequest,
    SurfaceKind::Browser,
];

impl SurfaceKind {
    /// Kinds a thread shows at most once; opening another one focuses it.
    pub fn single_instance(self) -> bool {
        self != Self::Terminal
    }

    /// The single key that opens it from the launcher.
    pub fn shortcut(self) -> &'static str {
        match self {
            Self::Terminal => "T",
            Self::Diff => "D",
            Self::PullRequest => "P",
            Self::Browser => "B",
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::Terminal => m::surface_terminal(),
            Self::Diff => m::surface_diff(),
            Self::PullRequest => m::surface_pull_request(),
            Self::Browser => m::surface_browser(),
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            Self::Terminal => IconName::SquareTerminal,
            Self::Diff => IconName::FileDiff,
            Self::PullRequest => IconName::GitPullRequest,
            Self::Browser => IconName::Globe,
        }
    }

    pub fn available(self, context: SurfaceContext) -> bool {
        match self {
            Self::PullRequest => context.has_pull_request,
            Self::Terminal => context.has_terminal,
            Self::Diff | Self::Browser => true,
        }
    }
}

/// What decides whether a kind of surface can open for a thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceContext {
    pub has_pull_request: bool,
    /// A terminal host is plugged into the panel.
    pub has_terminal: bool,
}

/// The available kind whose launcher shortcut is `key`, ignoring case.
pub fn kind_for_shortcut(key: &str, context: SurfaceContext) -> Option<SurfaceKind> {
    SURFACE_KINDS
        .into_iter()
        .find(|kind| kind.shortcut().eq_ignore_ascii_case(key) && kind.available(context))
}

/// What the diff surface compares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffShown {
    Scope(DiffScope),
    /// What the turn of the surface's `turn_id` changed.
    Turn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceState {
    Diff {
        shown: DiffShown,
        turn_id: Option<String>,
        /// The file to open and scroll to, as a turn's changes ask.
        focus_path: Option<String>,
    },
    PullRequest,
    /// Its id is also its terminal's id.
    Terminal,
    Browser,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Surface {
    pub id: String,
    pub state: SurfaceState,
}

impl Surface {
    pub fn new(kind: SurfaceKind, id: String) -> Self {
        let state = match kind {
            SurfaceKind::Diff => SurfaceState::Diff {
                shown: DiffShown::Scope(DiffScope::Uncommitted),
                turn_id: None,
                focus_path: None,
            },
            SurfaceKind::PullRequest => SurfaceState::PullRequest,
            SurfaceKind::Terminal => SurfaceState::Terminal,
            SurfaceKind::Browser => SurfaceState::Browser,
        };
        Self { id, state }
    }

    pub fn kind(&self) -> SurfaceKind {
        match self.state {
            SurfaceState::Diff { .. } => SurfaceKind::Diff,
            SurfaceState::PullRequest => SurfaceKind::PullRequest,
            SurfaceState::Terminal => SurfaceKind::Terminal,
            SurfaceState::Browser => SurfaceKind::Browser,
        }
    }
}

/// The surfaces a thread has open, in tab order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThreadSurfaces {
    pub surfaces: Vec<Surface>,
    pub active_id: Option<String>,
}

impl ThreadSurfaces {
    pub fn active(&self) -> Option<&Surface> {
        let id = self.active_id.as_deref()?;
        self.surfaces.iter().find(|surface| surface.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Surface> {
        self.surfaces.iter_mut().find(|surface| surface.id == id)
    }

    /// Focuses the surface of that kind if it is single-instance and open, or adds one.
    pub fn open(&mut self, kind: SurfaceKind, make_id: impl FnOnce() -> String) {
        if kind.single_instance()
            && let Some(existing) = self.surfaces.iter().find(|surface| surface.kind() == kind)
        {
            self.active_id = Some(existing.id.clone());
            return;
        }
        let surface = Surface::new(kind, make_id());
        self.active_id = Some(surface.id.clone());
        self.surfaces.push(surface);
    }

    /// Opens the diff surface on what a turn changed, at `focus_path` when given.
    pub fn show_turn_diff(
        &mut self,
        turn_id: &str,
        focus_path: Option<String>,
        make_id: impl FnOnce() -> String,
    ) {
        self.open(SurfaceKind::Diff, make_id);
        let Some(id) = self.active_id.clone() else {
            return;
        };
        if let Some(surface) = self.get_mut(&id) {
            surface.state = SurfaceState::Diff {
                shown: DiffShown::Turn,
                turn_id: Some(turn_id.to_owned()),
                focus_path,
            };
        }
    }

    pub fn activate(&mut self, id: &str) {
        if self.surfaces.iter().any(|surface| surface.id == id) {
            self.active_id = Some(id.to_owned());
        }
    }

    /// Closing the active surface focuses its right neighbour, or the left one at the end.
    pub fn close(&mut self, id: &str) -> Option<Surface> {
        let index = self.surfaces.iter().position(|surface| surface.id == id)?;
        let closed = self.surfaces.remove(index);
        if self.active_id.as_deref() == Some(id) {
            self.active_id = self
                .surfaces
                .get(index)
                .or_else(|| {
                    index
                        .checked_sub(1)
                        .and_then(|left| self.surfaces.get(left))
                })
                .map(|surface| surface.id.clone());
        }
        Some(closed)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DiffShown, Surface, SurfaceContext, SurfaceKind, SurfaceState, ThreadSurfaces,
        kind_for_shortcut,
    };
    use hoshi_core::DiffScope;

    fn ids() -> impl FnMut() -> String {
        let mut next = 0;
        move || {
            next += 1;
            format!("s{next}")
        }
    }

    fn kinds(panel: &ThreadSurfaces) -> Vec<SurfaceKind> {
        panel.surfaces.iter().map(Surface::kind).collect()
    }

    #[test]
    fn opening_adds_a_surface_with_its_defaults_and_focuses_it() {
        let mut panel = ThreadSurfaces::default();
        panel.open(SurfaceKind::Diff, ids());

        assert_eq!(
            panel.surfaces,
            [Surface {
                id: "s1".into(),
                state: SurfaceState::Diff {
                    shown: DiffShown::Scope(DiffScope::Uncommitted),
                    turn_id: None,
                    focus_path: None,
                },
            }]
        );
        assert_eq!(panel.active_id.as_deref(), Some("s1"));
    }

    #[test]
    fn a_single_instance_kind_already_open_is_focused_not_duplicated() {
        let mut make_id = ids();
        let mut panel = ThreadSurfaces::default();
        panel.open(SurfaceKind::PullRequest, &mut make_id);
        panel.open(SurfaceKind::Diff, &mut make_id);

        panel.open(SurfaceKind::PullRequest, &mut make_id);

        assert_eq!(kinds(&panel), [SurfaceKind::PullRequest, SurfaceKind::Diff]);
        assert_eq!(panel.active_id.as_deref(), Some("s1"));
    }

    #[test]
    fn terminals_open_side_by_side() {
        let mut make_id = ids();
        let mut panel = ThreadSurfaces::default();
        panel.open(SurfaceKind::Terminal, &mut make_id);
        panel.open(SurfaceKind::Terminal, &mut make_id);

        assert_eq!(
            kinds(&panel),
            [SurfaceKind::Terminal, SurfaceKind::Terminal]
        );
        assert_eq!(panel.active_id.as_deref(), Some("s2"));
    }

    fn two() -> ThreadSurfaces {
        let mut make_id = ids();
        let mut panel = ThreadSurfaces::default();
        panel.open(SurfaceKind::Diff, &mut make_id);
        panel.open(SurfaceKind::PullRequest, &mut make_id);
        panel.activate("s1");
        panel
    }

    #[test]
    fn closing_the_active_one_focuses_its_right_neighbour() {
        let mut panel = two();
        panel.close("s1");
        assert_eq!(panel.active_id.as_deref(), Some("s2"));
    }

    #[test]
    fn closing_the_last_active_one_focuses_the_left_neighbour() {
        let mut panel = two();
        panel.activate("s2");
        panel.close("s2");
        assert_eq!(panel.active_id.as_deref(), Some("s1"));
    }

    #[test]
    fn closing_an_inactive_one_keeps_the_focus() {
        let mut panel = two();
        panel.close("s2");
        assert_eq!(panel.active_id.as_deref(), Some("s1"));
    }

    #[test]
    fn closing_everything_leaves_nothing_active() {
        let mut panel = two();
        panel.close("s1");
        panel.close("s2");
        assert_eq!(panel, ThreadSurfaces::default());
        assert!(panel.close("s3").is_none());
    }

    #[test]
    fn a_turn_diff_opens_on_a_turn_focused_on_a_file() {
        let mut panel = ThreadSurfaces::default();
        panel.show_turn_diff("m1", Some("src/a.ts".into()), ids());

        assert_eq!(
            panel.active().map(|surface| surface.state.clone()),
            Some(SurfaceState::Diff {
                shown: DiffShown::Turn,
                turn_id: Some("m1".into()),
                focus_path: Some("src/a.ts".into()),
            })
        );
    }

    #[test]
    fn a_turn_diff_reuses_the_diff_surface_already_open() {
        let mut make_id = ids();
        let mut panel = ThreadSurfaces::default();
        panel.open(SurfaceKind::Diff, &mut make_id);
        panel.open(SurfaceKind::Browser, &mut make_id);

        panel.show_turn_diff("m2", None, &mut make_id);

        assert_eq!(kinds(&panel), [SurfaceKind::Diff, SurfaceKind::Browser]);
        assert_eq!(panel.active_id.as_deref(), Some("s1"));
        assert!(matches!(
            panel.surfaces[0].state,
            SurfaceState::Diff {
                shown: DiffShown::Turn,
                ..
            }
        ));
    }

    #[test]
    fn shortcuts_ignore_case_and_skip_what_cannot_open() {
        let none = SurfaceContext::default();
        assert_eq!(kind_for_shortcut("d", none), Some(SurfaceKind::Diff));
        assert_eq!(kind_for_shortcut("B", none), Some(SurfaceKind::Browser));
        assert_eq!(kind_for_shortcut("p", none), None);
        assert_eq!(kind_for_shortcut("t", none), None);
        let all = SurfaceContext {
            has_pull_request: true,
            has_terminal: true,
        };
        assert_eq!(kind_for_shortcut("p", all), Some(SurfaceKind::PullRequest));
        assert_eq!(kind_for_shortcut("t", all), Some(SurfaceKind::Terminal));
        assert_eq!(kind_for_shortcut("x", all), None);
    }
}
