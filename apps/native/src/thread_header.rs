//! The open thread in the title bar: where it lives, its title, and its branch and pull request.

use crate::details::Surface;
use crate::sections::View;
use crate::sidebar::{Sidebar, initials, project_mark, pull_request_mark};
use crate::{HoshiWindow, m};
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_core::{ChecksState, Thread};
use hoshi_ui::prelude::{DragRegion as _, IconName, Side, Tooltip, WithTooltip as _};
use hoshi_ui::theme::{ActiveTheme as _, Colors};
use std::time::Duration;

const COPIED: Duration = Duration::from_millis(1500);

actions!(thread_header, [TogglePanel]);

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("ctrl-alt-b", TogglePanel, None)]);
}

/// Redraws the window when the sidebar switches views: the inbox puts the project before the
/// title.
pub fn follow_sidebar_view(sidebar: &Entity<Sidebar>, cx: &mut Context<HoshiWindow>) {
    let mut shown = sidebar.read(cx).view();
    cx.observe(sidebar, move |_, sidebar, cx| {
        let view = sidebar.read(cx).view();
        if view != shown {
            shown = view;
            cx.notify();
        }
    })
    .detach();
}

impl HoshiWindow {
    /// Everything between the sidebar button and the header's actions; what isn't a button
    /// moves the window.
    pub(crate) fn thread_heading(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let colors = cx.hoshi().clone();
        let workspace = self.workspace.read(cx);
        let thread = self
            .transcript
            .as_ref()
            .and_then(|_| workspace.open_thread_id(cx))
            .and_then(|id| workspace.model.thread(&id).cloned());
        let heading = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .gap_2()
            .when(self.sidebar_open, |this| this.pl_3())
            .when(!self.sidebar_open, |this| this.pl_2());
        let Some(thread) = thread else {
            return heading.child(drag_space());
        };
        let from_inbox = self.sidebar.read(cx).view() == View::Inbox;
        let in_subagent = self
            .subagents
            .last()
            .map(|(view, _)| view.read(cx).title(cx));
        let title: SharedString = if thread.title.is_empty() {
            m::untitled_thread().into()
        } else {
            thread.title.clone().into()
        };
        heading
            .when(from_inbox, |this| {
                this.children(self.place_crumb(&thread, &colors, cx))
            })
            .when(in_subagent.is_some(), |this| {
                this.child(
                    crumb_button("leave-subagents", &colors)
                        .max_w(relative(0.3))
                        .child(div().truncate().child(title.clone()))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.subagents.clear();
                            cx.notify();
                        })),
                )
                .child(slash(&colors))
            })
            .child(
                div()
                    .drag_region()
                    .h_full()
                    .min_w_0()
                    .max_w(relative(0.4))
                    .flex()
                    .items_center()
                    .truncate()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(in_subagent.unwrap_or(title)),
            )
            .children(self.git_chip(&thread, &colors, window, cx))
            .child(drag_space())
    }

    /// The thread's project, or Chats, opening that list in the sidebar.
    fn place_crumb(
        &self,
        thread: &Thread,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let Some(project_id) = thread.project_id.clone() else {
            let button = crumb_button("chats-crumb", colors)
                .flex_none()
                .child(Icon::new(IconName::MessageCircle).size_4())
                .child(m::chats())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.sidebar
                        .update(cx, |sidebar, cx| sidebar.show(View::Chats, None, cx));
                }));
            return Some(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(button)
                    .child(slash(colors))
                    .into_any_element(),
            );
        };
        let project = self.workspace.read(cx).model.project(&project_id)?.clone();
        let icon = self.sidebar.read(cx).project_icon(&project_id);
        let mark = div()
            .size_4()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .bg(colors.muted)
            .text_color(colors.muted_foreground)
            .text_size(px(8.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(project_mark(
                icon,
                initials(&project.title),
                px(16.),
                px(4.),
            ));
        let button = crumb_button("project-crumb", colors)
            .flex_shrink_0()
            .min_w_0()
            .child(mark)
            .child(div().truncate().child(project.title.clone()))
            .on_click(cx.listener(move |this, _, _, cx| {
                let project_id = Some(project_id.clone());
                this.sidebar.update(cx, |sidebar, cx| {
                    sidebar.show(View::Project, project_id, cx)
                });
            }))
            .with_tooltip(Tooltip::new(project.workspace_root.clone()).side(Side::Bottom));
        Some(
            div()
                .max_w(relative(0.3))
                .min_w_0()
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap_2()
                .child(button)
                .child(slash(colors))
                .into_any_element(),
        )
    }

    /// The branch, copied on click, and the pull request, opened in the panel, as one chip.
    fn git_chip(
        &self,
        thread: &Thread,
        colors: &Colors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let pull_request = self
            .workspace
            .read(cx)
            .pull_requests
            .get(&thread.id)
            .cloned();
        if thread.branch.is_none() && pull_request.is_none() {
            return None;
        }
        let both = thread.branch.is_some() && pull_request.is_some();
        let branch = thread.branch.clone().map(|branch| {
            let copied = window.use_keyed_state("header-branch-copied", cx, |_, _| false);
            let shown = *copied.read(cx);
            let icon = if shown {
                IconName::Check
            } else if thread.worktree_path.is_some() {
                IconName::GitFork
            } else {
                IconName::GitBranch
            };
            let label: SharedString = if shown {
                m::copied().into()
            } else {
                m::copy_branch(&branch).into()
            };
            segment("header-branch", colors)
                .rounded_l_full()
                .when(pull_request.is_none(), |this| this.rounded_r_full())
                .child(
                    Icon::new(icon)
                        .size_3p5()
                        .text_color(colors.muted_foreground),
                )
                .child(
                    div()
                        .max_w_48()
                        .truncate()
                        .text_color(colors.muted_foreground)
                        .child(branch.clone()),
                )
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(branch.clone()));
                    copied.update(cx, |copied, cx| {
                        *copied = true;
                        cx.notify();
                    });
                    let copied = copied.downgrade();
                    cx.spawn(async move |cx| {
                        cx.background_executor().timer(COPIED).await;
                        copied
                            .update(cx, |copied, cx| {
                                *copied = false;
                                cx.notify();
                            })
                            .ok();
                    })
                    .detach();
                })
                .with_tooltip(Tooltip::new(label).side(Side::Bottom))
        });
        let pull_request = pull_request.map(|pull_request| {
            let (icon, color) = pull_request_mark(&pull_request, colors);
            let checks = pull_request.checks.map(|checks| match checks {
                ChecksState::Passing => colors.pr_open,
                ChecksState::Failing => colors.status_error,
                ChecksState::Pending => colors.status_waiting,
            });
            let thread_id = thread.id.clone();
            segment("header-pull-request", colors)
                .rounded_r_full()
                .when(!both, |this| this.rounded_l_full())
                .child(Icon::new(icon).size_3p5().text_color(color))
                .child(format!("#{}", pull_request.number))
                .when_some(checks, |this, color| {
                    this.child(div().size_1p5().rounded_full().bg(color))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.show_surface(&thread_id, Surface::PullRequest, cx)
                }))
        });
        Some(
            div()
                .h_6()
                .flex_none()
                .flex()
                .items_center()
                .rounded_full()
                .border_1()
                .border_color(colors.border)
                .text_xs()
                .children(branch)
                .when(both, |this| {
                    this.child(div().w_px().h_3().flex_none().bg(colors.border))
                })
                .children(pull_request),
        )
    }
}

