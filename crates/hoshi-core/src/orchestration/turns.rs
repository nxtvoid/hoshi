use super::*;

/// Closes a turn the app's close cut short. Its questions stay open: the agent that asked
/// is gone, so the user's answers go to the thread as a message.
pub fn close_stale_turn(
    model: &ReadModel,
    thread_id: &str,
    turn_id: &str,
    now: &str,
) -> Vec<Event> {
    let questions: Vec<&str> = model
        .pending_requests(thread_id)
        .into_iter()
        .filter(|item| matches!(item.content, ItemContent::Questions { .. }))
        .map(|item| item.id.as_str())
        .collect();
    let outcome = TurnOutcome::Interrupted;
    record_output(
        model,
        thread_id,
        turn_id,
        ProviderOutput::TurnCompleted { outcome },
        now,
    )
    .into_iter()
    .filter(|event| !matches!(event, Event::ItemAdded { item } if questions.contains(&item.id.as_str())))
    .map(|event| match event {
        Event::ItemAdded {
            item:
                item @ ThreadItem {
                    content: ItemContent::TurnInterrupted,
                    ..
                },
        } => Event::ItemAdded {
            item: ThreadItem {
                content: ItemContent::TurnCut,
                ..item
            },
        },
        event => event,
    })
    .collect()
}

/// Whether hoshi may go on with the thread's conversation by itself, as after a restart or
/// once a usage limit reset: nothing waits for the user but questions, which the user answers
/// by message.
pub fn may_continue_alone(model: &ReadModel, thread_id: &str) -> bool {
    let Some(thread) = model.threads.get(thread_id) else {
        return false;
    };
    let waits_for_user = model
        .pending_requests(thread_id)
        .iter()
        .any(|item| !matches!(item.content, ItemContent::Questions { .. }));
    !thread.archived && !thread.task_cancelled && thread.session_id.is_some() && !waits_for_user
}

/// Gives the thread back the session a turn tried to resume, should a turn that failed to
/// resume it have replaced it.
pub fn restore_session(model: &ReadModel, thread_id: &str, session_id: &str) -> Vec<Event> {
    match model.threads.get(thread_id) {
        Some(thread) if thread.session_id.as_deref() != Some(session_id) => {
            vec![Event::SessionAttached {
                thread_id: thread_id.to_owned(),
                session_id: session_id.to_owned(),
            }]
        }
        _ => Vec::new(),
    }
}

/// Turns provider output into events. Output from a turn that is no longer
/// active (interrupted, or the thread was deleted) is dropped.
pub fn record_output(
    model: &ReadModel,
    thread_id: &str,
    turn_id: &str,
    output: ProviderOutput,
    now: &str,
) -> Vec<Event> {
    let Some(thread) = model
        .threads
        .get(thread_id)
        .filter(|thread| thread.active_turn_id.as_deref() == Some(turn_id))
    else {
        return Vec::new();
    };
    let mut events = retry_settled(model, thread_id, turn_id, &output, now);
    events.extend(record_turn_output(model, thread, turn_id, output, now));
    events
}

