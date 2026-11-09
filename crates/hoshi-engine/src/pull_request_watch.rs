//! What wakes an agent watching its pull request, after t3code's watch: checks that fail,
//! checks that all pass, new comments and reviews, a conflict, or the pull request closing.

use crate::{State, delegation, now};
use chrono::{DateTime, FixedOffset};
use hoshi_core::{
    ChecksState, ForgeKind, Mergeability, PullRequest, PullRequestDetail, PullRequestState,
    PullRequestUpdate, TimelineItem, record_pull_request_watch, send_pull_request_update,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// Comment-only updates in a row before the watch stops, so bots replying to each other
/// can't wake the agent forever. Check or conflict news resets the count.
const COMMENT_WAKE_LIMIT: u32 = 10;
const LISTED_CHECKS: usize = 10;
const SNIPPET_CHARS: usize = 200;

/// What the agent was already told about the pull request it watches.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct WatchState {
    head_sha: Option<String>,
    told_failed: Vec<String>,
    told_passed: bool,
    told_conflict: bool,
    /// Comments up to this moment were seen. RFC 3339.
    remarks_through: String,
    comment_wakes: u32,
}

/// A message for the agent, and what it says for the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Wake {
    pub update: PullRequestUpdate,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub state: WatchState,
    pub wake: Option<Wake>,
    /// The watch is over: merged, closed, or too many comment-only updates.
    pub stop: bool,
}

struct Remark {
    author: String,
    path: Option<String>,
    text: String,
    url: String,
}

fn time(text: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(text).ok()
}

fn after(text: &str, through: &str) -> bool {
    match (time(text), time(through)) {
        (Some(at), Some(through)) => at > through,
        _ => false,
    }
}

fn failing(detail: &PullRequestDetail) -> Vec<String> {
    detail
        .checks
        .iter()
        .filter(|check| check.state == ChecksState::Failing)
        .map(|check| check.name.clone())
        .collect()
}

fn all_passed(detail: &PullRequestDetail) -> bool {
    !detail.checks.is_empty()
        && detail
            .checks
            .iter()
            .all(|check| check.state == ChecksState::Passing)
}

impl WatchState {
    /// From now on: what is already there was seen, so it never wakes the agent.
    pub(crate) fn start(pull_request: &PullRequest, detail: &PullRequestDetail, now: &str) -> Self {
        Self {
            head_sha: pull_request.head_sha.clone(),
            told_failed: failing(detail),
            told_passed: all_passed(detail),
            told_conflict: detail.mergeability == Mergeability::Conflicting,
            remarks_through: now.to_owned(),
            comment_wakes: 0,
        }
    }
}

/// Comments and reviews since `through`, except those by `viewer`, the account hoshi reads
/// the forge with, whose remarks the agent made itself.
fn remarks(
    detail: &PullRequestDetail,
    through: &str,
    viewer: Option<&str>,
) -> (Vec<Remark>, String) {
    let mut latest = through.to_owned();
    let mut found = Vec::new();
    let mut take =
        |author: &Option<String>, path: Option<&str>, text: String, url: &str, at: &str| {
            if !after(at, through) {
                return;
            }
            if after(at, &latest) {
                latest = at.to_owned();
            }
            let Some(author) = author.clone() else { return };
            if viewer.is_some_and(|viewer| viewer.eq_ignore_ascii_case(&author)) {
                return;
            }
            found.push(Remark {
                author,
                path: path.map(str::to_owned),
                text,
                url: url.to_owned(),
            });
        };
    for item in &detail.timeline {
        match item {
            TimelineItem::Comment {
                author,
                body,
                created_at,
                url,
            } => take(author, None, body.clone(), url, created_at),
            TimelineItem::Review {
                author,
                state,
                body,
                created_at,
                url,
            } => {
                let text = if body.trim().is_empty() {
                    format!("{state:?}")
                } else {
                    body.clone()
                };
                take(author, None, text, url, created_at);
            }
            TimelineItem::Commit { .. } => {}
        }
    }
    for thread in &detail.review_threads {
        for comment in &thread.comments {
            take(
                &comment.author,
                Some(&thread.path),
                comment.body.clone(),
                &comment.url,
                &comment.created_at,
            );
        }
    }
    (found, latest)
}

fn snippet(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(SNIPPET_CHARS) {
        Some((index, _)) => format!("{}…", &flat[..index]),
        None => flat,
    }
}

