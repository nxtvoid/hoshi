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
        open_tasks: 0,
        changes_requested: false,
        turn: None,
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
