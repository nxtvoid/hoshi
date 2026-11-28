#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_icon;
#[cfg(windows)]
mod app_user_model;
mod browser;
mod calendar;
mod choices;
mod compaction;
mod composer;
mod delegations;
mod details;
mod dialog_samples;
mod drafts;
mod edit_menu;
mod fold;
mod folder_picker;
mod fork;
mod format;
mod home;
mod html_page;
mod locale;
mod look;
mod markdown;
mod minimap;
mod moment;
mod notifications;
mod onboarding;
mod request_rows;
mod requests;
mod rewind;
#[allow(dead_code)]
mod m {
    include!(concat!(env!("OUT_DIR"), "/messages.rs"));
}
mod palette;
mod panel;
#[cfg(test)]
mod perf_tests;
mod providers;
mod pull_request_update;
mod sections;
mod settings;
mod sidebar;
mod snooze;
mod state;
mod subagent_view;
mod subagents;
mod task_list;
mod task_runs;
mod terminal;
mod themes;
mod thread_header;
mod toast;
mod tool_calls;
mod transcript;
mod turn_changes;
mod turn_status;
mod updates;
mod usage;
mod user_message;
mod window_state;
mod work;

use composer::Composer;
use details::{DetailsEvent, ThreadDetails};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_core::ThreadStatus;
use hoshi_ui::prelude::{
    Button, ButtonSize, ButtonVariant, Dialog, IconName, InitialFocus, OpenChange, Side, Tooltip,
    WindowControls,
};
use hoshi_ui::theme::{self, ActiveTheme};
use panel::RightPanel;
use settings::{Section, SettingsView};
use sidebar::{Sidebar, SidebarEvent};
use state::Workspace;
use std::sync::Arc;
use std::time::Duration;
use terminal::TerminalDrawer;
use transcript::TranscriptView;

struct HoshiWindow {
    workspace: Entity<Workspace>,
    transcript: Option<Entity<TranscriptView>>,
    /// The transcripts of the threads open just before, the latest first, kept with their
    /// scroll and what was opened in them.
    kept_transcripts: Vec<Entity<TranscriptView>>,
    composer: Entity<Composer>,
    sidebar: Entity<Sidebar>,
    sidebar_open: bool,
    details: Entity<ThreadDetails>,
    terminal: Entity<TerminalDrawer>,
    setup_bar: Entity<terminal::WorktreeSetupBar>,
    snooze_status: Entity<snooze::SnoozeLine>,
    snooze_woke: Entity<snooze::SnoozeLine>,
    panel: Entity<RightPanel>,
    mini: Entity<browser::MiniBrowser>,
    edit_menu: Entity<edit_menu::ReadOnlyMenu>,
    merge_back: Entity<fork::MergeBack>,
    toasts: Entity<toast::Toasts>,
    drafting: Option<state::Draft>,
    settings: Option<Entity<SettingsView>>,
    palette: Option<(Entity<palette::Palette>, Subscription)>,
    usage: Option<Entity<usage::UsageView>>,
    onboarding: Option<(Entity<onboarding::Onboarding>, Subscription)>,
    _observe: Subscription,
    _sidebar_events: Subscription,
    _details: [Subscription; 2],
    _transcript_events: Option<Subscription>,
    /// The subagents shown in place of the thread, the innermost last.
    subagents: Vec<(Entity<subagent_view::SubagentView>, [Subscription; 2])>,
    _subagent_opens: Option<Subscription>,
    _rewinds: Option<Subscription>,
    _sign_ins: Option<Subscription>,
}

