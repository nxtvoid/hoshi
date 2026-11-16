//! Which threads the sidebar lists and under which heading, as the web app groups them.

use chrono::{DateTime, Duration, Utc};
use hoshi_core::{
    ChecksState, ForgeKind, PullRequest, PullRequestState, ReadModel, ReviewTurn, Thread,
    ThreadStatus, active_since, is_snoozed, snooze_pending,
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SectionId {
    Pinned,
    Attention,
    Active,
    Review,
    /// Someone else's open pull requests, which the user reviews.
    Reviewing,
    Inactive,
    /// Snoozed until a time, soonest to wake first.
    Snoozed,
    Done,
    /// Inbox only: turns that finished while the user looked elsewhere.
    Completed,
}

const SECTION_ORDER: [SectionId; 8] = [
    SectionId::Pinned,
    SectionId::Attention,
    SectionId::Active,
    SectionId::Review,
    SectionId::Reviewing,
    SectionId::Inactive,
    SectionId::Snoozed,
    SectionId::Done,
];

const INBOX_SECTIONS: [SectionId; 5] = [
    SectionId::Attention,
    SectionId::Active,
    SectionId::Completed,
    SectionId::Review,
    SectionId::Reviewing,
];

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub id: SectionId,
    pub threads: Vec<Thread>,
}

/// One project's threads, the chats outside any project, or what needs the user everywhere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Project,
    Chats,
    Inbox,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Activity,
    Created,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Worktrees,
    PullRequests,
}

/// Who the user is signed in as on each forge; a forge left out is unknown.
pub type Logins = HashMap<ForgeKind, String>;
pub type PullRequests = HashMap<String, PullRequest>;

pub struct Query<'a> {
    pub view: View,
    pub project_id: Option<&'a str>,
    pub search: &'a str,
    pub filter: Filter,
    pub sort: Sort,
    pub inactive_after: Duration,
    pub logins: &'a Logins,
    /// What snoozed threads wake for.
    pub model: &'a ReadModel,
}

/// Idle threads untouched for this many days stop counting as active.
pub fn inactive_after(days: u32) -> Duration {
    Duration::days(if days > 0 { days.into() } else { 3 })
}

fn is_open(pull_request: &PullRequest) -> bool {
    matches!(
        pull_request.state,
        PullRequestState::Open | PullRequestState::Draft
    )
}

/// Whether the next move on an open pull request is its author's.
fn authors_turn(pull_request: &PullRequest) -> bool {
    if !is_open(pull_request) {
        return false;
    }
    let unanswered = match &pull_request.turn {
        None => pull_request.changes_requested,
        Some(turn) => matches!(turn, ReviewTurn::Author { .. }),
    };
    unanswered || pull_request.unresolved_comments > 0
}

/// Failing checks, conflicts or the author's turn wait on the user.
fn needs_attention(pull_request: &PullRequest) -> bool {
    is_open(pull_request)
        && (pull_request.checks == Some(ChecksState::Failing)
            || pull_request.conflicts
            || authors_turn(pull_request))
}

/// `false` while the user's login is unknown.
pub fn is_others(pull_request: &PullRequest, logins: &Logins) -> bool {
    logins.get(&pull_request.forge).is_some_and(|login| {
        pull_request
            .author
            .as_deref()
            .is_none_or(|author| !author.eq_ignore_ascii_case(login))
    })
}

fn age(at: &str, now: DateTime<Utc>) -> Duration {
    DateTime::parse_from_rfc3339(at)
        .map(|at| now - at.with_timezone(&Utc))
        .unwrap_or_default()
}

pub fn section_of(
    thread: &Thread,
    pull_request: Option<&PullRequest>,
    model: &ReadModel,
    now: DateTime<Utc>,
    inactive_after: Duration,
    logins: &Logins,
) -> SectionId {
    if thread.archived {
        return SectionId::Done;
    }
    if is_snoozed(model, thread, now) {
        return SectionId::Snoozed;
    }
    if thread.pinned {
        return SectionId::Pinned;
    }
    match thread.status {
        ThreadStatus::WaitingForInput | ThreadStatus::Error => return SectionId::Attention,
        ThreadStatus::Working => return SectionId::Active,
        ThreadStatus::Idle => {}
    }
    if let Some(pull_request) = pull_request {
        let theirs = is_others(pull_request, logins);
        if !theirs && needs_attention(pull_request) {
            return SectionId::Attention;
        }
        if pull_request.state.is_finished() {
            return SectionId::Done;
        }
        return if theirs {
            SectionId::Reviewing
        } else {
            SectionId::Review
        };
    }
    let idle = active_since(thread, now).map_or_else(Duration::zero, |since| now - since);
    if idle > inactive_after {
        SectionId::Inactive
    } else {
        SectionId::Active
    }
}

