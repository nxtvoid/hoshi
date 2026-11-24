use super::*;

/// Decides the events a command produces. `now` is an RFC 3339 timestamp.
pub fn decide(model: &ReadModel, command: Command, now: &str) -> Result<Vec<Event>, CommandError> {
    match command {
        Command::ProjectCreate {
            project_id,
            title,
            workspace_root,
        } => {
            if model.projects.contains_key(&project_id) {
                return Err(CommandError::DuplicateProjectId { project_id });
            }
            if let Some(existing) = model
                .projects()
                .find(|project| same_path(&project.workspace_root, &workspace_root))
            {
                return Err(CommandError::DuplicateWorkspaceRoot {
                    project_id: existing.id.clone(),
                });
            }
            Ok(vec![Event::ProjectCreated {
                project: Project {
                    id: project_id,
                    title,
                    workspace_root,
                    created_at: now.to_owned(),
                    updated_at: now.to_owned(),
                    worktree_setup: None,
                    settings: ProjectSettings::default(),
                },
            }])
        }
        Command::ProjectWorktreeSetupSet { project_id, setup } => {
            if !model.projects.contains_key(&project_id) {
                return Err(CommandError::ProjectNotFound { project_id });
            }
            let setup = setup
                .map(|setup| {
                    Ok::<_, CommandError>(WorktreeSetup {
                        files: crate::setup_file_paths(&setup.files)?,
                        command: setup.command.trim().to_owned(),
                        wait: setup.wait,
                    })
                })
                .transpose()?
                .filter(|setup| !setup.files.is_empty() || !setup.command.is_empty());
            Ok(vec![Event::ProjectWorktreeSetupSet { project_id, setup }])
        }
        Command::ProjectSettingsSet {
            project_id,
            settings,
        } => {
            if !model.projects.contains_key(&project_id) {
                return Err(CommandError::ProjectNotFound { project_id });
            }
            Ok(vec![Event::ProjectSettingsSet {
                project_id,
                settings,
            }])
        }
        Command::ProjectRemove { project_id } => {
            if !model.projects.contains_key(&project_id) {
                return Err(CommandError::ProjectNotFound { project_id });
            }
            Ok(vec![Event::ProjectRemoved { project_id }])
        }
        Command::ThreadCreate {
            thread_id,
            project_id,
            title,
            workspace,
            provider,
            model: thread_model,
            effort,
            fast_mode,
            ultracode,
            access_mode,
            plan_mode,
        } => {
            if model.threads.contains_key(&thread_id) {
                return Err(CommandError::DuplicateThreadId { thread_id });
            }
            match &project_id {
                Some(id) if !model.projects.contains_key(id) => {
                    return Err(CommandError::ProjectNotFound {
                        project_id: id.clone(),
                    });
                }
                None if workspace != ThreadWorkspace::Local => {
                    return Err(CommandError::ChatNotLocal { thread_id });
                }
                _ => {}
            }
            let (worktree_path, branch) = match workspace {
                ThreadWorkspace::Local => (None, None),
                ThreadWorkspace::Worktree { path, branch } => (Some(path), Some(branch)),
                ThreadWorkspace::NewWorktree | ThreadWorkspace::BranchWorktree { .. } => {
                    return Err(CommandError::Internal {
                        detail: "the engine must create the worktree first".into(),
                    });
                }
            };
            Ok(vec![Event::ThreadCreated {
                thread: Box::new(Thread {
                    id: thread_id,
                    project_id,
                    title,
                    status: ThreadStatus::Idle,
                    branch,
                    worktree_path,
                    session_id: None,
                    conversation_cursor: None,
                    resume_at: None,
                    watching_pull_request: false,
                    provider,
                    model: thread_model,
                    effort,
                    fast_mode,
                    ultracode,
                    access_mode,
                    plan_mode,
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
                    created_at: now.to_owned(),
                    updated_at: now.to_owned(),
                    last_turn_model: None,
                }),
            }])
        }
        Command::ThreadDelete { thread_id } => {
            if !model.threads.contains_key(&thread_id) {
                return Err(CommandError::ThreadNotFound { thread_id });
            }
            Ok(vec![Event::ThreadDeleted { thread_id }])
        }
        Command::ThreadRewind {
            thread_id,
            message_id,
            rewind,
        } => {
            let Some(thread) = model.threads.get(&thread_id) else {
                return Err(CommandError::ThreadNotFound { thread_id });
            };
            if thread.active_turn_id.is_some() {
                return Err(CommandError::ThreadBusy { thread_id });
            }
            let resume_at = match rewind {
                Rewind::Start => None,
                Rewind::After { cursor, session } => {
                    let elsewhere = session.is_some_and(|session| {
                        thread
                            .provider_sessions
                            .iter()
                            .any(|parked| parked.session_id == session)
                    });
                    if elsewhere {
                        return Err(CommandError::CannotRewind {
                            thread_id,
                            message_id,
                        });
                    }
                    Some(cursor)
                }
            };
            let mut events = Vec::with_capacity(2);
            if resume_at.is_none() {
                let mut handoffs: Vec<PendingHandoff> = thread
                    .pending_handoffs
                    .iter()
                    .filter(|handoff| matches!(handoff, PendingHandoff::MergeBack { .. }))
                    .cloned()
                    .collect();
                if thread.forked_from.is_some() {
                    handoffs.insert(0, PendingHandoff::Fork { native: true });
                }
                if handoffs != thread.pending_handoffs {
                    events.push(Event::ContextHandoffsSet {
                        thread_id: thread_id.clone(),
                        handoffs,
                    });
                }
            }
            events.insert(
                0,
                Event::ThreadRewound {
                    thread_id,
                    message_id,
                    resume_at,
                    last_turn_model: None,
                },
            );
            Ok(events)
        }
        Command::ThreadFork {
            thread_id,
            fork_thread_id,
            turn,
        } => {
            let Some(source) = model.threads.get(&thread_id) else {
                return Err(CommandError::ThreadNotFound { thread_id });
            };
            if model.threads.contains_key(&fork_thread_id) {
                return Err(CommandError::DuplicateThreadId {
                    thread_id: fork_thread_id,
                });
            }
            if turn.status == TurnStatus::Running {
                return Err(CommandError::TurnNotForkable {
                    thread_id,
                    turn_id: turn.turn_id,
                });
            }
            Ok(vec![Event::ThreadCreated {
                thread: Box::new(forked(source, fork_thread_id, turn, now)),
            }])
        }
        Command::ThreadMergeBack { thread_id, turn } => {
            let Some(fork) = model.threads.get(&thread_id) else {
                return Err(CommandError::ThreadNotFound { thread_id });
            };
            let Some(parent) = fork
                .forked_from
                .as_ref()
                .and_then(|origin| model.threads.get(&origin.thread_id))
            else {
                return Err(CommandError::NotAFork { thread_id });
            };
            if turn.status != TurnStatus::Completed {
                return Err(CommandError::MergeBackTurnNotFinished {
                    thread_id,
                    turn_id: turn.turn_id,
                });
            }
            if parent.active_turn_id.is_some() {
                return Err(CommandError::MergeBackTargetBusy {
                    thread_id: parent.id.clone(),
                });
            }
            let other = parent
                .pending_handoffs
                .iter()
                .find_map(|handoff| match handoff {
                    PendingHandoff::MergeBack { fork_thread_id, .. }
                        if *fork_thread_id != thread_id =>
                    {
                        Some(fork_thread_id.clone())
                    }
                    _ => None,
                });
            if let Some(fork_thread_id) = other {
                return Err(CommandError::MergeBackPending {
                    thread_id: parent.id.clone(),
                    fork_thread_id,
                });
            }
            let mut handoffs: Vec<PendingHandoff> = parent
                .pending_handoffs
                .iter()
                .filter(|handoff| {
                    !matches!(handoff, PendingHandoff::MergeBack { fork_thread_id, .. }
                        if *fork_thread_id == thread_id)
                })
                .cloned()
                .collect();
            handoffs.push(PendingHandoff::MergeBack {
                fork_thread_id: thread_id,
                turn,
            });
            Ok(vec![Event::ContextHandoffsSet {
                thread_id: parent.id.clone(),
                handoffs,
            }])
        }
        Command::ThreadWorktreeRelease { thread_id } => {
            let Some(thread) = model.threads.get(&thread_id) else {
                return Err(CommandError::ThreadNotFound { thread_id });
            };
            if thread.active_turn_id.is_some() {
                return Err(CommandError::ThreadBusy { thread_id });
            }
            if thread.worktree_path.is_none() {
                return Err(CommandError::NothingToClean { thread_id });
            }
            if thread.worktree_adopted {
                return Err(CommandError::WorktreeAdopted { thread_id });
            }
            if thread.worktree_released {
                return Ok(Vec::new());
            }
            Ok(vec![Event::ThreadWorktreeReleased { thread_id }])
        }
        Command::ThreadCleanUp { thread_id } => {
            let Some(thread) = model.threads.get(&thread_id) else {
                return Err(CommandError::ThreadNotFound { thread_id });
            };
            if thread.active_turn_id.is_some() {
                return Err(CommandError::ThreadBusy { thread_id });
            }
            if thread.worktree_path.is_none() && thread.branch.is_none() {
                return Err(CommandError::NothingToClean { thread_id });
            }
            if thread.worktree_adopted {
                return Err(CommandError::WorktreeAdopted { thread_id });
            }
            let mut events = Vec::with_capacity(3);
            if thread.pinned {
                events.push(Event::ThreadPinnedSet {
                    thread_id: thread_id.clone(),
                    pinned: false,
                });
            }
            if !thread.archived {
                events.push(Event::ThreadArchivedSet {
                    thread_id: thread_id.clone(),
                    archived: true,
                });
            }
            events.extend(snooze::unsnoozed(thread, None));
            events.push(Event::ThreadCleanedUp { thread_id });
            Ok(events)
        }
        Command::ThreadArchive {
            thread_id,
            archived,
        } => {
            let Some(thread) = model.threads.get(&thread_id) else {
                return Err(CommandError::ThreadNotFound { thread_id });
            };
            let mut events = Vec::with_capacity(3);
            if archived && thread.pinned {
                events.push(Event::ThreadPinnedSet {
                    thread_id: thread_id.clone(),
                    pinned: false,
                });
            }
            if archived {
                events.extend(snooze::unsnoozed(thread, None));
            }
            if thread.archived != archived {
                events.push(Event::ThreadArchivedSet {
                    thread_id,
                    archived,
                });
            }
            Ok(events)
        }
        Command::ThreadPin { thread_id, pinned } => {
            let Some(thread) = model.threads.get(&thread_id) else {
                return Err(CommandError::ThreadNotFound { thread_id });
            };
            let mut events = Vec::with_capacity(3);
            if pinned && thread.archived {
                events.push(Event::ThreadArchivedSet {
                    thread_id: thread_id.clone(),
                    archived: false,
                });
            }
            if pinned {
                events.extend(snooze::unsnoozed(thread, Some(now)));
            }
            if thread.pinned != pinned {
                events.push(Event::ThreadPinnedSet { thread_id, pinned });
            }
            Ok(events)
        }
        Command::ThreadSnooze { thread_id, until } => snooze::snooze(model, thread_id, &until, now),
        Command::ThreadWake { thread_id } => {
            let Some(thread) = model.threads.get(&thread_id) else {
                return Err(CommandError::ThreadNotFound { thread_id });
            };
            Ok(snooze::unsnoozed(thread, Some(now)).into_iter().collect())
        }
        Command::ThreadVisit { thread_id } => {
            let Some(thread) = model.threads.get(&thread_id) else {
                return Err(CommandError::ThreadNotFound { thread_id });
            };
            Ok(snooze::visited(model, thread, now).into_iter().collect())
        }
        Command::ThreadWokeDismiss { thread_id } => {
            let Some(thread) = model.threads.get(&thread_id) else {
                return Err(CommandError::ThreadNotFound { thread_id });
            };
            Ok(snooze::woke_dismissed(model, thread, now)
                .into_iter()
                .collect())
        }
        Command::ThreadAccessSet {
            thread_id,
            access_mode,
            plan_mode,
        } => {
            if !model.threads.contains_key(&thread_id) {
                return Err(CommandError::ThreadNotFound { thread_id });
            }
            Ok(vec![Event::ThreadAccessSet {
                thread_id,
                access_mode,
                plan_mode,
            }])
        }
        Command::ThreadModelSet {
            thread_id,
            provider,
            model: thread_model,
            effort,
            fast_mode,
            ultracode,
        } => {
            let Some(thread) = model.threads.get(&thread_id) else {
                return Err(CommandError::ThreadNotFound { thread_id });
            };
            let has_conversation = thread.active_turn_id.is_some()
                || thread.session_id.is_some()
                || !thread.provider_sessions.is_empty();
            if thread.provider != provider && has_conversation {
                let choice = Some(ModelChoice {
                    provider,
                    model: thread_model,
                    effort,
                    fast_mode,
                    ultracode,
                });
                if thread.pending_model.as_deref() == choice.as_ref() {
                    return Ok(Vec::new());
                }
                return Ok(vec![Event::ThreadModelPending { thread_id, choice }]);
            }
            let mut events = Vec::with_capacity(2);
            if thread.pending_model.is_some() {
                events.push(Event::ThreadModelPending {
                    thread_id: thread_id.clone(),
                    choice: None,
                });
            }
            events.push(Event::ThreadModelSet {
                thread_id,
                provider,
                model: thread_model,
                effort,
                fast_mode,
                ultracode,
            });
            Ok(events)
        }
        Command::MessageSend {
            thread_id,
            message_id,
            text,
            attachments,
        } => message_events(
            model,
            thread_id,
            message_id,
            text,
            attachments,
            Origin::User,
            now,
        ),
        Command::TurnInterrupt { thread_id } => {
            let Some(thread) = model.threads.get(&thread_id) else {
                return Err(CommandError::ThreadNotFound { thread_id });
            };
            if thread.active_turn_id.is_none() {
                return Err(CommandError::ThreadNotWorking { thread_id });
            }
            Ok(Vec::new())
        }
        Command::SubagentStop { thread_id, .. } => {
            let Some(thread) = model.threads.get(&thread_id) else {
                return Err(CommandError::ThreadNotFound { thread_id });
            };
            if thread.active_turn_id.is_none() {
                return Err(CommandError::ThreadNotWorking { thread_id });
            }
            Ok(Vec::new())
        }
        Command::BackgroundTaskStop { thread_id, .. } => {
            if !model.threads.contains_key(&thread_id) {
                return Err(CommandError::ThreadNotFound { thread_id });
            }
            Ok(Vec::new())
        }
        Command::RequestRespond {
            thread_id,
            request_id,
            response,
        } => {
            let Some(thread) = model.threads.get(&thread_id) else {
                return Err(CommandError::ThreadNotFound { thread_id });
            };
            let Some(item) = model
                .requests
                .get(&request_id)
                .filter(|item| item.thread_id == thread_id)
            else {
                return Err(CommandError::RequestNotPending { request_id });
            };
            let Some(content) = resolved(&item.content, &response) else {
                return Err(CommandError::RequestResponseInvalid { request_id });
            };
            let implements_plan = matches!(
                (&item.content, &response),
                (ItemContent::ProposedPlan { .. }, RequestResponse::Decision { decision, .. })
                    if decision.accepts()
            );
            let wakes = matches!(
                response,
                RequestResponse::Answers { .. } | RequestResponse::Secret { saved: true }
            );
            let mut events = vec![Event::ItemAdded {
                item: ThreadItem {
                    id: request_id.clone(),
                    thread_id: thread_id.clone(),
                    created_at: now.to_owned(),
                    content,
                },
            }];
            if wakes {
                events.extend(snooze::unsnoozed(thread, None));
            }
            if implements_plan && thread.plan_mode {
                events.push(Event::ThreadAccessSet {
                    thread_id: thread_id.clone(),
                    access_mode: thread.access_mode,
                    plan_mode: false,
                });
            }
            events.extend(working_again(model, thread, &request_id));
            Ok(events)
        }
    }
}

