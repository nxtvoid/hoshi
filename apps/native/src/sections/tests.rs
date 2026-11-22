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
        last_turn_model: None,
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
