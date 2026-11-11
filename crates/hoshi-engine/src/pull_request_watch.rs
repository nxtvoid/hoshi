//! What wakes an agent watching its pull request, after t3code's watch: checks that fail,
//! checks that all pass, new comments and reviews, a conflict, or the pull request closing.
//! What the agent was told is saved, so news from while hoshi was closed still wakes it.

use crate::{State, delegation, now};
use chrono::{DateTime, FixedOffset};
use hoshi_core::{
    ChecksState, ForgeKind, Mergeability, PullRequest, PullRequestDetail, PullRequestState,
    PullRequestUpdate, PullRequestWatch, TimelineItem, record_pull_request_watch,
    send_pull_request_update,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// Comment-only updates in a row before the watch stops, so bots replying to each other
/// can't wake the agent forever. Check, conflict or push news resets the count.
const COMMENT_WAKE_LIMIT: u32 = 10;
const LISTED_CHECKS: usize = 10;
const SNIPPET_CHARS: usize = 200;
const WATCH_NEXT: &str = "End your turn; hoshi wakes you on news.";

/// A message for the agent, and what it says for the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Wake {
    pub update: PullRequestUpdate,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub watch: PullRequestWatch,
    pub wake: Option<Wake>,
    /// The watch is over: merged, closed, or too many comment-only updates.
    pub stop: bool,
}

struct Remark {
    author: String,
    path: Option<String>,
    text: String,
    url: String,
    at: DateTime<FixedOffset>,
    stamp: String,
}