/// Compares the pull request with what the agent was told, and words what is new.
pub(crate) fn evaluate(
    state: &WatchState,
    pull_request: &PullRequest,
    detail: &PullRequestDetail,
    viewer: Option<&str>,
) -> Outcome {
    let mut next = state.clone();
    let number = pull_request.number;
    match pull_request.state {
        PullRequestState::Merged => {
            return Outcome {
                state: next,
                wake: None,
                stop: true,
            };
        }
        PullRequestState::Closed => {
            let text = format!(
                "Pull request #{number} ({}) was closed, so hoshi stopped watching it. Call watch_pull_request if it reopens.",
                pull_request.url
            );
            let update = PullRequestUpdate {
                number,
                closed: true,
                stopped: true,
                ..PullRequestUpdate::default()
            };
            return Outcome {
                state: next,
                wake: Some(Wake { update, text }),
                stop: true,
            };
        }
        PullRequestState::Open | PullRequestState::Draft => {}
    }
    if pull_request.head_sha.is_some() && pull_request.head_sha != state.head_sha {
        next.head_sha.clone_from(&pull_request.head_sha);
        next.told_failed.clear();
        next.told_passed = false;
        next.told_conflict = false;
        next.comment_wakes = 0;
    }
    let sha: String = next
        .head_sha
        .as_deref()
        .unwrap_or_default()
        .chars()
        .take(7)
        .collect();
    let mut update = PullRequestUpdate {
        number,
        ..PullRequestUpdate::default()
    };
    let mut lines = Vec::new();

    let failed: Vec<_> = detail
        .checks
        .iter()
        .filter(|check| check.state == ChecksState::Failing)
        .filter(|check| !next.told_failed.contains(&check.name))
        .collect();
    if !failed.is_empty() {
        lines.push(format!("- Checks failed on {sha}:"));
        for check in failed.iter().take(LISTED_CHECKS) {
            let url = check
                .url
                .as_deref()
                .map(|url| format!(" {url}"))
                .unwrap_or_default();
            lines.push(format!("  - {}{url}", check.name));
        }
        if failed.len() > LISTED_CHECKS {
            lines.push(format!("  - and {} more", failed.len() - LISTED_CHECKS));
        }
        update.failed_checks = failed.iter().map(|check| check.name.clone()).collect();
        next.told_failed
            .extend(update.failed_checks.iter().cloned());
    }
    if all_passed(detail) && !next.told_passed {
        lines.push(format!(
            "- All {} checks passed on {sha}.",
            detail.checks.len()
        ));
        update.checks_passed = true;
        next.told_passed = true;
    }

    let (found, latest) = remarks(detail, &state.remarks_through, viewer);
    next.remarks_through = latest;
    if !found.is_empty() {
        lines.push(format!("- {} new comments:", found.len()));
        for remark in &found {
            let path = remark
                .path
                .as_deref()
                .map(|path| format!(" on {path}"))
                .unwrap_or_default();
            lines.push(format!(
                "  - {}{path}: \"{}\" {}",
                remark.author,
                snippet(&remark.text),
                remark.url
            ));
        }
        update.comments = found.len() as u32;
    }

    match detail.mergeability {
        Mergeability::Conflicting if !next.told_conflict => {
            lines.push(format!(
                "- The branch now conflicts with {}.",
                pull_request.base_branch
            ));
            update.conflicting = true;
            next.told_conflict = true;
        }
        Mergeability::Mergeable => next.told_conflict = false,
        _ => {}
    }

    if lines.is_empty() {
        return Outcome {
            state: next,
            wake: None,
            stop: false,
        };
    }
    let comments_only =
        update.failed_checks.is_empty() && !update.checks_passed && !update.conflicting;
    next.comment_wakes = if comments_only {
        next.comment_wakes + 1
    } else {
        0
    };
    let stop = next.comment_wakes >= COMMENT_WAKE_LIMIT;
    update.stopped = stop;
    let closing = if stop {
        format!(
            "hoshi stopped watching after {COMMENT_WAKE_LIMIT} comment-only updates in a row. Call watch_pull_request to watch it again."
        )
    } else {
        "Look into each item and act on it as your task requires. This pull request stays watched, so do not watch it again. End your turn when you are done, and hoshi wakes you on the next change. When you hand the work back to the user, call unwatch_pull_request first.".to_owned()
    };
    let text = format!(
        "Update on pull request #{number} ({}), which hoshi is watching for you:\n{}\n\n{closing}",
        pull_request.url,
        lines.join("\n")
    );
    Outcome {
        state: next,
        wake: Some(Wake { update, text }),
        stop,
    }
}