/// The request with the user's response, or `None` when the response does not fit it.
pub(super) fn resolved(content: &ItemContent, response: &RequestResponse) -> Option<ItemContent> {
    let feedback = |decision: ApprovalDecision, feedback: &Option<String>| {
        let feedback = feedback.as_deref().map(str::trim).unwrap_or_default();
        (decision == ApprovalDecision::Decline && !feedback.is_empty()).then(|| feedback.to_owned())
    };
    match (content.clone(), response) {
        (
            ItemContent::Approval {
                tool_call_id,
                tool_name,
                input,
                reason,
                can_accept_for_session,
                ..
            },
            RequestResponse::Decision {
                decision,
                feedback: text,
            },
        ) => {
            if *decision == ApprovalDecision::AcceptForSession && !can_accept_for_session {
                return None;
            }
            Some(ItemContent::Approval {
                tool_call_id,
                tool_name,
                input,
                reason,
                can_accept_for_session,
                status: RequestStatus::Resolved,
                decision: Some(*decision),
                feedback: feedback(*decision, text),
            })
        }
        (
            ItemContent::ProposedPlan { plan, .. },
            RequestResponse::Decision {
                decision,
                feedback: text,
            },
        ) if *decision != ApprovalDecision::AcceptForSession => Some(ItemContent::ProposedPlan {
            plan,
            status: RequestStatus::Resolved,
            decision: Some(*decision),
            feedback: feedback(*decision, text),
        }),
        (
            ItemContent::SecretRequest {
                label,
                reason,
                placeholder,
                ..
            },
            RequestResponse::Secret { saved },
        ) => Some(ItemContent::SecretRequest {
            label,
            reason,
            placeholder,
            status: RequestStatus::Resolved,
            saved: Some(*saved),
        }),
        (ItemContent::Questions { questions, .. }, RequestResponse::Answers { answers })
            if answers.len() == questions.len() =>
        {
            Some(ItemContent::Questions {
                questions,
                status: RequestStatus::Resolved,
                answers: Some(answers.clone()),
            })
        }
        _ => None,
    }
}

