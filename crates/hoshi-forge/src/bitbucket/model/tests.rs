use super::*;

const PULL_REQUEST: &str = r#"{
        "id": 12, "title": "Fix login", "description": "Body", "state": "OPEN", "draft": false,
        "author": {"uuid": "{a}", "nickname": "ana", "display_name": "Ana"},
        "source": {"branch": {"name": "fix/login"}, "commit": {"hash": "abc123"}},
        "destination": {"branch": {"name": "main"}},
        "updated_on": "2026-11-07T10:00:00.000000+00:00",
        "links": {"html": {"href": "https://bitbucket.org/acme/web/pull-requests/12"}},
        "reviewers": [{"uuid": "{b}", "nickname": "bo"}, {"uuid": "{c}", "display_name": "Cy Bot"}],
        "participants": [
            {"user": {"nickname": "bo"}, "role": "REVIEWER", "approved": true, "state": "approved", "participated_on": "2026-11-07T09:00:00+00:00"},
            {"user": {"nickname": "dee"}, "role": "PARTICIPANT", "approved": false, "state": "changes_requested", "participated_on": "2026-11-07T09:30:00+00:00"},
            {"user": {"nickname": "eve"}, "role": "PARTICIPANT", "approved": false, "state": null, "participated_on": null}
        ]
    }"#;

fn raw() -> RawPullRequest {
    serde_json::from_str(PULL_REQUEST).unwrap()
}

fn comment(id: u64, parent: Option<u64>, inline: bool, created: &str) -> RawComment {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "content": {"raw": format!("comment {id}")},
        "user": {"nickname": "bo"},
        "created_on": created,
        "inline": inline.then(|| serde_json::json!({"path": "src/a.rs", "from": null, "to": 4})),
        "parent": parent.map(|id| serde_json::json!({"id": id})),
        "resolution": null
    }))
    .unwrap()
}

#[test]
fn maps_a_pull_request() {
    let pull_request = pull_request(&raw(), Summary::default());
    assert_eq!(pull_request.number, 12);
    assert_eq!(pull_request.author.as_deref(), Some("ana"));
    assert_eq!(pull_request.head_branch, "fix/login");
    assert_eq!(pull_request.base_branch, "main");
    assert_eq!(pull_request.state, PullRequestState::Open);
    assert_eq!(
        pull_request.url,
        "https://bitbucket.org/acme/web/pull-requests/12"
    );
}

#[test]
fn reads_every_state() {
    let mut raw = raw();
    for (state, draft, expected) in [
        ("OPEN", true, PullRequestState::Draft),
        ("MERGED", false, PullRequestState::Merged),
        ("DECLINED", false, PullRequestState::Closed),
        ("SUPERSEDED", false, PullRequestState::Closed),
    ] {
        raw.state = state.into();
        raw.draft = draft;
        assert_eq!(raw.state(), expected, "{state}");
    }
}

#[test]
fn reviewers_carry_their_votes() {
    let reviewers = reviewers(&raw());
    let pairs: Vec<_> = reviewers
        .iter()
        .map(|reviewer| (reviewer.name.as_str(), reviewer.state))
        .collect();
    assert_eq!(
        pairs,
        [
            ("bo", ReviewState::Approved),
            ("Cy Bot", ReviewState::Requested),
            ("dee", ReviewState::ChangesRequested),
        ]
    );
}

#[test]
fn the_author_answering_a_request_for_changes_waits_on_the_reviewer() {
    let raw = raw();
    let mut reply = comment(1, None, false, "2026-11-07T09:45:00+00:00");
    reply.user = raw.author.clone();
    let mut bot = comment(2, None, false, "2026-11-07T09:50:00+00:00");
    bot.user = Some(account("app", "CI", None));
    bot.user.as_mut().unwrap().kind = Some("app_user".into());
    let comments = [reply, bot];
    let summary = Summary {
        changes_requested: changes_requested(&raw),
        changes_requested_by: changes_requested_by(&raw),
        vote_actions: vote_actions(&raw, &[]),
        comment_actions: comment_actions(&comments),
        ..Summary::default()
    };

    assert_eq!(
        pull_request(&raw, summary.clone()).turn,
        Some(ReviewTurn::Reviewers {
            waiting_on: vec!["dee".into()],
            replied_at: "2026-11-07T09:45:00+00:00".into(),
        })
    );

    let mut early = comment(3, None, false, "2026-11-07T09:00:00+00:00");
    early.user = raw.author.clone();
    let answered_before = Summary {
        comment_actions: comment_actions(&[early]),
        ..summary
    };
    assert_eq!(
        pull_request(&raw, answered_before).turn,
        Some(ReviewTurn::Author {
            by: "dee".into(),
            ask: ReviewAsk::ChangesRequested,
            at: "2026-11-07T09:30:00+00:00".into(),
        })
    );
}

