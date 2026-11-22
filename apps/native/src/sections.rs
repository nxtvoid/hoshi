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
mod tests;