fn drag_space() -> impl IntoElement {
    div().drag_region().flex_1().min_w_4().h_full()
}

fn slash(colors: &Colors) -> impl IntoElement {
    div()
        .flex_none()
        .text_sm()
        .text_color(colors.muted_foreground.opacity(0.6))
        .child("/")
}

fn crumb_button(id: &'static str, colors: &Colors) -> Stateful<Div> {
    let hover = colors.foreground;
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex()
        .items_center()
        .gap_1p5()
        .rounded_md()
        .cursor_pointer()
        .text_sm()
        .text_color(colors.muted_foreground)
        .hover(move |style| style.text_color(hover))
}

fn segment(id: &'static str, colors: &Colors) -> Stateful<Div> {
    let hover = colors.accent;
    div()
        .id(id)
        .debug_selector(move || id.into())
        .h_full()
        .min_w_0()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
}

#[cfg(test)]
mod tests {
    use crate::HoshiWindow;
    use crate::sections::View;
    use crate::state::{self, Workspace};
    use gpui_kit::component::theme::ThemeMode;
    use gpui_kit::{
        AppContext as _, Bounds, Entity, Modifiers, Point, TestAppContext, VisualTestContext,
        WindowBounds, WindowOptions, px, size,
    };
    use hoshi_core::{AccessMode, Command, PullRequest, ThreadWorkspace};
    use std::sync::Arc;

