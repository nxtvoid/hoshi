use crate::{ContextUsage, IDLE_COMPACTION_TOKENS, IdleCompaction};
use std::ops::Range;

/// How long before the prompt cache expires an idle thread compacts: enough for its session
/// to resume and send the compaction while the history is still cached.
pub const IDLE_COMPACTION_LEAD_MS: u64 = 2 * 60 * 1000;

const COMPACT_COMMAND: &str = "/compact";

/// The context size from which compacting is worth a turn: half the window, or the size an
/// idle thread compacts at when that comes first.
pub fn compaction_threshold(usage: &ContextUsage) -> u64 {
    let half = usage.max_tokens.filter(|max| *max > 0).map(|max| max / 2);
    half.map_or(IDLE_COMPACTION_TOKENS, |half| {
        half.min(IDLE_COMPACTION_TOKENS)
    })
}

pub fn worth_compacting(usage: &ContextUsage) -> bool {
    usage.used_tokens >= compaction_threshold(usage)
}

/// Whether `text` is the `/compact` command, which hoshi sends without showing it.
pub fn is_compact_command(text: &str) -> bool {
    let text = text.trim();
    text.get(..COMPACT_COMMAND.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(COMPACT_COMMAND))
        && text[COMPACT_COMMAND.len()..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
}

/// Whether the next message to an idle thread compacts its conversation first, under `mode`:
/// the context is large and its prompt cache ran out, so the message would pay to read all
/// of it again. `now` is in Unix milliseconds.
pub fn compacts_on_return(mode: IdleCompaction, usage: Option<&ContextUsage>, now: u64) -> bool {
    mode != IdleCompaction::Never
        && usage.is_some_and(|usage| {
            usage.used_tokens >= IDLE_COMPACTION_TOKENS
                && usage.cache_expires_at.is_some_and(|expires| now >= expires)
        })
}

/// When, in Unix milliseconds, an idle thread compacts before its prompt cache expires under
/// `mode`; `None` when it does not. A compaction outside the window is skipped.
pub fn compaction_before_expiry(
    mode: IdleCompaction,
    usage: Option<&ContextUsage>,
) -> Option<Range<u64>> {
    if mode != IdleCompaction::BeforeCacheExpires {
        return None;
    }
    let usage = usage.filter(|usage| usage.used_tokens >= IDLE_COMPACTION_TOKENS)?;
    let expires = usage.cache_expires_at?;
    Some(expires.saturating_sub(IDLE_COMPACTION_LEAD_MS)..expires)
}

#[cfg(test)]
mod tests {
    use super::{
        IDLE_COMPACTION_LEAD_MS, compaction_before_expiry, compacts_on_return, worth_compacting,
    };
    use crate::{ContextUsage, IdleCompaction};

    fn usage(used_tokens: u64, cache_expires_at: Option<u64>) -> ContextUsage {
        ContextUsage {
            used_tokens,
            max_tokens: Some(1_000_000),
            cache_ttl_secs: Some(3600),
            cache_expires_at,
        }
    }

    #[test]
    fn a_large_thread_compacts_on_return_once_its_cache_ran_out() {
        let large = usage(200_000, Some(1_000));
        for mode in [
            IdleCompaction::WhenYouReturn,
            IdleCompaction::BeforeCacheExpires,
        ] {
            assert!(compacts_on_return(mode, Some(&large), 1_000));
            assert!(!compacts_on_return(mode, Some(&large), 999));
        }
        assert!(!compacts_on_return(
            IdleCompaction::Never,
            Some(&large),
            1_000
        ));
    }

    #[test]
    fn a_small_or_uncached_thread_never_compacts_on_return() {
        let mode = IdleCompaction::WhenYouReturn;
        assert!(!compacts_on_return(mode, Some(&usage(199_999, Some(1))), 5));
        assert!(!compacts_on_return(mode, Some(&usage(250_000, None)), 5));
        assert!(!compacts_on_return(mode, None, 5));
    }

    #[test]
    fn compacting_before_expiry_opens_shortly_before_the_cache_runs_out() {
        let expires = 10 * IDLE_COMPACTION_LEAD_MS;
        let window = compaction_before_expiry(
            IdleCompaction::BeforeCacheExpires,
            Some(&usage(250_000, Some(expires))),
        );
        assert_eq!(window, Some(expires - IDLE_COMPACTION_LEAD_MS..expires));
    }

    #[test]
    fn only_a_large_cached_thread_in_its_mode_compacts_before_expiry() {
        let large = usage(250_000, Some(1_000_000));
        for mode in [IdleCompaction::WhenYouReturn, IdleCompaction::Never] {
            assert_eq!(compaction_before_expiry(mode, Some(&large)), None);
        }
        let mode = IdleCompaction::BeforeCacheExpires;
        assert_eq!(
            compaction_before_expiry(mode, Some(&usage(199_999, Some(1_000_000)))),
            None
        );
        assert_eq!(
            compaction_before_expiry(mode, Some(&usage(250_000, None))),
            None
        );
        assert_eq!(compaction_before_expiry(mode, None), None);
    }

    #[test]
    fn compacting_is_worth_it_from_half_the_window_or_the_idle_size_whichever_comes_first() {
        let usage = |used_tokens, max_tokens| ContextUsage {
            used_tokens,
            max_tokens,
            cache_ttl_secs: None,
            cache_expires_at: None,
        };
        assert!(!worth_compacting(&usage(99_999, Some(200_000))));
        assert!(worth_compacting(&usage(100_000, Some(200_000))));
        assert!(!worth_compacting(&usage(199_999, Some(1_000_000))));
        assert!(worth_compacting(&usage(200_000, Some(1_000_000))));
        assert!(!worth_compacting(&usage(150_000, None)));
        assert!(worth_compacting(&usage(200_000, None)));
    }
}
