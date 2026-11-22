use super::*;
use crate::{AccessMode, Command, Question, RequestResponse, RequestStatus, Snapshot, decide};
use chrono::FixedOffset;

const NOW: &str = "2026-11-15T12:00:00.000Z";
const LATER: &str = "2026-11-15T15:00:00.000Z";

fn at(text: &str) -> DateTime<Utc> {
    instant(text).unwrap()
}

fn thread(id: &str) -> Thread {
    Thread {
        id: id.into(),
        project_id: Some("p".into()),
        title: "thread".into(),
        status: ThreadStatus::Idle,
        branch: None,
        worktree_path: None,
        session_id: Some("s".into()),
        conversation_cursor: None,
        resume_at: None,
        watching_pull_request: false,
        provider: crate::default_provider(),
        model: None,
        effort: None,
        fast_mode: false,
        ultracode: false,
        access_mode: AccessMode::AcceptEdits,
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
        created_at: "2026-11-15T09:00:00.000Z".into(),
        updated_at: "2026-11-15T09:00:00.000Z".into(),
        last_turn_model: None,
    }
}

fn snoozed(id: &str) -> Thread {
    Thread {
        snoozed_until: Some(LATER.into()),
        snoozed_at: Some(NOW.into()),
        ..thread(id)
    }
}

fn model(threads: Vec<Thread>, requests: Vec<ThreadItem>) -> ReadModel {
    ReadModel::from_snapshot(&Snapshot {
        threads,
        pending_requests: requests,
        ..Snapshot::default()
    })
}

fn approval(thread_id: &str, created_at: &str) -> ThreadItem {
    ThreadItem {
        id: format!("approval-{thread_id}"),
        thread_id: thread_id.into(),
        created_at: created_at.into(),
        content: ItemContent::Approval {
            tool_call_id: None,
            tool_name: "Bash".into(),
            input: "{}".into(),
            reason: None,
            can_accept_for_session: false,
            status: RequestStatus::Pending,
            decision: None,
            feedback: None,
        },
    }
}

fn question(thread_id: &str, created_at: &str) -> ThreadItem {
    ThreadItem {
        id: format!("question-{created_at}"),
        thread_id: thread_id.into(),
        created_at: created_at.into(),
        content: ItemContent::Questions {
            questions: vec![Question {
                question: "Which one?".into(),
                header: "Pick".into(),
                options: Vec::new(),
                multi_select: false,
            }],
            status: RequestStatus::Pending,
            answers: None,
        },
    }
}

fn snooze_command(until: &str) -> Command {
    Command::ThreadSnooze {
        thread_id: "t".into(),
        until: until.into(),
    }
}

#[test]
fn a_snooze_holds_until_its_time_and_needs_no_timer() {
    let thread = snoozed("t");
    let model = model(vec![thread.clone()], Vec::new());

    assert!(is_snoozed(&model, &thread, at("2026-11-15T14:59:59.999Z")));
    assert!(!is_snoozed(&model, &thread, at(LATER)));
    let awake = Thread {
        snoozed_until: None,
        ..thread.clone()
    };
    assert!(!is_snoozed(&model, &awake, at(NOW)));
    assert_eq!(woke_at(&model, &thread, at(NOW)), None);
    assert_eq!(woke_at(&model, &thread, at(LATER)), Some(at(LATER)));
}

#[test]
fn approvals_new_questions_and_new_failures_wake_it_early_but_a_finished_turn_does_not() {
    let now = at("2026-11-15T13:00:00.000Z");
    let finished = Thread {
        last_turn_ended_at: Some("2026-11-15T12:30:00.000Z".into()),
        ..snoozed("t")
    };
    assert!(is_snoozed(
        &model(vec![finished.clone()], Vec::new()),
        &finished,
        now
    ));

    let asking = model(
        vec![snoozed("t")],
        vec![approval("t", "2026-11-15T11:00:00.000Z")],
    );
    assert!(raised_hand_while_snoozed(&asking, &snoozed("t")));
    assert!(!is_snoozed(&asking, &snoozed("t"), now));

    let old_question = model(
        vec![snoozed("t")],
        vec![question("t", "2026-11-15T11:00:00.000Z")],
    );
    assert!(is_snoozed(&old_question, &snoozed("t"), now));
    let new_question = model(
        vec![snoozed("t")],
        vec![question("t", "2026-11-15T12:10:00.000Z")],
    );
    assert!(!is_snoozed(&new_question, &snoozed("t"), now));
    assert_eq!(
        woke_at(&new_question, &snoozed("t"), now),
        Some(at("2026-11-15T12:10:00.000Z"))
    );

    let failed_before = Thread {
        status: ThreadStatus::Error,
        last_turn_ended_at: Some("2026-11-15T11:00:00.000Z".into()),
        ..snoozed("t")
    };
    assert!(is_snoozed(
        &model(vec![failed_before.clone()], Vec::new()),
        &failed_before,
        now
    ));
    let failed_after = Thread {
        last_turn_ended_at: Some("2026-11-15T12:20:00.000Z".into()),
        ..failed_before
    };
    let failing = model(vec![failed_after.clone()], Vec::new());
    assert!(!is_snoozed(&failing, &failed_after, now));
    assert_eq!(
        woke_at(&failing, &failed_after, now),
        Some(at("2026-11-15T12:20:00.000Z"))
    );
}

