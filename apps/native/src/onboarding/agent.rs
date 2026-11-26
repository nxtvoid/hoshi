//! The agents band: whether each provider's CLI is installed and signed in, and a terminal
//! under its row that gets it there.

use super::Onboarding;
use super::page::{band, row_icon};
use crate::m;
use crate::providers::{CODEX, COPILOT, KNOWN_PROVIDERS, KnownProvider, sign_in_failure};
use crate::terminal::{ShellExited, TerminalView};
use crate::updates::provider::{self, running_text, update_status};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_core::{
    DEFAULT_PROVIDER, ProviderAccountStatus, ProviderSignIn, ProviderUpdate, UpdateStep,
};
use hoshi_ui::prelude::*;
use std::time::Duration;

const CLAUDE_INSTALL: &str = if cfg!(windows) {
    "irm https://claude.ai/install.ps1 | iex"
} else {
    "curl -fsSL https://claude.ai/install.sh | bash"
};
const CLAUDE_SIGN_IN: &str = "claude auth login";
const COPILOT_INSTALL: &str = "npm install -g @github/copilot";
const COPILOT_SIGN_IN: &str = "copilot login";
const CODEX_INSTALL: &str = "npm install -g @openai/codex";
const CODEX_SIGN_IN: &str = "codex login";
const TERMINAL_HEIGHT: f32 = 200.;

/// What to run in a terminal to get `provider` ready, with the hint above it.
pub fn command(
    provider: &str,
    status: Option<ProviderAccountStatus>,
) -> Option<(String, &'static str)> {
    let (install, sign_in) = match provider {
        DEFAULT_PROVIDER => (CLAUDE_INSTALL, CLAUDE_SIGN_IN),
        COPILOT => (COPILOT_INSTALL, COPILOT_SIGN_IN),
        CODEX => (CODEX_INSTALL, CODEX_SIGN_IN),
        _ => return None,
    };
    match status? {
        ProviderAccountStatus::NotInstalled => Some((m::onboarding_agent_install_hint(), install)),
        ProviderAccountStatus::SignedOut => Some((m::onboarding_agent_sign_in_hint(), sign_in)),
        ProviderAccountStatus::Ready | ProviderAccountStatus::Unknown => None,
    }
}

/// Where an agent stands, in the order the band lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AgentState {
    Ready,
    SignedOut,
    Unknown,
    NotInstalled,
    Checking,
    Off,
}

pub fn agent_state(
    enabled: bool,
    account: Option<ProviderAccountStatus>,
    checking: bool,
) -> AgentState {
    if !enabled {
        return AgentState::Off;
    }
    if checking {
        return AgentState::Checking;
    }
    match account {
        Some(ProviderAccountStatus::Ready) => AgentState::Ready,
        Some(ProviderAccountStatus::SignedOut) => AgentState::SignedOut,
        Some(ProviderAccountStatus::NotInstalled) => AgentState::NotInstalled,
        Some(ProviderAccountStatus::Unknown) | None => AgentState::Unknown,
    }
}

fn account(state: AgentState) -> Option<ProviderAccountStatus> {
    match state {
        AgentState::SignedOut => Some(ProviderAccountStatus::SignedOut),
        AgentState::NotInstalled => Some(ProviderAccountStatus::NotInstalled),
        _ => None,
    }
}

/// Whether starting leaves hoshi without an agent to run threads with.
pub fn none_ready(states: impl IntoIterator<Item = AgentState>) -> bool {
    !states
        .into_iter()
        .any(|state| matches!(state, AgentState::Ready | AgentState::Checking))
}

pub fn detail(state: AgentState, version: Option<&str>) -> String {
    match state {
        AgentState::Checking => m::provider_status_checking(),
        AgentState::Ready => match version {
            Some(version) => m::onboarding_agent_ready_version(version),
            None => m::onboarding_agent_ready_detail(),
        },
        AgentState::NotInstalled => m::onboarding_agent_not_installed(),
        AgentState::SignedOut => m::onboarding_agent_signed_out(),
        AgentState::Off => m::onboarding_agent_off(),
        AgentState::Unknown => m::onboarding_agent_unknown(),
    }
}

/// A terminal running what gets an agent ready.
pub(super) struct Setup {
    pub provider: &'static str,
    pub command: &'static str,
    pub view: Entity<TerminalView>,
    pub ended: bool,
    pub succeeded: bool,
    /// Where the agent stood when it started.
    from: Option<AgentState>,
    _exit: Subscription,
}