pub(super) fn compaction_started(thread_id: &str, turn_id: &str, now: &str) -> Event {
    Event::ItemAdded {
        item: ThreadItem {
            id: format!("{turn_id}:compaction"),
            thread_id: thread_id.to_owned(),
            created_at: now.to_owned(),
            content: compaction(CompactionStatus::Running),
        },
    }
}

pub(super) fn compaction(status: CompactionStatus) -> ItemContent {
    ItemContent::Compaction {
        status,
        before_tokens: None,
        after_tokens: None,
        error: None,
    }
}

/// The request as it stands once nobody waits for it any more.
pub(super) fn cancelled(content: &ItemContent) -> ItemContent {
    let mut content = content.clone();
    match &mut content {
        ItemContent::Approval { status, .. }
        | ItemContent::Questions { status, .. }
        | ItemContent::ProposedPlan { status, .. }
        | ItemContent::SecretRequest { status, .. } => *status = RequestStatus::Cancelled,
        _ => {}
    }
    content
}

/// Puts the thread back to work once `closed` was its last pending request.
pub(super) fn working_again(model: &ReadModel, thread: &Thread, closed: &str) -> Option<Event> {
    let others = model
        .pending_requests(&thread.id)
        .iter()
        .any(|item| item.id != closed);
    (thread.status == ThreadStatus::WaitingForInput && !others).then(|| Event::ThreadStatusSet {
        thread_id: thread.id.clone(),
        status: ThreadStatus::Working,
    })
}