#[test]
fn a_vote_is_dated_by_the_activity_log_over_the_last_comment() {
    let raw = raw();
    let activity: Vec<RawActivity> = serde_json::from_str(
            r#"[
                {"comment": {"id": 9}},
                {"changes_requested": {"date": "2026-11-07T09:50:00+00:00", "user": {"nickname": "dee"}}},
                {"changes_requested": {"date": "2026-11-07T08:00:00+00:00", "user": {"nickname": "dee"}}}
            ]"#,
        )
        .unwrap();
    let mut reply = comment(1, None, false, "2026-11-07T09:45:00+00:00");
    reply.user = raw.author.clone();
    let summary = Summary {
        changes_requested_by: changes_requested_by(&raw),
        vote_actions: vote_actions(&raw, &activity),
        comment_actions: comment_actions(&[reply]),
        ..Summary::default()
    };

    assert_eq!(
        pull_request(&raw, summary).turn,
        Some(ReviewTurn::Author {
            by: "dee".into(),
            ask: ReviewAsk::ChangesRequested,
            at: "2026-11-07T09:50:00+00:00".into(),
        })
    );
}

#[test]
fn a_vote_for_changes_marks_the_pull_request() {
    let mut raw = raw();
    assert!(changes_requested(&raw));
    raw.participants
        .retain(|participant| participant.state.as_deref() != Some("changes_requested"));
    assert!(!changes_requested(&raw));
}

#[test]
fn later_reruns_of_a_check_win() {
    let statuses: Vec<RawStatus> = serde_json::from_str(
        r#"[
                {"key": "build", "name": "Build", "state": "FAILED", "url": null},
                {"key": "lint", "name": null, "state": "SUCCESSFUL", "url": null},
                {"key": "build", "name": "Build", "state": "INPROGRESS", "url": "https://ci/1"}
            ]"#,
    )
    .unwrap();
    let checks = checks(&statuses);
    assert_eq!(checks.len(), 2);
    assert_eq!(checks[0].state, ChecksState::Pending);
    assert_eq!(checks[1].name, "lint");
    assert_eq!(checks_state(&checks), Some(ChecksState::Pending));
    assert_eq!(checks_state(&[]), None);
}

#[test]
fn groups_inline_comments_into_threads() {
    let mut resolved = comment(5, None, true, "2026-11-07T08:00:00+00:00");
    resolved.resolution = Some(serde_json::json!({"type": "comment_resolution"}));
    let comments = vec![
        comment(1, None, true, "2026-11-07T07:00:00+00:00"),
        comment(3, Some(2), true, "2026-11-07T07:20:00+00:00"),
        comment(2, Some(1), true, "2026-11-07T07:10:00+00:00"),
        comment(4, None, false, "2026-11-07T07:30:00+00:00"),
        resolved,
    ];
    let threads = review_threads(&comments, "");
    assert_eq!(threads.len(), 2);
    assert_eq!(threads[0].id, "1");
    assert_eq!(threads[0].line, Some(4));
    let bodies: Vec<_> = threads[0]
        .comments
        .iter()
        .map(|c| c.body.as_str())
        .collect();
    assert_eq!(bodies, ["comment 1", "comment 2", "comment 3"]);
    assert!(threads[1].resolved);
    assert_eq!(unresolved_threads(&comments, Some("ana")), 1);
    assert_eq!(unresolved_threads(&comments, Some("Bo")), 0);
}