/// How often a watched pull request is read in full, unless its summary changed sooner.
const DETAIL_EVERY: Duration = Duration::from_secs(2 * 60);

impl State {
    /// The watched pull request of a thread when its full read is due, with the forge's
    /// account if known.
    pub(crate) fn watch_due(&mut self, thread_id: &str) -> Option<(PullRequest, Option<String>)> {
        let thread = self.model.thread(thread_id)?;
        if !thread.watching_pull_request {
            return None;
        }
        let pull_request = self.pull_requests.get(thread_id)?.clone();
        let fresh = self
            .watch_reads
            .get(thread_id)
            .is_some_and(|(at, read)| at.elapsed() < DETAIL_EVERY && *read == pull_request);
        if fresh {
            return None;
        }
        self.watch_reads
            .insert(thread_id.to_owned(), (Instant::now(), pull_request.clone()));
        let viewer = self.viewers.get(&pull_request.forge).cloned();
        Some((pull_request, viewer))
    }

    /// Weighs a fresh read of a watched pull request, and wakes the agent on what is new.
    pub(crate) fn observe_watch(
        &mut self,
        thread_id: &str,
        forge: ForgeKind,
        pull_request: &PullRequest,
        detail: &PullRequestDetail,
        viewer: Option<String>,
    ) {
        if let Some(viewer) = &viewer {
            self.viewers.insert(forge, viewer.clone());
        }
        if !self
            .model
            .thread(thread_id)
            .is_some_and(|thread| thread.watching_pull_request)
        {
            return;
        }
        let Some(state) = self.watches.get(thread_id) else {
            if pull_request.state.is_finished() {
                self.stop_watching(thread_id);
            } else {
                let started = WatchState::start(pull_request, detail, &now());
                self.watches.insert(thread_id.to_owned(), started);
            }
            return;
        };
        let outcome = evaluate(state, pull_request, detail, viewer.as_deref());
        self.watches.insert(thread_id.to_owned(), outcome.state);
        if let Some(wake) = outcome.wake {
            self.deliver_wake(thread_id, wake);
        }
        if outcome.stop {
            self.stop_watching(thread_id);
        }
    }

    pub(crate) fn stop_watching(&mut self, thread_id: &str) {
        self.watches.remove(thread_id);
        self.watch_reads.remove(thread_id);
        let events = record_pull_request_watch(&self.model, thread_id, false);
        if let Err(error) = self.commit(&events, &now()) {
            eprintln!("hoshi: failed to stop watching a pull request: {error}");
        }
    }

    /// Starts the thread's turn with the update, or keeps it until the running turn ends.
    fn deliver_wake(&mut self, thread_id: &str, wake: Wake) {
        let busy = self
            .model
            .thread(thread_id)
            .is_some_and(|thread| thread.active_turn_id.is_some());
        if busy {
            self.pending_wakes
                .entry(thread_id.to_owned())
                .or_default()
                .push(wake);
            return;
        }
        let now = now();
        let Ok(events) = send_pull_request_update(
            &self.model,
            thread_id,
            &delegation::new_id(),
            &wake.text,
            wake.update,
            &now,
        ) else {
            return;
        };
        if let Err(error) = self.commit(&events, &now) {
            eprintln!("hoshi: failed to wake a thread on its pull request: {error}");
            return;
        }
        self.react(&events);
    }

    /// Updates that came while the thread's turn ran, now that it is idle.
    pub(crate) fn deliver_pending_wakes(&mut self, thread_id: &str) {
        let idle = self
            .model
            .thread(thread_id)
            .is_some_and(|thread| thread.active_turn_id.is_none());
        if !idle {
            return;
        }
        let waiting = self.pending_wakes.remove(thread_id).unwrap_or_default();
        if let Some(wake) = merge(waiting) {
            self.deliver_wake(thread_id, wake);
        }
    }