/// A message to the thread: it starts a turn, or joins the running one.
/// Who a message comes from: the user, or hoshi on the agent's behalf.
pub(super) enum Origin {
    User,
    /// The answer of the task child thread `0` worked on.
    TaskResult(String),
    PullRequest(PullRequestUpdate),
}

pub(super) fn message_events(
    model: &ReadModel,
    thread_id: String,
    message_id: String,
    text: String,
    attachments: Vec<MessageAttachment>,
    origin: Origin,
    now: &str,
) -> Result<Vec<Event>, CommandError> {
    let Some(thread) = model.threads.get(&thread_id) else {
        return Err(CommandError::ThreadNotFound { thread_id });
    };
    let text = text.trim().to_owned();
    let carried = attachments.iter().any(|attachment| {
        matches!(
            attachment,
            MessageAttachment::Image { .. } | MessageAttachment::File { .. }
        )
    });
    if text.is_empty() && !carried {
        return Err(CommandError::EmptyMessage);
    }
    let mut events = Vec::with_capacity(6);
    if thread.worktree_released {
        // The engine recreated the folder before this command reached here.
        events.push(Event::ThreadWorktreeRestored {
            thread_id: thread_id.clone(),
        });
    }
    if thread.archived {
        events.push(Event::ThreadArchivedSet {
            thread_id: thread_id.clone(),
            archived: false,
        });
    }
    if matches!(origin, Origin::User) {
        events.extend(snooze::unsnoozed(thread, None));
    }
    if thread.title.is_empty() {
        events.push(Event::ThreadTitleSet {
            thread_id: thread_id.clone(),
            title: title_from(&text, &attachments),
        });
    }
    let compacts = crate::is_compact_command(&text);
    let shown = matches!(origin, Origin::User) && !compacts;
    let switch = (thread.active_turn_id.is_none() && shown)
        .then(|| switch_provider(thread, &message_id))
        .flatten();
    let rewind = match switch {
        Some(_) => None,
        None => rewind_point(thread, !matches!(origin, Origin::User)),
    };
    // Only a turn the transcript opens with the user's message records what it runs on, so a
    // change shows over that message rather than over one hoshi sent unseen.
    let runs_on = (thread.active_turn_id.is_none() && shown).then(|| runs_on(thread));
    let model_switch = runs_on.as_ref().and_then(|to| turn_switch(thread, to));
    events.extend(switch);
    events.push(Event::ItemAdded {
        item: ThreadItem {
            id: message_id.clone(),
            thread_id: thread_id.clone(),
            created_at: now.to_owned(),
            content: ItemContent::UserMessage {
                rewind,
                text,
                attachments,
                steer: thread.active_turn_id.is_some(),
                task_result: match &origin {
                    Origin::TaskResult(child) => Some(child.clone()),
                    _ => None,
                },
                pull_request_update: match origin {
                    Origin::PullRequest(update) => Some(update),
                    _ => None,
                },
                resumed: None,
                dropped: false,
                model_switch,
            },
        },
    });
    // A running turn takes the message in; the engine passes it on.
    if thread.active_turn_id.is_none() {
        events.push(Event::TurnStarted {
            thread_id: thread_id.clone(),
            turn_id: message_id.clone(),
            at: now.to_owned(),
            model: runs_on,
        });
        if compacts {
            events.push(compaction_started(&thread_id, &message_id, now));
        }
    }
    Ok(events)
}

