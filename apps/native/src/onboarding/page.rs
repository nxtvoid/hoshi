//! The first-run page: the theme, the agents and the first projects, then one button to start.

use super::agent::{Setup, none_ready};
use crate::look;
use crate::m;
use crate::settings::appearance::{scheme_icon, scheme_label};
use crate::state::Workspace;
use crate::themes::appearance::ThemePreference;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_core::ProviderAccountStatus;
use hoshi_engine::RecentFolder;
use hoshi_ui::assets::brand;
use hoshi_ui::prelude::*;
use std::collections::HashSet;

/// What the window does once the setup closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterExit {
    /// Opens the palette that clones or creates a folder.
    AddProject,
}

pub enum OnboardingEvent {
    Exit(Option<AfterExit>),
    ProjectAdded(String),
}

pub struct Onboarding {
    pub(super) workspace: Entity<Workspace>,
    /// Whether the agents were checked since the setup opened.
    pub(super) checked: bool,
    pub(super) checking: bool,
    /// The agents checked again on their own, as after turning one on.
    pub(super) rechecking: HashSet<&'static str>,
    /// Codex's own setup shows under its row.
    pub(super) codex_open: bool,
    /// Codex runs the user's own CLI rather than the one hoshi installs and signs in.
    pub(super) codex_cli: bool,
    pub(super) setup: Option<Setup>,
    /// The folders offered as projects; `None` while the agents' history is read.
    pub(super) folders: Option<Vec<RecentFolder>>,
    /// The offered folders to add, by path.
    pub(super) selected: HashSet<String>,
    /// Whether the user picked folders, which a later read then leaves alone.
    pub(super) picked: bool,
    pub(super) reviewing: bool,
    pub(super) adding: bool,
    pub(super) error: Option<String>,
    exited: bool,
    pub focus: FocusHandle,
    /// The page takes the focus back on its next frame, as after its terminal closes.
    pub(super) refocus: bool,
    /// The agents found installed, whose history a new one adds to.
    installed: HashSet<String>,
    pub(super) scan: Option<Task<()>>,
    _observe: Subscription,
}

impl EventEmitter<OnboardingEvent> for Onboarding {}

