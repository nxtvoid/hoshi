//! The usage modal: each provider plan's limits, one row per window, with a card on hover that
//! details it and spends a banked reset.

pub(crate) mod limits;
#[cfg(test)]
mod ui_tests;

use crate::m;
use crate::providers::{CODEX, KNOWN_PROVIDERS, KnownProvider, UsageReport, chatgpt_plan_label};
use crate::state::Workspace;
use chrono::{DateTime, Local, Utc};
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::{
    BankedGrant, BankedResetSpend, BankedResets, DEFAULT_PROVIDER, ExtraUsage, SpendUnit,
    UsageLimits, UsageWindow,
};
use hoshi_ui::prelude::*;
use limits::{
    ExtraTexts, LimitTone, SectionState, banked_resets_for, even_pace_left, extra_left_percent,
    extra_texts, format_expiry, format_span, grant_expiry, limit_tone, masked_email, plan_label,
    remaining_percent, reset_kind_description, reset_kind_label, reset_moment, reset_when,
    restored_percent, section_state, spend_message, spendable_resets, time_left_percent,
    time_to_expiry, time_to_reset, window_label,
};
use std::collections::HashMap;
use std::time::Duration;

const TICK: Duration = Duration::from_secs(60);
const HOVER_DELAY: Duration = Duration::from_millis(300);
const HOVER_CLOSE_DELAY: Duration = Duration::from_millis(100);

pub enum UsageEvent {
    /// Opens the providers' settings at this one.
    OpenSettings(&'static str),
}

/// How asking one provider for its limits goes.
#[derive(Default)]
struct ProviderUsage {
    checking: bool,
    /// The last time it was asked, it couldn't say.
    failed: bool,
    spending: bool,
    /// The grant being spent, when the user picked one.
    spending_grant: Option<String>,
    result: Option<BankedResetSpend>,
    email_revealed: bool,
}

pub struct UsageView {
    workspace: Entity<Workspace>,
    usage: HashMap<&'static str, ProviderUsage>,
    /// The provider whose banked reset waits for a yes.
    confirming: Option<&'static str>,
    /// The row whose card shows, by its provider and window.
    card: Option<(&'static str, usize)>,
    now: DateTime<Utc>,
    bars: std::cell::RefCell<Bars>,
    _observe: Subscription,
    _tick: Task<()>,
}

/// Each row's bar as it moves to a new value, by provider and window: from, to, and how many
/// times it moved.
type Bars = HashMap<(&'static str, usize), (u8, u8, usize)>;

impl EventEmitter<UsageEvent> for UsageView {}

/// Spends one of `provider`'s banked resets once the user says yes, as the usage modal does;
/// `None` when there is none to spend or the session limit isn't what ran out.
pub fn spend_reset_button(id: &str, provider: &'static str, cx: &App) -> Option<AnyElement> {
    let workspace = crate::markdown::workspace(cx)?;
    let limits = workspace.read(cx).usage_limits(provider);
    if !limits::session_spent(limits) {
        return None;
    }
    spendable_resets(limits)?;
    let selector = format!("use-reset-{id}");
    let button = Button::new(SharedString::from(selector.clone()))
        .label(m::usage_use_reset())
        .variant(ButtonVariant::Outline)
        .size(ButtonSize::Xs)
        .on_click(move |_, window, cx| confirm_reset(workspace.clone(), provider, window, cx));
    Some(
        div()
            .debug_selector(move || selector.clone())
            .child(button)
            .into_any_element(),
    )
}

/// The plan's extra usage as one more row of the limits: what is left of its cap, of the
/// credits bought ahead, or what was spent when there is neither.
fn extra_row(
    provider: &'static KnownProvider,
    extra: &ExtraUsage,
    below_windows: bool,
    now: DateTime<Utc>,
    colors: &Colors,
) -> impl IntoElement {
    let id = provider.id;
    let left = extra_left_percent(extra);
    let ExtraTexts {
        shown,
        note,
        detail,
    } = extra_texts(extra, now);
    let label = match extra.unit {
        SpendUnit::Money { .. } => m::usage_extra(),
        SpendUnit::Credits { .. } => m::usage_credits(),
    };
    let fill = match left.map(limit_tone) {
        Some(LimitTone::Low) => colors.status_error.opacity(0.6),
        Some(LimitTone::Warning) => colors.warning.opacity(0.6),
        _ => (provider.tint)(colors).opacity(0.45),
    };
    div()
        .debug_selector(move || format!("usage-extra-{id}"))
        .flex()
        .items_center()
        .gap_4()
        .px_4()
        .py_3()
        .when(below_windows, |this| {
            this.border_t_1().border_color(colors.border)
        })
        .child(
            div()
                .w(px(144.))
                .flex_none()
                .truncate()
                .text_sm()
                .child(label),
        )
        .child(
            div()
                .relative()
                .h_5()
                .flex_1()
                .min_w_0()
                .when_some(left, |this, left| {
                    this.child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(px(6.))
                            .bottom(px(6.))
                            .rounded_full()
                            .bg(colors.muted),
                    )
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top(px(6.))
                            .bottom(px(6.))
                            .w(relative(f32::from(left) / 100.))
                            .rounded_full()
                            .bg(fill),
                    )
                }),
        )
        .child(
            div()
                .w_20()
                .flex_none()
                .flex()
                .items_baseline()
                .justify_end()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(shown),
                )
                .when_some(note, |this, note| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .child(note),
                    )
                }),
        )
        .child(
            div()
                .w(px(112.))
                .flex_none()
                .truncate()
                .text_right()
                .text_xs()
                .text_color(colors.muted_foreground)
                .child(detail),
        )
}

