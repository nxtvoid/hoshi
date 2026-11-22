use super::*;

/// Takes the handoffs a turn delivered off what the thread's agent has yet to be told.
pub fn consume_handoffs(
    model: &ReadModel,
    thread_id: &str,
    delivered: &[PendingHandoff],
) -> Vec<Event> {
    let Some(thread) = model.threads.get(thread_id) else {
        return Vec::new();
    };
    let handoffs: Vec<PendingHandoff> = thread
        .pending_handoffs
        .iter()
        .filter(|handoff| !delivered.contains(handoff))
        .cloned()
        .collect();
    if handoffs == thread.pending_handoffs {
        return Vec::new();
    }
    vec![Event::ContextHandoffsSet {
        thread_id: thread_id.to_owned(),
        handoffs,
    }]
}

/// The fork's source session could not be forked: its next turn takes the source's history
/// as a transcript instead.
pub fn fork_without_native(model: &ReadModel, thread_id: &str) -> Vec<Event> {
    let Some(thread) = model.threads.get(thread_id) else {
        return Vec::new();
    };
    let handoffs: Vec<PendingHandoff> = thread
        .pending_handoffs
        .iter()
        .map(|handoff| match handoff {
            PendingHandoff::Fork { .. } => PendingHandoff::Fork { native: false },
            other => other.clone(),
        })
        .collect();
    if handoffs == thread.pending_handoffs {
        return Vec::new();
    }
    vec![Event::ContextHandoffsSet {
        thread_id: thread_id.to_owned(),
        handoffs,
    }]
}

/// Records the provider's last message of a turn, which later rewinds come back to.
pub fn record_conversation_cursor(
    model: &ReadModel,
    thread_id: &str,
    cursor: String,
) -> Vec<Event> {
    if model.thread(thread_id).is_none() {
        return Vec::new();
    }
    vec![Event::ConversationCursorSet {
        thread_id: thread_id.to_owned(),
        cursor,
    }]
}

/// Hands the answer of a task this thread delegated back to it, as a message from hoshi.
pub fn send_task_result(
    model: &ReadModel,
    thread_id: &str,
    child_thread_id: &str,
    message_id: &str,
    text: &str,
    now: &str,
) -> Result<Vec<Event>, CommandError> {
    message_events(
        model,
        thread_id.to_owned(),
        message_id.to_owned(),
        text.to_owned(),
        Vec::new(),
        Origin::TaskResult(child_thread_id.to_owned()),
        now,
    )
}

/// Tells a thread's agent what changed on the pull request it watches, as a message from
/// hoshi that starts its turn.
pub fn send_pull_request_update(
    model: &ReadModel,
    thread_id: &str,
    message_id: &str,
    text: &str,
    update: PullRequestUpdate,
    now: &str,
) -> Result<Vec<Event>, CommandError> {
    message_events(
        model,
        thread_id.to_owned(),
        message_id.to_owned(),
        text.to_owned(),
        Vec::new(),
        Origin::PullRequest(update),
        now,
    )
}

/// Starts or stops waking the thread's agent on its pull request's news. Delegated tasks
/// never watch: their parent owns the pull request.
pub fn record_pull_request_watch(model: &ReadModel, thread_id: &str, watching: bool) -> Vec<Event> {
    match model.threads.get(thread_id) {
        Some(thread)
            if thread.watching_pull_request != watching
                && (!watching || thread.parent_thread_id.is_none()) =>
        {
            vec![Event::PullRequestWatchSet {
                thread_id: thread_id.to_owned(),
                watching,
            }]
        }
        _ => Vec::new(),
    }
}

/// Marks queued messages the user removed as never read by the agent.
pub fn drop_queued_messages(
    model: &ReadModel,
    thread_id: &str,
    message_ids: Vec<String>,
) -> Vec<Event> {
    if message_ids.is_empty() || !model.threads.contains_key(thread_id) {
        return Vec::new();
    }
    vec![Event::MessagesDropped {
        thread_id: thread_id.to_owned(),
        message_ids,
    }]
}

/// The user's answers to `questions`, as a message to the agent: each question, then its
/// answer.
pub fn answers_message(questions: &ItemContent, response: &RequestResponse) -> Option<String> {
    let (ItemContent::Questions { questions, .. }, RequestResponse::Answers { answers }) =
        (questions, response)
    else {
        return None;
    };
    let replies: Vec<String> = questions
        .iter()
        .zip(answers)
        .filter(|(_, answer)| !answer.trim().is_empty())
        .map(|(question, answer)| format!("{}\n{}", question.question, answer.trim()))
        .collect();
    (!replies.is_empty()).then(|| replies.join("\n\n"))
}

/// Cancels the delegated task thread `thread_id` works on, once.
pub fn cancel_task(model: &ReadModel, thread_id: &str) -> Vec<Event> {
    match model.threads.get(thread_id) {
        Some(thread) if thread.parent_thread_id.is_some() && !thread.task_cancelled => {
            vec![Event::TaskCancelled {
                thread_id: thread_id.to_owned(),
            }]
        }
        _ => Vec::new(),
    }
}