/// Clears the running turn's retry once the run moves on: any output that comes from the
/// model, or the turn's end. When the model answered, the turn keeps a note that the
/// provider recovered.
pub fn retry_settled(
    model: &ReadModel,
    thread_id: &str,
    turn_id: &str,
    output: &ProviderOutput,
    now: &str,
) -> Vec<Event> {
    let active = model
        .threads
        .get(thread_id)
        .is_some_and(|thread| thread.active_turn_id.as_deref() == Some(turn_id));
    let moves_on = !matches!(
        output,
        ProviderOutput::Retrying { .. }
            | ProviderOutput::SessionStarted { .. }
            | ProviderOutput::WorkingDirectory { .. }
            | ProviderOutput::UsageLimits(_)
            | ProviderOutput::BackgroundTasks { .. }
            | ProviderOutput::ConversationCursor { .. }
            | ProviderOutput::ContextHandoffDelivered { .. }
            | ProviderOutput::MessagesDropped { .. }
            | ProviderOutput::ModelRerouted { .. }
            | ProviderOutput::McpServerFailed { .. }
            | ProviderOutput::HookStopped { .. }
            | ProviderOutput::TurnCompleted { .. }
    );
    let Some(retry) = model.retries.get(thread_id).filter(|_| active && moves_on) else {
        return Vec::new();
    };
    let mut events = vec![Event::TurnRetrying {
        thread_id: thread_id.to_owned(),
        retry: None,
    }];
    let answered = matches!(
        output,
        ProviderOutput::TextDelta { .. }
            | ProviderOutput::ReasoningDelta { .. }
            | ProviderOutput::Reasoning { .. }
            | ProviderOutput::AssistantMessage { .. }
            | ProviderOutput::ToolCallStarted { .. }
            | ProviderOutput::RequestOpened { .. }
            | ProviderOutput::TaskList { .. }
            | ProviderOutput::ContextUsage(_)
            | ProviderOutput::CompactionFinished { .. }
    );
    if answered {
        events.push(Event::ItemAdded {
            item: ThreadItem {
                id: format!("{turn_id}:recovered:{}", retry.at),
                thread_id: thread_id.to_owned(),
                created_at: now.to_owned(),
                content: ItemContent::ProviderRecovered {
                    attempt: retry.attempt,
                    max_attempts: retry.max_attempts,
                },
            },
        });
    }
    events
}

