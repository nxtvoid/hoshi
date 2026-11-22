use super::SettingsView;
use crate::m;
use crate::providers::{CODEX, COPILOT, KNOWN_PROVIDERS, KnownProvider, known};
use crate::providers::{chatgpt_plan_label, sign_in_failure};
use crate::updates::provider::{self, update_status, version_summary};
use crate::usage::limits::plan_label;
use chrono::Utc;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_core::{
    CodexAccount, DEFAULT_PROVIDER, IdleCompaction, ProviderAccount, ProviderAccountStatus,
    ProviderSettings, ProviderSignIn, ProviderUpdate, ProviderVersion, SubagentEffort,
};
use hoshi_ui::prelude::*;

const COPILOT_SIGN_IN_COMMAND: &str = "copilot login";
const HEALTH_CHECK_STEP_SECS: u32 = 30;
const MAX_HEALTH_CHECK_SECS: u32 = 86_400;

fn idle_compaction_label(mode: IdleCompaction) -> String {
    match mode {
        IdleCompaction::WhenYouReturn => m::idle_compaction_when_you_return(),
        IdleCompaction::BeforeCacheExpires => m::idle_compaction_before_cache_expires(),
        IdleCompaction::Never => m::idle_compaction_never(),
    }
}

/// The providers the list shows: the added ones.
pub(super) fn listed(settings: &ProviderSettings) -> Vec<&'static KnownProvider> {
    KNOWN_PROVIDERS
        .iter()
        .filter(|provider| (provider.switches)(settings).0)
        .collect()
}

/// The providers the "Add provider" menu offers: the ones not added yet.
pub(super) fn addable(settings: &ProviderSettings) -> Vec<&'static KnownProvider> {
    KNOWN_PROVIDERS
        .iter()
        .filter(|provider| !(provider.switches)(settings).0)
        .collect()
}