/// A quiet text button, for the way around the main action.
fn link(
    id: String,
    label: String,
    colors: &Colors,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let hover = colors.foreground;
    div()
        .id(SharedString::from(id))
        .text_xs()
        .text_color(colors.muted_foreground)
        .underline()
        .cursor_pointer()
        .hover(move |style| style.text_color(hover))
        .on_click(on_click)
        .child(label)
}

fn actions() -> Div {
    div().flex().flex_wrap().items_center().gap_2()
}

impl Onboarding {
    /// Every known agent with where it stands, the ready ones first.
    pub(super) fn agents(&self, cx: &App) -> Vec<(&'static KnownProvider, AgentState)> {
        let workspace = self.workspace.read(cx);
        let mut agents: Vec<_> = KNOWN_PROVIDERS
            .iter()
            .map(|provider| {
                let state = agent_state(
                    provider.enabled(&workspace.settings.providers),
                    workspace.providers.accounts.get(provider.id).copied(),
                    !self.checked || self.rechecking.contains(provider.id),
                );
                (provider, state)
            })
            .collect();
        agents.sort_by_key(|(_, state)| *state);
        agents
    }

    pub(super) fn state_of(&self, provider: &str, cx: &App) -> Option<AgentState> {
        self.agents(cx)
            .into_iter()
            .find(|(known, _)| known.id == provider)
            .map(|(_, state)| state)
    }

    /// Runs `command` in a terminal under the agent's row, closing any other.
    pub(super) fn start_setup(
        &mut self,
        provider: &'static str,
        command: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_setup(cx);
        let from = self.state_of(provider, cx);
        let terminal_id = format!("onboarding-{provider}-{}", uuid::Uuid::new_v4());
        let workspace = self.workspace.clone();
        let view = cx.new(|cx| TerminalView::run(workspace, terminal_id, command.to_owned(), cx));
        let exit = cx.subscribe(&view, |this, _, exited: &ShellExited, cx| {
            if let Some(setup) = &mut this.setup {
                setup.ended = true;
                setup.succeeded = exited.code == Some(0);
            }
            this.check(cx);
        });
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.setup = Some(Setup {
            provider,
            command,
            view,
            ended: false,
            succeeded: false,
            from,
            _exit: exit,
        });
        cx.notify();
    }

    pub(super) fn close_setup(&mut self, cx: &mut Context<Self>) {
        let Some(setup) = self.setup.take() else {
            return;
        };
        let engine = self.workspace.read(cx).engine.clone();
        let terminal_id = setup.view.read(cx).terminal_id().to_owned();
        setup.view.update(cx, |view, _| view.detach());
        self.refocus = true;
        cx.background_spawn(async move { engine.close_terminal(&terminal_id) })
            .detach();
        cx.notify();
    }

    /// Closes the terminal once what it ran moved its agent on, leaving the row's next step.
    pub(super) fn settle_setup(&mut self, cx: &mut Context<Self>) {
        let done = self.setup.as_ref().is_some_and(|setup| {
            let state = self.state_of(setup.provider, cx);
            setup.ended && state != Some(AgentState::Checking) && state != setup.from
        });
        if done {
            self.close_setup(cx);
        }
    }

    pub(super) fn agents_band(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.hoshi().clone();
        let rows: Vec<AnyElement> = self
            .agents(cx)
            .into_iter()
            .map(|(provider, state)| self.agent_row(provider, state, &colors, cx))
            .collect();
        band(m::onboarding_band_agents(), div().children(rows), &colors)
    }

