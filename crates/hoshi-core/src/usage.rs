use crate::ProviderAccount;
use serde::{Deserialize, Serialize};

/// How full a thread's context window was after its last model call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    pub used_tokens: u64,
    /// `None` while the provider has not said how large the window is.
    pub max_tokens: Option<u64>,
    /// How long the provider keeps the conversation's prompt cached after a model call.
    #[serde(default)]
    pub cache_ttl_secs: Option<u64>,
    /// When that cache runs out, in Unix milliseconds: past it, the next message pays to
    /// read the whole context again, which compacting first avoids.
    #[serde(default)]
    pub cache_expires_at: Option<u64>,
}

impl ContextUsage {
    /// Keeps the known window size and cache lifetime when an update does not carry them.
    pub fn merged_into(self, previous: Option<ContextUsage>) -> Self {
        Self {
            used_tokens: self.used_tokens,
            max_tokens: self
                .max_tokens
                .or(previous.and_then(|usage| usage.max_tokens)),
            cache_ttl_secs: self
                .cache_ttl_secs
                .or(previous.and_then(|usage| usage.cache_ttl_secs)),
            cache_expires_at: self
                .cache_expires_at
                .or(previous.and_then(|usage| usage.cache_expires_at)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadContextUsage {
    pub thread_id: String,
    pub usage: ContextUsage,
}

/// The provider plan's rolling limits. Empty when the plan has none, as with API keys.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageLimits {
    pub windows: Vec<UsageWindow>,
    /// `None` when the plan can't bank resets or they couldn't be read.
    #[serde(default)]
    pub banked_resets: Option<BankedResets>,
    /// `None` when the provider's login couldn't be read.
    #[serde(default)]
    pub account: Option<ProviderAccount>,
    /// `None` when the plan has no extra usage turned on, or the provider doesn't say.
    #[serde(default)]
    pub extra_usage: Option<ExtraUsage>,
}

impl UsageLimits {
    /// Replaces the windows `update` reports and keeps the others, since a
    /// provider may report some windows mid-turn and all of them on request.
    pub fn merge(&mut self, update: UsageLimits) {
        if update.banked_resets.is_some() {
            self.banked_resets = update.banked_resets;
        }
        if update.account.is_some() {
            self.account = update.account;
        }
        if update.extra_usage.is_some() {
            self.extra_usage = update.extra_usage;
        }
        for window in update.windows {
            match self
                .windows
                .iter_mut()
                .find(|known| known.kind == window.kind && known.model == window.model)
            {
                Some(known) => *known = window,
                None => self.windows.push(window),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub kind: UsageWindowKind,
    /// The model a window is limited to, as the provider names it; `None` covers every model.
    pub model: Option<String>,
    /// 0 to 100.
    pub used_percent: u8,
    pub window_minutes: u32,
    /// RFC 3339.
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UsageWindowKind {
    Session,
    Weekly,
}

/// Session resets the plan banked, each spendable once to clear the session limit early.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BankedResets {
    /// Zero when none can be spent now.
    pub count: u32,
    /// The grant the next spend draws from; `None` leaves nothing to spend.
    pub next_grant_id: Option<String>,
    /// When each spendable grant expires, RFC 3339, soonest first. Grants that never
    /// expire are left out.
    pub expires_at: Vec<String>,
    /// The kinds of reset that can be spent now, one entry each, when resets differ in what
    /// they refill; empty when they don't.
    #[serde(default)]
    pub grants: Vec<BankedGrant>,
}

/// What a banked reset refills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResetKind {
    /// The session limit.
    Session,
    /// The weekly limit.
    Weekly,
    /// One model's allowance, giving back what it used of the weekly limit.
    Fable,
    /// The session and weekly limits.
    Full,
}

/// The banked resets of one kind; spending draws from grant `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BankedGrant {
    pub id: String,
    pub kind: ResetKind,
    /// How many resets of this kind are left.
    pub resets: u32,
    /// When grant `id` expires, RFC 3339; `None` when it never does.
    pub expires_at: Option<String>,
}

/// Usage paid past the plan's limits, for the current period: what was spent, its cap, or
/// what is left of credits bought ahead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraUsage {
    pub unit: SpendUnit,
    /// Spent this period; `None` when the provider tells only a balance.
    pub used: Option<u64>,
    /// The cap for the period; `None` when there is none.
    pub limit: Option<u64>,
    /// Credits bought ahead that are left; `None` when there are none to tell.
    pub balance: Option<u64>,
    /// The provider lets usage go on without a cap or a balance.
    pub unlimited: bool,
    /// When the period's spend starts over, RFC 3339.
    pub resets_at: Option<String>,
}

/// What the amounts of [`ExtraUsage`] count, each an integer in `10^-exponent` of its unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SpendUnit {
    /// `currency` is ISO 4217; `None` when the provider doesn't say.
    Money {
        currency: Option<String>,
        exponent: u8,
    },
    /// The provider's own credits.
    Credits { exponent: u8 },
}

/// What spending a banked reset did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BankedResetSpend {
    /// The session limit was cleared.
    Reset,
    /// The session limit wasn't reached, so nothing was spent.
    NothingToReset,
    /// That grant was already spent.
    AlreadyUsed,
    NoneLeft,
    Failed {
        reason: BankedResetFailure,
    },
}

impl BankedResetSpend {
    /// Whether the provider answered, so a retry is a new spend rather than the same one.
    pub fn settled(&self) -> bool {
        !matches!(
            self,
            BankedResetSpend::Failed {
                reason: BankedResetFailure::Unconfirmed | BankedResetFailure::RequestFailed
            }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BankedResetFailure {
    /// The provider's login or account file couldn't be read.
    LoginUnreadable,
    SignedOut,
    RateLimited,
    CoolingDown,
    /// The provider couldn't say whether it reset; spending again retries the same request.
    Unconfirmed,
    /// No answer; spending again retries the same request.
    RequestFailed,
    /// The reset went through but the limits couldn't be read again.
    LimitsUnread,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(kind: UsageWindowKind, model: Option<&str>, used_percent: u8) -> UsageWindow {
        UsageWindow {
            kind,
            model: model.map(str::to_owned),
            used_percent,
            window_minutes: 300,
            resets_at: None,
        }
    }

    #[test]
    fn a_partial_update_keeps_the_windows_it_does_not_report() {
        let banked = BankedResets {
            count: 1,
            next_grant_id: Some("grant_a".into()),
            expires_at: Vec::new(),
            grants: Vec::new(),
        };
        let mut limits = UsageLimits {
            windows: vec![
                window(UsageWindowKind::Session, None, 10),
                window(UsageWindowKind::Weekly, None, 20),
                window(UsageWindowKind::Weekly, Some("Fable"), 0),
            ],
            banked_resets: Some(banked.clone()),
            account: Some(ProviderAccount {
                email: Some("nxtvoid@example.com".into()),
                plan: Some(crate::ProviderPlan::Max),
            }),
            extra_usage: None,
        };

        limits.merge(UsageLimits {
            windows: vec![window(UsageWindowKind::Session, None, 35)],
            banked_resets: None,
            account: None,
            extra_usage: None,
        });

        assert_eq!(limits.windows[0].used_percent, 35);
        assert_eq!(limits.windows[1].used_percent, 20);
        assert_eq!(limits.windows[2].model.as_deref(), Some("Fable"));
        assert_eq!(limits.banked_resets, Some(banked));
        assert!(limits.account.is_some());
    }

    #[test]
    fn an_update_without_a_window_size_keeps_the_known_one() {
        let known = ContextUsage {
            used_tokens: 10,
            max_tokens: Some(200_000),
            ..ContextUsage::default()
        };
        let update = ContextUsage {
            used_tokens: 12,
            max_tokens: None,
            ..ContextUsage::default()
        };

        assert_eq!(update.merged_into(Some(known)).max_tokens, Some(200_000));
    }
}