/// The provider to select once `removed` leaves `listed`: the next one, else the one before.
pub(super) fn after_removal(listed: &[&'static str], removed: &str) -> Option<&'static str> {
    let Some(index) = listed.iter().position(|provider| *provider == removed) else {
        return listed.first().copied();
    };
    listed
        .get(index + 1)
        .or_else(|| index.checked_sub(1).and_then(|before| listed.get(before)))
        .copied()
}

fn effort_label(effort: SubagentEffort) -> String {
    match effort {
        SubagentEffort::Thread => m::subagent_effort_thread(),
        SubagentEffort::AgentDecides => m::subagent_effort_agent_decides(),
    }
}

fn effort_description(effort: SubagentEffort) -> String {
    match effort {
        SubagentEffort::Thread => m::subagent_effort_thread_description(),
        SubagentEffort::AgentDecides => m::subagent_effort_agent_decides_description(),
    }
}

/// What a provider's list item says under its name, with a dot when it needs attention.
pub(super) fn status_line(
    provider: &str,
    enabled: bool,
    account: Option<ProviderAccountStatus>,
    version: Option<&ProviderVersion>,
) -> (String, Option<Status>) {
    if !enabled {
        return (m::provider_status_off(), None);
    }
    let status = match account {
        None => m::provider_status_checking(),
        Some(ProviderAccountStatus::Ready) => m::provider_status_signed_in(),
        Some(ProviderAccountStatus::NotInstalled) if provider == CODEX => {
            return (m::codex_not_installed(), Some(Status::Waiting));
        }
        Some(ProviderAccountStatus::NotInstalled) => {
            return (m::provider_status_not_found(), Some(Status::Error));
        }
        Some(ProviderAccountStatus::SignedOut) => {
            return (m::provider_status_signed_out(), Some(Status::Waiting));
        }
        Some(ProviderAccountStatus::Unknown) => {
            return (m::provider_status_unknown(), Some(Status::Waiting));
        }
    };
    match version.and_then(|version| version.installed.as_deref()) {
        Some(installed) => (
            m::provider_status_line(status, m::provider_version_short(installed)),
            None,
        ),
        None => (status, None),
    }
}

/// What the selected provider's Status row says, with a dot when it needs attention. `binary`
/// is the CLI it runs.
pub(super) fn status_detail(
    provider: &str,
    enabled: bool,
    status: Option<ProviderAccountStatus>,
    account: Option<&ProviderAccount>,
    login: Option<&str>,
    binary: &str,
) -> (String, Option<Status>) {
    if !enabled {
        return (m::provider_status_off(), None);
    }
    match status {
        None => (m::provider_status_checking(), None),
        Some(ProviderAccountStatus::Ready) if provider == DEFAULT_PROVIDER => {
            let signed_in = m::provider_status_signed_in();
            let text = match account.and_then(|account| account.plan) {
                Some(plan) => m::provider_status_with_plan(plan_label(plan), signed_in),
                None => signed_in,
            };
            (text, None)
        }
        Some(ProviderAccountStatus::Ready) => (
            login.map_or_else(
                m::provider_status_signed_in,
                m::provider_status_signed_in_as,
            ),
            None,
        ),
        Some(ProviderAccountStatus::NotInstalled) => (
            m::provider_status_not_found_detail(binary),
            Some(Status::Error),
        ),
        Some(ProviderAccountStatus::SignedOut) if provider == COPILOT => (
            m::provider_status_signed_out_detail(COPILOT_SIGN_IN_COMMAND),
            Some(Status::Waiting),
        ),
        Some(ProviderAccountStatus::SignedOut) => {
            let text = crate::onboarding::agent::command(provider, status)
                .map(|(_, command)| m::provider_status_signed_out_detail(command))
                .unwrap_or_else(m::provider_status_signed_out);
            (text, Some(Status::Waiting))
        }
        Some(ProviderAccountStatus::Unknown) => {
            (m::provider_status_unknown(), Some(Status::Waiting))
        }
    }
}

/// What Codex's Status row says. `binary` is the one the settings name: without one, hoshi can
/// install Codex itself.
pub(super) fn codex_status_detail(
    enabled: bool,
    status: Option<ProviderAccountStatus>,
    account: Option<&CodexAccount>,
    binary: Option<&str>,
) -> (String, Option<Status>) {
    if !enabled {
        return (m::provider_status_off(), None);
    }
    match status {
        None => (m::provider_status_checking(), None),
        Some(ProviderAccountStatus::Ready) => {
            let text = match account {
                Some(account) if !account.chatgpt => m::provider_status_api_key(),
                Some(account) => {
                    let signed_in = account.email.as_deref().map_or_else(
                        m::provider_status_signed_in,
                        m::provider_status_signed_in_as,
                    );
                    match account.plan {
                        Some(plan) => {
                            m::provider_status_with_plan(chatgpt_plan_label(plan), signed_in)
                        }
                        None => signed_in,
                    }
                }
                None => m::provider_status_signed_in(),
            };
            (text, None)
        }
        Some(ProviderAccountStatus::NotInstalled) => match binary {
            Some(binary) => (
                m::provider_status_not_found_detail(binary),
                Some(Status::Error),
            ),
            None => (m::codex_not_installed(), Some(Status::Waiting)),
        },
        Some(ProviderAccountStatus::SignedOut) => {
            (m::provider_status_signed_out(), Some(Status::Waiting))
        }
        Some(ProviderAccountStatus::Unknown) => {
            (m::provider_status_unknown(), Some(Status::Waiting))
        }
    }
}

/// Codex's own fields on the providers page, and how its sign-out goes.
pub(super) struct CodexFields {
    pub binary_path: Entity<InputState>,
    pub home: Entity<InputState>,
    pub signing_out: bool,
    pub sign_out_failed: bool,
}

fn status_text(text: String, dot: Option<Status>) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .when_some(dot, |this, dot| this.child(StatusDot::new(Some(dot))))
        .child(text)
}

impl SettingsView {
    pub(super) fn providers(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut page = vec![
            SettingsGroup::new()
                .title(m::settings_providers())
                .action(self.check_button(cx))
                .child(self.provider_split(cx))
                .into_any_element(),
            SettingsGroup::new()
                .title(m::advanced())
                .child(self.health_check_row(cx))
                .child(self.idle_compaction_row(cx))
                .into_any_element(),
        ];
        page.extend(self.save_error(cx));
        page
    }

    fn provider_enabled(&self, provider: &str, cx: &App) -> bool {
        known(provider).is_some_and(|known| known.enabled(&self.settings(cx).providers))
    }

    /// Shows the providers page with `provider` selected.
    pub fn show_provider(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        self.section = super::Section::Providers;
        self.provider = provider;
        cx.notify();
    }

    /// The provider whose settings show: the chosen one while it is listed, else the first.
    fn selected_provider(&self, cx: &App) -> Option<&'static KnownProvider> {
        let listed = listed(&self.settings(cx).providers);
        listed
            .iter()
            .find(|provider| provider.id == self.provider)
            .or(listed.first())
            .copied()
    }

    fn add_provider(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        let Some(known) = known(provider) else {
            return;
        };
        self.provider = provider;
        self.change(cx, |settings| {
            let (added, enabled) = (known.switches_mut)(&mut settings.providers);
            (*added, *enabled) = (true, true);
        });
    }

    fn remove_provider(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        let Some(known) = known(provider) else {
            return;
        };
        let ids: Vec<_> = listed(&self.settings(cx).providers)
            .iter()
            .map(|provider| provider.id)
            .collect();
        if let Some(next) = after_removal(&ids, provider) {
            self.provider = next;
        }
        self.change(cx, |settings| {
            let (added, enabled) = (known.switches_mut)(&mut settings.providers);
            (*added, *enabled) = (false, false);
        });
    }

    fn check_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let checked_at = self.workspace.read(cx).providers.checked_at.clone();
        let label = if self.checking_providers {
            Some(m::provider_status_checking())
        } else {
            checked_at
                .map(|at| m::providers_checked(crate::sections::relative_time(&at, Utc::now())))
        };
        let button = Button::new("providers-check")
            .variant(ButtonVariant::Ghost)
            .text_color(colors.muted_foreground)
            .tooltip(m::providers_check_again())
            .loading(self.checking_providers)
            .disabled(self.checking_providers)
            .on_click(cx.listener(|this, _, _, cx| this.check_providers(cx)));
        match label {
            Some(label) => button
                .size(ButtonSize::Xs)
                .label(label)
                .when(!self.checking_providers, |this| {
                    this.suffix(Icon::new(IconName::RefreshCw).size_3())
                }),
            None => button.size(ButtonSize::IconXs).icon(IconName::RefreshCw),
        }
    }

    fn provider_split(&mut self, cx: &mut Context<Self>) -> SettingsSplit {
        let colors = cx.hoshi().clone();
        let selected = self.selected_provider(cx);
        let settings = self.settings(cx).providers.clone();
        let workspace = self.workspace.read(cx);
        let items: Vec<_> = listed(&settings)
            .into_iter()
            .map(|provider| {
                let id = provider.id;
                let enabled = (provider.switches)(&settings).1;
                let (subtitle, dot) = status_line(
                    id,
                    enabled,
                    workspace.providers.accounts.get(id).copied(),
                    workspace.providers.versions.get(id),
                );
                let switch = Switch::new(SharedString::from(format!("{id}-enabled")))
                    .checked(enabled)
                    .on_change(self.toggler(cx, move |settings, on| {
                        *(provider.switches_mut)(&mut settings.providers).1 = on
                    }));
                SettingsListItem::new(
                    SharedString::from(format!("provider-{id}")),
                    (provider.name)(),
                    subtitle,
                )
                .leading(
                    svg()
                        .path(provider.mark)
                        .size_4()
                        .text_color((provider.tint)(&colors)),
                )
                .status(dot)
                .control(switch)
                .selected(selected.is_some_and(|selected| selected.id == id))
                .dimmed(!enabled)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.provider = id;
                    cx.notify();
                }))
            })
            .collect();

        let mut split = SettingsSplit::new()
            .height(px(424.))
            .items(items)
            .when_some(self.add_provider_row(&settings, cx), |split, row| {
                split.list_end(row)
            });
        let Some(provider) = selected else {
            return split.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_4()
                    .py_6()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(m::providers_empty()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .child(m::providers_empty_description()),
                    ),
            );
        };
        let id = provider.id;
        split = split
            .heading(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        svg()
                            .path(provider.mark)
                            .size_4()
                            .text_color((provider.tint)(&colors)),
                    )
                    .child(div().flex_1().child((provider.name)()))
                    .child(
                        Button::new("provider-remove")
                            .label(m::provider_remove())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Xs)
                            .text_color(colors.destructive)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.remove_provider(id, cx)),
                            ),
                    ),
            )
            .child(self.status_row(id, cx));
        let version = match self.version_row(id, cx) {
            Some(row) => Some(row),
            None if id == CODEX => self.install_row(cx),
            None => None,
        };
        if let Some(row) = version {
            split = split.child(row);
        }
        split = split.child(self.binary_path_row(id));
        if id == DEFAULT_PROVIDER {
            split = split.child(self.subagent_effort_row(cx));
        }
        if id == CODEX {
            split = split.child(
                SettingsRow::new(m::codex_home())
                    .description(m::codex_home_description())
                    .child(div().mt_3().child(Input::new(&self.codex.home))),
            );
        }
        split
    }

    /// Codex's Install button, while hoshi finds no Codex to run and can install one.
    fn install_row(&mut self, cx: &mut Context<Self>) -> Option<SettingsRow> {
        if !self.provider_enabled(CODEX, cx) {
            return None;
        }
        let colors = cx.hoshi().clone();
        let workspace = self.workspace.read(cx);
        let version = workspace.providers.versions.get(CODEX)?.clone();
        if version.installed.is_some() || version.update_command.is_none() {
            return None;
        }
        let turns_running = provider::turns_running(workspace);
        let running = matches!(version.update, ProviderUpdate::Running { .. });
        let button = if running {
            Button::new("codex-installing")
                .loading(true)
                .label(m::codex_installing())
                .variant(ButtonVariant::Outline)
                .size(ButtonSize::Sm)
                .disabled(true)
        } else {
            Button::new("codex-install")
                .label(m::codex_install())
                .size(ButtonSize::Sm)
                .disabled(turns_running)
                .when(turns_running, |this| {
                    this.tooltip(m::provider_update_busy())
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    provider::start_update_of(&this.workspace, CODEX, cx)
                }))
        };
        let mono = cx.theme().mono_font_family.clone();
        let status = update_status(&version, &colors, mono);
        Some(
            SettingsRow::new(m::provider_version())
                .description(m::codex_not_installed_description())
                .control(button)
                .children(status.map(|status| div().mt_3().child(status))),
        )
    }

    /// Sign in with ChatGPT or sign out, and how a sign-in goes.
    fn codex_account_controls(
        &self,
        status: Option<ProviderAccountStatus>,
        cx: &mut Context<Self>,
    ) -> (Option<AnyElement>, Option<AnyElement>) {
        let colors = cx.hoshi().clone();
        let sign_in = self
            .workspace
            .read(cx)
            .providers
            .sign_ins
            .get(CODEX)
            .cloned()
            .unwrap_or_default();
        let note = |text: String, tone: Hsla| {
            div()
                .mt_2()
                .text_xs()
                .text_color(tone)
                .child(text)
                .into_any_element()
        };
        match (&sign_in, status) {
            (ProviderSignIn::Starting, _) => (
                Some(
                    Button::new("codex-sign-in-starting")
                        .loading(true)
                        .label(m::codex_sign_in())
                        .variant(ButtonVariant::Outline)
                        .size(ButtonSize::Sm)
                        .disabled(true)
                        .into_any_element(),
                ),
                Some(note(m::codex_sign_in_starting(), colors.muted_foreground)),
            ),
            (ProviderSignIn::Waiting { url }, _) => {
                let url = url.clone();
                let waiting = div()
                    .mt_2()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .child(hoshi_ui::spinner(px(12.), colors.muted_foreground))
                    .child(m::codex_sign_in_waiting())
                    .child(
                        div()
                            .id("codex-sign-in-open")
                            .text_color(colors.link)
                            .cursor_pointer()
                            .hover(|style| style.underline())
                            .on_click(move |_, _, cx| cx.open_url(&url))
                            .child(m::codex_sign_in_open_again()),
                    )
                    .into_any_element();
                (
                    Some(
                        Button::new("codex-sign-in-cancel")
                            .label(m::codex_sign_in_cancel())
                            .variant(ButtonVariant::Outline)
                            .size(ButtonSize::Sm)
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_codex_sign_in(cx)))
                            .into_any_element(),
                    ),
                    Some(waiting),
                )
            }
            (_, Some(ProviderAccountStatus::Ready)) => (
                Some(
                    Button::new("codex-sign-out")
                        .label(m::codex_sign_out())
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .loading(self.codex.signing_out)
                        .disabled(self.codex.signing_out)
                        .on_click(cx.listener(|this, _, _, cx| this.sign_out_codex(cx)))
                        .into_any_element(),
                ),
                self.codex
                    .sign_out_failed
                    .then(|| note(m::codex_sign_out_failed(), colors.status_error)),
            ),
            (_, Some(ProviderAccountStatus::SignedOut | ProviderAccountStatus::Unknown)) => {
                let failure = match &sign_in {
                    ProviderSignIn::Failed { reason } => {
                        Some(note(sign_in_failure(reason), colors.status_error))
                    }
                    _ => None,
                };
                (
                    Some(
                        Button::new("codex-sign-in")
                            .label(m::codex_sign_in())
                            .size(ButtonSize::Sm)
                            .on_click(cx.listener(|this, _, _, cx| this.sign_in_codex(cx)))
                            .into_any_element(),
                    ),
                    failure,
                )
            }
            _ => (None, None),
        }
    }

    fn sign_in_codex(&mut self, cx: &mut Context<Self>) {
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

    fn cancel_codex_sign_in(&mut self, cx: &mut Context<Self>) {
        let engine = self.workspace.read(cx).engine.clone();
        cx.background_spawn(async move { engine.cancel_codex_sign_in() })
            .detach();
    }

    fn sign_out_codex(&mut self, cx: &mut Context<Self>) {
        if self.codex.signing_out {
            return;
        }
        self.codex.signing_out = true;
        self.codex.sign_out_failed = false;
        cx.notify();
        let engine = self.workspace.read(cx).engine.clone();
        cx.spawn(async move |this, cx| {
            let signed_out = cx
                .background_spawn(async move { engine.sign_out_codex() })
                .await;
            this.update(cx, |this, cx| {
                this.codex.signing_out = false;
                this.codex.sign_out_failed = signed_out.is_err();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Opens a menu of the providers not added yet; `None` once every one is.
    fn add_provider_row(
        &self,
        settings: &ProviderSettings,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let addable = addable(settings);
        if addable.is_empty() {
            return None;
        }
        let colors = cx.hoshi().clone();
        let (muted, foreground, hover) = (
            colors.muted_foreground,
            colors.foreground,
            colors.muted.opacity(0.6),
        );
        let this = cx.weak_entity();
        Some(
            Popover::new("add-provider-menu")
                .side(Side::Bottom)
                .align(Align::Start)
                .trigger(move |open| {
                    div()
                        .id("add-provider")
                        .debug_selector(|| "add-provider".into())
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(10.))
                        .py_2()
                        .rounded_lg()
                        .text_sm()
                        .text_color(muted)
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover).text_color(foreground))
                        .when(open, |this| this.bg(hover).text_color(foreground))
                        .child(Icon::new(IconName::Plus).size_4().flex_none())
                        .child(m::provider_add())
                })
                .menu(move |menu, _, _| {
                    menu.items(addable.into_iter().map(|provider| {
                        let this = this.clone();
                        let id = provider.id;
                        MenuOption::new(
                            SharedString::from(format!("add-provider-{id}")),
                            (provider.name)(),
                        )
                        .leading(
                            svg()
                                .path(provider.mark)
                                .size_4()
                                .text_color((provider.tint)(&colors)),
                        )
                        .on_select(move |_, cx| {
                            this.update(cx, |this, cx| this.add_provider(id, cx)).ok();
                        })
                    }))
                })
                .into_any_element(),
        )
    }

    fn status_row(&self, provider: &str, cx: &mut Context<Self>) -> SettingsRow {
        if provider == CODEX {
            return self.codex_status_row(cx);
        }
        let workspace = self.workspace.read(cx);
        let settings = &self.settings(cx).providers;
        let binary = match provider {
            COPILOT => settings
                .copilot
                .binary_path
                .clone()
                .unwrap_or_else(m::copilot_binary_placeholder),
            _ => settings
                .claude_code
                .binary_path
                .clone()
                .unwrap_or_else(m::claude_code_binary_placeholder),
        };
        let (text, dot) = status_detail(
            provider,
            self.provider_enabled(provider, cx),
            workspace.providers.accounts.get(provider).copied(),
            workspace
                .usage_limits(provider)
                .and_then(|limits| limits.account.as_ref()),
            workspace.providers.logins.get(provider).map(String::as_str),
            &binary,
        );
        SettingsRow::new(m::provider_status()).description(status_text(text, dot))
    }

    fn codex_status_row(&self, cx: &mut Context<Self>) -> SettingsRow {
        let workspace = self.workspace.read(cx);
        let status = workspace.providers.accounts.get(CODEX).copied();
        let account = workspace.providers.codex_account.clone();
        let binary = self.settings(cx).providers.codex.binary_path.clone();
        let enabled = self.provider_enabled(CODEX, cx);
        let (text, dot) = codex_status_detail(enabled, status, account.as_ref(), binary.as_deref());
        let row = SettingsRow::new(m::provider_status()).description(status_text(text, dot));
        if !enabled {
            return row;
        }
        let (control, note) = self.codex_account_controls(status, cx);
        match control {
            Some(control) => row.control(control),
            None => row,
        }
        .children(note)
    }

    fn binary_path_row(&self, provider: &str) -> SettingsRow {
        let (state, description) = match provider {
            COPILOT => (
                &self.copilot_binary_path,
                m::copilot_binary_path_description(),
            ),
            CODEX => (&self.codex.binary_path, m::codex_binary_path_description()),
            _ => (&self.binary_path, m::claude_code_binary_path_description()),
        };
        SettingsRow::new(m::binary_path())
            .description(description)
            .child(div().mt_3().child(Input::new(state)))
    }

    fn health_check_row(&self, cx: &mut Context<Self>) -> SettingsRow {
        let seconds = self.settings(cx).providers.health_check_interval_secs;
        let step = |id: &'static str, icon: IconName, change: fn(u32) -> u32| {
            Button::new(id)
                .icon(icon)
                .variant(ButtonVariant::Outline)
                .size(ButtonSize::IconXs)
                .on_click({
                    let set = self.setter(cx, move |settings| {
                        let interval = &mut settings.providers.health_check_interval_secs;
                        *interval = change(*interval).min(MAX_HEALTH_CHECK_SECS);
                    });
                    move |_, window, cx| set(window, cx)
                })
        };
        let colors = cx.hoshi().clone();
        SettingsRow::new(m::health_check_interval())
            .description(m::health_check_interval_description())
            .control(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(step("health-check-less", IconName::Minus, |seconds| {
                        seconds.saturating_sub(HEALTH_CHECK_STEP_SECS)
                    }))
                    .child(
                        div()
                            .w(px(40.))
                            .text_center()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(seconds.to_string()),
                    )
                    .child(step("health-check-more", IconName::Plus, |seconds| {
                        seconds + HEALTH_CHECK_STEP_SECS
                    }))
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .child(m::seconds_unit()),
                    ),
            )
    }

    fn idle_compaction_row(&self, cx: &mut Context<Self>) -> SettingsRow {
        let current = self.settings(cx).providers.idle_compaction;
        let modes = [
            IdleCompaction::WhenYouReturn,
            IdleCompaction::BeforeCacheExpires,
            IdleCompaction::Never,
        ]
        .map(|mode| {
            SelectOption::new(
                SharedString::from(format!("idle-compaction-{mode:?}")),
                idle_compaction_label(mode),
                mode == current,
            )
            .on_select(self.setter(cx, move |settings| {
                settings.providers.idle_compaction = mode
            }))
        });
        SettingsRow::new(m::compact_idle_threads())
            .description(m::compact_idle_threads_description())
            .control(
                Select::new("idle-compaction", idle_compaction_label(current))
                    .align(Align::End)
                    .options(modes),
            )
    }

    fn subagent_effort_row(&self, cx: &mut Context<Self>) -> SettingsRow {
        let current = self.settings(cx).providers.claude_code.subagent_effort;
        let efforts = [SubagentEffort::Thread, SubagentEffort::AgentDecides].map(|effort| {
            SelectOption::new(
                SharedString::from(format!("subagent-effort-{effort:?}")),
                effort_label(effort),
                effort == current,
            )
            .description(effort_description(effort))
            .on_select(self.setter(cx, move |settings| {
                settings.providers.claude_code.subagent_effort = effort
            }))
        });
        SettingsRow::new(m::subagent_effort())
            .description(m::subagent_effort_description())
            .control(
                Select::new("claude-code-subagent-effort", effort_label(current))
                    .align(Align::End)
                    .menu_width(px(288.))
                    .options(efforts),
            )
    }

    /// The installed CLI, with an Update button only when a newer release can be installed.
    fn version_row(
        &mut self,
        provider: &'static str,
        cx: &mut Context<Self>,
    ) -> Option<SettingsRow> {
        if !self.provider_enabled(provider, cx) {
            return None;
        }
        let colors = cx.hoshi().clone();
        let workspace = self.workspace.read(cx);
        let version = workspace.providers.versions.get(provider)?.clone();
        version.installed.as_ref()?;
        let turns_running = provider::turns_running(workspace);
        let running = matches!(version.update, ProviderUpdate::Running { .. });
        let update_to = version
            .latest
            .clone()
            .filter(|_| !running && version.update_available && version.update_command.is_some());

        let control = if running {
            Some(
                Button::new("provider-updating")
                    .loading(true)
                    .label(m::provider_update_running())
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::Sm)
                    .disabled(true),
            )
        } else {
            update_to.map(|latest| {
                Button::new("provider-update")
                    .label(m::provider_update_to(latest))
                    .size(ButtonSize::Sm)
                    .disabled(turns_running)
                    .when(turns_running, |this| {
                        this.tooltip(m::provider_update_busy())
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        provider::start_update_of(&this.workspace, provider, cx)
                    }))
            })
        };

        let mono = cx.theme().mono_font_family.clone();
        let status = update_status(&version, &colors, mono);
        let row = SettingsRow::new(m::provider_version()).description(version_summary(&version));
        Some(
            match control {
                Some(control) => row.control(control),
                None => row,
            }
            .children(status.map(|status| div().mt_3().child(status))),
        )
    }

    /// Checks the providers once, when they never were.
    pub(super) fn load_providers(&mut self, cx: &mut Context<Self>) {
        if self.workspace.read(cx).providers.checked_at.is_none() {
            self.check_providers(cx);
        }
    }

    pub(super) fn check_providers(&mut self, cx: &mut Context<Self>) {
        if self.checking_providers {
            return;
        }
        self.checking_providers = true;
        cx.notify();
        let engine = self.workspace.read(cx).engine.clone();
        let workspace = self.workspace.downgrade();
        cx.spawn(async move |this, cx| {
            let status = cx
                .background_spawn(async move { engine.check_providers() })
                .await;
            workspace
                .update(cx, |workspace, cx| {
                    workspace.set_providers(status);
                    cx.notify();
                })
                .ok();
            this.update(cx, |this, cx| {
                this.checking_providers = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests;