#[test]
fn the_timeline_has_general_comments_votes_and_commits_in_order() {
    let commits: Vec<RawCommit> = serde_json::from_str(
        r#"[{"hash": "abcdef123", "message": "Fix\n\nmore", "date": "2026-11-07T08:30:00+00:00",
                 "author": {"raw": "Ana <ana@example.com>"}}]"#,
    )
    .unwrap();
    let comments = vec![
        comment(1, None, true, "2026-11-07T07:00:00+00:00"),
        comment(4, None, false, "2026-11-07T10:30:00+00:00"),
    ];
    let items = timeline(&raw(), &comments, &commits);
    let kinds: Vec<_> = items
        .iter()
        .map(|item| match item {
            TimelineItem::Comment { .. } => "comment",
            TimelineItem::Review { .. } => "review",
            TimelineItem::Commit { .. } => "commit",
        })
        .collect();
    assert_eq!(kinds, ["commit", "review", "review", "comment"]);
    let TimelineItem::Commit {
        sha,
        message,
        author,
        ..
    } = &items[0]
    else {
        panic!("expected a commit");
    };
    assert_eq!((sha.as_str(), message.as_str()), ("abcdef1", "Fix"));
    assert_eq!(author.as_deref(), Some("Ana"));
}

#[test]
fn reads_the_diffstat() {
    let stats: Vec<RawDiffStat> = serde_json::from_str(
            r#"[
                {"status": "added", "lines_added": 3, "lines_removed": 0, "old": null, "new": {"path": "a.rs"}},
                {"status": "removed", "lines_added": 0, "lines_removed": 2, "old": {"path": "b.rs"}, "new": null},
                {"status": "renamed", "lines_added": 1, "lines_removed": 1, "old": {"path": "c.rs"}, "new": {"path": "d.rs"}}
            ]"#,
        )
        .unwrap();
    let files = diff_files(&stats);
    assert_eq!(files[0].status, DiffFileStatus::Added);
    assert_eq!(files[1].path, "b.rs");
    assert_eq!(files[1].status, DiffFileStatus::Deleted);
    assert_eq!(files[2].old_path.as_deref(), Some("c.rs"));
}

#[test]
fn mentions_read_as_the_names_bitbucket_renders() {
    let content: Content = serde_json::from_value(serde_json::json!({
            "raw": "@{70121:32bd} ready to review, cc @{1:unknown}",
            "html": "<p><span class=\"ap-mention\" data-atlassian-id=\"70121:32bd\">@Ana O&#39;Neil</span> ready to review, cc @{1:unknown}</p>"
        }))
        .unwrap();

    assert_eq!(
        content.readable(),
        "@Ana O'Neil ready to review, cc @{1:unknown}"
    );
}

#[test]
fn images_lose_the_attribute_list_bitbucket_adds() {
    let content: Content = serde_json::from_value(serde_json::json!({
        "raw": "error:
![](https://bitbucket.org/x/images/1-image.png){: data-layout='center' }
then `f(){:x}`"
    }))
    .unwrap();

    assert_eq!(
        content.readable(),
        "error:
![](https://bitbucket.org/x/images/1-image.png)
then `f(){:x}`"
    );
}

#[test]
fn the_description_names_its_mentions() {
    let mut pull_request = raw();
    pull_request.description = "for @{7:a}".into();
    pull_request.summary = Some(Content {
        raw: "for @{7:a}".into(),
        html: Some("<p>for <span data-atlassian-id=\"7:a\">@Bo</span></p>".into()),
    });

    assert_eq!(pull_request.body(), "for @Bo");
}

fn account(id: &str, display_name: &str, nickname: Option<&str>) -> User {
    User {
        uuid: None,
        account_id: Some(id.into()),
        nickname: nickname.map(Into::into),
        display_name: Some(display_name.into()),
        kind: None,
    }
}

#[test]
fn names_written_after_an_at_become_mentions_bitbucket_notifies() {
    let users = [
        account("1:a", "Gabriel Jara", Some("gjara")),
        account("2:b", "Gabriel", None),
    ];

    assert_eq!(
        encode_mentions("@Gabriel Jara - ready, cc @gjara, @gabriel.", &users),
        "@{1:a} - ready, cc @{1:a}, @{2:b}."
    );
}

#[test]
fn mentions_skip_code_emails_and_unknown_names() {
    let users = [account("1:a", "Gabriel Jara", None)];

    assert_eq!(
        encode_mentions(
            "`@Gabriel Jara` me@Gabriel Jara.com @Gabriel Jaramillo @Nobody",
            &users
        ),
        "`@Gabriel Jara` me@Gabriel Jara.com @Gabriel Jaramillo @Nobody"
    );
}