fn confirm_reset(
    workspace: Entity<Workspace>,
    provider: &'static str,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(banked) = spendable_resets(workspace.read(cx).usage_limits(provider)) else {
        return;
    };
    let ask = DialogLayer::confirm(
        ConfirmRequest::new(m::usage_use_reset_title(), m::usage_use_reset())
            .description(m::usage_use_reset_description(
                banked.count.saturating_sub(1),
            ))
            .cancel_label(m::cancel()),
        window,
        cx,
    );
    let engine = workspace.read(cx).engine.clone();
    window
        .spawn(cx, async move |cx| {
            if !ask.await {
                return;
            }
            let (result, limits) = cx
                .background_spawn(async move {
                    let result = engine.spend_banked_reset_of(provider);
                    (result, engine.usage_limits_of(provider, false))
                })
                .await;
            cx.update(|window, cx| {
                if let Some(limits) = limits {
                    workspace.update(cx, |workspace, cx| {
                        workspace
                            .providers
                            .usage_limits
                            .insert(provider.to_owned(), limits);
                        cx.notify();
                    });
                }
                let tone = if matches!(result, BankedResetSpend::Failed { .. }) {
                    ConfirmTone::Warning
                } else {
                    ConfirmTone::Default
                };
                let notice = ConfirmRequest::new(spend_message(result), m::dialog_ok()).tone(tone);
                DialogLayer::confirm(notice, window, cx).detach();
            })
            .ok();
        })
        .detach();
}

/// The enabled providers: the ones hoshi reads the limits of, then the others.
fn sections(workspace: &Workspace) -> (Vec<&'static KnownProvider>, Vec<&'static KnownProvider>) {
    KNOWN_PROVIDERS
        .iter()
        .filter(|provider| provider.enabled(&workspace.settings.providers))
        .partition(|provider| matches!(provider.usage, UsageReport::Limits))
}