/// What a turn the user's message starts now runs on: the provider they picked for it, else the
/// thread's own.
fn runs_on(thread: &Thread) -> ModelChoice {
    thread
        .pending_model
        .as_deref()
        .filter(|to| to.provider != thread.provider)
        .cloned()
        .unwrap_or_else(|| ModelChoice::of(thread))
}

/// The change a turn starting now on `to` shows: none when it also moves the thread to
/// another provider, which shows as its handoff.
pub(super) fn turn_switch(thread: &Thread, to: &ModelChoice) -> Option<Box<ModelSwitch>> {
    if to.provider != thread.provider {
        return None;
    }
    model_switch(thread.last_turn_model.as_deref()?, to)
}

/// The change from what the last turn ran on to `to`, when it stays on the same provider and
/// changes its model or effort. A provider switch shows as its handoff instead.
pub(super) fn model_switch(from: &ModelChoice, to: &ModelChoice) -> Option<Box<ModelSwitch>> {
    (from.provider == to.provider && (from.model != to.model || from.effort != to.effort)).then(
        || {
            Box::new(ModelSwitch {
                from: from.clone(),
                to: to.clone(),
            })
        },
    )
}

/// Where a message that starts a turn lets the conversation come back to.
pub(super) fn rewind_point(thread: &Thread, task_result: bool) -> Option<Rewind> {
    if thread.active_turn_id.is_some() || task_result {
        return None;
    }
    match (&thread.session_id, &thread.conversation_cursor) {
        (None, _) => Some(Rewind::Start),
        (Some(session), Some(cursor)) => Some(Rewind::After {
            cursor: cursor.clone(),
            session: Some(session.clone()),
        }),
        (Some(_), None) => None,
    }
}