    fn redraw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        for _ in 0..3 {
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.run_until_parked();
        }
    }

    fn pull_request() -> PullRequest {
        serde_json::from_value(serde_json::json!({
            "forge": "github",
            "number": 7,
            "title": "Header",
            "url": "https://example.com/7",
            "headBranch": "feature",
            "baseBranch": "main",
            "state": "open",
            "checks": "passing",
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

    fn open_thread(cx: &mut TestAppContext) -> (VisualTestContext, Entity<HoshiWindow>) {
        cx.executor().allow_parking();
        let dir = std::env::temp_dir().join(format!("hoshi-header-{}", uuid::Uuid::new_v4()));
        let home = dir.join("home");
        std::fs::create_dir_all(dir.join("project")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let (notify, notifications) = state::channel();
        let engine = Arc::new(hoshi_engine::Engine::open(&home, notify).unwrap());
        engine
            .dispatch(Command::ProjectCreate {
                project_id: "p".into(),
                title: "project".into(),
                workspace_root: dir.join("project").to_string_lossy().into_owned(),
            })
            .unwrap();
        engine
            .dispatch(Command::ThreadCreate {
                thread_id: "t".into(),
                project_id: Some("p".into()),
                title: String::new(),
                workspace: ThreadWorkspace::Local,
                provider: hoshi_core::default_provider(),
                model: None,
                effort: None,
                fast_mode: false,
                ultracode: false,
                access_mode: AccessMode::default(),
                plan_mode: false,
            })
            .unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            hoshi_ui::DialogLayer::init(cx);
            crate::terminal::init(cx);
            crate::sidebar::init(cx);
            crate::palette::init(cx);
            super::init(cx);
            crate::markdown::init(engine.clone(), home.clone(), cx);
            hoshi_ui::theme::apply(ThemeMode::Light, cx);
        });
        let workspace = cx.update(|cx| cx.new(|cx| Workspace::new(engine, notifications, cx)));
        let (window, shown) = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1200.), px(800.)),
                })),
                ..Default::default()
            };
            let mut shown = None;
            let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
                let view = cx.new(|cx| HoshiWindow::new(workspace.clone(), window, cx));
                shown = Some(view.clone());
                view
            })
            .unwrap();
            (window, shown.unwrap())
        });
        let mut cx = VisualTestContext::from_window(window, cx);
        workspace.update(&mut cx, |workspace, cx| workspace.open_thread("t", cx));
        redraw(&mut cx);
        (cx, shown)
    }

    #[gpui_kit::test]
    fn the_inbox_puts_the_project_before_the_title_and_opens_its_list(cx: &mut TestAppContext) {
        let (mut cx, shown) = open_thread(cx);
        assert!(cx.debug_bounds("project-crumb").is_none());
        let sidebar = cx.read(|cx| shown.read(cx).sidebar.clone());
        sidebar.update(&mut cx, |sidebar, cx| sidebar.show(View::Inbox, None, cx));
        redraw(&mut cx);
        let crumb = cx.debug_bounds("project-crumb").expect("the project crumb");
        cx.simulate_click(crumb.center(), Modifiers::default());
        redraw(&mut cx);
        assert_eq!(cx.read(|cx| sidebar.read(cx).view()), View::Project);
        assert!(cx.debug_bounds("project-crumb").is_none());
    }

    #[gpui_kit::test]
    fn a_notice_over_the_empty_draft_leaves_its_heading_uncovered(cx: &mut TestAppContext) {
        let (mut cx, shown) = open_thread(cx);
        let workspace = cx.read(|cx| shown.read(cx).workspace.clone());
        workspace.update(&mut cx, |workspace, cx| {
            workspace.start_draft(Some("p".into()), cx)
        });
        cx.update(|_, cx| {
            crate::toast::show(
                crate::toast::Toast {
                    icon: hoshi_ui::prelude::IconName::AlarmClock,
                    message: "Snoozed thread".into(),
                    action: None,
                },
                cx,
            )
        });
        redraw(&mut cx);
        let heading = cx.debug_bounds("draft-prompt").expect("the heading");
        let notice = cx.debug_bounds("toast").expect("the notice");
        assert!(
            notice.top() >= heading.bottom() || notice.bottom() <= heading.top(),
            "the notice {notice:?} covers the heading {heading:?}"
        );
    }

    #[gpui_kit::test]
    fn the_pull_request_in_the_chip_opens_in_the_panel(cx: &mut TestAppContext) {
        let (mut cx, shown) = open_thread(cx);
        assert!(cx.debug_bounds("header-pull-request").is_none());
        let workspace = cx.read(|cx| shown.read(cx).workspace.clone());
        workspace.update(&mut cx, |workspace, cx| {
            workspace.pull_requests.insert("t".into(), pull_request());
            cx.notify();
        });
        redraw(&mut cx);
        let pill = cx
            .debug_bounds("header-pull-request")
            .expect("the pull request");
        let panel = cx.read(|cx| shown.read(cx).panel.clone());
        assert!(!cx.read(|cx| panel.read(cx).is_open("t")));
        cx.simulate_click(pill.center(), Modifiers::default());
        redraw(&mut cx);
        assert!(cx.read(|cx| panel.read(cx).is_open("t")));
    }
}