impl UsageView {
    pub fn new(workspace: Entity<Workspace>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&workspace, |_, _, cx| cx.notify());
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this
                    .update(cx, |this, cx| {
                        this.now = Utc::now();
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut view = Self {
            workspace,
            usage: HashMap::new(),
            confirming: None,
            card: None,
            now: Utc::now(),
            bars: Default::default(),
            _observe: observe,
            _tick: tick,
        };
        view.check_accounts(cx);
        view.refresh(false, cx);
        view
    }

    /// Asks each provider, which answers at once if it did within the last minute; `force`
    /// waits for a fresh answer.
    pub fn refresh(&mut self, force: bool, cx: &mut Context<Self>) {
        let engine = self.workspace.read(cx).engine.clone();
        let reporting = engine.usage_limit_providers();
        let (providers, _) = sections(self.workspace.read(cx));
        for provider in providers {
            let id = provider.id;
            let usage = self.usage.entry(id).or_default();
            if usage.checking {
                continue;
            }
            if !reporting.iter().any(|reports| reports == id) {
                usage.failed = true;
                continue;
            }
            usage.checking = true;
            let engine = engine.clone();
            cx.spawn(async move |this, cx| {
                let limits = cx
                    .background_spawn(async move { engine.usage_limits_of(id, force) })
                    .await;
                this.update(cx, |this, cx| {
                    let usage = this.usage.entry(id).or_default();
                    usage.checking = false;
                    usage.failed = limits.is_none();
                    if let Some(limits) = limits {
                        this.workspace.update(cx, |workspace, cx| {
                            workspace
                                .providers
                                .usage_limits
                                .insert(id.to_owned(), limits);
                            cx.notify();
                        });
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    /// Checks whether the providers are installed and signed in, when nothing did yet.
    fn check_accounts(&self, cx: &mut Context<Self>) {
        if self.workspace.read(cx).providers.checked_at.is_some() {
            return;
        }
        let engine = self.workspace.read(cx).engine.clone();
        let workspace = self.workspace.downgrade();
        cx.spawn(async move |_, cx| {
            let status = cx
                .background_spawn(async move { engine.check_providers() })
                .await;
            workspace
                .update(cx, |workspace, cx| {
                    workspace.set_providers(status);
                    cx.notify();
                })
                .ok();
        })
        .detach();
    }

    fn checking(&self) -> bool {
        self.usage.values().any(|usage| usage.checking)
    }

    fn busy(&self, provider: &str) -> bool {
        self.usage
            .get(provider)
            .is_some_and(|usage| usage.checking || usage.spending)
    }

    /// Asks before spending a banked reset of `grant`, or the next one when `None`.
    fn confirm_spend(
        &mut self,
        provider: &'static str,
        grant: Option<BankedGrant>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(banked) = spendable_resets(self.workspace.read(cx).usage_limits(provider)) else {
            return;
        };
        if self.busy(provider) || self.confirming.is_some() {
            return;
        }
        let description = match &grant {
            Some(grant) => m::usage_use_grant_description(reset_kind_description(grant.kind)),
            None => m::usage_use_reset_description(banked.count.saturating_sub(1)),
        };
        self.confirming = Some(provider);
        let ask = DialogLayer::confirm(
            ConfirmRequest::new(m::usage_use_reset_title(), m::usage_use_reset())
                .description(description)
                .cancel_label(m::cancel()),
            window,
            cx,
        );
        let grant_id = grant.map(|grant| grant.id);
        cx.spawn(async move |this, cx| {
            let go = ask.await;
            this.update(cx, |this, cx| {
                this.confirming = None;
                if go {
                    this.spend(provider, grant_id, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn spend(&mut self, provider: &'static str, grant_id: Option<String>, cx: &mut Context<Self>) {
        let usage = self.usage.entry(provider).or_default();
        if usage.spending {
            return;
        }
        usage.spending = true;
        usage.spending_grant = grant_id.clone();
        usage.result = None;
        let engine = self.workspace.read(cx).engine.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    engine.spend_banked_grant(provider, grant_id.as_deref())
                })
                .await;
            this.update(cx, |this, cx| {
                let usage = this.usage.entry(provider).or_default();
                usage.spending = false;
                usage.spending_grant = None;
                usage.result = Some(result);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn header(&self, colors: &Colors, cx: &mut Context<Self>) -> impl IntoElement {
        let refresh = if self.checking() {
            div()
                .id("usage-refresh")
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(28.))
                .opacity(0.5)
                .child(
                    Icon::new(IconName::RefreshCw)
                        .size_4()
                        .text_color(colors.foreground)
                        .with_loop(hoshi_ui::looping(Duration::from_secs(1)), |icon, delta| {
                            icon.transform(Transformation::rotate(percentage(delta)))
                        }),
                )
                .into_any_element()
        } else {
            Button::new("usage-refresh")
                .icon(IconName::RefreshCw)
                .size(ButtonSize::IconSm)
                .variant(ButtonVariant::Ghost)
                .tooltip(Tooltip::new(m::usage_refresh()).side(Side::Bottom))
                .on_click(cx.listener(|this, _, _, cx| this.refresh(true, cx)))
                .into_any_element()
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .h_12()
            .pl_5()
            .pr_2()
            .border_b_1()
            .border_color(colors.border)
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap_1p5()
                    .text_sm()
                    .child(div().text_color(colors.muted_foreground).child(m::usage()))
                    .child(
                        Icon::new(IconName::ChevronRight)
                            .size_3p5()
                            .text_color(colors.muted_foreground),
                    )
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(m::usage_limits()),
                    ),
            )
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(refresh)
                    .child(
                        DialogButton::new("close-usage")
                            .icon(IconName::X)
                            .variant(DialogButtonVariant::Ghost)
                            .size(DialogButtonSize::IconSm)
                            .accessible_label(m::close_usage())
                            .close(),
                    ),
            )
    }

    /// Who the provider is signed in as and on which plan; `None` hides both.
    fn signed_in_as(&self, provider: &KnownProvider, cx: &App) -> (Option<String>, Option<String>) {
        let workspace = self.workspace.read(cx);
        let account = workspace
            .usage_limits(provider.id)
            .and_then(|limits| limits.account.as_ref());
        let email = account.and_then(|account| account.email.clone());
        let plan = account.and_then(|account| account.plan).map(plan_label);
        match &workspace.providers.codex_account {
            Some(codex) if provider.id == CODEX && codex.chatgpt => (
                codex.email.clone().or(email),
                codex.plan.map(chatgpt_plan_label).or(plan),
            ),
            Some(_) if provider.id == CODEX => (None, None),
            _ => (email, plan),
        }
    }

    fn account(
        &self,
        provider: &'static KnownProvider,
        signed_in: (Option<String>, Option<String>),
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = provider.id;
        let (email, plan) = signed_in;
        let revealed = self.usage.get(id).is_some_and(|usage| usage.email_revealed);
        let tint = (provider.tint)(colors);
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(tile(provider, tint, px(36.), px(18.)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child((provider.name)()),
                    )
                    .when_some(email, |this, email| {
                        let shown = if revealed {
                            email
                        } else {
                            masked_email(&email)
                        };
                        this.child(
                            div()
                                .id(SharedString::from(format!("usage-email-{id}")))
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(colors.muted_foreground)
                                .cursor_pointer()
                                .child(shown)
                                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                    gpui_kit::base::GlobalState::suppress_text_selection(cx)
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    let usage = this.usage.entry(id).or_default();
                                    usage.email_revealed = !usage.email_revealed;
                                    cx.notify();
                                }))
                                .with_tooltip(
                                    Tooltip::new(if revealed {
                                        m::usage_hide_email()
                                    } else {
                                        m::usage_reveal_email()
                                    })
                                    .side(Side::Bottom),
                                ),
                        )
                    }),
            )
            .when_some(plan, |this, plan| {
                this.child(
                    div()
                        .ml_auto()
                        .flex_none()
                        .px_2p5()
                        .py_0p5()
                        .rounded_full()
                        .border_1()
                        .border_color(colors.border)
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(colors.muted_foreground)
                        .child(plan),
                )
            })
    }

    fn section(
        &self,
        provider: &'static KnownProvider,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> Div {
        let id = provider.id;
        let workspace = self.workspace.read(cx);
        let limits = workspace.usage_limits(id).cloned();
        let account = workspace.providers.accounts.get(id).copied();
        let usage = self.usage.get(id);
        let state = section_state(
            account,
            limits.as_ref(),
            usage.is_some_and(|usage| usage.checking),
            usage.is_some_and(|usage| usage.failed),
        );
        let signed_in = match state {
            SectionState::SignedOut | SectionState::NotInstalled => (None, None),
            _ => self.signed_in_as(provider, cx),
        };
        let open_settings = |cx: &mut Context<Self>| {
            Button::new(SharedString::from(format!("usage-open-settings-{id}")))
                .label(m::usage_open_settings())
                .variant(ButtonVariant::Outline)
                .size(ButtonSize::Xs)
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(UsageEvent::OpenSettings(id))))
                .into_any_element()
        };
        let name = (provider.name)();
        let body = match (state, &limits) {
            (SectionState::Limits, Some(limits)) => {
                self.rows(provider, limits, colors, cx).into_any_element()
            }
            (SectionState::NoLimits, _) => note(
                format!("usage-no-limits-{id}"),
                IconName::Infinity,
                None,
                m::usage_no_limits(),
                m::usage_no_limits_description(),
                None,
                colors,
            )
            .into_any_element(),
            (SectionState::SignedOut, _) => note(
                format!("usage-signed-out-{id}"),
                IconName::KeyRound,
                None,
                m::usage_signed_out(),
                m::usage_signed_out_description(&name),
                Some(open_settings(cx)),
                colors,
            )
            .into_any_element(),
            (SectionState::NotInstalled, _) => note(
                format!("usage-not-installed-{id}"),
                IconName::Download,
                None,
                m::usage_not_installed(),
                m::usage_not_installed_description(&name),
                Some(open_settings(cx)),
                colors,
            )
            .into_any_element(),
            (SectionState::Unavailable, _) => note(
                format!("usage-unavailable-{id}"),
                IconName::CircleAlert,
                Some(colors.status_error),
                m::usage_unavailable(),
                m::usage_unavailable_provider_description(&name),
                Some(
                    Button::new(SharedString::from(format!("usage-try-again-{id}")))
                        .label(m::usage_try_again())
                        .variant(ButtonVariant::Outline)
                        .size(ButtonSize::Xs)
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(true, cx)))
                        .into_any_element(),
                ),
                colors,
            )
            .into_any_element(),
            _ => skeleton(id, colors)
                .with_loop(
                    hoshi_ui::looping(Duration::from_secs(2)),
                    |skeleton, delta| {
                        skeleton.opacity(1. - 0.5 * (delta * std::f32::consts::PI).sin())
                    },
                )
                .into_any_element(),
        };
        let result = usage.and_then(|usage| usage.result).map(|result| {
            let failed = matches!(result, BankedResetSpend::Failed { .. });
            div()
                .id(SharedString::from(format!("usage-result-{id}")))
                .text_xs()
                .text_color(if failed {
                    colors.status_error
                } else {
                    colors.muted_foreground
                })
                .child(spend_message(result))
        });
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(self.account(provider, signed_in, colors, cx))
            .child(body)
            .children(result)
    }

    fn rows(
        &self,
        provider: &'static KnownProvider,
        limits: &UsageLimits,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .flex()
            .flex_col()
            .rounded_xl()
            .border_1()
            .border_color(colors.border)
            .overflow_hidden()
            .children(limits.windows.iter().enumerate().map(|(index, window)| {
                let banked = banked_resets_for(window, Some(limits)).cloned();
                self.row(provider, index, window, banked, colors, cx)
            }))
            .when_some(limits.extra_usage.as_ref(), |this, extra| {
                this.child(extra_row(
                    provider,
                    extra,
                    !limits.windows.is_empty(),
                    self.now,
                    colors,
                ))
            })
    }

    fn row(
        &self,
        provider: &'static KnownProvider,
        index: usize,
        window: &UsageWindow,
        banked: Option<BankedResets>,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = provider.id;
        let left = remaining_percent(window);
        let mark = time_left_percent(window, self.now);
        let reset = time_to_reset(window, self.now);
        let pinned = banked.is_some() && (self.confirming == Some(id) || self.busy(id));
        let open = self.card == Some((id, index)) || pinned;
        let fill = match limit_tone(left) {
            LimitTone::Low => colors.status_error.opacity(0.6),
            LimitTone::Warning => colors.warning.opacity(0.6),
            LimitTone::Normal => (provider.tint)(colors).opacity(0.45),
        };
        let hover_bg = colors.muted.opacity(0.5);
        let bar = div()
            .relative()
            .h_5()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(6.))
                    .bottom(px(6.))
                    .rounded_full()
                    .bg(colors.muted),
            )
            .child({
                let (from, to, moves) = {
                    let mut bars = self.bars.borrow_mut();
                    let bar = bars.entry((id, index)).or_insert((left, left, 0));
                    if bar.1 != left {
                        *bar = (bar.1, left, bar.2 + 1);
                    }
                    *bar
                };
                div()
                    .absolute()
                    .left_0()
                    .top(px(6.))
                    .bottom(px(6.))
                    .w(relative(f32::from(to) / 100.))
                    .rounded_full()
                    .bg(fill)
                    .with_animation(
                        ElementId::NamedInteger(
                            format!("usage-bar-{id}-{index}").into(),
                            moves as u64,
                        ),
                        Animation::new(Duration::from_millis(500)).with_easing(ease_out_quint()),
                        move |fill, delta| {
                            let width = f32::from(from) + (f32::from(to) - f32::from(from)) * delta;
                            fill.w(relative(width / 100.))
                        },
                    )
            })
            .when_some(mark, |this, mark| {
                this.child(
                    div()
                        .absolute()
                        .top(px(2.))
                        .bottom(px(2.))
                        .left(relative(mark / 100.))
                        .ml(px(-1.))
                        .w(px(2.))
                        .rounded_full()
                        .bg(colors.foreground),
                )
            });
        let count = banked.as_ref().map(|banked| banked.count);
        let row = div()
            .id(SharedString::from(format!("usage-row-{id}-{index}")))
            .debug_selector(move || format!("usage-row-{id}-{index}"))
            .flex()
            .flex_1()
            .items_center()
            .gap_4()
            .px_4()
            .py_3()
            .when(index > 0, |this| {
                this.border_t_1().border_color(colors.border)
            })
            .when(open, |this| this.bg(hover_bg))
            .hover(move |style| style.bg(hover_bg))
            .child(
                div()
                    .w(px(144.))
                    .flex_none()
                    .truncate()
                    .text_sm()
                    .child(window_label(window)),
            )
            .child(bar)
            .child(
                div()
                    .w_20()
                    .flex_none()
                    .flex()
                    .items_baseline()
                    .justify_end()
                    .gap_1()
                    .child(
                        div()
                            .debug_selector(move || format!("usage-left-{id}-{index}"))
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(m::usage_percent(left)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .child(m::usage_left()),
                    ),
            )
            .child(
                div()
                    .w(px(112.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1p5()
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .when_some(reset, |this, reset| this.child(format_span(reset)))
                    .when_some(count, |this, count| {
                        this.when(reset.is_some(), |this| this.child("·")).child(
                            div()
                                .id(SharedString::from(format!("usage-banked-{id}-{index}")))
                                .debug_selector(move || format!("usage-banked-{id}-{index}"))
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(Icon::new(IconName::Ticket).size_3p5())
                                .child(count.to_string()),
                        )
                    }),
            );
        let card = self.card(id, window, banked.as_ref(), colors, cx);
        Popover::new(SharedString::from(format!("usage-card-{id}-{index}")))
            .open(open)
            .open_on_hover(true)
            .delay(HOVER_DELAY)
            .close_delay(HOVER_CLOSE_DELAY)
            .side(Side::Bottom)
            .align(Align::End)
            .side_offset(px(6.))
            .initial_focus(PopoverFocus::None)
            .w_80()
            .p_0()
            .on_open_change(cx.listener(move |this, change: &PopoverChange, _, cx| {
                if change.open {
                    this.card = Some((id, index));
                } else if this.card == Some((id, index)) {
                    this.card = None;
                }
                cx.notify();
            }))
            .trigger(move |_| row)
            .content(move |_, _, _| card)
            .into_any_element()
    }

    fn card(
        &self,
        provider: &'static str,
        window: &UsageWindow,
        banked: Option<&BankedResets>,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> Div {
        let now = self.now;
        let reset = time_to_reset(window, now);
        let moment = reset_moment(window, &now.with_timezone(&Local));
        let mut rows = vec![(
            m::usage_detail_left(),
            format!("{}%", remaining_percent(window)),
        )];
        if let (Some(moment), Some(reset)) = (moment, reset) {
            rows.push((
                m::usage_detail_resets(),
                m::usage_reset_when(format_span(reset), reset_when(moment)),
            ));
        }
        if let Some(pace) = even_pace_left(window, now) {
            rows.push((m::usage_detail_even_pace(), m::usage_even_pace_left(pace)));
        }
        rows.push((
            m::usage_detail_restores(),
            m::usage_restores_at_reset(restored_percent(window)),
        ));
        let (terms, values): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
        let column = |texts: Vec<String>| div().flex().flex_col().gap_1p5().children(texts);
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .gap_4()
                    .p_3()
                    .text_xs()
                    .child(column(terms).text_color(colors.muted_foreground))
                    .child(column(values).flex_1().min_w_0()),
            )
            .when_some(banked, |this, banked| {
                this.child(self.banked_footer(provider, banked, colors, cx))
            })
    }

    fn banked_footer(
        &self,
        provider: &'static str,
        banked: &BankedResets,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let usage = self.usage.get(provider);
        let spending = usage.is_some_and(|usage| usage.spending);
        let spending_grant = usage.and_then(|usage| usage.spending_grant.clone());
        let disabled = self.busy(provider) || self.confirming.is_some();
        let use_button =
            |id: String, label: String, grant: Option<BankedGrant>, cx: &mut Context<Self>| {
                Button::new(SharedString::from(id))
                    .label(label)
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::Xs)
                    .disabled(disabled)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.confirm_spend(provider, grant.clone(), window, cx)
                    }))
            };
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .border_t_1()
            .border_color(colors.border);
        if banked.grants.is_empty() {
            let text = match time_to_expiry(banked, self.now) {
                Some(expiry) => {
                    m::usage_banked_resets_expiring(banked.count, format_expiry(expiry))
                }
                None => m::usage_banked_resets(banked.count),
            };
            let label = if spending {
                m::usage_using_reset()
            } else {
                m::usage_use_reset()
            };
            return footer
                .py_2()
                .text_xs()
                .text_color(colors.muted_foreground)
                .child(Icon::new(IconName::Ticket).size_3p5().flex_none())
                .child(div().flex_1().min_w_0().truncate().child(text))
                .child(div().flex_none().child(use_button(
                    format!("usage-use-reset-{provider}"),
                    label,
                    None,
                    cx,
                )))
                .into_any_element();
        }
        let mut grants = Vec::with_capacity(banked.grants.len());
        for (index, grant) in banked.grants.iter().enumerate() {
            let detail = match grant_expiry(grant, self.now) {
                Some(expiry) => m::usage_grant_left_expiring(grant.resets, format_expiry(expiry)),
                None => m::usage_grant_left(grant.resets),
            };
            let label = if spending_grant.as_deref() == Some(grant.id.as_str()) {
                m::usage_using_reset()
            } else {
                m::usage_use_grant()
            };
            grants.push(
                div()
                    .debug_selector(move || format!("usage-grant-{provider}-{index}"))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(colors.border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_color(colors.foreground)
                                    .child(reset_kind_label(grant.kind)),
                            )
                            .child(div().truncate().child(detail)),
                    )
                    .child(div().flex_none().child(use_button(
                        format!("usage-use-grant-{provider}-{index}"),
                        label,
                        Some(grant.clone()),
                        cx,
                    ))),
            );
        }
        div()
            .flex()
            .flex_col()
            .text_xs()
            .text_color(colors.muted_foreground)
            .child(
                footer
                    .pt_2()
                    .pb_1p5()
                    .child(Icon::new(IconName::Ticket).size_3p5().flex_none())
                    .child(m::usage_banked_resets(banked.count)),
            )
            .children(grants)
            .into_any_element()
    }

    /// The switch that shows the limits in the thread's card.
    fn in_details(&self, colors: &Colors, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let on = self.workspace.read(cx).settings.usage_limits_in_details;
        let workspace = self.workspace.downgrade();
        div()
            .debug_selector(|| "usage-in-details".into())
            .pt_5()
            .border_t_1()
            .border_color(colors.border)
            .child(
                SettingsRow::new(m::usage_in_details())
                    .description(m::usage_in_details_description())
                    .control(
                        div()
                            .debug_selector(|| "usage-in-details-switch".into())
                            .child(Switch::new("usage-in-details").checked(on).on_change(
                                move |on, _, cx| {
                                    workspace
                                        .update(cx, |workspace, cx| {
                                            workspace.set_usage_limits_in_details(on, cx)
                                        })
                                        .ok();
                                },
                            )),
                    ),
            )
    }

    /// A provider whose limits only its own page shows.
    fn elsewhere(
        &self,
        provider: &'static KnownProvider,
        first: bool,
        colors: &Colors,
    ) -> Option<impl IntoElement + use<>> {
        let UsageReport::Elsewhere { label, url } = provider.usage else {
            return None;
        };
        let id = provider.id;
        Some(
            div()
                .id(SharedString::from(format!("usage-not-reported-{id}")))
                .debug_selector(move || format!("usage-not-reported-{id}"))
                .flex()
                .items_center()
                .gap_3()
                .when(!first, |this| {
                    this.pt_5().border_t_1().border_color(colors.border)
                })
                .child(tile(provider, (provider.tint)(colors), px(28.), px(14.)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .child((provider.name)())
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.muted_foreground)
                                .child(m::usage_not_reported()),
                        ),
                )
                .child(
                    Button::new(SharedString::from(format!("usage-elsewhere-{id}")))
                        .label(label())
                        .suffix(Icon::new(IconName::ExternalLink).size_3())
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Xs)
                        .on_click(move |_, _, cx| cx.open_url(url)),
                ),
        )
    }
}

