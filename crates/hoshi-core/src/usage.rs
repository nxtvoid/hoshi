use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// How full a thread's context window was after its last model call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThreadContextUsage {
    pub thread_id: String,
    pub usage: ContextUsage,
}

/// The provider plan's rolling limits. Empty when the plan has none, as with API keys.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UsageLimits {
    pub windows: Vec<UsageWindow>,
}

impl UsageLimits {
    /// Replaces the windows `update` reports and keeps the others, since a
    /// provider may report some windows mid-turn and all of them on request.
    pub fn merge(&mut self, update: UsageLimits) {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum UsageWindowKind {
    Session,
    Weekly,
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
        let mut limits = UsageLimits {
            windows: vec![
                window(UsageWindowKind::Session, None, 10),
                window(UsageWindowKind::Weekly, None, 20),
                window(UsageWindowKind::Weekly, Some("Fable"), 0),
            ],
        };

        limits.merge(UsageLimits {
            windows: vec![window(UsageWindowKind::Session, None, 35)],
        });

        assert_eq!(limits.windows[0].used_percent, 35);
        assert_eq!(limits.windows[1].used_percent, 20);
        assert_eq!(limits.windows[2].model.as_deref(), Some("Fable"));
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