#[test]
fn a_request_of_a_subagent_or_a_delegated_task_wakes_it_too() {
    let now = at("2026-11-15T13:00:00.000Z");
    let subagent = model(
        vec![snoozed("t")],
        vec![approval("t:toolu_1", "2026-11-15T12:30:00.000Z")],
    );
    assert!(!is_snoozed(&subagent, &snoozed("t"), now));

    let task = Thread {
        parent_thread_id: Some("t".into()),
        ..thread("child")
    };
    let delegated = model(
        vec![snoozed("t"), task],
        vec![question("child", "2026-11-15T11:00:00.000Z")],
    );
    assert!(!is_snoozed(&delegated, &snoozed("t"), now));
}

#[test]
fn the_woke_indicator_shows_until_a_visit_or_a_later_turn_and_dismissing_hides_it() {
    let thread = snoozed("t");
    let mut model = model(vec![thread.clone()], Vec::new());
    let after = at("2026-11-15T16:00:00.000Z");
    assert!(!shows_woke(&model, &thread, at(NOW)));
    assert!(shows_woke(&model, &thread, after));

    let seen_turn = Thread {
        last_turn_ended_at: Some("2026-11-15T15:30:00.000Z".into()),
        ..thread.clone()
    };
    assert!(!shows_woke(&model, &seen_turn, after));
    let visited_before = Thread {
        last_visited_at: Some("2026-11-15T13:00:00.000Z".into()),
        ..thread.clone()
    };
    assert!(shows_woke(&model, &visited_before, after));

    let dismissed = decide(
        &model,
        Command::ThreadWokeDismiss {
            thread_id: "t".into(),
        },
        "2026-11-15T16:00:00.000Z",
    )
    .unwrap();
    assert_eq!(
        dismissed,
        vec![Event::ThreadVisited {
            thread_id: "t".into(),
            at: LATER.into(),
        }]
    );
    for event in &dismissed {
        model.apply(event);
    }
    assert!(!shows_woke(&model, model.thread("t").unwrap(), after));
    assert!(
        decide(
            &model,
            Command::ThreadWokeDismiss {
                thread_id: "t".into()
            },
            "2026-11-15T16:00:00.000Z",
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn a_visit_is_kept_only_when_it_hides_the_woke_indicator() {
    let visit = Command::ThreadVisit {
        thread_id: "t".into(),
    };
    let awake = model(vec![thread("t")], Vec::new());
    assert!(decide(&awake, visit.clone(), NOW).unwrap().is_empty());

    let mut woken = model(vec![snoozed("t")], Vec::new());
    assert!(decide(&woken, visit.clone(), NOW).unwrap().is_empty());
    let after = "2026-11-15T16:00:00.000Z";
    let events = decide(&woken, visit.clone(), after).unwrap();
    for event in &events {
        woken.apply(event);
    }
    let thread = woken.thread("t").unwrap();
    assert_eq!(thread.last_visited_at.as_deref(), Some(after));
    assert!(!shows_woke(&woken, thread, at(after)));
    assert!(decide(&woken, visit, after).unwrap().is_empty());
}

#[test]
fn snoozing_needs_a_future_time_and_nothing_waiting_on_the_user() {
    let model_with = |requests| model(vec![thread("t")], requests);

    let passed = decide(&model_with(Vec::new()), snooze_command(NOW), NOW);
    assert_eq!(
        passed,
        Err(CommandError::SnoozeTimePassed {
            thread_id: "t".into()
        })
    );
    let garbage = decide(&model_with(Vec::new()), snooze_command("soon"), NOW);
    assert!(matches!(
        garbage,
        Err(CommandError::SnoozeTimePassed { .. })
    ));

    let archived = model(
        vec![Thread {
            archived: true,
            ..thread("t")
        }],
        Vec::new(),
    );
    assert_eq!(
        decide(&archived, snooze_command(LATER), NOW),
        Err(CommandError::ThreadArchived {
            thread_id: "t".into()
        })
    );

    let approval = model_with(vec![approval("t", NOW)]);
    assert_eq!(
        decide(&approval, snooze_command(LATER), NOW),
        Err(CommandError::ThreadWaitingForUser {
            thread_id: "t".into()
        })
    );
    let task = Thread {
        parent_thread_id: Some("t".into()),
        ..thread("child")
    };
    let delegated = model(vec![thread("t"), task], vec![question("child", NOW)]);
    assert!(matches!(
        decide(&delegated, snooze_command(LATER), NOW),
        Err(CommandError::ThreadWaitingForUser { .. })
    ));

    let question = model_with(vec![question("t", NOW)]);
    let events = decide(&question, snooze_command("2026-11-15T16:00:00+01:00"), NOW).unwrap();
    assert_eq!(
        events,
        vec![Event::ThreadSnoozed {
            thread_id: "t".into(),
            until: LATER.into(),
            at: NOW.into(),
        }]
    );
}

#[test]
fn a_running_turn_can_be_snoozed_and_snoozing_again_moves_the_wake() {
    let working = Thread {
        status: ThreadStatus::Working,
        active_turn_id: Some("m1".into()),
        ..snoozed("t")
    };
    let mut model = model(vec![working], Vec::new());
    let events = decide(&model, snooze_command("2026-11-16T09:00:00.000Z"), LATER).unwrap();
    for event in &events {
        model.apply(event);
    }
    let thread = model.thread("t").unwrap();
    assert_eq!(
        thread.snoozed_until.as_deref(),
        Some("2026-11-16T09:00:00.000Z")
    );
    assert_eq!(thread.snoozed_at.as_deref(), Some(LATER));
}

fn run(model: &mut ReadModel, command: Command, now: &str) -> Vec<Event> {
    let events = decide(model, command, now).unwrap();
    for event in &events {
        model.apply(event);
    }
    events
}

fn woke(events: &[Event]) -> Option<Option<String>> {
    events.iter().find_map(|event| match event {
        Event::ThreadUnsnoozed { woke_at, .. } => Some(woke_at.clone()),
        _ => None,
    })
}

#[test]
fn the_user_s_message_answer_secret_pin_archive_or_wake_end_the_snooze() {
    let message = Command::MessageSend {
        thread_id: "t".into(),
        message_id: "m".into(),
        text: "hi".into(),
        attachments: Vec::new(),
    };
    let mut sent = model(vec![snoozed("t")], Vec::new());
    assert_eq!(woke(&run(&mut sent, message, NOW)), Some(None));
    assert_eq!(sent.thread("t").unwrap().snoozed_until, None);

    let answer = Command::RequestRespond {
        thread_id: "t".into(),
        request_id: "question-2026-11-15T11:00:00.000Z".into(),
        response: RequestResponse::Answers {
            answers: vec!["A".into()],
        },
    };
    let mut answered = model(
        vec![snoozed("t")],
        vec![question("t", "2026-11-15T11:00:00.000Z")],
    );
    assert_eq!(woke(&run(&mut answered, answer, NOW)), Some(None));

    let secret = ThreadItem {
        id: "secret".into(),
        thread_id: "t".into(),
        created_at: NOW.into(),
        content: ItemContent::SecretRequest {
            label: "Token".into(),
            reason: "To push".into(),
            placeholder: None,
            status: RequestStatus::Pending,
            saved: None,
        },
    };
    for saved in [true, false] {
        let mut model = model(vec![snoozed("t")], vec![secret.clone()]);
        let events = run(
            &mut model,
            Command::RequestRespond {
                thread_id: "t".into(),
                request_id: "secret".into(),
                response: RequestResponse::Secret { saved },
            },
            NOW,
        );
        assert_eq!(woke(&events).is_some(), saved);
    }

    let mut pinned = model(vec![snoozed("t")], Vec::new());
    let pin = Command::ThreadPin {
        thread_id: "t".into(),
        pinned: true,
    };
    assert_eq!(woke(&run(&mut pinned, pin, NOW)), Some(Some(NOW.into())));
    assert_eq!(
        pinned.thread("t").unwrap().last_snooze_wake_at.as_deref(),
        Some(NOW)
    );

    let mut archived = model(vec![snoozed("t")], Vec::new());
    let archive = Command::ThreadArchive {
        thread_id: "t".into(),
        archived: true,
    };
    assert_eq!(woke(&run(&mut archived, archive, NOW)), Some(None));

    let mut woken = model(vec![snoozed("t")], Vec::new());
    let wake = Command::ThreadWake {
        thread_id: "t".into(),
    };
    assert_eq!(
        woke(&run(&mut woken, wake.clone(), NOW)),
        Some(Some(NOW.into()))
    );
    assert!(run(&mut woken, wake, NOW).is_empty());
}

#[test]
fn hoshi_s_own_messages_leave_the_snooze_alone() {
    let mut model = model(vec![snoozed("t"), thread("child")], Vec::new());
    let events = crate::send_task_result(&model, "t", "child", "m", "done", NOW).unwrap();
    assert_eq!(woke(&events), None);
    for event in &events {
        model.apply(event);
    }
    assert!(model.thread("t").unwrap().snoozed_until.is_some());
}

#[test]
fn a_usage_limit_snoozes_until_its_reset_unless_it_passed() {
    let model = model(vec![thread("t")], Vec::new());
    assert_eq!(
        snooze_until_reset(&model, "t", LATER, NOW),
        vec![Event::ThreadSnoozed {
            thread_id: "t".into(),
            until: LATER.into(),
            at: NOW.into(),
        }]
    );
    assert!(snooze_until_reset(&model, "t", NOW, LATER).is_empty());
}

#[test]
fn inactivity_counts_from_the_wake_once_it_passed() {
    let thread = Thread {
        updated_at: "2026-11-01T09:00:00.000Z".into(),
        ..snoozed("t")
    };
    assert_eq!(
        active_since(&thread, at(NOW)),
        Some(at("2026-11-01T09:00:00.000Z"))
    );
    assert_eq!(
        active_since(&thread, at("2026-11-16T00:00:00.000Z")),
        Some(at(LATER))
    );
}

fn local(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

#[test]
fn the_default_menu_lists_soonest_first_and_hides_a_passed_time_today() {
    // A Sunday, 10:00 at UTC+1.
    let now = local("2026-11-15T10:00:00+01:00");
    let choices = snooze_choices(&default_snooze_presets(), &now);
    let rows: Vec<(&str, SnoozeLabel, bool)> = choices
        .iter()
        .map(|choice| (choice.id.as_str(), choice.label, choice.shows_weekday))
        .collect();
    let hours = |count| {
        SnoozeLabel::In(SnoozeAmount {
            count,
            unit: SnoozeUnit::Hours,
        })
    };
    assert_eq!(
        rows,
        vec![
            ("after:60", hours(1), false),
            ("after:180", hours(3), false),
            ("at:today:18:00", SnoozeLabel::ThisEvening, false),
            ("at:tomorrow:09:00", SnoozeLabel::Tomorrow, false),
        ]
    );
    assert_eq!(choices[2].until, at("2026-11-15T17:00:00.000Z"));
    assert_eq!(choices[3].until, at("2026-11-16T08:00:00.000Z"));

    let evening = local("2026-11-16T19:00:00+01:00");
    let ids: Vec<String> = snooze_choices(&default_snooze_presets(), &evening)
        .into_iter()
        .map(|choice| choice.id)
        .collect();
    assert_eq!(
        ids,
        [
            "after:60",
            "after:180",
            "at:tomorrow:09:00",
            "at:monday:09:00"
        ]
    );
    let monday = snooze_choice(
        &SnoozePreset::At {
            day: SnoozeDay::Monday,
            time: "09:00".into(),
        },
        &evening,
    )
    .unwrap();
    assert_eq!(monday.label, SnoozeLabel::NextWeek);
    assert!(monday.shows_weekday);
    assert_eq!(monday.until, at("2026-11-23T08:00:00.000Z"));
}

#[test]
fn weekday_labels_follow_the_days_ahead() {
    let sunday = local("2026-11-15T10:00:00+00:00");
    let label = |day, now: &DateTime<FixedOffset>| {
        snooze_choice(
            &SnoozePreset::At {
                day,
                time: "09:00".into(),
            },
            now,
        )
        .unwrap()
        .label
    };
    assert_eq!(label(SnoozeDay::Monday, &sunday), SnoozeLabel::Tomorrow);
    assert_eq!(
        label(SnoozeDay::Friday, &sunday),
        SnoozeLabel::Weekday(Weekday::Fri)
    );
    assert_eq!(
        label(SnoozeDay::Sunday, &sunday),
        SnoozeLabel::NextWeekday(Weekday::Sun)
    );
    let morning = snooze_choice(
        &SnoozePreset::At {
            day: SnoozeDay::Today,
            time: "11:30".into(),
        },
        &sunday,
    )
    .unwrap();
    assert_eq!(morning.label, SnoozeLabel::ThisMorning);
    let late = snooze_choice(&SnoozePreset::After { minutes: 900 }, &sunday).unwrap();
    assert!(late.shows_weekday);
    assert_eq!(
        amount(90),
        SnoozeAmount {
            count: 90,
            unit: SnoozeUnit::Minutes
        }
    );
    assert_eq!(
        amount(2 * 24 * 60),
        SnoozeAmount {
            count: 2,
            unit: SnoozeUnit::Days
        }
    );
}

#[test]
fn the_menu_keeps_one_row_per_minute_and_at_most_eight_valid_presets() {
    let now = local("2026-11-15T17:00:00+00:00");
    let presets = vec![
        SnoozePreset::After { minutes: 60 },
        SnoozePreset::At {
            day: SnoozeDay::Today,
            time: "18:00".into(),
        },
        SnoozePreset::After { minutes: 0 },
        SnoozePreset::After {
            minutes: MAX_SNOOZE_MINUTES + 1,
        },
        SnoozePreset::At {
            day: SnoozeDay::Today,
            time: "25:00".into(),
        },
    ];
    let choices = snooze_choices(&presets, &now);
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].id, "after:60");

    let many: Vec<SnoozePreset> = (1..=10)
        .map(|minutes| SnoozePreset::After { minutes })
        .collect();
    assert_eq!(snooze_choices(&many, &now).len(), MAX_SNOOZE_PRESETS);
    assert!(snooze_choices(&[], &now).is_empty());
}

#[test]
fn a_picked_duration_starts_when_picked_and_a_passed_set_time_is_refused() {
    let shown_at = local("2026-11-15T17:58:00+00:00");
    let presets = default_snooze_presets();
    let shown = snooze_choices(&presets, &shown_at);
    let picked_at = local("2026-11-15T18:05:00+00:00");

    let hour = pick_snooze_choice(&shown, "after:60", &presets, &picked_at).unwrap();
    assert_eq!(hour.until, at("2026-11-15T19:05:00.000Z"));
    assert_eq!(
        pick_snooze_choice(&shown, "at:today:18:00", &presets, &picked_at),
        None
    );
    let tomorrow = pick_snooze_choice(&shown, "at:tomorrow:09:00", &presets, &picked_at);
    assert_eq!(tomorrow.unwrap().until, at("2026-11-16T09:00:00.000Z"));
}

#[test]
fn a_custom_snooze_takes_a_future_local_time_or_a_positive_amount() {
    let now = local("2026-11-15T10:00:00+01:00");
    let typed = |date: &str, time: &str| CustomSnooze::At {
        date: date.into(),
        time: time.into(),
    };
    assert_eq!(
        custom_snooze_until(&typed("2026-11-15", "12:30"), &now),
        Some(at("2026-11-15T11:30:00.000Z"))
    );
    for (date, time) in [
        ("2026-11-15", "09:59"),
        ("2026-02-30", "12:00"),
        ("2026-1-15", "12:00"),
        ("2026-11-15", "9:00"),
    ] {
        assert_eq!(custom_snooze_until(&typed(date, time), &now), None);
    }
    let duration = |amount: &str, unit| CustomSnooze::For {
        amount: amount.into(),
        unit,
    };
    assert_eq!(
        custom_snooze_until(&duration("1.5", SnoozeUnit::Hours), &now),
        Some(at("2026-11-15T10:30:00.000Z"))
    );
    for amount in ["0", "-2", "abc", "inf", "1e300"] {
        assert_eq!(
            custom_snooze_until(&duration(amount, SnoozeUnit::Days), &now),
            None
        );
    }
}

#[test]
fn time_left_rounds_up_to_the_largest_unit() {
    let now = at(NOW);
    assert_eq!(snooze_left(NOW, now), SnoozeLeft::Now);
    assert_eq!(
        snooze_left("2026-11-15T12:00:01.000Z", now),
        SnoozeLeft::Minutes(1)
    );
    assert_eq!(
        snooze_left("2026-11-15T12:45:00.000Z", now),
        SnoozeLeft::Minutes(45)
    );
    assert_eq!(
        snooze_left("2026-11-15T14:10:00.000Z", now),
        SnoozeLeft::Hours(3)
    );
    assert_eq!(
        snooze_left("2026-11-17T11:00:00.000Z", now),
        SnoozeLeft::Days(2)
    );
}

#[test]
fn time_left_that_rounds_up_to_a_whole_unit_reads_in_that_unit() {
    let now = at(NOW);
    assert_eq!(
        snooze_left("2026-11-15T12:59:59.500Z", now),
        SnoozeLeft::Hours(1)
    );
    assert_eq!(
        snooze_left("2026-11-16T11:59:59.500Z", now),
        SnoozeLeft::Days(1)
    );
}