impl HoshiWindow {
    fn new(workspace: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| Composer::new(workspace.clone(), window, cx));
        let sidebar = cx.new(|cx| Sidebar::new(workspace.clone(), window, cx));
        thread_header::follow_sidebar_view(&sidebar, cx);
        let details = cx.new(|cx| ThreadDetails::new(workspace.clone(), window, cx));
        let details_subscriptions = [
            cx.observe(&details, |_, _, cx| cx.notify()),
            cx.subscribe(&details, |this, _, event, cx| match event {
                DetailsEvent::ShowSurface { thread_id, surface } => {
                    this.show_surface(thread_id, *surface, cx)
                }
                DetailsEvent::ShowItem { thread_id, item_id } => {
                    this.show_item(thread_id, item_id, cx)
                }
            }),
        ];
        let terminal = cx.new(|cx| TerminalDrawer::new(workspace.clone(), window, cx));
        let panel = cx.new(|cx| RightPanel::new(workspace.clone(), window, cx));
        markdown::attach(&workspace, &panel, cx);
        terminal::PanelTerminals::register(&workspace, &panel, cx);
        let setup_bar =
            cx.new(|cx| terminal::WorktreeSetupBar::new(workspace.clone(), &terminal, cx));
        let snooze_status = cx.new(|cx| snooze::SnoozeLine::status(workspace.clone(), cx));
        let snooze_woke = cx.new(|cx| snooze::SnoozeLine::woke(workspace.clone(), cx));
        cx.observe(&panel, |_, _, cx| cx.notify()).detach();
        let mini = cx.new(|cx| browser::MiniBrowser::new(workspace.clone(), panel.clone(), cx));
        cx.observe(&mini, |_, _, cx| cx.notify()).detach();
        let sidebar_events =
            cx.subscribe_in(&sidebar, window, |this, _, event, window, cx| match event {
                SidebarEvent::Toggle => this.toggle_sidebar(cx),
                SidebarEvent::OpenSettings => this.open_settings(Section::General, window, cx),
                SidebarEvent::OpenUsage => this.open_usage(window, cx),
                SidebarEvent::OpenPalette(mode) => this.open_palette(*mode, window, cx),
                SidebarEvent::OpenPullRequest(thread_id) => {
                    this.show_surface(thread_id, details::Surface::PullRequest, cx)
                }
            });
        let observe = cx.observe_in(&workspace, window, |this, workspace, window, cx| {
            let open = workspace.read(cx).open.clone();
            let shown = this
                .transcript
                .as_ref()
                .map(|transcript| transcript.read(cx).thread_id(cx));
            let wanted = open.as_ref().map(|open| open.read(cx).thread_id.clone());
            let drafting = workspace.read(cx).draft.clone();
            if drafting != this.drafting {
                this.drafting = drafting;
                this.composer
                    .update(cx, |composer, cx| composer.focus(window, cx));
            }
            let working = wanted
                .as_deref()
                .is_some_and(|id| thread_working(workspace.read(cx), id));
            if shown != wanted {
                if let Some(left) = this.transcript.take() {
                    this.kept_transcripts.insert(0, left);
                }
                this.kept_transcripts
                    .retain(|kept| workspace.read(cx).holds(kept.read(cx).thread_state()));
                let reused = open.as_ref().and_then(|open| {
                    let index = this
                        .kept_transcripts
                        .iter()
                        .position(|kept| kept.read(cx).thread_state() == open)?;
                    Some(this.kept_transcripts.remove(index))
                });
                this.transcript = match reused {
                    Some(transcript) => {
                        transcript.update(cx, |transcript, cx| transcript.set_working(working, cx));
                        Some(transcript)
                    }
                    None => {
                        open.map(|thread| cx.new(|cx| TranscriptView::new(thread, working, cx)))
                    }
                };
                this._transcript_events = this.transcript.as_ref().map(|transcript| {
                    cx.subscribe(transcript, |this, transcript, event, cx| {
                        let thread_id = transcript.read(cx).thread_id(cx);
                        this.open_turn_diff(&thread_id, event, cx)
                    })
                });
                this.subagents.clear();
                this._subagent_opens = this.transcript.as_ref().map(|transcript| {
                    cx.subscribe(
                        transcript,
                        |this, _, event: &subagents::OpenSubagent, cx| {
                            this.open_subagent(event.0.clone(), cx)
                        },
                    )
                });
                this._rewinds = this.transcript.as_ref().map(|transcript| {
                    cx.subscribe_in(
                        transcript,
                        window,
                        |this, _, event: &rewind::Rewound, window, cx| {
                            let draft = event.0.clone();
                            this.composer.update(cx, |composer, cx| {
                                composer.restore_message(draft, window, cx)
                            });
                        },
                    )
                });
                this._sign_ins = this.transcript.as_ref().map(|transcript| {
                    cx.subscribe_in(
                        transcript,
                        window,
                        |this, _, event: &request_rows::OpenProviderSettings, window, cx| {
                            let provider = event.0;
                            this.open_settings(Section::Providers, window, cx);
                            if let Some(settings) = &this.settings {
                                settings.update(cx, |settings, cx| {
                                    settings.show_provider(provider, cx)
                                });
                            }
                        },
                    )
                });
                this.composer
                    .update(cx, |composer, cx| composer.focus(window, cx));
            } else if let Some(transcript) = &this.transcript {
                transcript.update(cx, |transcript, cx| transcript.set_working(working, cx));
            }
            if let Some(transcript) = &this.transcript {
                let background = wanted
                    .as_deref()
                    .and_then(|id| workspace.read(cx).background.get(id).cloned());
                let retry = wanted
                    .as_deref()
                    .and_then(|id| workspace.read(cx).model.retry(id).cloned());
                let stopping = wanted
                    .as_deref()
                    .and_then(|id| workspace.read(cx).stopping_since(id));
                transcript.update(cx, |transcript, cx| {
                    transcript.set_background(background, cx);
                    transcript.set_retry(retry, cx);
                    transcript.set_stopping(stopping, cx);
                });
            }
            cx.notify();
        });
        let merge_back = cx.new(|cx| fork::MergeBack::new(workspace.clone(), cx));
        let toasts = toast::install(cx);
        Self {
            workspace,
            transcript: None,
            kept_transcripts: Vec::new(),
            composer,
            sidebar,
            sidebar_open: true,
            details,
            terminal,
            setup_bar,
            snooze_status,
            snooze_woke,
            panel,
            mini,
            edit_menu: cx.new(|_| edit_menu::ReadOnlyMenu::new()),
            merge_back,
            toasts,
            drafting: None,
            settings: None,
            palette: None,
            usage: None,
            onboarding: None,
            _observe: observe,
            _sidebar_events: sidebar_events,
            _details: details_subscriptions,
            _transcript_events: None,
            subagents: Vec::new(),
            _subagent_opens: None,
            _rewinds: None,
            _sign_ins: None,
        }
    }

    fn open_settings(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        match &self.settings {
            Some(view) => view.update(cx, |view, cx| view.open_section(section, window, cx)),
            None => {
                let workspace = self.workspace.clone();
                self.settings =
                    Some(cx.new(|cx| SettingsView::new(workspace, section, window, cx)));
            }
        }
        cx.notify();
    }

    pub fn open_usage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.usage.is_none() {
            let workspace = self.workspace.clone();
            let view = cx.new(|cx| usage::UsageView::new(workspace, cx));
            cx.subscribe_in(&view, window, |this, _, event, window, cx| match event {
                usage::UsageEvent::OpenSettings(provider) => {
                    let provider = *provider;
                    this.usage = None;
                    this.open_settings(Section::Providers, window, cx);
                    if let Some(settings) = &this.settings {
                        settings.update(cx, |settings, cx| settings.show_provider(provider, cx));
                    }
                }
            })
            .detach();
            self.usage = Some(view);
            cx.notify();
        }
    }

    pub fn open_palette(
        &mut self,
        mode: palette::Mode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let scope = self.sidebar.read(cx).scope(cx);
        if let Some((palette, _)) = &self.palette {
            palette.update(cx, |palette, cx| {
                palette.set_sidebar(scope);
                palette.set_mode(mode, window, cx);
                palette.focus(window, cx);
            });
            return;
        }
        let workspace = self.workspace.clone();
        let palette = cx.new(|cx| {
            let mut palette = palette::Palette::new(workspace, mode, window, cx);
            palette.set_sidebar(scope);
            palette
        });
        let events = cx.subscribe_in(&palette, window, |this, _, event, window, cx| {
            match event {
                palette::PaletteEvent::Close => this.palette = None,
                palette::PaletteEvent::OpenSettings(section) => {
                    this.palette = None;
                    this.open_settings(*section, window, cx);
                }
                palette::PaletteEvent::ShowView { view, project_id } => {
                    let (view, project_id) = (*view, project_id.clone());
                    this.sidebar
                        .update(cx, |sidebar, cx| sidebar.show(view, project_id, cx));
                }
                palette::PaletteEvent::OpenUsage => {
                    this.palette = None;
                    this.open_usage(window, cx);
                }
            }
            cx.notify();
        });
        self.palette = Some((palette, events));
        cx.notify();
    }

    fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.take().is_some() {
            cx.notify();
        } else {
            self.open_palette(palette::Mode::Commands, window, cx);
        }
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        cx.notify();
    }

    fn show_surface(&mut self, thread_id: &str, surface: details::Surface, cx: &mut Context<Self>) {
        let kind = match surface {
            details::Surface::Diff => panel::SurfaceKind::Diff,
            details::Surface::PullRequest => panel::SurfaceKind::PullRequest,
        };
        self.panel
            .update(cx, |panel, cx| panel.open_surface(thread_id, kind, cx));
        cx.notify();
    }

    fn open_turn_diff(
        &mut self,
        thread_id: &str,
        event: &turn_changes::OpenTurnDiff,
        cx: &mut Context<Self>,
    ) {
        self.panel.update(cx, |panel, cx| {
            panel.open_turn_diff(thread_id, &event.turn_id, event.path.clone(), cx)
        });
        cx.notify();
    }

    fn show_item(&mut self, thread_id: &str, item_id: &str, cx: &mut Context<Self>) {
        if let Some(transcript) = &self.transcript
            && transcript.read(cx).thread_id(cx) == thread_id
        {
            transcript.update(cx, |transcript, cx| transcript.reveal(item_id, cx));
        }
    }

    fn toggle_terminal(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.workspace.read(cx).shown_thread_id(cx) {
            self.terminal
                .update(cx, |terminal, cx| terminal.toggle(&id, cx));
            cx.notify();
        }
    }

    fn toggle_panel(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.workspace.read(cx).open_thread_id(cx) {
            self.panel.update(cx, |panel, cx| panel.toggle(&id, cx));
            cx.notify();
        }
    }

    fn header_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let thread_id = self.workspace.read(cx).open_thread_id(cx);
        let shown_thread_id = self.workspace.read(cx).shown_thread_id(cx);
        let details_toggle = self
            .details
            .update(cx, |details, cx| details.header_toggle(cx));
        let terminal_open = shown_thread_id
            .as_deref()
            .is_some_and(|id| self.terminal.read(cx).is_open(id));
        let panel_open = thread_id
            .as_deref()
            .is_some_and(|id| self.panel.read(cx).is_open(id));
        let toggle = |id: &'static str, icon: IconName, label: String, active: bool| {
            Button::new(id)
                .icon(icon)
                .size(ButtonSize::IconSm)
                .variant(if active {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .tooltip(Tooltip::new(label).side(Side::Bottom))
        };
        div()
            .flex_none()
            .h_full()
            .flex()
            .items_center()
            .gap_1()
            .px_1()
            .when(self.merge_back.read(cx).is_shown(), |this| {
                this.child(self.merge_back.clone())
            })
            .child(details_toggle)
            .when(shown_thread_id.is_some(), |this| {
                this.child(
                    toggle(
                        "toggle-terminal",
                        IconName::PanelBottom,
                        m::toggle_terminal(),
                        terminal_open,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_terminal(cx))),
                )
            })
            .when(thread_id.is_some(), |this| {
                this.child(
                    toggle(
                        "toggle-panel",
                        IconName::PanelRight,
                        m::toggle_panel(),
                        panel_open,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_panel(cx))),
                )
            })
    }
}