/// Records that the delegated task thread `thread_id` works on answered, once.
pub fn answer_task(model: &ReadModel, thread_id: &str) -> Vec<Event> {
    match model.threads.get(thread_id) {
        Some(thread) if thread.parent_thread_id.is_some() && !thread.task_answered => {
            vec![Event::TaskAnswered {
                thread_id: thread_id.to_owned(),
            }]
        }
        _ => Vec::new(),
    }
}

/// Reopens the answered task thread `thread_id` works on, unless it was cancelled or the
/// thread that delegated it is gone or put away.
pub fn reopen_task(model: &ReadModel, thread_id: &str) -> Vec<Event> {
    match model.threads.get(thread_id) {
        Some(thread)
            if thread.task_answered
                && !thread.task_cancelled
                && thread
                    .parent_thread_id
                    .as_ref()
                    .and_then(|parent| model.threads.get(parent))
                    .is_some_and(|parent| !parent.archived) =>
        {
            vec![Event::TaskReopened {
                thread_id: thread_id.to_owned(),
            }]
        }
        _ => Vec::new(),
    }
}

pub fn record_delegation(model: &ReadModel, thread_id: &str, parent_thread_id: &str) -> Vec<Event> {
    let (Some(thread), Some(parent)) = (
        model.threads.get(thread_id),
        model.threads.get(parent_thread_id),
    ) else {
        return Vec::new();
    };
    vec![Event::ThreadDelegated {
        thread_id: thread_id.to_owned(),
        parent_thread_id: parent_thread_id.to_owned(),
        shares_worktree: thread.worktree_path.is_some()
            && thread.worktree_path == parent.worktree_path,
    }]
}

/// Records the branch a thread's folder is on, when it changed.
pub fn record_branch(model: &ReadModel, thread_id: &str, branch: Option<String>) -> Vec<Event> {
    match model.threads.get(thread_id) {
        // A released worktree has no folder to read; its branch is what brings it back.
        Some(thread) if !thread.worktree_released && thread.branch != branch => {
            vec![Event::ThreadBranchSet {
                thread_id: thread_id.to_owned(),
                branch,
            }]
        }
        _ => Vec::new(),
    }
}

/// Moves the thread into a worktree; `adopted` when it existed before, so it is not the
/// thread's to remove.
pub fn record_worktree(
    model: &ReadModel,
    thread_id: &str,
    worktree_path: String,
    branch: Option<String>,
    adopted: bool,
) -> Vec<Event> {
    match model.threads.get(thread_id) {
        Some(thread) if thread.worktree_path.as_deref() != Some(worktree_path.as_str()) => {
            vec![Event::ThreadWorktreeAttached {
                thread_id: thread_id.to_owned(),
                worktree_path,
                branch,
                adopted,
            }]
        }
        _ => Vec::new(),
    }
}

/// Moves the thread back to its project root, on the branch the root is on.
pub fn record_worktree_left(
    model: &ReadModel,
    thread_id: &str,
    branch: Option<String>,
) -> Vec<Event> {
    match model.threads.get(thread_id) {
        Some(thread) if thread.worktree_path.is_some() => vec![Event::ThreadWorktreeDetached {
            thread_id: thread_id.to_owned(),
            branch,
        }],
        _ => Vec::new(),
    }
}

/// The message's first line of prose, without its markdown, or the name of its
/// first image or file when it has none. Stands until a generated title replaces it.
pub(super) fn title_from(text: &str, attachments: &[MessageAttachment]) -> String {
    let mut in_code = false;
    let prose = text.lines().find_map(|line| {
        let line = line.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            in_code = !in_code;
            return None;
        }
        if in_code {
            return None;
        }
        let line = line.trim_start_matches(['#', '>', ' ']);
        let line = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
            .unwrap_or(line);
        let line = match line.split_once(". ") {
            Some((number, rest)) if number.chars().all(|char| char.is_ascii_digit()) => rest,
            _ => line,
        };
        let line = line.replace(['*', '`', '~'], "");
        let line = line.trim();
        (!line.is_empty() && line != "---").then(|| line.to_owned())
    });
    let attached = attachments.iter().find_map(|attachment| match attachment {
        MessageAttachment::Image { name, .. } | MessageAttachment::File { name, .. } => {
            Some(name.clone())
        }
        _ => None,
    });
    prose
        .or(attached)
        .unwrap_or_default()
        .chars()
        .take(TITLE_MAX_CHARS)
        .collect()
}

/// Names a thread with a generated title, unless its title changed since the
/// generation started from `previous`.
pub fn record_title(model: &ReadModel, thread_id: &str, previous: &str, title: &str) -> Vec<Event> {
    let title: String = title.trim().chars().take(TITLE_MAX_CHARS).collect();
    match model.threads.get(thread_id) {
        Some(thread) if thread.title == previous && !title.is_empty() && thread.title != title => {
            vec![Event::ThreadTitleSet {
                thread_id: thread_id.to_owned(),
                title,
            }]
        }
        _ => Vec::new(),
    }
}

/// Windows paths are case-insensitive and accept either separator.
pub(super) fn same_path(a: &str, b: &str) -> bool {
    let normalize = |path: &str| {
        let path = path.replace('\\', "/");
        let path = path.trim_end_matches('/');
        if cfg!(windows) {
            path.to_lowercase()
        } else {
            path.to_owned()
        }
    };
    normalize(a) == normalize(b)
}