    fn agent_row(
        &self,
        provider: &'static KnownProvider,
        state: AgentState,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = provider.id;
        let tint = (provider.tint)(colors);
        let version = self
            .workspace
            .read(cx)
            .providers
            .versions
            .get(id)
            .and_then(|version| version.installed.clone());
        let setup = self.setup.as_ref().filter(|setup| setup.provider == id);
        let codex_open = id == CODEX
            && self.codex_open
            && matches!(state, AgentState::NotInstalled | AgentState::SignedOut);
        div()
            .debug_selector(move || format!("onboarding-agent-{id}"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .min_h(px(40.))
                    .py_1()
                    .when(state == AgentState::Off, |this| this.opacity(0.6))
                    .child(row_icon(
                        svg().path(provider.mark).size(px(17.)).text_color(tint),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child((provider.name)()),
                            )
                            .child(
                                div()
                                    .debug_selector(move || format!("onboarding-agent-detail-{id}"))
                                    .truncate()
                                    .text_xs()
                                    .text_color(colors.muted_foreground)
                                    .child(detail(state, version.as_deref())),
                            ),
                    )
                    .child(self.action(id, state, setup.is_some() || codex_open, colors, cx)),
            )
            .when_some(setup, |this, setup| {
                this.child(self.setup_box(setup, provider, colors, cx))
            })
            .when(codex_open, |this| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap_2p5()
                        .pl(px(32.))
                        .pb_3()
                        .children(self.codex_help(state, colors, cx)),
                )
            })
            .into_any_element()
    }

    /// The row's one action, which carries its state.
    fn action(
        &self,
        id: &'static str,
        state: AgentState,
        open: bool,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let button =
            |name: &str| Button::new(SharedString::from(format!("onboarding-{name}-{id}")));
        match state {
            AgentState::Checking => spinner(px(14.), colors.muted_foreground),
            AgentState::Ready => div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors.status_done)
                .child(Icon::new(IconName::Check).size_3p5())
                .child(m::onboarding_agent_ready())
                .into_any_element(),
            AgentState::Off => button("turn-on")
                .label(m::onboarding_agent_turn_on())
                .variant(ButtonVariant::Outline)
                .size(ButtonSize::Xs)
                .on_click(cx.listener(move |this, _, _, cx| this.turn_on(id, cx)))
                .into_any_element(),
            AgentState::Unknown => button("check")
                .icon(IconName::RefreshCw)
                .label(m::onboarding_check_again())
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Xs)
                .disabled(self.checking)
                .on_click(cx.listener(|this, _, _, cx| this.check(cx)))
                .into_any_element(),
            _ if open => button("close")
                .label(
                    if self
                        .setup
                        .as_ref()
                        .is_some_and(|setup| setup.provider == id)
                    {
                        m::onboarding_terminal_close()
                    } else {
                        m::onboarding_agent_hide()
                    },
                )
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Xs)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this
                        .setup
                        .as_ref()
                        .is_some_and(|setup| setup.provider == id)
                    {
                        this.close_setup(cx);
                    } else {
                        this.codex_open = false;
                        cx.notify();
                    }
                }))
                .into_any_element(),
            _ if id == CODEX => button("set-up")
                .label(m::onboarding_agent_set_up())
                .variant(ButtonVariant::Outline)
                .size(ButtonSize::Xs)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.codex_open = true;
                    cx.notify();
                }))
                .into_any_element(),
            _ => {
                let Some((_, command)) = command(id, account(state)) else {
                    return div().into_any_element();
                };
                button("set-up")
                    .icon(IconName::SquareTerminal)
                    .label(if state == AgentState::SignedOut {
                        m::onboarding_agent_sign_in()
                    } else {
                        m::onboarding_agent_install()
                    })
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::Xs)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.start_setup(id, command, window, cx)
                    }))
                    .into_any_element()
            }
        }
    }

    fn setup_box(
        &self,
        setup: &Setup,
        provider: &'static KnownProvider,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> Div {
        let id = provider.id;
        let note = if setup.ended && !self.checking {
            if setup.succeeded && self.state_of(id, cx) == Some(AgentState::NotInstalled) {
                m::onboarding_terminal_not_found((provider.name)())
            } else {
                m::onboarding_terminal_not_ready((provider.name)())
            }
        } else {
            m::onboarding_terminal_running((provider.name)())
        };
        div()
            .debug_selector(move || format!("onboarding-terminal-{id}"))
            .ml(px(32.))
            .mt_1()
            .mb_2()
            .rounded_lg()
            .border_1()
            .border_color(colors.border)
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_1p5()
                    .border_b_1()
                    .border_color(colors.border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .child(setup.command),
                    ),
            )
            .child(div().h(px(TERMINAL_HEIGHT)).child(setup.view.clone()))
            .child(
                div()
                    .px_3()
                    .py_1p5()
                    .border_t_1()
                    .border_color(colors.border)
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .child(note),
            )
    }

    fn codex_help(
        &self,
        state: AgentState,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let workspace = self.workspace.read(cx);
        let version = workspace.providers.versions.get(CODEX).cloned();
        let sign_in = workspace
            .providers
            .sign_ins
            .get(CODEX)
            .cloned()
            .unwrap_or_default();
        let can_install = version
            .as_ref()
            .is_some_and(|version| version.installed.is_none() && version.update_command.is_some());
        let mono = cx.theme().mono_font_family.clone();
        match state {
            AgentState::NotInstalled => {
                if let Some(ProviderUpdate::Running { line, step }) =
                    version.as_ref().map(|version| &version.update)
                {
                    let text = match step {
                        Some(_) => running_text(line.as_deref(), *step),
                        None => m::onboarding_codex_installing(),
                    };
                    let percent = match step {
                        Some(UpdateStep::Downloading { percent }) => *percent,
                        _ => None,
                    };
                    return vec![
                        waiting(text, colors).into_any_element(),
                        progress(percent, colors).into_any_element(),
                    ];
                }
                if can_install && !self.codex_cli {
                    let status = version
                        .as_ref()
                        .and_then(|version| update_status(CODEX, version, colors, mono));
                    return [
                        Some(
                            hint_text(m::onboarding_codex_install_hint(), colors)
                                .into_any_element(),
                        ),
                        Some(
                            actions()
                                .child(
                                    Button::new("onboarding-codex-install")
                                        .icon(IconName::Download)
                                        .label(m::onboarding_codex_install())
                                        .size(ButtonSize::Sm)
                                        .disabled(provider::any_running(self.workspace.read(cx)))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            provider::start_update_of(&this.workspace, CODEX, cx)
                                        })),
                                )
                                .child(self.codex_cli_link(
                                    "onboarding-codex-bring-cli",
                                    m::onboarding_codex_bring_cli(),
                                    true,
                                    colors,
                                    cx,
                                ))
                                .into_any_element(),
                        ),
                        status,
                    ]
                    .into_iter()
                    .flatten()
                    .collect();
                }
                let mut actions = actions().child(self.terminal_button(
                    CODEX_INSTALL,
                    m::onboarding_agent_install(),
                    cx,
                ));
                if can_install {
                    actions = actions.child(self.codex_cli_link(
                        "onboarding-codex-install-instead",
                        m::onboarding_codex_install_instead(),
                        false,
                        colors,
                        cx,
                    ));
                }
                vec![
                    hint_text(m::onboarding_agent_install_hint(), colors).into_any_element(),
                    actions.into_any_element(),
                ]
            }
            AgentState::SignedOut => match sign_in {
                ProviderSignIn::Waiting { url } => vec![
                    waiting(m::codex_sign_in_waiting(), colors).into_any_element(),
                    actions()
                        .child(
                            Button::new("onboarding-codex-open-again")
                                .icon(IconName::ExternalLink)
                                .label(m::codex_sign_in_open_again())
                                .variant(ButtonVariant::Outline)
                                .size(ButtonSize::Sm)
                                .on_click(move |_, _, cx| cx.open_url(&url)),
                        )
                        .child(
                            Button::new("onboarding-codex-cancel")
                                .label(m::codex_sign_in_cancel())
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Sm)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.cancel_codex_sign_in(cx)),
                                ),
                        )
                        .into_any_element(),
                ],
                _ if self.codex_cli => vec![
                    hint_text(m::onboarding_agent_sign_in_hint(), colors).into_any_element(),
                    actions()
                        .child(self.terminal_button(
                            CODEX_SIGN_IN,
                            m::onboarding_agent_sign_in(),
                            cx,
                        ))
                        .child(self.codex_cli_link(
                            "onboarding-codex-use-chatgpt",
                            m::onboarding_codex_use_chatgpt(),
                            false,
                            colors,
                            cx,
                        ))
                        .into_any_element(),
                ],
                sign_in => {
                    let starting = sign_in == ProviderSignIn::Starting;
                    let failure = match &sign_in {
                        ProviderSignIn::Failed { reason } => Some(
                            div()
                                .text_xs()
                                .text_color(colors.status_error)
                                .child(sign_in_failure(reason))
                                .into_any_element(),
                        ),
                        _ => None,
                    };
                    [
                        Some(
                            hint_text(m::onboarding_codex_sign_in_hint(), colors)
                                .into_any_element(),
                        ),
                        Some(
                            actions()
                                .child(
                                    Button::new("onboarding-codex-sign-in")
                                        .label(m::codex_sign_in())
                                        .size(ButtonSize::Sm)
                                        .loading(starting)
                                        .disabled(starting)
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.sign_in_codex(cx)),
                                        ),
                                )
                                .child(self.codex_cli_link(
                                    "onboarding-codex-use-cli",
                                    m::onboarding_codex_use_cli(),
                                    true,
                                    colors,
                                    cx,
                                ))
                                .into_any_element(),
                        ),
                        failure,
                    ]
                    .into_iter()
                    .flatten()
                    .collect()
                }
            },
            _ => Vec::new(),
        }
    }

    /// Runs Codex's own command in the terminal under its row.
    fn terminal_button(
        &self,
        command: &'static str,
        label: String,
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(SharedString::from(format!(
            "onboarding-codex-terminal-{label}"
        )))
        .icon(IconName::SquareTerminal)
        .label(label)
        .variant(ButtonVariant::Outline)
        .size(ButtonSize::Sm)
        .on_click(
            cx.listener(move |this, _, window, cx| this.start_setup(CODEX, command, window, cx)),
        )
    }

    /// Switches Codex between hoshi's install and sign-in and the user's own CLI.
    fn codex_cli_link(
        &self,
        id: &'static str,
        label: String,
        cli: bool,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let this = cx.weak_entity();
        link(id.into(), label, colors, move |_, _, cx| {
            this.update(cx, |this, cx| {
                this.codex_cli = cli;
                cx.notify();
            })
            .ok();
        })
    }
}