fn thread_working(workspace: &Workspace, thread_id: &str) -> bool {
    workspace.model.thread(thread_id).is_some_and(|thread| {
        matches!(
            thread.status,
            ThreadStatus::Working | ThreadStatus::WaitingForInput
        )
    })
}

impl HoshiWindow {
    /// Shows a subagent in place of the thread, or of the subagent shown, that started it.
    fn open_subagent(&mut self, call_id: String, cx: &mut Context<Self>) {
        let (parent, parent_title) = match self.subagents.last() {
            Some((view, _)) => {
                let view = view.read(cx);
                (view.shown().clone(), view.title(cx))
            }
            None => {
                let Some(open) = self.workspace.read(cx).open.clone() else {
                    return;
                };
                let title = self
                    .workspace
                    .read(cx)
                    .model
                    .thread(&open.read(cx).thread_id)
                    .map(|thread| SharedString::from(thread.title.clone()))
                    .unwrap_or_default();
                (open, title)
            }
        };
        let workspace = self.workspace.clone();
        let view = cx.new(|cx| {
            subagent_view::SubagentView::new(&workspace, parent, call_id, parent_title, cx)
        });
        let subscriptions = [
            cx.subscribe(&view, |this, _, _: &subagent_view::LeaveSubagent, cx| {
                this.subagents.pop();
                cx.notify();
            }),
            cx.subscribe(&view, |this, _, event: &subagents::OpenSubagent, cx| {
                this.open_subagent(event.0.clone(), cx)
            }),
        ];
        self.subagents.push((view, subscriptions));
        cx.notify();
    }
}