    /// `watch_pull_request`: wakes this thread's agent on its pull request's news.
    pub(crate) fn watch_pull_request(&mut self, caller: &str) -> Result<Value, String> {
        let thread = self
            .model
            .thread(caller)
            .ok_or("this thread no longer exists")?;
        if thread.parent_thread_id.is_some() {
            return Err("this thread works on a delegated task, so it can't watch pull requests: its parent owns the pull request. Finish your task and report back instead.".into());
        }
        let was_watching = thread.watching_pull_request;
        let pull_request = self
            .pull_requests
            .get(caller)
            .filter(|pull_request| !pull_request.state.is_finished())
            .cloned()
            .ok_or("this thread's branch has no open pull request yet: push it and open one first, then watch it")?;
        let events = record_pull_request_watch(&self.model, caller, true);
        self.commit(&events, &now())
            .map_err(|error| error.to_string())?;
        self.watch_reads.remove(caller);
        self.pull_requests.check_soon(caller);
        self.wake_sync();
        Ok(json!({
            "number": pull_request.number,
            "url": pull_request.url,
            "watching": true,
            "wasWatching": was_watching,
        }))
    }

    /// `unwatch_pull_request`: no more wakes; the pull request stays the thread's.
    pub(crate) fn unwatch_pull_request(&mut self, caller: &str) -> Result<Value, String> {
        let was_watching = self
            .model
            .thread(caller)
            .ok_or("this thread no longer exists")?
            .watching_pull_request;
        self.stop_watching(caller);
        self.pending_wakes.remove(caller);
        Ok(json!({ "watching": false, "wasWatching": was_watching }))
    }
}