fn record_turn_output(
    model: &ReadModel,
    thread: &Thread,
    turn_id: &str,
    output: ProviderOutput,
    now: &str,
) -> Vec<Event> {
    let thread_id = thread.id.as_str();
    let item = |id: String, content: ItemContent| Event::ItemAdded {
        item: ThreadItem {
            id,
            thread_id: thread_id.to_owned(),
            created_at: now.to_owned(),
            content,
        },
    };
    match output {
        ProviderOutput::SessionStarted { session_id } => vec![Event::SessionAttached {
            thread_id: thread_id.to_owned(),
            session_id,
        }],
        ProviderOutput::RequestOpened {
            request_id,
            content,
        } => {
            if content.request_status() != Some(RequestStatus::Pending)
                || model.requests.contains_key(&request_id)
            {
                return Vec::new();
            }
            let mut events = vec![item(request_id, content)];
            if thread.status != ThreadStatus::WaitingForInput {
                events.push(Event::ThreadStatusSet {
                    thread_id: thread_id.to_owned(),
                    status: ThreadStatus::WaitingForInput,
                });
            }
            events
        }
        ProviderOutput::RequestWithdrawn { request_id } => {
            let Some(pending) = model
                .requests
                .get(&request_id)
                .filter(|item| item.thread_id == thread_id)
            else {
                return Vec::new();
            };
            let mut events = vec![item(request_id.clone(), cancelled(&pending.content))];
            events.extend(working_again(model, thread, &request_id));
            events
        }
        ProviderOutput::TextDelta { .. }
        | ProviderOutput::ReasoningDelta { .. }
        | ProviderOutput::ToolOutput { .. }
        | ProviderOutput::ContextUsage(_)
        | ProviderOutput::UsageLimits(_)
        | ProviderOutput::WorkingDirectory { .. }
        | ProviderOutput::BackgroundTasks { .. }
        | ProviderOutput::ConversationCursor { .. }
        | ProviderOutput::ContextHandoffDelivered { .. }
        | ProviderOutput::Woke
        | ProviderOutput::ProcessEnded => Vec::new(),
        ProviderOutput::Retrying {
            attempt,
            max_attempts,
            retry_in_ms,
            error,
        } => vec![Event::TurnRetrying {
            thread_id: thread_id.to_owned(),
            retry: Some(TurnRetry {
                attempt,
                max_attempts,
                retry_in_ms,
                error,
                at: now.to_owned(),
            }),
        }],
        ProviderOutput::MessagesDropped { message_ids } if !message_ids.is_empty() => {
            vec![Event::MessagesDropped {
                thread_id: thread_id.to_owned(),
                message_ids,
            }]
        }
        ProviderOutput::MessagesDropped { .. } => Vec::new(),
        ProviderOutput::ModelRerouted { model, reason } => vec![item(
            format!("{turn_id}:rerouted"),
            ItemContent::ModelRerouted { model, reason },
        )],
        ProviderOutput::McpServerFailed { name, .. }
            if model
                .mcp_failures
                .get(thread_id)
                .is_some_and(|names| names.contains(&name)) =>
        {
            Vec::new()
        }
        ProviderOutput::McpServerFailed { name, error } => vec![item(
            format!("{turn_id}:mcp:{name}"),
            ItemContent::McpServerFailed { name, error },
        )],
        ProviderOutput::HookStopped {
            event,
            source,
            reason,
        } => vec![item(
            format!("{turn_id}:hook-stopped"),
            ItemContent::HookStopped {
                event,
                source,
                reason,
            },
        )],
        ProviderOutput::Subagent {
            tool_use_id,
            output,
        } => record_subagent(&subagent_thread_id(thread_id, &tool_use_id), *output, now),
        ProviderOutput::TaskList { tasks } => {
            vec![item(
                format!("{turn_id}:tasks"),
                ItemContent::TaskList { tasks },
            )]
        }
        ProviderOutput::CompactionStarted => vec![item(
            format!("{turn_id}:compaction"),
            compaction(CompactionStatus::Running),
        )],
        ProviderOutput::CompactionFinished {
            before_tokens,
            after_tokens,
        } => vec![item(
            format!("{turn_id}:compaction"),
            ItemContent::Compaction {
                status: CompactionStatus::Done,
                before_tokens: Some(before_tokens),
                after_tokens: Some(after_tokens),
                error: None,
            },
        )],
        ProviderOutput::CompactionFailed { error } => vec![item(
            format!("{turn_id}:compaction"),
            ItemContent::Compaction {
                status: CompactionStatus::Failed,
                before_tokens: None,
                after_tokens: None,
                error: Some(error),
            },
        )],
        ProviderOutput::RunEnded { run } => {
            vec![item(format!("{turn_id}:run{run}"), ItemContent::RunEnded)]
        }
        ProviderOutput::BackgroundTaskEnded {
            task_id,
            tool_use_id,
            failed,
        } => vec![item(
            format!("{turn_id}:ended:{task_id}"),
            ItemContent::BackgroundTaskEnded {
                tool_call_id: tool_use_id,
                failed,
            },
        )],
        ProviderOutput::AssistantMessage { item_id, text } => {
            vec![item(item_id, ItemContent::AssistantMessage { text })]
        }
        ProviderOutput::Reasoning { item_id, text } => {
            vec![item(item_id, ItemContent::Reasoning { text })]
        }
        ProviderOutput::ToolCallStarted {
            item_id,
            name,
            input,
        } => vec![item(
            item_id,
            ItemContent::ToolCall {
                name,
                input,
                output: None,
                is_error: false,
                detail: None,
                output_omitted: false,
                interrupted: false,
            },
        )],
        ProviderOutput::ToolCallCompleted {
            item_id,
            output,
            is_error,
            detail,
        } => vec![Event::ToolCallCompleted {
            thread_id: thread_id.to_owned(),
            item_id,
            output,
            is_error,
            detail,
        }],
        ProviderOutput::ToolCallDetail { item_id, detail } => vec![Event::ToolCallDetailSet {
            thread_id: thread_id.to_owned(),
            item_id,
            detail,
        }],
        ProviderOutput::ToolCallStopped { item_id } => vec![Event::ToolCallInterrupted {
            thread_id: thread_id.to_owned(),
            item_id,
        }],
        ProviderOutput::TurnCompleted { outcome } => {
            let mut events: Vec<_> = model
                .pending_requests(thread_id)
                .into_iter()
                .map(|pending| item(pending.id.clone(), cancelled(&pending.content)))
                .collect();
            events.extend(
                model
                    .open_calls
                    .iter()
                    .filter(|call| root_thread(&call.thread_id) == thread_id)
                    .map(|call| Event::ToolCallInterrupted {
                        thread_id: call.thread_id.clone(),
                        item_id: call.item_id.clone(),
                    }),
            );
            if let Some(id) = model.compacting.get(thread_id) {
                events.push(item(id.clone(), compaction(CompactionStatus::Failed)));
            }
            match &outcome {
                TurnOutcome::Failed { reason } => events.push(item(
                    format!("{turn_id}:failed"),
                    ItemContent::TurnFailed {
                        reason: reason.clone(),
                    },
                )),
                TurnOutcome::Interrupted => events.push(item(
                    format!("{turn_id}:interrupted"),
                    ItemContent::TurnInterrupted,
                )),
                TurnOutcome::Completed => {}
            }
            events.push(Event::TurnCompleted {
                thread_id: thread_id.to_owned(),
                turn_id: turn_id.to_owned(),
                outcome,
                at: now.to_owned(),
            });
            events
        }
    }
}

