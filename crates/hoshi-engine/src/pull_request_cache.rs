//! What the side panel last read of each thread's pull request. It answers at once with
//! what it has, and says when that is old enough to read again in the background
//! (stale-while-revalidate), so opening a pull request never waits on the network twice.

use hoshi_core::{ChecksState, PullRequest, PullRequestDetail};
use hoshi_forge::PullRequestFile;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// How long a detail read stays fresh.
const DETAIL_FRESH_FOR: Duration = Duration::from_secs(60);
/// Shorter while a check runs, so its result shows up soon.
const DETAIL_FRESH_FOR_PENDING_CHECKS: Duration = Duration::from_secs(15);

/// Which read of a pull request a cached value belongs to: its number, and the
/// `updated_at` the background sync last saw. Any change to it makes the value stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub number: u32,
    pub updated_at: String,
}

impl Version {
    pub fn of(pull_request: &PullRequest) -> Self {
        Self {
            number: pull_request.number,
            updated_at: pull_request.updated_at.clone(),
        }
    }
}

struct Entry<T> {
    version: Version,
    /// `None` when loaded from disk: shown at once, but read again.
    read_at: Option<Instant>,
    value: T,
}

/// Values by thread, plus the threads being read now so a read is never started twice.
struct Cache<T> {
    entries: HashMap<String, Entry<T>>,
    reading: HashSet<String>,
}

impl<T> Default for Cache<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            reading: HashSet::new(),
        }
    }
}

impl<T> Cache<T> {
    /// The cached value for this pull request (`None` if there is none or it belongs to
    /// another pull request), and whether to read it again.
    fn get(
        &self,
        thread_id: &str,
        version: &Version,
        fresh_for: impl Fn(&T) -> Duration,
        now: Instant,
    ) -> (Option<&T>, bool) {
        let Some(entry) = self
            .entries
            .get(thread_id)
            .filter(|entry| entry.version.number == version.number)
        else {
            return (None, true);
        };
        let fresh = entry.version == *version
            && entry
                .read_at
                .is_some_and(|read_at| now.duration_since(read_at) < fresh_for(&entry.value));
        (Some(&entry.value), !fresh)
    }

    /// Marks the thread as being read. `false` when a read is already on its way.
    fn start_reading(&mut self, thread_id: &str) -> bool {
        self.reading.insert(thread_id.to_owned())
    }

    fn finish_reading(&mut self, thread_id: &str, read: Option<(Version, T)>, now: Instant) {
        self.reading.remove(thread_id);
        if let Some((version, value)) = read {
            let entry = Entry {
                version,
                read_at: Some(now),
                value,
            };
            self.entries.insert(thread_id.to_owned(), entry);
        }
    }

    /// Keeps the value to show, but makes the next request read it again.
    fn invalidate(&mut self, thread_id: &str) {
        if let Some(entry) = self.entries.get_mut(thread_id) {
            entry.read_at = None;
        }
    }

    fn forget(&mut self, thread_id: &str) {
        self.entries.remove(thread_id);
        self.reading.remove(thread_id);
    }
}

fn detail_fresh_for(detail: &PullRequestDetail) -> Duration {
    let pending = detail
        .checks
        .iter()
        .any(|check| check.state == ChecksState::Pending);
    if pending {
        DETAIL_FRESH_FOR_PENDING_CHECKS
    } else {
        DETAIL_FRESH_FOR
    }
}

/// The detail and the changed files of each thread's pull request.
#[derive(Default)]
pub struct PullRequestCache {
    details: Cache<PullRequestDetail>,
    files: Cache<Vec<PullRequestFile>>,
}

impl PullRequestCache {
    /// Adds details saved by an earlier run; they show at once and are read again. A
    /// detail read in this run already is kept.
    pub fn restore_details(
        &mut self,
        saved: impl IntoIterator<Item = (String, Version, PullRequestDetail)>,
    ) {
        for (thread_id, version, detail) in saved {
            self.details.entries.entry(thread_id).or_insert(Entry {
                version,
                read_at: None,
                value: detail,
            });
        }
    }

    /// The detail to show now, and whether to start reading it again.
    pub fn detail(
        &mut self,
        thread_id: &str,
        version: &Version,
        now: Instant,
    ) -> (Option<PullRequestDetail>, bool) {
        let (detail, stale) = self.details.get(thread_id, version, detail_fresh_for, now);
        let detail = detail.cloned();
        let start = stale && self.details.start_reading(thread_id);
        (detail, start)
    }

    pub fn finish_detail(
        &mut self,
        thread_id: &str,
        read: Option<(Version, PullRequestDetail)>,
        now: Instant,
    ) {
        self.details.finish_reading(thread_id, read, now);
    }

    /// The files to show now, if they are current; whether to start reading them.
    pub fn files(
        &mut self,
        thread_id: &str,
        version: &Version,
        now: Instant,
    ) -> (Option<&Vec<PullRequestFile>>, bool) {
        let (files, stale) = self.files.get(thread_id, version, |_| Duration::MAX, now);
        let current = files.filter(|_| !stale);
        if current.is_some() {
            return (self.current_files(thread_id), false);
        }
        let start = self.files.start_reading(thread_id);
        (None, start)
    }

