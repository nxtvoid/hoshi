//! The first-run setup: one page that picks a theme and language, gets the agents' CLIs ready and adds the
//! first projects. It opens once for a new user; anyone who already used hoshi gets it marked
//! done instead.

pub(crate) mod agent;
mod page;
mod projects;

#[cfg(test)]
mod ui_tests;

use crate::sections::View;
use crate::state::Workspace;
use crate::{HoshiWindow, look, palette};
use gpui_kit::*;
use hoshi_ui::prelude::*;

pub use page::{AfterExit, Onboarding, OnboardingEvent};

/// Asks the window to show the setup again, as settings' "Run setup again" does.
pub struct RunOnboarding;

impl EventEmitter<RunOnboarding> for Workspace {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirstRun {
    Welcome,
    /// Save it as done without showing it.
    Complete,
    Nothing,
}

/// Only an untouched workspace gets the setup; anyone who used hoshi before skips it.
pub fn first_run(completed_at: Option<&str>, used_before: bool) -> FirstRun {
    match (completed_at, used_before) {
        (Some(_), _) => FirstRun::Nothing,
        (None, true) => FirstRun::Complete,
        (None, false) => FirstRun::Welcome,
    }
}

fn used_before(workspace: &Workspace) -> bool {
    workspace.model.projects().next().is_some()
        || workspace.model.threads().next().is_some()
        || workspace.engine.snapshot().sequence > 0
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Saves the setup as done, finished or skipped.
pub fn complete(workspace: &Entity<Workspace>, cx: &mut App) {
    let (engine, path, previous, next) = workspace.update(cx, |workspace, cx| {
        let previous = workspace.settings.clone();
        workspace.settings.onboarding_completed_at = Some(now());
        cx.notify();
        (
            workspace.engine.clone(),
            workspace.settings_path.clone(),
            previous,
            hoshi_app::settings::Snapshot::of(&workspace.settings),
        )
    });
    if let Some(path) = path {
        cx.background_spawn(
            async move { hoshi_app::apply_settings(&engine, &path, &previous, &next) },
        )
        .detach_and_log_err(cx);
    }
}

/// Lets settings open the setup again, and opens it at startup when it is due, or when
/// `HOSHI_NATIVE_ONBOARDING` is set.
pub fn install(shown: &Entity<HoshiWindow>, window: &mut Window, cx: &mut App) {
    let workspace = shown.read(cx).workspace.clone();
    shown.update(cx, |_, cx| {
        cx.subscribe_in(
            &workspace,
            window,
            |this, _, _: &RunOnboarding, window, cx| {
                look::set_preview(None, cx);
                this.settings = None;
                this.open_onboarding(window, cx);
            },
        )
        .detach();
    });
    let action = if std::env::var_os("HOSHI_NATIVE_ONBOARDING").is_some() {
        FirstRun::Welcome
    } else {
        let workspace = workspace.read(cx);
        let completed_at = workspace.settings.onboarding_completed_at.as_deref();
        first_run(
            completed_at,
            completed_at.is_none() && used_before(workspace),
        )
    };
    match action {
        FirstRun::Welcome => shown.update(cx, |shown, cx| shown.open_onboarding(window, cx)),
        FirstRun::Complete => complete(&workspace, cx),
        FirstRun::Nothing => {}
    }
}

impl HoshiWindow {
    pub(crate) fn open_onboarding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.onboarding.is_some() {
            return;
        }
        let workspace = self.workspace.clone();
        let view = cx.new(|cx| Onboarding::new(workspace, cx));
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        let events = cx.subscribe_in(&view, window, |this, _, event, window, cx| {
            match event {
                OnboardingEvent::Exit(then) => {
                    complete(&this.workspace, cx);
                    this.onboarding = None;
                    if *then == Some(AfterExit::AddProject) {
                        this.open_palette(palette::Mode::AddProject, window, cx);
                    }
                }
                OnboardingEvent::ProjectAdded(project_id) => {
                    let project_id = project_id.clone();
                    this.sidebar.update(cx, |sidebar, cx| {
                        sidebar.show(View::Project, Some(project_id.clone()), cx)
                    });
                    this.workspace.update(cx, |workspace, cx| {
                        workspace.start_draft(Some(project_id), cx)
                    });
                }
            }
            cx.notify();
        });
        self.onboarding = Some((view, events));
        cx.notify();
    }

    /// The setup filling the window, under a strip that moves it, while it is open.
    pub(crate) fn onboarding_page(&self, cx: &App) -> Option<AnyElement> {
        let (view, _) = self.onboarding.as_ref()?;
        let colors = cx.hoshi();
        Some(
            div()
                .size_full()
                .flex()
                .flex_col()
                .font_family(gpui_kit::component::Theme::global(cx).font_family.clone())
                .bg(colors.background)
                .text_color(colors.foreground)
                .child(
                    div()
                        .h_10()
                        .flex_none()
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .id("onboarding-title-bar")
                                .drag_region()
                                .flex_1()
                                .h_full(),
                        )
                        .child(hoshi_ui::prelude::WindowControls::new()),
                )
                .child(div().flex_1().min_h_0().child(view.clone()))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{FirstRun, first_run};

    #[test]
    fn only_an_untouched_workspace_gets_the_setup() {
        assert_eq!(first_run(None, false), FirstRun::Welcome);
        assert_eq!(first_run(None, true), FirstRun::Complete);
        assert_eq!(
            first_run(Some("2026-11-13T00:00:00.000Z"), false),
            FirstRun::Nothing
        );
        assert_eq!(
            first_run(Some("2026-11-13T00:00:00.000Z"), true),
            FirstRun::Nothing
        );
    }
}
