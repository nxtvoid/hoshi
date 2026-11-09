use chrono::{DateTime, Utc};
use hoshi_core::{ForgeKind, PullRequest};
use hoshi_forge::ForgeError;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How often the sync loop looks for threads that are due.
pub const TICK: Duration = Duration::from_secs(5);

const ACTIVE: Duration = Duration::from_secs(30);
const QUIET: Duration = Duration::from_secs(5 * 60);
/// Merged or closed long ago: read again only when something asks for it.
const SETTLED: Duration = Duration::from_secs(24 * 60 * 60);
/// A pull request updated within this window counts as active.
const RECENT_SECS: i64 = 30 * 60;
const SETTLED_AFTER_SECS: i64 = 24 * 60 * 60;
/// Refreshing on focus skips threads read this recently.
const FOCUS_MIN_AGE: Duration = Duration::from_secs(30);
const AFTER_NETWORK_ERROR: Duration = Duration::from_secs(60);
const AFTER_UNAUTHORIZED: Duration = Duration::from_secs(5 * 60);
const AFTER_NOT_FOUND: Duration = Duration::from_secs(30 * 60);

/// How long until a thread's pull request is read again.
pub fn next_interval(
    working: bool,
    pull_request: Option<&PullRequest>,
    now: DateTime<Utc>,
) -> Duration {
    if working {
        return ACTIVE;
    }
    let Some(pull_request) = pull_request else {
        return QUIET;
    };
    let age_secs = DateTime::parse_from_rfc3339(&pull_request.updated_at)
        .map(|updated| (now - updated.with_timezone(&Utc)).num_seconds())
        .unwrap_or(i64::MAX);
    if pull_request.state.is_finished() {
        if age_secs > SETTLED_AFTER_SECS {
            SETTLED
        } else {
            QUIET
        }
    } else if age_secs < RECENT_SECS {
        ACTIVE
    } else {
        QUIET
    }
}

/// What was last read for each thread, and when each one is read next.
#[derive(Default)]
pub struct PullRequestSync {
    known: HashMap<String, PullRequest>,
    next_check: HashMap<String, Instant>,
    last_check: HashMap<String, Instant>,
    paused_until: HashMap<ForgeKind, Instant>,
}

impl PullRequestSync {
    pub fn new(known: impl IntoIterator<Item = (String, PullRequest)>) -> Self {
        Self {
            known: known.into_iter().collect(),
            ..Self::default()
        }
    }

    pub fn known(&self) -> impl Iterator<Item = (&String, &PullRequest)> {
        self.known.iter()
    }

    pub fn get(&self, thread_id: &str) -> Option<&PullRequest> {
        self.known.get(thread_id)
    }

    pub fn number(&self, thread_id: &str) -> Option<u32> {
        self.known
            .get(thread_id)
            .map(|pull_request| pull_request.number)
    }

    pub fn has_open(&self, thread_id: &str) -> bool {
        self.known
            .get(thread_id)
            .is_some_and(|pull_request| !pull_request.state.is_finished())
    }

    pub fn is_due(&self, thread_id: &str, forge: ForgeKind, now: Instant) -> bool {
        let paused = self
            .paused_until
            .get(&forge)
            .is_some_and(|until| now < *until);
        !paused && self.next_check.get(thread_id).is_none_or(|at| now >= *at)
    }

    pub fn check_soon(&mut self, thread_id: &str) {
        self.next_check.remove(thread_id);
    }

    /// On window focus: everything not read in the last few seconds is due.
    pub fn check_stale(&mut self, now: Instant) {
        let stale: Vec<String> = self
            .last_check
            .iter()
            .filter(|(_, at)| now.duration_since(**at) >= FOCUS_MIN_AGE)
            .map(|(thread_id, _)| thread_id.clone())
            .collect();
        for thread_id in stale {
            self.next_check.remove(&thread_id);
        }
    }

    /// After signing in to a forge: lifts its pause and reads every thread again.
    pub fn resume(&mut self, forge: ForgeKind) {
        self.paused_until.remove(&forge);
        self.next_check.clear();
    }

    pub fn forget(&mut self, thread_id: &str) {
        self.known.remove(thread_id);
        self.next_check.remove(thread_id);
        self.last_check.remove(thread_id);
    }