/// The provider's mark on a tile of its color.
fn tile(provider: &KnownProvider, tint: Hsla, size: Pixels, mark: Pixels) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(size)
        .rounded_lg()
        .bg(tint.opacity(0.15))
        .child(svg().path(provider.mark).size(mark).text_color(tint))
}

/// Why a provider shows no rows, in place of them.
fn note(
    id: String,
    icon: IconName,
    tone: Option<Hsla>,
    title: String,
    description: String,
    action: Option<AnyElement>,
    colors: &Colors,
) -> Stateful<Div> {
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector)
        .flex()
        .items_center()
        .gap_3()
        .px_4()
        .py_3()
        .rounded_xl()
        .border_1()
        .border_dashed()
        .border_color(colors.border)
        .text_sm()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size_7()
                .rounded_lg()
                .bg(colors.muted)
                .child(
                    Icon::new(icon)
                        .size_3p5()
                        .text_color(tone.unwrap_or(colors.foreground)),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(div().font_weight(FontWeight::MEDIUM).child(title))
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(description),
                ),
        )
        .children(action)
}

fn skeleton(provider: &'static str, colors: &Colors) -> Stateful<Div> {
    let bar = |width: Pixels| div().h_3().w(width).rounded_sm().bg(colors.muted);
    div()
        .id(SharedString::from(format!("usage-checking-{provider}")))
        .debug_selector(move || format!("usage-checking-{provider}"))
        .flex()
        .flex_col()
        .rounded_xl()
        .border_1()
        .border_color(colors.border)
        .children((0..2).map(|index| {
            div()
                .flex()
                .items_center()
                .gap_4()
                .px_4()
                .py_3()
                .when(index > 0, |this| {
                    this.border_t_1().border_color(colors.border)
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .h_5()
                        .w(px(144.))
                        .flex_none()
                        .child(bar(px(64.))),
                )
                .child(
                    div()
                        .h_2()
                        .flex_1()
                        .min_w_0()
                        .rounded_full()
                        .bg(colors.muted),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .w_20()
                        .flex_none()
                        .child(bar(px(48.))),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .w(px(112.))
                        .flex_none()
                        .child(bar(px(56.))),
                )
        }))
}

fn empty(title: String, description: String, action: AnyElement, colors: &Colors) -> Stateful<Div> {
    div()
        .id("usage-no-providers")
        .debug_selector(|| "usage-no-providers".into())
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_4()
        .p_6()
        .py_10()
        .rounded_xl()
        .border_1()
        .border_dashed()
        .border_color(colors.border)
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_2()
                .max_w_96()
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
                .child(
                    div()
                        .text_sm()
                        .text_center()
                        .text_color(colors.muted_foreground)
                        .child(description),
                ),
        )
        .child(action)
}