/// Where a subagent's own transcript is kept: beside its thread's, under the id of the
/// call that started it. Subagents of subagents sit beside it too.
pub fn subagent_thread_id(thread_id: &str, tool_use_id: &str) -> String {
    format!("{}:{tool_use_id}", root_thread(thread_id))
}

/// The thread a transcript belongs to: itself, or the thread of a subagent's.
pub(super) fn root_thread(transcript: &str) -> &str {
    transcript.split(':').next().unwrap_or(transcript)
}

/// A subagent's messages, reasoning and tool calls, in its own transcript. It has no turn
/// or status of its own: the call that started it stands for it in the thread.
pub(super) fn record_subagent(transcript: &str, output: ProviderOutput, now: &str) -> Vec<Event> {
    let item = |id: String, content: ItemContent| Event::ItemAdded {
        item: ThreadItem {
            id,
            thread_id: transcript.to_owned(),
            created_at: now.to_owned(),
            content,
        },
    };
    match output {
        ProviderOutput::AssistantMessage { item_id, text } => {
            vec![item(item_id, ItemContent::AssistantMessage { text })]
        }
        ProviderOutput::Reasoning { item_id, text } => {
            vec![item(item_id, ItemContent::Reasoning { text })]
        }
        ProviderOutput::ToolCallStarted {
            item_id,
            name,
            input,
        } => vec![item(
            item_id,
            ItemContent::ToolCall {
                name,
                input,
                output: None,
                is_error: false,
                detail: None,
                output_omitted: false,
                interrupted: false,
            },
        )],
        ProviderOutput::ToolCallCompleted {
            item_id,
            output,
            is_error,
            detail,
        } => vec![Event::ToolCallCompleted {
            thread_id: transcript.to_owned(),
            item_id,
            output,
            is_error,
            detail,
        }],
        ProviderOutput::ToolCallDetail { item_id, detail } => vec![Event::ToolCallDetailSet {
            thread_id: transcript.to_owned(),
            item_id,
            detail,
        }],
        ProviderOutput::ToolCallStopped { item_id } => vec![Event::ToolCallInterrupted {
            thread_id: transcript.to_owned(),
            item_id,
        }],
        _ => Vec::new(),
    }
}

/// Starts a turn for a message sent while the previous turn was ending, too late to join
/// it; `message` is its item, as kept. When the turn runs on another model or effort, a line
/// saying so goes where the turn starts, since the message stays where it was sent. Nothing
/// when the thread is gone or busy again.
pub fn start_queued_turn(
    model: &ReadModel,
    thread_id: &str,
    message_id: &str,
    message: Option<&ThreadItem>,
    now: &str,
) -> Vec<Event> {
    let compacts = matches!(
        message.map(|message| &message.content),
        Some(ItemContent::UserMessage { text, .. }) if crate::is_compact_command(text)
    );
    let shown = match message.map(|message| &message.content) {
        Some(ItemContent::UserMessage {
            task_result,
            pull_request_update,
            ..
        }) => task_result.is_none() && pull_request_update.is_none() && !compacts,
        Some(_) => false,
        None => true,
    };
    let mut events = queued_turn(model, thread_id, message_id, shown, now);
    let switch = model.threads.get(thread_id).and_then(|thread| {
        let to = shown.then(|| runs_on(thread))?;
        turn_switch(thread, &to)
    });
    if let (false, Some(switch)) = (events.is_empty(), switch) {
        let ModelSwitch { from, to } = *switch;
        let marked = Event::ItemAdded {
            item: ThreadItem {
                id: format!("{message_id}:model"),
                thread_id: thread_id.to_owned(),
                created_at: now.to_owned(),
                content: ItemContent::ModelSwitched { from, to },
            },
        };
        events.insert(events.len() - 1, marked);
    }
    if compacts && !events.is_empty() {
        events.push(compaction_started(thread_id, message_id, now));
    }
    events
}