    pub fn finish_files(
        &mut self,
        thread_id: &str,
        read: Option<(Version, Vec<PullRequestFile>)>,
        now: Instant,
    ) {
        self.files.finish_reading(thread_id, read, now);
    }

    /// The files of the last read, whatever their version, to look up one patch.
    pub fn current_files(&self, thread_id: &str) -> Option<&Vec<PullRequestFile>> {
        self.files.entries.get(thread_id).map(|entry| &entry.value)
    }

    /// After a change made from hoshi: show what is cached, but read everything again.
    pub fn invalidate(&mut self, thread_id: &str) {
        self.details.invalidate(thread_id);
        self.files.invalidate(thread_id);
    }

    pub fn forget(&mut self, thread_id: &str) {
        self.details.forget(thread_id);
        self.files.forget(thread_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoshi_core::{Mergeability, PullRequestCheck};

    fn version(number: u32, updated_at: &str) -> Version {
        Version {
            number,
            updated_at: updated_at.into(),
        }
    }

    fn detail(pending: bool) -> PullRequestDetail {
        PullRequestDetail {
            supports: hoshi_core::PullRequestSupport::all(),
            forge_id: "PR_1".into(),
            repository: "o/r".into(),
            mergeability: Mergeability::Mergeable,
            merge_methods: Vec::new(),
            can_edit: true,
            body: String::new(),
            changed_files: 1,
            reviewers: Vec::new(),
            labels: Vec::new(),
            checks: vec![PullRequestCheck {
                name: "build".into(),
                state: if pending {
                    ChecksState::Pending
                } else {
                    ChecksState::Passing
                },
                url: None,
            }],
            review_threads: Vec::new(),
            timeline: Vec::new(),
        }
    }

    #[test]
    fn nothing_cached_reads_once_until_the_read_finishes() {
        let mut cache = PullRequestCache::default();
        let now = Instant::now();
        let v1 = version(3, "a");

        assert_eq!(cache.detail("t", &v1, now), (None, true));
        assert_eq!(cache.detail("t", &v1, now), (None, false));

        cache.finish_detail("t", Some((v1.clone(), detail(false))), now);
        assert_eq!(cache.detail("t", &v1, now), (Some(detail(false)), false));
    }

    #[test]
    fn a_stale_detail_still_shows_while_it_is_read_again() {
        let mut cache = PullRequestCache::default();
        let start = Instant::now();
        cache.finish_detail("t", Some((version(3, "a"), detail(false))), start);

        let later = start + DETAIL_FRESH_FOR + Duration::from_secs(1);
        assert_eq!(
            cache.detail("t", &version(3, "a"), later),
            (Some(detail(false)), true)
        );
        assert_eq!(
            cache.detail("t", &version(3, "b"), start),
            (Some(detail(false)), false),
            "already being read"
        );
    }

    #[test]
    fn a_newer_summary_or_running_checks_make_it_stale_sooner() {
        let mut cache = PullRequestCache::default();
        let start = Instant::now();
        cache.finish_detail("t", Some((version(3, "a"), detail(true))), start);
        cache.finish_detail("u", Some((version(4, "a"), detail(false))), start);
        let soon = start + DETAIL_FRESH_FOR_PENDING_CHECKS + Duration::from_secs(1);

        assert!(cache.detail("t", &version(3, "a"), soon).1);
        assert!(cache.detail("u", &version(4, "b"), start).1);
        assert!(!cache.detail("u", &version(4, "a"), soon).1);
    }

    #[test]
    fn another_pull_request_on_the_thread_shows_nothing_old() {
        let mut cache = PullRequestCache::default();
        let now = Instant::now();
        cache.finish_detail("t", Some((version(3, "a"), detail(false))), now);

        assert_eq!(cache.detail("t", &version(9, "a"), now), (None, true));
    }

    #[test]
    fn saved_details_show_at_once_but_are_read_again() {
        let saved = [("t".to_owned(), version(3, "a"), detail(false))];
        let mut cache = PullRequestCache::default();
        cache.restore_details(saved);

        assert_eq!(
            cache.detail("t", &version(3, "a"), Instant::now()),
            (Some(detail(false)), true)
        );
    }

    #[test]
    fn saved_details_never_replace_a_fresh_read() {
        let mut cache = PullRequestCache::default();
        let now = Instant::now();
        cache.finish_detail("t", Some((version(3, "b"), detail(true))), now);
        cache.restore_details([("t".to_owned(), version(3, "a"), detail(false))]);

        assert_eq!(
            cache.detail("t", &version(3, "b"), now),
            (Some(detail(true)), false)
        );
    }

    #[test]
    fn files_are_kept_until_the_pull_request_changes_or_is_invalidated() {
        let mut cache = PullRequestCache::default();
        let now = Instant::now();
        assert_eq!(cache.files("t", &version(3, "a"), now), (None, true));
        cache.finish_files("t", Some((version(3, "a"), Vec::new())), now);

        let far = now + Duration::from_secs(3600);
        assert_eq!(
            cache.files("t", &version(3, "a"), far),
            (Some(&Vec::new()), false)
        );
        assert_eq!(cache.files("t", &version(3, "b"), now), (None, true));
        cache.finish_files("t", Some((version(3, "b"), Vec::new())), now);
        cache.invalidate("t");
        assert_eq!(cache.files("t", &version(3, "b"), now), (None, true));
    }
}