fn time(text: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(text).ok()
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

/// A new watch of pull request `number`. Its commit isn't read yet, so failing checks and a
/// conflict already there are news; comments from before `now` aren't. Watching again
/// keeps what the last watch of the same pull request told about checks and conflicts.
pub(crate) fn start(number: u32, ended: Option<&PullRequestWatch>, now: &str) -> PullRequestWatch {
    let told = ended
        .filter(|watch| watch.number == number)
        .cloned()
        .unwrap_or_else(|| PullRequestWatch {
            number,
            ..PullRequestWatch::default()
        });
    PullRequestWatch {
        remarks_through: now.to_owned(),
        remark_ids: Vec::new(),
        wakes: 0,
        ..told
    }
}

/// A watch from before hoshi saved what it told: what is there now counts as told, and
/// comments since `since` don't.
fn resumed(
    pull_request: &PullRequest,
    detail: &PullRequestDetail,
    since: &str,
) -> PullRequestWatch {
    PullRequestWatch {
        number: pull_request.number,
        head_sha: pull_request.head_sha.clone(),
        failed_checks: failing(detail),
        passed: all_passed(detail),
        conflicting: detail.mergeability == Mergeability::Conflicting,
        remarks_through: since.to_owned(),
        remark_ids: Vec::new(),
        wakes: 0,
    }
}

/// Comments and reviews not told yet, except those by `viewer`, the account hoshi reads
/// the forge with, whose remarks the agent made itself. Also where the told ones end now.
fn remarks(
    detail: &PullRequestDetail,
    watch: &PullRequestWatch,
    viewer: Option<&str>,
) -> (Vec<Remark>, String, Vec<String>) {
    let through = time(&watch.remarks_through);
    let mut found = Vec::new();
    let mut take =
        |author: &Option<String>, path: Option<&str>, text: String, url: &str, stamp: &str| {
            let (Some(author), Some(through), Some(at)) = (author, through, time(stamp)) else {
                return;
            };
            if viewer.is_some_and(|viewer| viewer.eq_ignore_ascii_case(author)) {
                return;
            }
            let fresh =
                at > through || (at == through && !watch.remark_ids.iter().any(|id| id == url));
            if fresh {
                found.push(Remark {
                    author: author.clone(),
                    path: path.map(str::to_owned),
                    text,
                    url: url.to_owned(),
                    at,
                    stamp: stamp.to_owned(),
                });
            }
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
    let Some(latest) = found.iter().max_by_key(|remark| remark.at) else {
        return (
            found,
            watch.remarks_through.clone(),
            watch.remark_ids.clone(),
        );
    };
    let latest_at = latest.at;
    let (stamp, mut ids) = if through == Some(latest_at) {
        (watch.remarks_through.clone(), watch.remark_ids.clone())
    } else {
        (latest.stamp.clone(), Vec::new())
    };
    ids.extend(
        found
            .iter()
            .filter(|remark| remark.at == latest_at)
            .map(|remark| remark.url.clone()),
    );
    (found, stamp, ids)
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
    watch: &PullRequestWatch,
    pull_request: &PullRequest,
    detail: &PullRequestDetail,
    viewer: Option<&str>,
) -> Outcome {
    let number = pull_request.number;
    match pull_request.state {
        PullRequestState::Merged => {
            return Outcome {
                watch: watch.clone(),
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
                watch: watch.clone(),
                wake: Some(Wake { update, text }),
                stop: true,
            };
        }
        PullRequestState::Open | PullRequestState::Draft => {}
    }
    let head_sha = pull_request
        .head_sha
        .clone()
        .or_else(|| watch.head_sha.clone());
    let head_moved = head_sha != watch.head_sha;
    let sha: String = head_sha
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

    // A read with no checks may be a failed one, so it keeps what was told.
    let mut failed_checks = if head_moved {
        Vec::new()
    } else {
        watch.failed_checks.clone()
    };
    let mut passed = !head_moved && watch.passed;
    if !detail.checks.is_empty() {
        let failed: Vec<_> = detail
            .checks
            .iter()
            .filter(|check| check.state == ChecksState::Failing)
            .filter(|check| !failed_checks.contains(&check.name))
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
        }
        failed_checks = failing(detail);
        let passed_now = all_passed(detail);
        if passed_now && !passed {
            lines.push(format!(
                "- All {} checks passed on {sha}.",
                detail.checks.len()
            ));
            update.checks_passed = true;
        }
        passed = passed_now;
    }

    let (found, remarks_through, remark_ids) = remarks(detail, watch, viewer);
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

    // A conflict a push didn't resolve is news for the new commit.
    let told_conflict = !head_moved && watch.conflicting;
    if detail.mergeability == Mergeability::Conflicting && !told_conflict {
        lines.push(format!(
            "- The branch now conflicts with {}.",
            pull_request.base_branch
        ));
        update.conflicting = true;
    }
    // Unknown is a forge still computing after a push; only a clean answer clears a conflict.
    let conflicting = match detail.mergeability {
        Mergeability::Unknown => told_conflict,
        mergeability => mergeability == Mergeability::Conflicting,
    };

    let news = !lines.is_empty();
    let comments_only =
        news && update.failed_checks.is_empty() && !update.checks_passed && !update.conflicting;
    let progress = head_moved || (news && !comments_only);
    let wakes = if progress { 0 } else { watch.wakes } + u32::from(comments_only);
    let next = PullRequestWatch {
        number,
        head_sha,
        failed_checks,
        passed,
        conflicting,
        remarks_through,
        remark_ids,
        wakes,
    };
    if !news {
        return Outcome {
            watch: next,
            wake: None,
            stop: false,
        };
    }
    let stop = comments_only && wakes >= COMMENT_WAKE_LIMIT;
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
        watch: next,
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
        let Some(thread) = self
            .model
            .thread(thread_id)
            .filter(|thread| thread.watching_pull_request)
        else {
            return;
        };
        let watch = match self.watches.get(thread_id) {
            Some(watch) if watch.number == pull_request.number => watch.clone(),
            Some(_) => start(pull_request.number, None, &now()),
            None if pull_request.state.is_finished() => {
                self.stop_watching(thread_id);
                return;
            }
            // Comments since the thread last moved came while hoshi was closed.
            None => resumed(pull_request, detail, &thread.updated_at),
        };
        let outcome = evaluate(&watch, pull_request, detail, viewer.as_deref());
        if self.watches.get(thread_id) != Some(&outcome.watch) {
            self.save_watch(thread_id, outcome.watch);
        }
        if let Some(wake) = outcome.wake {
            self.deliver_wake(thread_id, wake);
        }
        if outcome.stop {
            self.stop_watching(thread_id);
        }
    }

    fn save_watch(&mut self, thread_id: &str, watch: PullRequestWatch) {
        if let Err(error) = self.store.save_pull_request_watch(thread_id, &watch) {
            eprintln!("hoshi: failed to save what a pull request watch told: {error}");
        }
        self.watches.insert(thread_id.to_owned(), watch);
    }

    /// Ends the watch; what it told stays, so watching again doesn't repeat it.
    pub(crate) fn stop_watching(&mut self, thread_id: &str) {
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
        let ongoing = self
            .watches
            .get(caller)
            .is_some_and(|watch| was_watching && watch.number == pull_request.number);
        if !ongoing {
            let started = start(pull_request.number, self.watches.get(caller), &now());
            self.save_watch(caller, started);
        }
        self.watch_reads.remove(caller);
        self.pull_requests.check_soon(caller);
        self.wake_sync();
        Ok(json!({
            "number": pull_request.number,
            "url": pull_request.url,
            "watching": true,
            "wasWatching": was_watching,
            "next": WATCH_NEXT,
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
            changes_requested: false,
            conflicts: false,
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

    fn comment(author: &str, at: &str, url: &str) -> TimelineItem {
        TimelineItem::Comment {
            author: Some(author.into()),
            body: "Please rename this.".into(),
            created_at: at.into(),
            url: url.into(),
        }
    }

    const START: &str = "2026-11-08T10:00:00Z";

    #[test]
    fn failures_and_a_conflict_already_there_wake_once_when_watching_starts() {
        let open = pull_request(PullRequestState::Open, "abc1234");
        let failing = detail(
            vec![check("lint", ChecksState::Failing)],
            Mergeability::Conflicting,
        );
        let watch = start(7, None, START);

        let first = evaluate(&watch, &open, &failing, Some("me"));
        let again = evaluate(&first.watch, &open, &failing, Some("me"));

        let wake = first.wake.unwrap();
        assert_eq!(wake.update.failed_checks, ["lint"]);
        assert!(wake.update.conflicting);
        assert_eq!(again.wake, None);
    }

    #[test]
    fn a_watch_from_before_hoshi_saved_it_only_tells_what_comes_after() {
        let open = pull_request(PullRequestState::Open, "abc1234");
        let failing = detail(
            vec![check("lint", ChecksState::Failing)],
            Mergeability::Conflicting,
        );
        let watch = resumed(&open, &failing, START);

        assert_eq!(evaluate(&watch, &open, &failing, None).wake, None);
    }

    #[test]
    fn watching_again_keeps_what_the_last_watch_told() {
        let open = pull_request(PullRequestState::Open, "abc1234");
        let failing = detail(
            vec![check("lint", ChecksState::Failing)],
            Mergeability::Mergeable,
        );
        let told = evaluate(&start(7, None, START), &open, &failing, None).watch;

        let again = start(7, Some(&told), "2026-11-08T11:00:00Z");
        let other = start(8, Some(&told), "2026-11-08T11:00:00Z");

        assert_eq!(evaluate(&again, &open, &failing, None).wake, None);
        assert_eq!(again.remarks_through, "2026-11-08T11:00:00Z");
        assert_eq!(other.head_sha, None);
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
        let watch = evaluate(&start(7, None, START), &open, &pending, None).watch;
        let failed = detail(
            vec![
                check("lint", ChecksState::Failing),
                check("test", ChecksState::Pending),
            ],
            Mergeability::Mergeable,
        );

        let first = evaluate(&watch, &open, &failed, None);
        let again = evaluate(&first.watch, &open, &failed, None);
        let unread = evaluate(
            &again.watch,
            &open,
            &detail(Vec::new(), Mergeability::Mergeable),
            None,
        );
        let pushed = pull_request(PullRequestState::Open, "def5678");
        let passed = detail(
            vec![
                check("lint", ChecksState::Passing),
                check("test", ChecksState::Passing),
            ],
            Mergeability::Mergeable,
        );
        let fixed = evaluate(&unread.watch, &pushed, &passed, None);

        let wake = first.wake.unwrap();
        assert_eq!(wake.update.failed_checks, ["lint"]);
        assert!(wake.text.contains("- Checks failed on abc1234:\n  - lint"));
        assert_eq!(again.wake, None);
        assert_eq!(unread.wake, None);
        assert_eq!(unread.watch.failed_checks, ["lint"]);
        let wake = fixed.wake.unwrap();
        assert!(wake.update.checks_passed);
        assert!(wake.text.contains("All 2 checks passed on def5678."));
    }

    #[test]
    fn a_check_that_fails_again_after_passing_wakes_again() {
        let open = pull_request(PullRequestState::Open, "abc1234");
        let failing = detail(
            vec![check("lint", ChecksState::Failing)],
            Mergeability::Mergeable,
        );
        let passing = detail(
            vec![check("lint", ChecksState::Passing)],
            Mergeability::Mergeable,
        );

        let failed = evaluate(&start(7, None, START), &open, &failing, None);
        let rerun = evaluate(&failed.watch, &open, &passing, None);
        let again = evaluate(&rerun.watch, &open, &failing, None);

        assert!(rerun.wake.unwrap().update.checks_passed);
        assert_eq!(again.wake.unwrap().update.failed_checks, ["lint"]);
    }

    #[test]
    fn new_comments_by_others_wake_and_a_conflict_is_told_once() {
        let open = pull_request(PullRequestState::Open, "abc1234");
        let quiet = detail(Vec::new(), Mergeability::Mergeable);
        let watch = evaluate(&start(7, None, START), &open, &quiet, None).watch;
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

        let first = evaluate(&watch, &open, &busy, Some("me"));
        let again = evaluate(&first.watch, &open, &busy, Some("me"));
        busy.mergeability = Mergeability::Unknown;
        let computing = evaluate(&again.watch, &open, &busy, Some("me"));

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
        assert!(computing.watch.conflicting);
    }

    #[test]
    fn a_comment_stamped_the_second_the_last_ones_were_told_still_wakes() {
        let open = pull_request(PullRequestState::Open, "abc1234");
        let mut read = detail(Vec::new(), Mergeability::Mergeable);
        read.timeline = vec![comment("reviewer", "2026-11-08T10:05:00Z", "c1")];
        let watch = evaluate(&start(7, None, START), &open, &read, None).watch;

        let first = evaluate(&watch, &open, &read, None);
        read.timeline
            .push(comment("other", "2026-11-08T10:05:00Z", "c2"));
        let late = evaluate(&first.watch, &open, &read, None);

        assert_eq!(watch.remarks_through, "2026-11-08T10:05:00Z");
        assert_eq!(first.wake, None);
        assert_eq!(late.wake.unwrap().update.comments, 1);
        assert_eq!(late.watch.remark_ids, ["c1", "c2"]);
    }

    #[test]
    fn comment_only_wakes_stop_the_watch_at_the_limit() {
        let open = pull_request(PullRequestState::Open, "abc1234");
        let mut watch = evaluate(
            &start(7, None, START),
            &open,
            &detail(Vec::new(), Mergeability::Mergeable),
            None,
        )
        .watch;
        let mut read = detail(Vec::new(), Mergeability::Mergeable);
        let mut last = None;
        for minute in 1..=COMMENT_WAKE_LIMIT {
            read.timeline.push(comment(
                "bot",
                &format!("2026-11-08T10:{minute:02}:00Z"),
                &format!("c{minute}"),
            ));
            let outcome = evaluate(&watch, &open, &read, None);
            watch = outcome.watch.clone();
            last = Some(outcome);
        }

        let last = last.unwrap();
        assert!(last.stop);
        assert!(last.wake.unwrap().update.stopped);
    }

    #[test]
    fn merged_stops_quietly_and_closed_says_so() {
        let quiet = detail(Vec::new(), Mergeability::Mergeable);
        let watch = start(7, None, START);

        let merged = evaluate(
            &watch,
            &pull_request(PullRequestState::Merged, "a"),
            &quiet,
            None,
        );
        let closed = evaluate(
            &watch,
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