impl Render for HoshiWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(page) = self.onboarding_page(cx) {
            return page;
        }
        let room = self
            .mini
            .read(cx)
            .lane_room()
            .unwrap_or_else(|| self.details.read(cx).lane_room(cx));
        let room_changed = hoshi_ui::LaneRoom::set(room, cx);
        let colors = cx.hoshi().clone();
        let workspace = self.workspace.read(cx);
        let draft = workspace.draft.clone();
        let prompt = draft.as_ref().map(|draft| match &draft.project_id {
            Some(project) => m::new_thread_prompt(
                workspace
                    .model
                    .project(project)
                    .map(|project| project.title.clone())
                    .unwrap_or_default(),
            ),
            None => m::new_chat_prompt(),
        });
        let thread_open = self.transcript.is_some();
        let composer_shown = self.subagents.is_empty() && (thread_open || draft.is_some());
        let panel_maximized = thread_open
            && workspace
                .open_thread_id(cx)
                .is_some_and(|id| self.panel.read(cx).is_maximized(&id));

        sidebar::shortcuts(div(), &self.sidebar)
            .size_full()
            .flex()
            .on_action(
                cx.listener(|this, _: &terminal::ToggleTerminal, _, cx| this.toggle_terminal(cx)),
            )
            .on_action(
                cx.listener(|this, _: &thread_header::TogglePanel, _, cx| this.toggle_panel(cx)),
            )
            .on_action(cx.listener(|this, _: &palette::TogglePalette, window, cx| {
                this.toggle_palette(window, cx)
            }))
            .font_family(gpui_kit::component::Theme::global(cx).font_family.clone())
            .bg(colors.background)
            .text_color(colors.foreground)
            .child(hoshi_ui::selection_release())
            .child(self.edit_menu.clone())
            .when(self.sidebar_open, |this| this.child(self.sidebar.clone()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("title-bar")
                            .h_10()
                            .flex_none()
                            .flex()
                            .items_center()
                            .border_b_1()
                            .border_color(colors.border)
                            .when(!self.sidebar_open, |this| {
                                this.child(
                                    div().pl_2().child(
                                        Button::new("open-sidebar")
                                            .icon(IconName::PanelLeft)
                                            .size(ButtonSize::IconSm)
                                            .variant(ButtonVariant::Ghost)
                                            .tooltip(
                                                Tooltip::new(m::toggle_sidebar())
                                                    .side(Side::Bottom),
                                            )
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.toggle_sidebar(cx)
                                            })),
                                    ),
                                )
                            })
                            .child(self.thread_heading(window, cx))
                            .child(self.header_actions(cx))
                            .child(WindowControls::new()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .child(
                                div()
                                    .relative()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .when(panel_maximized, |this| this.hidden())
                                    .when_some(self.subagents.last(), |this, (subagent, _)| {
                                        this.child(subagent.clone())
                                    })
                                    .when(self.subagents.is_empty() && thread_open, |this| {
                                        this.children(delegations::delegated_by_line(
                                            &self.workspace,
                                            cx,
                                        ))
                                        .child(self.snooze_status.clone())
                                    })
                                    .when(self.subagents.is_empty(), |this| {
                                        this.children(self.transcript.clone().map(|transcript| {
                                            transcript::element(transcript, room_changed)
                                        }))
                                    })
                                    .when_some(prompt, |this, prompt| {
                                        this.child(
                                            div()
                                                .debug_selector(|| "draft-prompt".into())
                                                .flex_1()
                                                .flex()
                                                .items_end()
                                                .pb_8()
                                                .text_3xl()
                                                .font_weight(FontWeight::NORMAL)
                                                .child(hoshi_ui::Lane::new().child(
                                                    div().w_full().text_center().child(prompt),
                                                )),
                                        )
                                    })
                                    .when(thread_open, |this| this.child(self.setup_bar.clone()))
                                    .when(self.subagents.is_empty() && thread_open, |this| {
                                        this.child(self.snooze_woke.clone())
                                    })
                                    .when(composer_shown, |this| {
                                        this.when(draft.is_none(), |this| {
                                            this.child(self.toasts.clone())
                                        })
                                        .child(browser::obstacle(browser::Obstacle::Composer))
                                        .child(self.composer.clone())
                                    })
                                    .when(draft.is_some(), |this| this.child(div().flex_1()))
                                    .when(thread_open || draft.is_some(), |this| {
                                        this.child(self.terminal.clone())
                                    })
                                    .when(thread_open || draft.is_some(), |this| {
                                        this.child(self.details.clone())
                                    })
                                    .when(!composer_shown || draft.is_some(), |this| {
                                        this.child(
                                            div()
                                                .absolute()
                                                .bottom_0()
                                                .left_0()
                                                .right_0()
                                                .child(self.toasts.clone()),
                                        )
                                    })
                                    .child(self.mini.clone()),
                            )
                            .when(thread_open, |this| this.child(self.panel.clone())),
                    ),
            )
            .when(!self.sidebar_open, |this| {
                this.children(
                    self.sidebar
                        .update(cx, |sidebar, cx| sidebar.custom_snooze_dialog(cx)),
                )
            })
            .children(self.palette.as_ref().map(|(palette, _)| palette.clone()))
            .children(markdown::hover_card::render(window, cx))
            .when_some(self.settings.clone(), |this, view| {
                this.child(
                    Dialog::new("settings")
                        .page()
                        .open(true)
                        .close_button(false)
                        .initial_focus(InitialFocus::Popup)
                        .on_open_change(cx.listener(|this, change: &OpenChange, _, cx| {
                            if !change.open {
                                look::set_preview(None, cx);
                                this.settings = None;
                                cx.notify();
                            }
                        }))
                        .child(view),
                )
            })
            .when_some(self.usage.clone(), |this, view| {
                this.child(
                    Dialog::new("usage")
                        .page()
                        .open(true)
                        .close_button(false)
                        .initial_focus(InitialFocus::Popup)
                        .on_open_change(cx.listener(|this, change: &OpenChange, _, cx| {
                            if !change.open {
                                this.usage = None;
                                cx.notify();
                            }
                        }))
                        .child(view),
                )
            })
            .into_any_element()
    }
}