/// One message for updates that waited for a busy turn to end.
pub(crate) fn merge(wakes: Vec<Wake>) -> Option<Wake> {
    let mut wakes = wakes.into_iter();
    let first = wakes.next()?;
    Some(wakes.fold(first, |mut merged, wake| {
        let update = wake.update;
        merged.update.failed_checks.extend(update.failed_checks);
        merged.update.checks_passed |= update.checks_passed;
        merged.update.comments += update.comments;
        merged.update.conflicting |= update.conflicting;
        merged.update.closed |= update.closed;
        merged.update.stopped |= update.stopped;
        merged.text = format!("{}\n\n{}", merged.text, wake.text);
        merged
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoshi_core::{
        ForgeKind, PullRequestCheck, PullRequestSupport, ReviewComment, ReviewState, ReviewThread,
    };

    fn pull_request(state: PullRequestState, head: &str) -> PullRequest {
        PullRequest {
            forge: ForgeKind::Github,
            number: 7,
            title: "Fix login".into(),
            url: "https://forge/pr/7".into(),
            author: Some("me".into()),
            head_branch: "hoshi/login".into(),
            head_sha: Some(head.into()),
            base_branch: "main".into(),
            state,
            checks: None,
            unresolved_comments: 0,
            additions: 1,
            deletions: 0,
            updated_at: "2026-11-08T10:00:00Z".into(),
        }
    }

    fn check(name: &str, state: ChecksState) -> PullRequestCheck {
        PullRequestCheck {
            name: name.into(),
            state,
            url: None,
        }
    }

    fn detail(checks: Vec<PullRequestCheck>, mergeability: Mergeability) -> PullRequestDetail {
        PullRequestDetail {
            supports: PullRequestSupport::all(),
            forge_id: "id".into(),
            repository: "o/r".into(),
            mergeability,
            merge_methods: Vec::new(),
            can_edit: true,
            body: String::new(),
            changed_files: 1,
            reviewers: Vec::new(),
            labels: Vec::new(),
            checks,
            review_threads: Vec::new(),
            timeline: Vec::new(),
        }
    }

    const START: &str = "2026-11-08T10:00:00Z";

    #[test]
    fn news_already_there_when_watching_starts_never_wakes() {
        let open = pull_request(PullRequestState::Open, "abc1234");
        let failing = detail(
            vec![check("lint", ChecksState::Failing)],
            Mergeability::Conflicting,
        );
        let state = WatchState::start(&open, &failing, START);

        let outcome = evaluate(&state, &open, &failing, Some("me"));

        assert_eq!(outcome.wake, None);
        assert!(!outcome.stop);
    }

    #[test]
    fn a_failing_check_then_all_passing_wake_once_each() {
        let open = pull_request(PullRequestState::Open, "abc1234");
        let pending = detail(
            vec![
                check("lint", ChecksState::Pending),
                check("test", ChecksState::Pending),
            ],
            Mergeability::Mergeable,
        );
        let state = WatchState::start(&open, &pending, START);
        let failed = detail(
            vec![
                check("lint", ChecksState::Failing),
                check("test", ChecksState::Pending),
            ],
            Mergeability::Mergeable,
        );

        let first = evaluate(&state, &open, &failed, None);
        let again = evaluate(&first.state, &open, &failed, None);
        let pushed = pull_request(PullRequestState::Open, "def5678");
        let passed = detail(
            vec![
                check("lint", ChecksState::Passing),
                check("test", ChecksState::Passing),
            ],
            Mergeability::Mergeable,
        );
        let fixed = evaluate(&again.state, &pushed, &passed, None);

        let wake = first.wake.unwrap();
        assert_eq!(wake.update.failed_checks, ["lint"]);
        assert!(wake.text.contains("- Checks failed on abc1234:\n  - lint"));
        assert_eq!(again.wake, None);
        let wake = fixed.wake.unwrap();
        assert!(wake.update.checks_passed);
        assert!(wake.text.contains("All 2 checks passed on def5678."));
    }

    #[test]
    fn new_comments_by_others_wake_and_a_conflict_is_told_once() {
        let open = pull_request(PullRequestState::Open, "abc1234");
        let quiet = detail(Vec::new(), Mergeability::Mergeable);
        let state = WatchState::start(&open, &quiet, START);
        let mut busy = detail(Vec::new(), Mergeability::Conflicting);
        busy.timeline = vec![
            TimelineItem::Comment {
                author: Some("reviewer".into()),
                body: "Please   rename this.".into(),
                created_at: "2026-11-08T10:05:00Z".into(),
                url: "https://forge/c/1".into(),
            },
            TimelineItem::Comment {
                author: Some("me".into()),
                body: "Done.".into(),
                created_at: "2026-11-08T10:06:00Z".into(),
                url: "https://forge/c/2".into(),
            },
            TimelineItem::Review {
                author: Some("lead".into()),
                state: ReviewState::Approved,
                body: String::new(),
                created_at: "2026-11-08T09:00:00Z".into(),
                url: "https://forge/r/1".into(),
            },
        ];
        busy.review_threads = vec![ReviewThread {
            id: "t".into(),
            path: "src/login.ts".into(),
            line: Some(3),
            resolved: false,
            comments: vec![ReviewComment {
                author: Some("reviewer".into()),
                body: "Nit".into(),
                created_at: "2026-11-08T10:07:00Z".into(),
                url: "https://forge/c/3".into(),
            }],
        }];

        let first = evaluate(&state, &open, &busy, Some("me"));
        let again = evaluate(&first.state, &open, &busy, Some("me"));

        let wake = first.wake.unwrap();
        assert_eq!(wake.update.comments, 2);
        assert!(wake.update.conflicting);
        assert!(
            wake.text
                .contains("  - reviewer: \"Please rename this.\" https://forge/c/1")
        );
        assert!(wake.text.contains("  - reviewer on src/login.ts: \"Nit\""));
        assert!(wake.text.contains("The branch now conflicts with main."));
        assert!(!wake.text.contains("Done."));
        assert_eq!(again.wake, None);
    }

    #[test]
    fn merged_stops_quietly_and_closed_says_so() {
        let quiet = detail(Vec::new(), Mergeability::Mergeable);
        let state = WatchState::start(&pull_request(PullRequestState::Open, "a"), &quiet, START);

        let merged = evaluate(
            &state,
            &pull_request(PullRequestState::Merged, "a"),
            &quiet,
            None,
        );
        let closed = evaluate(
            &state,
            &pull_request(PullRequestState::Closed, "a"),
            &quiet,
            None,
        );

        assert!(merged.stop && merged.wake.is_none());
        let wake = closed.wake.unwrap();
        assert!(closed.stop && wake.update.closed && wake.update.stopped);
    }

    #[test]
    fn waiting_updates_merge_into_one() {
        let wake = |comments: u32, failed: &[&str]| Wake {
            update: PullRequestUpdate {
                number: 7,
                comments,
                failed_checks: failed.iter().map(|name| (*name).to_owned()).collect(),
                ..PullRequestUpdate::default()
            },
            text: format!("{comments}"),
        };

        let merged = merge(vec![wake(1, &["lint"]), wake(2, &[])]).unwrap();

        assert_eq!(merged.update.comments, 3);
        assert_eq!(merged.update.failed_checks, ["lint"]);
        assert_eq!(merged.text, "1\n\n2");
        assert_eq!(merge(Vec::new()), None);
    }
}