impl Onboarding {
    pub fn new(workspace: Entity<Workspace>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&workspace, |this: &mut Self, workspace, cx| {
            let installed = installed_agents(workspace.read(cx));
            if this.checked && !installed.is_subset(&this.installed) {
                this.scan(cx);
            }
            this.installed = installed;
            cx.notify();
        });
        let mut this = Self {
            workspace,
            checked: false,
            checking: false,
            rechecking: HashSet::new(),
            codex_open: false,
            codex_cli: false,
            setup: None,
            folders: None,
            selected: HashSet::new(),
            picked: false,
            reviewing: false,
            adding: false,
            error: None,
            exited: false,
            focus: cx.focus_handle(),
            refocus: false,
            installed: HashSet::new(),
            scan: None,
            _observe: observe,
        };
        crate::updates::provider::load_version(&this.workspace, cx);
        this.check(cx);
        this.scan(cx);
        this
    }

    pub fn exit(&mut self, then: Option<AfterExit>, cx: &mut Context<Self>) {
        if self.exited {
            return;
        }
        self.exited = true;
        self.close_setup(cx);
        cx.emit(OnboardingEvent::Exit(then));
    }

    /// Checks every agent, which runs each CLI and takes a few seconds.
    pub(super) fn check(&mut self, cx: &mut Context<Self>) {
        if self.checking {
            return;
        }
        self.checking = true;
        cx.notify();
        let engine = self.workspace.read(cx).engine.clone();
        cx.spawn(async move |this, cx| {
            let status = cx
                .background_spawn(async move { engine.check_providers() })
                .await;
            this.update(cx, |this, cx| {
                this.workspace
                    .update(cx, |workspace, cx| receive_providers(workspace, status, cx));
                this.installed = installed_agents(this.workspace.read(cx));
                this.checking = false;
                this.checked = true;
                this.settle_setup(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Adds and enables the provider in the settings, then checks it again and reads the
    /// folders its history adds.
    pub(super) fn turn_on(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        let Some(known) = crate::providers::known(provider) else {
            return;
        };
        self.rechecking.insert(provider);
        let (engine, path, previous, next) = self.workspace.update(cx, |workspace, cx| {
            let previous = workspace.settings.clone();
            let (added, enabled) = (known.switches_mut)(&mut workspace.settings.providers);
            (*added, *enabled) = (true, true);
            cx.notify();
            (
                workspace.engine.clone(),
                workspace.settings_path.clone(),
                previous,
                workspace.settings.clone(),
            )
        });
        cx.spawn(async move |this, cx| {
            let (status, models) = cx
                .background_spawn(async move {
                    if let Some(path) = path
                        && let Err(error) =
                            hoshi_app::apply_settings(&engine, &path, &previous, &next)
                    {
                        eprintln!("hoshi-native: could not save settings: {error}");
                    }
                    (engine.check_providers(), engine.provider_models())
                })
                .await;
            this.update(cx, |this, cx| {
                this.workspace.update(cx, |workspace, cx| {
                    workspace.models = models;
                    workspace.providers = status;
                    cx.notify();
                });
                this.rechecking.remove(provider);
                this.scan(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn sign_in_codex(&mut self, cx: &mut Context<Self>) {
        let engine = self.workspace.read(cx).engine.clone();
        cx.spawn(async move |this, cx| {
            let started = cx
                .background_spawn(async move { engine.sign_in_codex() })
                .await;
            if let Ok(url) = started {
                this.update(cx, |_, cx| cx.open_url(&url)).ok();
            }
        })
        .detach();
    }

    pub(super) fn cancel_codex_sign_in(&mut self, cx: &mut Context<Self>) {
        let engine = self.workspace.read(cx).engine.clone();
        cx.background_spawn(async move { engine.cancel_codex_sign_in() })
            .detach();
    }

    fn agent_ready(&self, cx: &App) -> bool {
        !none_ready(self.agents(cx).into_iter().map(|(_, state)| state))
    }

    fn on_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" if !self.adding => self.exit(None, cx),
            "enter" if self.agent_ready(cx) => self.start(None, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn look_band(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.hoshi().clone();
        let preference = look::appearance(cx).preference;
        let options = [
            ThemePreference::System,
            ThemePreference::Light,
            ThemePreference::Dark,
        ]
        .map(|option| {
            let selected = option == preference;
            let (hover_bg, hover_fg) = (colors.muted.opacity(0.5), colors.foreground);
            div()
                .id(SharedString::from(format!("onboarding-theme-{option:?}")))
                .debug_selector(move || format!("onboarding-theme-{option:?}"))
                .flex()
                .items_center()
                .gap_1p5()
                .px_2p5()
                .py_1()
                .rounded_md()
                .text_xs()
                .cursor_pointer()
                .map(|this| {
                    if selected {
                        this.bg(colors.muted)
                            .text_color(colors.foreground)
                            .font_weight(FontWeight::MEDIUM)
                    } else {
                        this.text_color(colors.muted_foreground)
                            .hover(move |style| style.bg(hover_bg).text_color(hover_fg))
                    }
                })
                .on_click(cx.listener(move |_, _, _, cx| {
                    look::update(cx, |appearance| appearance.preference = option);
                    cx.notify();
                }))
                .child(Icon::new(scheme_icon(option)).size_3p5())
                .child(scheme_label(option))
        });
        let row = div()
            .flex()
            .items_center()
            .gap_3()
            .min_h(px(40.))
            .child(row_icon(
                Icon::new(IconName::Palette)
                    .size_4()
                    .text_color(colors.muted_foreground),
            ))
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(m::onboarding_theme()),
            )
            .child(
                div()
                    .flex()
                    .gap(px(2.))
                    .p(px(2.))
                    .rounded_lg()
                    .border_1()
                    .border_color(colors.border)
                    .children(options),
            );
        band(m::onboarding_band_look(), row, &colors)
    }

    fn footer(&self, cx: &mut Context<Self>) -> Div {
        let colors = cx.hoshi().clone();
        let ready = self.agent_ready(cx);
        let count = self.to_add().len();
        let leading = if ready {
            Button::new("onboarding-skip")
                .label(m::onboarding_skip())
                .variant(ButtonVariant::Ghost)
                .disabled(self.adding)
                .suffix(Kbd::new("Esc"))
                .on_click(cx.listener(|this, _, _, cx| this.exit(None, cx)))
        } else {
            Button::new("onboarding-start-without-agent")
                .label(m::onboarding_start_without_agent())
                .variant(ButtonVariant::Ghost)
                .disabled(self.adding)
                .on_click(cx.listener(|this, _, _, cx| this.start(None, cx)))
        };
        let label = if self.adding {
            m::onboarding_starting()
        } else if count > 0 {
            m::onboarding_start_adding(count)
        } else {
            m::onboarding_start()
        };
        let start = Button::new("onboarding-start")
            .label(label)
            .disabled(!ready || self.adding)
            .loading(self.adding)
            .when(ready && !self.adding, |this| this.suffix(Kbd::new("↵")))
            .on_click(cx.listener(|this, _, _, cx| this.start(None, cx)));
        div()
            .mt_6()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_3()
            .child(leading)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .when(!ready, |this| {
                        this.child(
                            div()
                                .debug_selector(|| "onboarding-needs-agent".into())
                                .text_xs()
                                .text_color(colors.muted_foreground)
                                .child(m::onboarding_needs_agent()),
                        )
                    })
                    .child(start),
            )
    }
}

impl Render for Onboarding {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.refocus) {
            window.focus(&self.focus, cx);
        }
        let colors = cx.hoshi().clone();
        let header = div()
            .child(
                svg()
                    .path(brand::MARK)
                    .size_6()
                    .text_color(colors.foreground),
            )
            .child(
                div()
                    .mt_5()
                    .text_2xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(m::onboarding_welcome_title()),
            )
            .child(
                div()
                    .mt_1p5()
                    .text_sm()
                    .text_color(colors.muted_foreground)
                    .child(m::onboarding_tagline()),
            );
        div()
            .id("onboarding")
            .track_focus(&self.focus)
            .key_context("Onboarding")
            .on_key_down(cx.listener(|this, event, _, cx| this.on_key(event, cx)))
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .mx_auto()
                    .w_full()
                    .max_w(px(640.))
                    .px_6()
                    .pt_12()
                    .pb_10()
                    .child(header)
                    .child(
                        div()
                            .mt_10()
                            .border_t_1()
                            .border_color(colors.border)
                            .child(self.look_band(cx))
                            .child(self.agents_band(cx))
                            .child(self.projects_band(cx)),
                    )
                    .child(self.footer(cx)),
            )
    }
}

/// One part of the page: its name on the left, its rows on the right.
pub(super) fn band(title: String, content: impl IntoElement, colors: &Colors) -> AnyElement {
    div()
        .flex()
        .gap(px(40.))
        .py_2p5()
        .border_b_1()
        .border_color(colors.border)
        .child(
            div()
                .flex_none()
                .w(px(96.))
                .h(px(40.))
                .flex()
                .items_center()
                .text_sm()
                .text_color(colors.muted_foreground)
                .child(title),
        )
        .child(div().flex_1().min_w_0().child(content))
        .into_any_element()
}

/// The column every row's icon, logo or checkbox sits in, so names line up down the page.
pub(super) fn row_icon(child: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size_5()
        .child(child)
}

/// The agents whose CLI is on this computer, signed in or not.
fn installed_agents(workspace: &Workspace) -> HashSet<String> {
    let providers = &workspace.providers;
    providers
        .accounts
        .iter()
        .filter(|(_, account)| {
            matches!(
                account,
                ProviderAccountStatus::Ready | ProviderAccountStatus::SignedOut
            )
        })
        .map(|(provider, _)| provider.clone())
        .chain(
            providers
                .versions
                .iter()
                .filter(|(_, version)| version.installed.is_some())
                .map(|(provider, _)| provider.clone()),
        )
        .collect()
}

/// Takes a fresh check of the providers, reading their models again when one was installed.
fn receive_providers(
    workspace: &mut Workspace,
    status: hoshi_engine::ProvidersStatus,
    cx: &mut Context<Workspace>,
) {
    let installed = status.accounts.iter().any(|(provider, account)| {
        *account != ProviderAccountStatus::NotInstalled
            && workspace.providers.accounts.get(provider)
                == Some(&ProviderAccountStatus::NotInstalled)
    });
    workspace.providers = status;
    if installed {
        workspace.reload_models(cx);
    }
    cx.notify();
}