/// Splits sorted threads into sections, keeping their order; empty sections are left out.
fn group(
    threads: Vec<Thread>,
    pull_requests: &PullRequests,
    model: &ReadModel,
    now: DateTime<Utc>,
    inactive_after: Duration,
    logins: &Logins,
) -> Vec<Section> {
    let mut by_section: HashMap<SectionId, Vec<Thread>> = HashMap::new();
    for thread in threads {
        let id = section_of(
            &thread,
            pull_requests.get(&thread.id),
            model,
            now,
            inactive_after,
            logins,
        );
        by_section.entry(id).or_default().push(thread);
    }
    if let Some(snoozed) = by_section.get_mut(&SectionId::Snoozed) {
        snoozed.sort_by(|a, b| a.snoozed_until.cmp(&b.snoozed_until));
    }
    SECTION_ORDER
        .iter()
        .filter_map(|id| {
            by_section
                .remove(id)
                .map(|threads| Section { id: *id, threads })
        })
        .collect()
}

/// Every word of the search appears in the title, the branch or the pull request (`#12`).
pub fn matches_search(thread: &Thread, pull_request: Option<&PullRequest>, search: &str) -> bool {
    let words: Vec<String> = search.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return true;
    }
    let mut haystack = format!(
        "{} {}",
        thread.title,
        thread.branch.as_deref().unwrap_or_default()
    );
    if let Some(pull_request) = pull_request {
        haystack.push_str(&format!(" #{} {}", pull_request.number, pull_request.title));
    }
    let haystack = haystack.to_lowercase();
    words.iter().all(|word| haystack.contains(word.as_str()))
}

fn matches_filter(thread: &Thread, pull_request: Option<&PullRequest>, filter: Filter) -> bool {
    match filter {
        Filter::All => true,
        Filter::Worktrees => thread.worktree_path.is_some(),
        Filter::PullRequests => pull_request.is_some(),
    }
}

fn in_view(thread: &Thread, query: &Query) -> bool {
    match query.view {
        View::Inbox => true,
        View::Chats => thread.project_id.is_none(),
        View::Project => {
            thread.project_id.is_some() && thread.project_id.as_deref() == query.project_id
        }
    }
}

fn escalation(status: ThreadStatus) -> Option<u8> {
    match status {
        ThreadStatus::Working => Some(1),
        ThreadStatus::WaitingForInput => Some(2),
        _ => None,
    }
}

/// The threads the user started. A task an agent delegated isn't listed: its work, or its
/// question for the user, shows on the thread that delegated it.
pub fn with_delegated_work(threads: &[Thread]) -> Vec<Thread> {
    if threads
        .iter()
        .all(|thread| thread.parent_thread_id.is_none())
    {
        return threads.to_vec();
    }
    let by_id: HashMap<&str, &Thread> = threads
        .iter()
        .map(|thread| (thread.id.as_str(), thread))
        .collect();
    let delegated = |thread: &Thread| {
        thread
            .parent_thread_id
            .as_deref()
            .is_some_and(|parent| by_id.contains_key(parent))
    };
    let root = |thread: &Thread| -> String {
        let mut current = thread;
        for _ in 0..threads.len() {
            if !delegated(current) {
                break;
            }
            match current
                .parent_thread_id
                .as_deref()
                .and_then(|parent| by_id.get(parent))
            {
                Some(parent) => current = parent,
                None => break,
            }
        }
        current.id.clone()
    };
    let mut busiest: HashMap<String, ThreadStatus> = HashMap::new();
    for thread in threads {
        let Some(rank) = escalation(thread.status) else {
            continue;
        };
        if !delegated(thread) {
            continue;
        }
        let top = root(thread);
        let current = busiest.get(&top).copied();
        if current.is_none_or(|current| rank > escalation(current).unwrap_or(0)) {
            busiest.insert(top, thread.status);
        }
    }
    threads
        .iter()
        .filter(|thread| !delegated(thread))
        .map(|thread| {
            let own =
                escalation(thread.status).unwrap_or(if thread.status == ThreadStatus::Error {
                    3
                } else {
                    0
                });
            match busiest.get(&thread.id) {
                Some(status) if escalation(*status).unwrap_or(0) > own => Thread {
                    status: *status,
                    ..thread.clone()
                },
                _ => thread.clone(),
            }
        })
        .collect()
}