/// Moves the thread to the provider the user picked as message `message_id` starts a turn:
/// the current provider's session is kept for a switch back, the picked one's resumed if
/// the thread has one, and the next turn hands it what it did not see.
fn switch_provider(thread: &Thread, message_id: &str) -> Option<Event> {
    let to = thread
        .pending_model
        .as_deref()
        .filter(|to| to.provider != thread.provider)?
        .clone();
    let resumed = thread
        .provider_sessions
        .iter()
        .find(|parked| parked.provider == to.provider)
        .cloned();
    let mut provider_sessions: Vec<ProviderSession> = thread
        .provider_sessions
        .iter()
        .filter(|parked| parked.provider != to.provider && parked.provider != thread.provider)
        .cloned()
        .collect();
    if let Some(session_id) = &thread.session_id {
        provider_sessions.push(ProviderSession {
            provider: thread.provider.clone(),
            session_id: session_id.clone(),
            conversation_cursor: thread.conversation_cursor.clone(),
            resume_at: thread.resume_at.clone(),
            seen_before: message_id.to_owned(),
        });
    }
    let (strategy, since) = match &resumed {
        Some(parked) => (
            HandoffStrategy::DeltaSinceTargetLastSeen,
            Some(parked.seen_before.clone()),
        ),
        None => (HandoffStrategy::FullThreadSummary, None),
    };
    let mut pending_handoffs: Vec<PendingHandoff> = thread
        .pending_handoffs
        .iter()
        .filter(|handoff| !matches!(handoff, PendingHandoff::ProviderSwitch { .. }))
        .cloned()
        .collect();
    pending_handoffs.push(PendingHandoff::ProviderSwitch {
        from: ModelChoice::of(thread),
        to: to.clone(),
        strategy,
        since,
        before: message_id.to_owned(),
    });
    Some(Event::ProviderSwitched {
        thread_id: thread.id.clone(),
        to,
        session_id: resumed.as_ref().map(|parked| parked.session_id.clone()),
        conversation_cursor: resumed
            .as_ref()
            .and_then(|parked| parked.conversation_cursor.clone()),
        resume_at: resumed.and_then(|parked| parked.resume_at),
        provider_sessions,
        pending_handoffs,
    })
}

