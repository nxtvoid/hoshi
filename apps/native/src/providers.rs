//! The providers hoshi can run, as the screens that list them name and draw them.

use crate::m;
use gpui_kit::Hsla;
use hoshi_core::{ChatGptPlan, DEFAULT_PROVIDER, ProviderSettings, ProviderSignInFailure};
use hoshi_ui::assets::brand;
use hoshi_ui::prelude::Colors;

pub const COPILOT: &str = "copilot";
pub const CODEX: &str = "codex";

/// Where a provider's plan limits show.
#[derive(Clone, Copy)]
pub enum UsageReport {
    /// hoshi reads them and draws them.
    Limits,
    /// Only the provider's own page shows them.
    Elsewhere {
        label: fn() -> String,
        url: &'static str,
    },
}

pub struct KnownProvider {
    pub id: &'static str,
    pub name: fn() -> String,
    pub mark: &'static str,
    pub tint: fn(&Colors) -> Hsla,
    /// Whether it is added and enabled.
    pub switches: fn(&ProviderSettings) -> (bool, bool),
    pub switches_mut: fn(&mut ProviderSettings) -> (&mut bool, &mut bool),
    pub usage: UsageReport,
}

pub static KNOWN_PROVIDERS: [KnownProvider; 3] = [
    KnownProvider {
        id: DEFAULT_PROVIDER,
        name: m::claude_code,
        mark: brand::CLAUDE,
        tint: |colors| colors.provider_claude,
        switches: |settings| (settings.claude_code.added, settings.claude_code.enabled),
        switches_mut: |settings| {
            let claude = &mut settings.claude_code;
            (&mut claude.added, &mut claude.enabled)
        },
        usage: UsageReport::Limits,
    },
    KnownProvider {
        id: COPILOT,
        name: m::copilot,
        mark: brand::COPILOT,
        tint: |colors| colors.provider_copilot,
        switches: |settings| (settings.copilot.added, settings.copilot.enabled),
        switches_mut: |settings| {
            let copilot = &mut settings.copilot;
            (&mut copilot.added, &mut copilot.enabled)
        },
        usage: UsageReport::Elsewhere {
            label: m::usage_view_on_github,
            url: "https://github.com/settings/copilot",
        },
    },
    KnownProvider {
        id: CODEX,
        name: m::codex,
        mark: brand::CODEX,
        tint: |colors| colors.provider_codex,
        switches: |settings| (settings.codex.added, settings.codex.enabled),
        switches_mut: |settings| {
            let codex = &mut settings.codex;
            (&mut codex.added, &mut codex.enabled)
        },
        usage: UsageReport::Limits,
    },
];

pub fn known(provider: &str) -> Option<&'static KnownProvider> {
    KNOWN_PROVIDERS.iter().find(|known| known.id == provider)
}

impl KnownProvider {
    /// Whether the settings add it and turn it on.
    pub fn enabled(&self, settings: &ProviderSettings) -> bool {
        let (added, enabled) = (self.switches)(settings);
        added && enabled
    }
}

pub fn chatgpt_plan_label(plan: ChatGptPlan) -> String {
    match plan {
        ChatGptPlan::Free => m::chatgpt_plan_free(),
        ChatGptPlan::Go => m::chatgpt_plan_go(),
        ChatGptPlan::Plus => m::chatgpt_plan_plus(),
        ChatGptPlan::Pro => m::chatgpt_plan_pro(),
        ChatGptPlan::Team => m::chatgpt_plan_team(),
        ChatGptPlan::Business => m::chatgpt_plan_business(),
        ChatGptPlan::Enterprise => m::chatgpt_plan_enterprise(),
        ChatGptPlan::Edu => m::chatgpt_plan_edu(),
    }
}

pub fn sign_in_failure(reason: &ProviderSignInFailure) -> String {
    match reason {
        ProviderSignInFailure::NotInstalled => m::codex_sign_in_not_installed(),
        ProviderSignInFailure::SpawnFailed => m::codex_sign_in_spawn_failed(),
        ProviderSignInFailure::Ended => m::codex_sign_in_ended(),
        ProviderSignInFailure::TimedOut => m::codex_sign_in_timed_out(),
        ProviderSignInFailure::Cancelled => m::codex_sign_in_cancelled(),
        ProviderSignInFailure::Rejected {
            detail: Some(detail),
        } => m::codex_sign_in_rejected_detail(detail),
        ProviderSignInFailure::Rejected { detail: None } => m::codex_sign_in_rejected(),
    }
}