fn hint_text(hint: String, colors: &Colors) -> Div {
    div()
        .text_sm()
        .text_color(colors.muted_foreground)
        .child(hint)
}

fn waiting(text: String, colors: &Colors) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_sm()
        .child(spinner(px(12.), colors.muted_foreground))
        .child(text)
}

/// How far an install got; a sweeping bar while it can't say.
fn progress(percent: Option<u8>, colors: &Colors) -> Div {
    let fill = div()
        .absolute()
        .top_0()
        .bottom_0()
        .rounded_full()
        .bg(colors.foreground.opacity(0.6));
    let fill = match percent {
        Some(percent) => fill
            .left_0()
            .w(relative(f32::from(percent.min(100)) / 100.))
            .into_any_element(),
        None => fill
            .w(relative(0.4))
            .with_loop(
                hoshi_ui::looping(Duration::from_millis(1400)),
                |fill, delta| fill.left(relative(0.6 * delta)),
            )
            .into_any_element(),
    };
    div()
        .relative()
        .w_full()
        .h_1()
        .rounded_full()
        .bg(colors.muted)
        .overflow_hidden()
        .child(fill)
}

#[cfg(test)]
mod tests {
    use super::{
        AgentState, CLAUDE_INSTALL, CLAUDE_SIGN_IN, CODEX_SIGN_IN, COPILOT_INSTALL, agent_state,
        command, detail, none_ready,
    };
    use crate::m;
    use crate::providers::{CODEX, COPILOT};
    use hoshi_core::{DEFAULT_PROVIDER, ProviderAccountStatus};