/// A new thread that goes on from `source`'s turn `turn`: in the same project, worktree and
/// branch, with its models and modes, and none of its place in the list.
pub(super) fn forked(source: &Thread, id: String, turn: TurnPoint, now: &str) -> Thread {
    Thread {
        id,
        project_id: source.project_id.clone(),
        title: format!("{} fork", source.title).trim().to_owned(),
        status: ThreadStatus::Idle,
        branch: source.branch.clone(),
        worktree_path: source.worktree_path.clone(),
        session_id: None,
        conversation_cursor: None,
        resume_at: None,
        watching_pull_request: false,
        provider: source.provider.clone(),
        model: source.model.clone(),
        effort: source.effort,
        fast_mode: source.fast_mode,
        ultracode: source.ultracode,
        access_mode: source.access_mode,
        plan_mode: source.plan_mode,
        active_turn_id: None,
        archived: false,
        pinned: false,
        worktree_released: source.worktree_released,
        worktree_adopted: source.worktree_adopted,
        parent_thread_id: None,
        task_cancelled: false,
        task_answered: false,
        forked_from: Some(Box::new(ForkOrigin {
            thread_id: source.id.clone(),
            turn,
        })),
        pending_model: None,
        provider_sessions: Vec::new(),
        pending_handoffs: vec![PendingHandoff::Fork { native: true }],
        snoozed_until: None,
        snoozed_at: None,
        last_snooze_wake_at: None,
        last_visited_at: None,
        last_turn_ended_at: None,
        created_at: now.to_owned(),
        updated_at: now.to_owned(),
        last_turn_model: None,
    }
}
