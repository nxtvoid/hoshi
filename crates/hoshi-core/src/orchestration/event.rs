use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Event {
    ProjectCreated {
        project: Project,
    },
    ProjectRemoved {
        project_id: String,
    },
    ProjectWorktreeSetupSet {
        project_id: String,
        setup: Option<WorktreeSetup>,
    },
    ProjectSettingsSet {
        project_id: String,
        settings: ProjectSettings,
    },
    ThreadCreated {
        thread: Box<Thread>,
    },
    ThreadDeleted {
        thread_id: String,
    },
    ThreadTitleSet {
        thread_id: String,
        title: String,
    },
    ThreadBranchSet {
        thread_id: String,
        branch: Option<String>,
    },
    /// The thread moved to a worktree it did not start in, such as one its agent created.
    ThreadWorktreeAttached {
        thread_id: String,
        worktree_path: String,
        branch: Option<String>,
        /// The worktree existed before, so it is not the thread's to remove.
        #[serde(default)]
        adopted: bool,
    },
    /// The agent left its worktree for the project root; its session goes on there.
    ThreadWorktreeDetached {
        thread_id: String,
        branch: Option<String>,
    },
    ThreadArchivedSet {
        thread_id: String,
        archived: bool,
    },
    ThreadWorktreeReleased {
        thread_id: String,
    },
    /// The released worktree was created again, for a new message.
    ThreadWorktreeRestored {
        thread_id: String,
    },
    /// Its worktree and branch are gone; it runs in the project root from now on, with a new
    /// session, since the old one belonged to the worktree.
    ThreadCleanedUp {
        thread_id: String,
    },
    ThreadPinnedSet {
        thread_id: String,
        pinned: bool,
    },
    ThreadSnoozed {
        thread_id: String,
        /// RFC 3339, in UTC.
        until: String,
        at: String,
    },
    /// The snooze ended before the thread opened. `woke_at` is set when the user woke it.
    ThreadUnsnoozed {
        thread_id: String,
        woke_at: Option<String>,
    },
    ThreadVisited {
        thread_id: String,
        at: String,
    },
    ThreadModelSet {
        thread_id: String,
        #[serde(default = "crate::default_provider")]
        provider: String,
        model: Option<String>,
        effort: Option<Effort>,
        fast_mode: bool,
        ultracode: bool,
    },
    ThreadAccessSet {
        thread_id: String,
        access_mode: AccessMode,
        plan_mode: bool,
    },
    TurnStarted {
        thread_id: String,
        turn_id: String,
        at: String,
        /// What the turn runs on; `None` in events from before hoshi recorded it.
        #[serde(default)]
        model: Option<ModelChoice>,
    },
    SessionAttached {
        thread_id: String,
        session_id: String,
    },
    ItemAdded {
        item: ThreadItem,
    },
    ToolCallCompleted {
        thread_id: String,
        item_id: String,
        output: String,
        is_error: bool,
        #[serde(default)]
        detail: Option<ToolDetail>,
    },
    /// What a running call did so far replaces what it reported before.
    ToolCallDetailSet {
        thread_id: String,
        item_id: String,
        detail: ToolDetail,
    },
    TurnCompleted {
        thread_id: String,
        turn_id: String,
        outcome: TurnOutcome,
        at: String,
    },
    /// The turn ended before the call did.
    ToolCallInterrupted {
        thread_id: String,
        item_id: String,
    },
    /// The running turn's provider tries to reach the model again; `None` once it got through.
    TurnRetrying {
        thread_id: String,
        retry: Option<TurnRetry>,
    },
    /// Messages sent to the running turn, by id, that its agent never read.
    MessagesDropped {
        thread_id: String,
        message_ids: Vec<String>,
    },
    /// The thread works on a task another thread's agent delegated to it.
    ThreadDelegated {
        thread_id: String,
        parent_thread_id: String,
        /// It works in the parent's worktree, which stays the parent's to remove.
        shares_worktree: bool,
    },
    /// The user, or the agent that delegated it, cancelled the thread's task.
    TaskCancelled {
        thread_id: String,
    },
    /// The thread's task answered the agent that delegated it.
    TaskAnswered {
        thread_id: String,
    },
    /// The thread's task, already answered, works again: its agent woke for a command it
    /// left running, and its next answer goes back too.
    TaskReopened {
        thread_id: String,
    },
    /// Mid-turn: the agent waits for the user, or works again.
    ThreadStatusSet {
        thread_id: String,
        status: ThreadStatus,
    },
    /// The thread's agent asked to be woken on its pull request's news, or stopped.
    PullRequestWatchSet {
        thread_id: String,
        watching: bool,
    },
    /// A turn ended with `cursor` as the provider's last message of the conversation.
    ConversationCursorSet {
        thread_id: String,
        cursor: String,
    },
    /// The transcript lost `message_id` and everything after it. The next turn resumes
    /// the session after `resume_at`, or starts a new one when it is `None`, which also
    /// forgets the sessions of other providers.
    ThreadRewound {
        thread_id: String,
        message_id: String,
        resume_at: Option<String>,
        /// What the last turn left before the message ran on; `None` when no turn recorded it.
        #[serde(default)]
        last_turn_model: Option<ModelChoice>,
    },
    /// The user picked another provider, which the next message moves the thread to; `None`
    /// once that no longer stands.
    ThreadModelPending {
        thread_id: String,
        choice: Option<ModelChoice>,
    },
    /// The thread moved to `to`, with the session it keeps with that provider and the
    /// sessions it keeps with the others.
    ProviderSwitched {
        thread_id: String,
        to: ModelChoice,
        session_id: Option<String>,
        conversation_cursor: Option<String>,
        resume_at: Option<String>,
        provider_sessions: Vec<ProviderSession>,
        pending_handoffs: Vec<PendingHandoff>,
    },
    /// What the thread's agent has yet to be told at the start of its next turn.
    ContextHandoffsSet {
        thread_id: String,
        handoffs: Vec<PendingHandoff>,
    },
}

/// An event as delivered to the UI, with its position in the event log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventEnvelope {
    pub sequence: u64,
    pub event: Event,
}

/// Which item a [`StreamDelta`] is writing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StreamKind {
    Text,
    Reasoning,
    /// A running tool's output so far: each delta replaces the previous one.
    ToolOutput,
}

/// Text an assistant is still writing. Sent to the UI as it streams, never stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamDelta {
    pub thread_id: String,
    pub item_id: String,
    pub kind: StreamKind,
    pub delta: String,
}