/// The sections the sidebar shows for a view, search and filter.
pub fn sidebar_sections(
    threads: &[Thread],
    pull_requests: &PullRequests,
    query: &Query,
    now: DateTime<Utc>,
    unseen: &HashSet<String>,
) -> Vec<Section> {
    let mut visible: Vec<Thread> = with_delegated_work(threads)
        .into_iter()
        .filter(|thread| {
            let pull_request = pull_requests.get(&thread.id);
            in_view(thread, query)
                && matches_search(thread, pull_request, query.search)
                && matches_filter(thread, pull_request, query.filter)
        })
        .collect();
    match query.sort {
        Sort::Activity => visible.sort_by(|a, b| b.updated_at.cmp(&a.updated_at)),
        Sort::Created => visible.sort_by(|a, b| b.created_at.cmp(&a.created_at)),
    }
    if query.view == View::Inbox {
        inbox_sections(
            visible,
            pull_requests,
            query.model,
            now,
            unseen,
            query.logins,
        )
    } else {
        group(
            visible,
            pull_requests,
            query.model,
            now,
            query.inactive_after,
            query.logins,
        )
    }
}

/// What waits on the user everywhere: needs attention, working now, finished but not seen,
/// in review, and others' pull requests. Pins don't apply here; idle and snoozed threads stay
/// out.
pub fn inbox_sections(
    threads: Vec<Thread>,
    pull_requests: &PullRequests,
    model: &ReadModel,
    now: DateTime<Utc>,
    unseen: &HashSet<String>,
    logins: &Logins,
) -> Vec<Section> {
    let (completed, rest): (Vec<Thread>, Vec<Thread>) = threads.into_iter().partition(|thread| {
        unseen.contains(&thread.id)
            && thread.status == ThreadStatus::Idle
            && !thread.archived
            && !is_snoozed(model, thread, now)
    });
    let original: HashMap<String, bool> = rest
        .iter()
        .map(|thread| (thread.id.clone(), thread.pinned))
        .collect();
    let unpinned = rest
        .into_iter()
        .map(|thread| Thread {
            pinned: false,
            ..thread
        })
        .collect();
    let mut sections: Vec<Section> = group(
        unpinned,
        pull_requests,
        model,
        now,
        inactive_after(0),
        logins,
    )
    .into_iter()
    .filter(|section| INBOX_SECTIONS.contains(&section.id))
    .filter_map(|section| {
        let threads: Vec<Thread> = section
            .threads
            .into_iter()
            .filter(|thread| {
                section.id != SectionId::Active || thread.status == ThreadStatus::Working
            })
            .map(|thread| Thread {
                pinned: original.get(&thread.id).copied().unwrap_or(false),
                ..thread
            })
            .collect();
        (!threads.is_empty()).then_some(Section {
            id: section.id,
            threads,
        })
    })
    .collect();
    if !completed.is_empty() {
        sections.push(Section {
            id: SectionId::Completed,
            threads: completed,
        });
    }
    INBOX_SECTIONS
        .iter()
        .flat_map(|id| {
            sections
                .iter()
                .filter(move |section| section.id == *id)
                .cloned()
        })
        .collect()
}

/// When the sidebar must sort its threads again: the soonest snooze to end, and at least each
/// minute while one is pending, so the time left stays current.
pub fn next_refresh<'a>(
    threads: impl IntoIterator<Item = &'a Thread>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    threads
        .into_iter()
        .filter(|thread| snooze_pending(thread, now))
        .filter_map(|thread| DateTime::parse_from_rfc3339(thread.snoozed_until.as_deref()?).ok())
        .map(|until| until.with_timezone(&Utc))
        .min()
        .map(|until| until.min(now + Duration::minutes(1)))
}

/// What the inbox badge counts: threads that need the user, and turns they haven't seen.
pub fn inbox_count(sections: &[Section]) -> usize {
    sections
        .iter()
        .filter(|section| matches!(section.id, SectionId::Attention | SectionId::Completed))
        .map(|section| section.threads.len())
        .sum()
}

/// The strongest thing a project's threads have to tell, for its rail button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Signal {
    Working,
    Done,
    Waiting,
    Error,
}