/// The system's allocator, held to a limit only while the program reads a workbook.
#[global_allocator]
static ALLOCATOR: cap::Cap<std::alloc::System> = cap::Cap::new(std::alloc::System, usize::MAX);

fn main() {
    if let Some(code) = hoshi_engine::table::serve(|limit| {
        let _ = ALLOCATOR.set_limit(limit);
    }) {
        std::process::exit(code);
    }
    let home = home::resolve().expect("hoshi needs a home folder");
    #[cfg(windows)]
    app_user_model::register(&home);
    let (notify, notifications) = state::channel();
    let engine = Arc::new(hoshi_app::open_engine(&home, notify).expect("the engine opens"));

    gpui_kit::application()
        .with_assets(hoshi_ui::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            app_icon::install();
            terminal::init(cx);
            thread_header::init(cx);
            sidebar::init(cx);
            palette::init(cx);
            hoshi_ui::DialogLayer::init(cx);
            cx.set_global(hoshi_ui::TooltipProvider {
                delay: Duration::from_millis(25),
                ..Default::default()
            });
            theme::load_fonts(cx);
            markdown::init(engine.clone(), home.clone(), cx);
            let (window_bounds, display_id) = window_state::initial(&home, cx);
            let options = WindowOptions {
                window_bounds: Some(window_bounds),
                display_id,
                titlebar: Some(TitlebarOptions {
                    title: Some("hoshi".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(12.), px(12.))),
                }),
                app_id: Some(app_icon::APP_ID.into()),
                icon: app_icon::window_icon(),
                ..Default::default()
            };
            let engine_at_quit = engine.clone();
            cx.on_app_quit(move |_| {
                engine_at_quit.end_agents();
                engine_at_quit.close_browser();
                async {}
            })
            .detach();
            let mut sample_workspace = None;
            let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
                look::init(Some(&home), window, cx);
                let settings_path = hoshi_app::home::settings_path(&home);
                let settings = hoshi_app::settings::load(&settings_path);
                locale::show(settings.general.language, cx);
                edit_menu::label_text_fields(cx);
                let workspace = cx.new(|cx| {
                    let mut workspace = Workspace::new(engine, notifications, cx);
                    workspace.settings = settings;
                    workspace.settings_path = Some(settings_path);
                    workspace.load_drafts(&home);
                    workspace
                });
                notifications::watch(&workspace, window, cx);
                let shown = cx.new(|cx| HoshiWindow::new(workspace.clone(), window, cx));
                if let Ok(thread_id) = std::env::var("HOSHI_NATIVE_THREAD") {
                    workspace.update(cx, |workspace, cx| workspace.open_thread(&thread_id, cx));
                }
                if let Ok(project) = std::env::var("HOSHI_NATIVE_DRAFT") {
                    let project = (!project.is_empty()).then_some(project);
                    workspace.update(cx, |workspace, cx| workspace.start_draft(project, cx));
                }
                if let Ok(text) = std::env::var("HOSHI_NATIVE_SEND") {
                    workspace.update(cx, |workspace, cx| workspace.new_chat_with(text, cx));
                }
                if let Some(section) = std::env::var("HOSHI_NATIVE_SETTINGS")
                    .ok()
                    .and_then(|name| Section::from_name(&name))
                {
                    shown.update(cx, |shown, cx| shown.open_settings(section, window, cx));
                }
                if std::env::var_os("HOSHI_NATIVE_USAGE").is_some() {
                    shown.update(cx, |shown, cx| shown.open_usage(window, cx));
                }
                onboarding::install(&shown, window, cx);
                window_state::remember(home.clone(), &shown, window, cx);
                sample_workspace = Some(workspace.clone());
                shown
            })
            .expect("the window opens");
            if let (Ok(kind), Some(workspace)) =
                (std::env::var("HOSHI_NATIVE_DIALOG"), sample_workspace)
            {
                window
                    .update(cx, |_, window, cx| {
                        dialog_samples::open(&kind, &workspace, window, cx)
                    })
                    .ok();
            }
            cx.on_window_closed(|cx, _| cx.quit()).detach();
        });
}