    #[test]
    fn a_command_shows_only_when_the_agent_needs_one() {
        let command_for = |provider, status| command(provider, Some(status)).map(|(_, c)| c);
        assert_eq!(
            command_for(DEFAULT_PROVIDER, ProviderAccountStatus::NotInstalled),
            Some(CLAUDE_INSTALL)
        );
        assert_eq!(
            command_for(DEFAULT_PROVIDER, ProviderAccountStatus::SignedOut),
            Some(CLAUDE_SIGN_IN)
        );
        assert_eq!(
            command_for(COPILOT, ProviderAccountStatus::NotInstalled),
            Some(COPILOT_INSTALL)
        );
        assert_eq!(
            command_for(CODEX, ProviderAccountStatus::SignedOut),
            Some(CODEX_SIGN_IN)
        );
        assert!(command_for(DEFAULT_PROVIDER, ProviderAccountStatus::Ready).is_none());
        assert!(command_for(COPILOT, ProviderAccountStatus::Unknown).is_none());
        assert!(command(DEFAULT_PROVIDER, None).is_none());
    }

    #[test]
    fn agents_list_ready_first_then_those_one_sign_in_away() {
        let mut states = vec![
            agent_state(false, Some(ProviderAccountStatus::Ready), false),
            agent_state(true, Some(ProviderAccountStatus::NotInstalled), false),
            agent_state(true, None, true),
            agent_state(true, Some(ProviderAccountStatus::SignedOut), false),
            agent_state(true, None, false),
            agent_state(true, Some(ProviderAccountStatus::Ready), false),
        ];
        states.sort();
        assert_eq!(
            states,
            [
                AgentState::Ready,
                AgentState::SignedOut,
                AgentState::Unknown,
                AgentState::NotInstalled,
                AgentState::Checking,
                AgentState::Off,
            ]
        );
    }

    #[test]
    fn starting_without_a_ready_agent_says_so() {
        assert!(none_ready([AgentState::NotInstalled, AgentState::Off]));
        assert!(none_ready([]));
        assert!(!none_ready([AgentState::SignedOut, AgentState::Ready]));
        assert!(!none_ready([AgentState::Checking]));
    }

    #[test]
    fn the_detail_names_the_version_once_ready() {
        assert_eq!(
            detail(AgentState::Checking, None),
            m::provider_status_checking()
        );
        assert_eq!(
            detail(AgentState::Ready, Some("2.1.0")),
            m::onboarding_agent_ready_version("2.1.0")
        );
        assert_eq!(
            detail(AgentState::Ready, None),
            m::onboarding_agent_ready_detail()
        );
        assert_eq!(detail(AgentState::Off, None), m::onboarding_agent_off());
    }
}