/// A queued message's turn, which records what it runs on when `records` says the message
/// shows as the user's.
pub(super) fn queued_turn(
    model: &ReadModel,
    thread_id: &str,
    message_id: &str,
    records: bool,
    now: &str,
) -> Vec<Event> {
    let mut events = start_idle_turn(model, thread_id, message_id, records, now);
    if !events.is_empty()
        && let Some(switch) = model
            .threads
            .get(thread_id)
            .and_then(|thread| switch_provider(thread, message_id))
    {
        events.insert(0, switch);
    }
    events
}

/// Starts a turn hoshi sends by itself to go on with the thread, opening with `text` marked
/// as `resumed`. Nothing when the thread is gone or busy.
pub fn start_resumed_turn(
    model: &ReadModel,
    thread_id: &str,
    turn_id: &str,
    text: &str,
    resumed: Resumption,
    now: &str,
) -> Vec<Event> {
    let mut events = queued_turn(model, thread_id, turn_id, false, now);
    if events.is_empty() {
        return events;
    }
    let message = Event::ItemAdded {
        item: ThreadItem {
            id: turn_id.to_owned(),
            thread_id: thread_id.to_owned(),
            created_at: now.to_owned(),
            content: ItemContent::UserMessage {
                text: text.to_owned(),
                attachments: Vec::new(),
                steer: false,
                task_result: None,
                pull_request_update: None,
                resumed: Some(resumed),
                rewind: None,
                dropped: false,
                model_switch: None,
            },
        },
    };
    events.insert(events.len() - 1, message);
    events
}

/// Starts a turn the agent began by itself after its own turn completed, as a command it
/// left running in the background ended. Nothing when the thread is gone or busy.
pub fn start_woken_turn(
    model: &ReadModel,
    thread_id: &str,
    turn_id: &str,
    now: &str,
) -> Vec<Event> {
    start_idle_turn(model, thread_id, turn_id, false, now)
}

/// A turn of the idle thread; it records what it runs on when `records`. A woken turn does
/// not: it goes on in the process the last turn left, on what that one ran.
pub(super) fn start_idle_turn(
    model: &ReadModel,
    thread_id: &str,
    message_id: &str,
    records: bool,
    now: &str,
) -> Vec<Event> {
    match model.threads.get(thread_id) {
        Some(thread) if thread.active_turn_id.is_none() => vec![Event::TurnStarted {
            thread_id: thread_id.to_owned(),
            turn_id: message_id.to_owned(),
            at: now.to_owned(),
            model: records.then(|| runs_on(thread)),
        }],
        _ => Vec::new(),
    }
}

/// What a turn changed in its working tree, after it ended. Also replaces the item while
/// the files were still being read.
pub fn record_turn_changes(
    model: &ReadModel,
    thread_id: &str,
    turn_id: &str,
    files: Option<Vec<DiffFile>>,
    now: &str,
) -> Vec<Event> {
    if model.thread(thread_id).is_none() {
        return Vec::new();
    }
    vec![Event::ItemAdded {
        item: ThreadItem {
            id: format!("{turn_id}:changes"),
            thread_id: thread_id.to_owned(),
            created_at: now.to_owned(),
            content: ItemContent::TurnChanges { files },
        },
    }]
}

/// The failure recorded when a turn cannot start at all.
pub fn turn_failed(reason: TurnFailure) -> ProviderOutput {
    ProviderOutput::TurnCompleted {
        outcome: TurnOutcome::Failed { reason },
    }
}