impl Render for UsageView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let (limited, elsewhere) = sections(self.workspace.read(cx));
        let mut content: Vec<AnyElement> = limited
            .iter()
            .map(|provider| self.section(provider, &colors, cx).into_any_element())
            .collect();
        let first = content.is_empty();
        content.extend(
            elsewhere
                .iter()
                .enumerate()
                .filter_map(|(index, provider)| {
                    self.elsewhere(provider, first && index == 0, &colors)
                })
                .map(IntoElement::into_any_element),
        );
        if !limited.is_empty() {
            content.push(self.in_details(&colors, cx).into_any_element());
        }
        if content.is_empty() {
            content.push(
                empty(
                    m::usage_no_providers(),
                    m::usage_no_providers_description(),
                    Button::new("usage-open-settings")
                        .label(m::usage_open_settings())
                        .variant(ButtonVariant::Outline)
                        .size(ButtonSize::Sm)
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(UsageEvent::OpenSettings(DEFAULT_PROVIDER))
                        }))
                        .into_any_element(),
                    &colors,
                )
                .into_any_element(),
            );
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .size_full()
            .child(self.header(&colors, cx))
            .child(
                div()
                    .id("usage-page")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .mx_auto()
                            .w_full()
                            .max_w(px(768.))
                            .px_6()
                            .py_8()
                            .flex()
                            .flex_col()
                            .gap_8()
                            .children(content),
                    ),
            )
    }
}