    /// Records a read and schedules the next one. Returns the new value when it changed.
    pub fn record(
        &mut self,
        thread_id: &str,
        forge: ForgeKind,
        result: Result<Option<PullRequest>, ForgeError>,
        working: bool,
        now: Instant,
        now_utc: DateTime<Utc>,
    ) -> Option<Option<PullRequest>> {
        self.last_check.insert(thread_id.to_owned(), now);
        let wait = match result {
            Ok(pull_request) => {
                let wait = next_interval(working, pull_request.as_ref(), now_utc);
                self.next_check.insert(thread_id.to_owned(), now + wait);
                let changed = self.known.get(thread_id) != pull_request.as_ref();
                if !changed {
                    return None;
                }
                match &pull_request {
                    Some(value) => self.known.insert(thread_id.to_owned(), value.clone()),
                    None => self.known.remove(thread_id),
                };
                return Some(pull_request);
            }
            Err(ForgeError::Unauthorized) => {
                self.paused_until.insert(forge, now + AFTER_UNAUTHORIZED);
                AFTER_UNAUTHORIZED
            }
            Err(ForgeError::RateLimited { retry_after_secs }) => {
                let wait = Duration::from_secs(retry_after_secs);
                self.paused_until.insert(forge, now + wait);
                wait
            }
            Err(ForgeError::NotFound) => AFTER_NOT_FOUND,
            Err(
                error @ (ForgeError::Network(_)
                | ForgeError::Unexpected(_)
                | ForgeError::Rejected(_)
                | ForgeError::Forbidden { .. }
                | ForgeError::Conflict(_)),
            ) => {
                eprintln!("hoshi: couldn't read a pull request: {error}");
                AFTER_NETWORK_ERROR
            }
        };
        self.next_check.insert(thread_id.to_owned(), now + wait);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoshi_core::PullRequestState;

    fn now_utc() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-07T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn pull_request(state: PullRequestState, updated_at: &str) -> PullRequest {
        PullRequest {
            forge: ForgeKind::Github,
            number: 1,
            title: "t".into(),
            url: "u".into(),
            author: None,
            head_branch: "feat".into(),
            head_sha: None,
            base_branch: "main".into(),
            state,
            checks: None,
            unresolved_comments: 0,
            additions: 0,
            deletions: 0,
            updated_at: updated_at.into(),
        }
    }

    #[test]
    fn cadence_follows_activity() {
        let open = |at| pull_request(PullRequestState::Open, at);
        let merged = |at| pull_request(PullRequestState::Merged, at);
        let interval = |working, value: Option<&PullRequest>| {
            next_interval(working, value, now_utc()).as_secs()
        };

        assert_eq!(interval(true, None), 30);
        assert_eq!(interval(false, None), 300);
        assert_eq!(interval(false, Some(&open("2026-11-07T11:50:00Z"))), 30);
        assert_eq!(interval(false, Some(&open("2026-11-06T12:00:00Z"))), 300);
        assert_eq!(interval(false, Some(&merged("2026-11-07T10:00:00Z"))), 300);
        assert_eq!(
            interval(false, Some(&merged("2026-11-01T10:00:00Z"))),
            24 * 60 * 60
        );
    }

    #[test]
    fn reports_only_changes_and_reschedules() {
        let mut sync = PullRequestSync::default();
        let now = Instant::now();
        let value = pull_request(PullRequestState::Open, "2026-11-07T11:59:00Z");

        assert!(sync.is_due("t", ForgeKind::Github, now));
        let first = sync.record(
            "t",
            ForgeKind::Github,
            Ok(Some(value.clone())),
            false,
            now,
            now_utc(),
        );
        let again = sync.record(
            "t",
            ForgeKind::Github,
            Ok(Some(value.clone())),
            false,
            now,
            now_utc(),
        );

        assert_eq!(first, Some(Some(value)));
        assert_eq!(again, None);
        assert!(!sync.is_due("t", ForgeKind::Github, now));
        sync.check_soon("t");
        assert!(sync.is_due("t", ForgeKind::Github, now));
    }

    #[test]
    fn a_rate_limit_pauses_the_whole_forge() {
        let mut sync = PullRequestSync::default();
        let now = Instant::now();
        let limited = Err(ForgeError::RateLimited {
            retry_after_secs: 120,
        });

        sync.record("a", ForgeKind::Github, limited, false, now, now_utc());

        assert!(!sync.is_due("b", ForgeKind::Github, now));
        assert!(sync.is_due("b", ForgeKind::Github, now + Duration::from_secs(121)));
    }

    #[test]
    fn focus_rechecks_only_threads_read_a_while_ago() {
        let mut sync = PullRequestSync::default();
        let start = Instant::now();
        sync.record("old", ForgeKind::Github, Ok(None), false, start, now_utc());
        let later = start + Duration::from_secs(40);
        sync.record("new", ForgeKind::Github, Ok(None), false, later, now_utc());

        sync.check_stale(later);

        assert!(sync.is_due("old", ForgeKind::Github, later));
        assert!(!sync.is_due("new", ForgeKind::Github, later));
    }
}