/// A failure first, then a question, then a finished turn not seen, then work going on.
/// `None` stands for the chats.
pub fn project_signal(
    threads: &[Thread],
    project_id: Option<&str>,
    unseen: &HashSet<String>,
) -> Option<Signal> {
    with_delegated_work(threads)
        .iter()
        .filter(|thread| thread.project_id.as_deref() == project_id && !thread.archived)
        .filter_map(|thread| match thread.status {
            ThreadStatus::Error => Some(Signal::Error),
            ThreadStatus::WaitingForInput => Some(Signal::Waiting),
            ThreadStatus::Working => Some(Signal::Working),
            ThreadStatus::Idle if unseen.contains(&thread.id) => Some(Signal::Done),
            ThreadStatus::Idle => None,
        })
        .max()
}

/// How long ago `at` was, as the sidebar shows it: "now", "5m ago", "yesterday".
pub fn relative_time(at: &str, now: DateTime<Utc>) -> String {
    use crate::m;
    let seconds = -age(at, now).num_seconds();
    const UNITS: [(char, i64); 6] = [
        ('y', 31_536_000),
        ('M', 2_592_000),
        ('w', 604_800),
        ('d', 86_400),
        ('h', 3_600),
        ('m', 60),
    ];
    let Some((unit, size)) = UNITS.iter().find(|(_, size)| seconds.abs() >= *size) else {
        return m::relative_now();
    };
    let count = (seconds / size).unsigned_abs();
    match (unit, count) {
        ('y', 1) => m::relative_last_year(),
        ('M', 1) => m::relative_last_month(),
        ('w', 1) => m::relative_last_week(),
        ('d', 1) => m::relative_yesterday(),
        ('y', _) => m::relative_years_ago(count),
        ('M', _) => m::relative_months_ago(count),
        ('w', _) => m::relative_weeks_ago(count),
        ('d', _) => m::relative_days_ago(count),
        ('h', _) => m::relative_hours_ago(count),
        _ => m::relative_minutes_ago(count),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoshi_core::AccessMode;

    const NOW: &str = "2026-05-10T12:00:00.000Z";

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(NOW)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn thread(id: &str, project: Option<&str>) -> Thread {
        Thread {
            id: id.into(),
            project_id: project.map(Into::into),
            title: id.into(),
            status: ThreadStatus::Idle,
            branch: None,
            worktree_path: None,
            session_id: None,
            conversation_cursor: None,
            resume_at: None,
            watching_pull_request: false,
            provider: hoshi_core::default_provider(),
            model: None,
            effort: None,
            fast_mode: false,
            ultracode: false,
            access_mode: AccessMode::default(),
            plan_mode: false,
            active_turn_id: None,
            archived: false,
            pinned: false,
            worktree_released: false,
            worktree_adopted: false,
            parent_thread_id: None,
            task_cancelled: false,
            task_answered: false,
            snoozed_until: None,
            snoozed_at: None,
            last_snooze_wake_at: None,
            last_visited_at: None,
            last_turn_ended_at: None,
            forked_from: None,
            pending_model: None,
            provider_sessions: Vec::new(),
            pending_handoffs: Vec::new(),
            created_at: NOW.into(),
            updated_at: NOW.into(),
        }
    }

    fn pull_request(state: PullRequestState) -> PullRequest {
        serde_json::from_value(serde_json::json!({
            "forge": "github",
            "number": 12,
            "title": "Add the rail",
            "url": "https://example.com/pr/12",
            "author": "nxtvoid",
            "headBranch": "rail",
            "baseBranch": "main",
            "state": state,
            "unresolvedComments": 0,
            "openTasks": 0,
            "changesRequested": false,
            "conflicts": false,
            "additions": 0,
            "deletions": 0,
            "updatedAt": NOW,
        }))
        .unwrap()
    }

    fn query<'a>(
        view: View,
        project: Option<&'a str>,
        logins: &'a Logins,
        model: &'a ReadModel,
    ) -> Query<'a> {
        Query {
            view,
            project_id: project,
            search: "",
            filter: Filter::All,
            sort: Sort::Activity,
            inactive_after: inactive_after(3),
            logins,
            model,
        }
    }

    fn snoozed(id: &str, until: &str) -> Thread {
        let mut thread = thread(id, Some("p"));
        thread.snoozed_until = Some(until.into());
        thread.snoozed_at = Some(NOW.into());
        thread
    }

    fn ids(sections: &[Section]) -> Vec<(SectionId, Vec<&str>)> {
        sections
            .iter()
            .map(|section| {
                (
                    section.id,
                    section
                        .threads
                        .iter()
                        .map(|thread| thread.id.as_str())
                        .collect(),
                )
            })
            .collect()
    }

    #[test]
    fn threads_fall_into_sections_in_order() {
        let mut waiting = thread("waiting", Some("p"));
        waiting.status = ThreadStatus::WaitingForInput;
        let mut pinned = thread("pinned", Some("p"));
        pinned.pinned = true;
        let mut old = thread("old", Some("p"));
        old.updated_at = "2026-05-01T12:00:00.000Z".into();
        let mut archived = thread("archived", Some("p"));
        archived.archived = true;
        let merged = thread("merged", Some("p"));
        let reviewed = thread("reviewed", Some("p"));
        let other = thread("other", Some("q"));
        let threads = [
            old,
            archived,
            merged,
            reviewed,
            other,
            waiting,
            pinned,
            thread("fresh", Some("p")),
        ];
        let pull_requests = PullRequests::from([
            ("merged".into(), pull_request(PullRequestState::Merged)),
            ("reviewed".into(), pull_request(PullRequestState::Open)),
        ]);
        let logins = Logins::new();

        let sections = sidebar_sections(
            &threads,
            &pull_requests,
            &query(View::Project, Some("p"), &logins, &ReadModel::default()),
            now(),
            &HashSet::new(),
        );

        assert_eq!(
            ids(&sections),
            [
                (SectionId::Pinned, vec!["pinned"]),
                (SectionId::Attention, vec!["waiting"]),
                (SectionId::Active, vec!["fresh"]),
                (SectionId::Review, vec!["reviewed"]),
                (SectionId::Inactive, vec!["old"]),
                (SectionId::Done, vec!["archived", "merged"]),
            ]
        );
    }

    #[test]
    fn snoozed_threads_wait_between_inactive_and_done_soonest_first() {
        let mut old = thread("old", Some("p"));
        old.updated_at = "2026-05-01T12:00:00.000Z".into();
        let mut archived = snoozed("archived", "2026-05-10T13:00:00.000Z");
        archived.archived = true;
        let mut pinned = snoozed("pinned", "2026-05-12T12:00:00.000Z");
        pinned.pinned = true;
        let threads = [
            pinned,
            snoozed("soon", "2026-05-10T13:00:00.000Z"),
            snoozed("passed", "2026-05-10T11:00:00.000Z"),
            old,
            archived,
        ];
        let logins = Logins::new();

        let sections = sidebar_sections(
            &threads,
            &PullRequests::new(),
            &query(View::Project, Some("p"), &logins, &ReadModel::default()),
            now(),
            &HashSet::new(),
        );

        assert_eq!(
            ids(&sections),
            [
                (SectionId::Active, vec!["passed"]),
                (SectionId::Inactive, vec!["old"]),
                (SectionId::Snoozed, vec!["soon", "pinned"]),
                (SectionId::Done, vec!["archived"]),
            ]
        );
    }

    #[test]
    fn a_long_snooze_does_not_leave_the_thread_inactive_once_it_wakes() {
        let mut woke = thread("woke", Some("p"));
        woke.updated_at = "2026-05-01T12:00:00.000Z".into();
        woke.snoozed_until = Some("2026-05-10T11:00:00.000Z".into());
        assert_eq!(
            section_of(
                &woke,
                None,
                &ReadModel::default(),
                now(),
                inactive_after(3),
                &Logins::new()
            ),
            SectionId::Active
        );
    }

    #[test]
    fn the_inbox_leaves_snoozed_threads_out() {
        let mut working = snoozed("working", "2026-05-10T13:00:00.000Z");
        working.status = ThreadStatus::Working;
        let finished = snoozed("finished", "2026-05-10T13:00:00.000Z");
        let logins = Logins::new();
        let unseen = HashSet::from(["finished".to_owned()]);

        let sections = sidebar_sections(
            &[working, finished],
            &PullRequests::new(),
            &query(View::Inbox, None, &logins, &ReadModel::default()),
            now(),
            &unseen,
        );

        assert!(sections.is_empty());
    }

    #[test]
    fn the_sidebar_refreshes_when_the_soonest_snooze_ends() {
        let threads = [
            snoozed("later", "2026-05-10T14:00:00.000Z"),
            snoozed("soon", "2026-05-10T12:00:30.000Z"),
            snoozed("passed", "2026-05-10T11:00:00.000Z"),
        ];
        assert_eq!(
            next_refresh(&threads, now()),
            Some(now() + Duration::seconds(30))
        );
        assert_eq!(
            next_refresh(&threads[..1], now()),
            Some(now() + Duration::minutes(1))
        );
        assert_eq!(next_refresh(&threads[2..], now()), None);
    }

    #[test]
    fn someone_elses_pull_request_is_one_to_review() {
        let logins = Logins::from([(ForgeKind::Github, "someone".to_owned())]);
        let open = pull_request(PullRequestState::Open);
        assert_eq!(
            section_of(
                &thread("t", Some("p")),
                Some(&open),
                &ReadModel::default(),
                now(),
                inactive_after(3),
                &logins
            ),
            SectionId::Reviewing
        );
        let mut failing = open.clone();
        failing.checks = Some(ChecksState::Failing);
        assert_eq!(
            section_of(
                &thread("t", Some("p")),
                Some(&failing),
                &ReadModel::default(),
                now(),
                inactive_after(3),
                &Logins::new()
            ),
            SectionId::Attention
        );
    }

    #[test]
    fn a_task_is_listed_through_the_thread_that_delegated_it() {
        let parent = thread("parent", Some("p"));
        let mut task = thread("task", Some("p"));
        task.parent_thread_id = Some("parent".into());
        task.status = ThreadStatus::WaitingForInput;
        let mut orphan = thread("orphan", Some("p"));
        orphan.parent_thread_id = Some("gone".into());

        let listed = with_delegated_work(&[parent, task, orphan]);

        assert_eq!(
            listed
                .iter()
                .map(|thread| (thread.id.as_str(), thread.status))
                .collect::<Vec<_>>(),
            [
                ("parent", ThreadStatus::WaitingForInput),
                ("orphan", ThreadStatus::Idle)
            ]
        );
    }

    #[test]
    fn search_needs_every_word_in_the_title_or_branch() {
        let mut found = thread("Fix the rail", Some("p"));
        found.branch = Some("feat/sidebar".into());
        assert!(matches_search(&found, None, "rail sidebar"));
        assert!(!matches_search(&found, None, "rail inbox"));
        let pr = pull_request(PullRequestState::Open);
        assert!(matches_search(&found, Some(&pr), "#12"));
    }

    #[test]
    fn the_inbox_holds_what_waits_and_what_finished_unseen() {
        let mut working = thread("working", Some("p"));
        working.status = ThreadStatus::Working;
        let mut error = thread("error", None);
        error.status = ThreadStatus::Error;
        let finished = thread("finished", Some("q"));
        let idle = thread("idle", Some("p"));
        let logins = Logins::new();
        let unseen = HashSet::from(["finished".to_owned()]);

        let sections = sidebar_sections(
            &[working, error, finished, idle],
            &PullRequests::new(),
            &query(View::Inbox, None, &logins, &ReadModel::default()),
            now(),
            &unseen,
        );

        assert_eq!(
            ids(&sections),
            [
                (SectionId::Attention, vec!["error"]),
                (SectionId::Active, vec!["working"]),
                (SectionId::Completed, vec!["finished"]),
            ]
        );
        assert_eq!(inbox_count(&sections), 2);
    }

    #[test]
    fn a_project_signals_its_strongest_news() {
        let mut working = thread("working", Some("p"));
        working.status = ThreadStatus::Working;
        let finished = thread("finished", Some("p"));
        let unseen = HashSet::from(["finished".to_owned()]);
        assert_eq!(
            project_signal(&[working.clone(), finished], Some("p"), &unseen),
            Some(Signal::Done)
        );
        assert_eq!(project_signal(&[working], None, &unseen), None);
    }

    #[test]
    fn times_read_like_the_web_sidebar() {
        let at = |seconds: i64| (now() - Duration::seconds(seconds)).to_rfc3339();
        assert_eq!(relative_time(&at(20), now()), "now");
        assert_eq!(relative_time(&at(300), now()), "5m ago");
        assert_eq!(relative_time(&at(3 * 3_600), now()), "3h ago");
        assert_eq!(relative_time(&at(86_400), now()), "yesterday");
        assert_eq!(relative_time(&at(3 * 86_400), now()), "3d ago");
    }
}
