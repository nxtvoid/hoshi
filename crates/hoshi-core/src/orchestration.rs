use crate::snooze;
use crate::{
    AccessMode, ApprovalDecision, CompactionStatus, DiffFile, Effort, ForkOrigin, HandoffStrategy,
    ItemContent, MessageAttachment, ModelChoice, ModelSwitch, OpenToolCall, PendingHandoff,
    Project, ProjectSettings, ProviderOutput, ProviderSession, PullRequestUpdate, RequestResponse,
    RequestStatus, Resumption, Rewind, Thread, ThreadBackgroundTasks, ThreadContextUsage,
    ThreadItem, ThreadPullRequest, ThreadStatus, ThreadWorkspace, ToolDetail, TurnFailure,
    TurnOutcome, TurnPoint, TurnRetry, TurnStatus, WorktreeSetup,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const TITLE_MAX_CHARS: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Command {
    ProjectCreate {
        project_id: String,
        title: String,
        workspace_root: String,
    },
    ProjectRemove {
        project_id: String,
    },
    /// `None`, or no files and a blank command, stops setting up the project's new worktrees.
    ProjectWorktreeSetupSet {
        project_id: String,
        setup: Option<WorktreeSetup>,
    },
    /// Replaces what the project sets for itself over the app's settings.
    ProjectSettingsSet {
        project_id: String,
        settings: ProjectSettings,
    },
    ThreadCreate {
        thread_id: String,
        /// `None` starts a chat, which always works locally in its own folder.
        project_id: Option<String>,
        title: String,
        workspace: ThreadWorkspace,
        #[serde(default = "crate::default_provider")]
        provider: String,
        model: Option<String>,
        effort: Option<Effort>,
        fast_mode: bool,
        ultracode: bool,
        access_mode: AccessMode,
        plan_mode: bool,
    },
    ThreadDelete {
        thread_id: String,
    },
    /// Removes the thread's worktree folder and keeps its branch. The engine removes it,
    /// after checking nothing would be lost, before this reaches `decide`.
    ThreadWorktreeRelease {
        thread_id: String,
    },
    /// Removes the thread's worktree and local branch once its pull request is merged, and
    /// archives it. Nothing on the forge is touched. The engine does the git work first.
    ThreadCleanUp {
        thread_id: String,
    },
    /// Archiving also unpins.
    ThreadArchive {
        thread_id: String,
        archived: bool,
    },
    ThreadPin {
        thread_id: String,
        pinned: bool,
    },
    /// Takes the thread out of its section until `until` (RFC 3339), unless it needs the user
    /// sooner. Snoozing again replaces the wake time.
    ThreadSnooze {
        thread_id: String,
        until: String,
    },
    /// Ends the thread's snooze now, passed or not.
    ThreadWake {
        thread_id: String,
    },
    /// The user opened the thread, which hides its Woke indicator.
    ThreadVisit {
        thread_id: String,
    },
    /// Hides the thread's Woke indicator, as a visit when it woke.
    ThreadWokeDismiss {
        thread_id: String,
    },
    /// Applies from the next turn. Another provider, once the thread has a session, waits
    /// for the next message, which hands it what it did not see; refused while a turn runs.
    ThreadModelSet {
        thread_id: String,
        #[serde(default = "crate::default_provider")]
        provider: String,
        model: Option<String>,
        effort: Option<Effort>,
        fast_mode: bool,
        ultracode: bool,
    },
    /// Applies from the next turn.
    ThreadAccessSet {
        thread_id: String,
        access_mode: AccessMode,
        plan_mode: bool,
    },
    /// Starts a turn, or joins the running one: the agent reads it at its next step.
    MessageSend {
        thread_id: String,
        message_id: String,
        text: String,
        #[serde(default)]
        attachments: Vec<MessageAttachment>,
    },
    TurnInterrupt {
        thread_id: String,
    },
    /// Stops one subagent of the running turn, which goes on without it. `tool_use_id` is
    /// the call that started the subagent.
    SubagentStop {
        thread_id: String,
        tool_use_id: String,
    },
    /// Stops one command the thread's agent left running in the background, during its turn
    /// or after it.
    BackgroundTaskStop {
        thread_id: String,
        task_id: String,
    },
    /// Answers what the agent asked mid-turn; the engine passes it on to the agent.
    RequestRespond {
        thread_id: String,
        request_id: String,
        response: RequestResponse,
    },
    /// Takes the conversation back to before `message_id`, which goes with all that came
    /// after it. The engine reads where to go back to from the message, and puts the files
    /// back first when asked.
    ThreadRewind {
        thread_id: String,
        message_id: String,
        rewind: Rewind,
    },
    /// Starts thread `fork_thread_id` from `thread_id`'s turn `turn`, which the engine reads
    /// from the transcript. It goes on from there on its first message.
    ThreadFork {
        thread_id: String,
        fork_thread_id: String,
        turn: TurnPoint,
    },
    /// Hands fork `thread_id`'s turns through `turn` to the thread it came from, on that
    /// thread's next turn. The engine reads `turn` from the transcript.
    ThreadMergeBack {
        thread_id: String,
        turn: TurnPoint,
    },
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CommandError {
    #[error("project {project_id} already exists")]
    DuplicateProjectId { project_id: String },
    #[error("workspace root is already project {project_id}")]
    DuplicateWorkspaceRoot { project_id: String },
    #[error("project {project_id} not found")]
    ProjectNotFound { project_id: String },
    #[error("thread {thread_id} already exists")]
    DuplicateThreadId { thread_id: String },
    #[error("thread {thread_id} not found")]
    ThreadNotFound { thread_id: String },
    /// Its session belongs to the provider it started on.
    #[error("thread {thread_id} already runs on another provider")]
    ProviderLocked { thread_id: String },
    #[error("thread {thread_id} is already running a turn")]
    ThreadBusy { thread_id: String },
    #[error("thread {thread_id} is not running a turn")]
    ThreadNotWorking { thread_id: String },
    /// It already went, was removed, or is news hoshi brought rather than the user's.
    #[error("message {message_id} is not queued for thread {thread_id}")]
    QueuedMessageNotFound {
        thread_id: String,
        message_id: String,
    },
    /// The message is a steer, a task's answer, or older than rewinds.
    #[error("thread {thread_id} can't go back to before message {message_id}")]
    CannotRewind {
        thread_id: String,
        message_id: String,
    },
    /// Nothing was staged, so there is nothing to commit.
    #[error("thread {thread_id} has nothing to commit")]
    NothingToCommit { thread_id: String },
    /// No commit message came back from the provider; the user can write one.
    #[error("no commit message could be written for thread {thread_id}")]
    CommitMessageUnavailable { thread_id: String },
    /// The remote has commits the branch lacks.
    #[error("the remote is ahead of the branch of thread {thread_id}")]
    PushBehind { thread_id: String },
    /// `origin` is not configured.
    #[error("thread {thread_id}'s repository has no remote")]
    NoRemote { thread_id: String },
    /// The browser couldn't start or do what was asked; `detail` is why, shown as is.
    #[error("the browser failed: {detail}")]
    BrowserFailed { detail: String },
    /// Neither a recorded base nor a default branch to open a pull request on.
    #[error("thread {thread_id}'s branch has no base to open a pull request on")]
    NoBaseBranch { thread_id: String },
    /// On a detached HEAD there is no branch to push or open a pull request from.
    #[error("thread {thread_id} is not on a branch")]
    DetachedHead { thread_id: String },
    /// Nothing recorded the files before that message's turn, as outside a git repository.
    #[error("thread {thread_id} kept no files from before message {message_id}")]
    NoCheckpoint {
        thread_id: String,
        message_id: String,
    },
    #[error("message is empty")]
    EmptyMessage,
    /// Answered already, withdrawn, or its turn is over.
    #[error("request {request_id} is not waiting for an answer")]
    RequestNotPending { request_id: String },
    /// The answer does not fit the request, such as answers to an approval.
    #[error("the response does not fit request {request_id}")]
    RequestResponseInvalid { request_id: String },
    #[error("{path} is not a git repository")]
    NotAGitRepository { path: String },
    #[error("could not create the worktree: {detail}")]
    WorktreeFailed { detail: String },
    #[error("folder {path} does not exist")]
    WorkspaceRootNotFound { path: String },
    /// The thread has no pull request, or its project no forge account.
    #[error("no pull request or forge for this thread")]
    ForgeUnavailable,
    #[error("not signed in to the forge")]
    ForgeUnauthorized,
    #[error("the forge is rate limiting requests")]
    ForgeRateLimited,
    /// The forge refused the change; `detail` is its own message, shown as is.
    #[error("the forge refused: {detail}")]
    ForgeRejected { detail: String },
    #[error("could not reach the forge: {detail}")]
    ForgeUnreachable { detail: String },
    /// The token lacks these scopes, named as the forge names them.
    #[error("the forge token lacks the scopes {scopes:?}")]
    ForgeScopesMissing { scopes: Vec<String> },
    /// A git command failed; `detail` is git's own message, shown as is.
    #[error("git failed: {detail}")]
    GitFailed { detail: String },
    #[error("thread {thread_id} has no worktree or branch to remove")]
    NothingToClean { thread_id: String },
    /// The thread works in a worktree it did not create, which stays as it is.
    #[error("the worktree of thread {thread_id} is not hoshi's to remove")]
    WorktreeAdopted { thread_id: String },
    /// Another thread works in the same worktree, such as a fork of it.
    #[error("the worktree of thread {thread_id} is also thread {other_thread_id}'s")]
    WorktreeShared {
        thread_id: String,
        other_thread_id: String,
    },
    /// Removing it would lose uncommitted or untracked files.
    #[error("the worktree of thread {thread_id} has uncommitted changes")]
    WorktreeDirty { thread_id: String },
    #[error("thread {thread_id} has terminals open in its worktree")]
    TerminalsOpen { thread_id: String },
    #[error("the pull request of thread {thread_id} is not merged")]
    PullRequestNotMerged { thread_id: String },
    /// The branch has commits no remote and no pull request holds.
    #[error("the branch of thread {thread_id} has commits that exist only here")]
    UnpushedCommits { thread_id: String },
    /// Someone locked the worktree with `git worktree lock`; `reason` is theirs, shown as is.
    #[error("the worktree of thread {thread_id} is locked: {reason}")]
    WorktreeLocked { thread_id: String, reason: String },
    /// The shell could not start; `detail` is the system's message, shown as is.
    #[error("could not start the terminal: {detail}")]
    TerminalFailed { detail: String },
    #[error("terminal {terminal_id} is not open")]
    TerminalNotFound { terminal_id: String },
    /// The port is no longer held by a process of the thread.
    #[error("no server of thread {thread_id} listens on port {port}")]
    ServerNotFound { thread_id: String, port: u16 },
    /// Stopping it would also stop hoshi, an agent or a terminal's shell.
    #[error("the server on port {port} shares its process with hoshi, an agent or a shell")]
    ServerStopRefused { port: u16 },
    /// The system refused to end the process; `detail` is its message, shown as is.
    #[error("could not stop the server: {detail}")]
    ServerStopFailed { detail: String },
    /// A chat has no repository, so it can't work in a worktree.
    #[error("chat {thread_id} can only work in its own folder")]
    ChatNotLocal { thread_id: String },
    #[error("folder {path} cannot be read or created")]
    FolderUnavailable { path: String },
    /// A new project or a clone would mix with files already there.
    #[error("folder {path} is not empty")]
    FolderNotEmpty { path: String },
    /// A file for worktrees to copy is absolute, or leads out of the project or into `.git`.
    #[error("{path} is not a path inside the project")]
    SetupFileInvalid { path: String },
    /// Not `owner/name` or a link to a repository on the chosen forge.
    #[error("{input} does not name a repository")]
    RepositoryNameInvalid { input: String },
    /// The snooze's wake time is not after now, or not a time.
    #[error("thread {thread_id} can't be snoozed until a time that passed")]
    SnoozeTimePassed { thread_id: String },
    #[error("thread {thread_id} is archived")]
    ThreadArchived { thread_id: String },
    /// An approval or another request that isn't a question waits for the user, in the thread
    /// or in a task it delegated.
    #[error("thread {thread_id} waits for the user")]
    ThreadWaitingForUser { thread_id: String },
    /// A message the user sent waits for the thread to take it.
    #[error("thread {thread_id} has a message waiting to be sent")]
    MessageQueued { thread_id: String },
    /// Switching provider would end the commands the thread's agent runs in the background.
    #[error("thread {thread_id} still runs background work")]
    BackgroundWorkRunning { thread_id: String },
    #[error("thread {thread_id} has no turn that ended")]
    NoStableTurn { thread_id: String },
    #[error("thread {thread_id} has no turn holding item {item_id}")]
    TurnNotFound { thread_id: String, item_id: String },
    /// The turn still runs.
    #[error("turn {turn_id} of thread {thread_id} still runs and can't be forked")]
    TurnNotForkable { thread_id: String, turn_id: String },
    /// It was not forked from that thread, or that thread is gone.
    #[error("thread {thread_id} is not a fork of a thread that exists")]
    NotAFork { thread_id: String },
    /// Only a turn that completed can be merged back.
    #[error("turn {turn_id} of fork {thread_id} did not complete")]
    MergeBackTurnNotFinished { thread_id: String, turn_id: String },
    /// The thread a fork merges into runs a turn or has messages waiting for one.
    #[error("thread {thread_id} has work running or waiting")]
    MergeBackTargetBusy { thread_id: String },
    /// Another fork's merge-back still waits for the thread's next turn.
    #[error("thread {thread_id} already waits on the merge-back of fork {fork_thread_id}")]
    MergeBackPending {
        thread_id: String,
        fork_thread_id: String,
    },
    /// Storage or other failures outside the domain; `detail` is for logs, not the UI.
    #[error("internal error: {detail}")]
    Internal { detail: String },
}

/// What the UI reads at startup: every project and thread, without transcripts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub sequence: u64,
    pub projects: Vec<Project>,
    pub threads: Vec<Thread>,
    /// The last known pull request of each thread that has one.
    pub pull_requests: Vec<ThreadPullRequest>,
    /// The last known context usage of each thread that ran a turn.
    pub context_usages: Vec<ThreadContextUsage>,
    /// What agents asked and still wait for, oldest first.
    pub pending_requests: Vec<ThreadItem>,
    /// Tool calls of running turns still waiting for their output.
    pub open_tool_calls: Vec<OpenToolCall>,
    /// Delegated tasks whose answer hasn't gone back yet, by thread id.
    pub pending_tasks: Vec<String>,
    /// The commands each running turn waits for in the background.
    pub background_tasks: Vec<ThreadBackgroundTasks>,
    /// The latest message of each delegated task, shortened.
    pub task_previews: Vec<TaskPreview>,
    /// The messages that wait for their thread to be compacted.
    pub held_messages: Vec<HeldMessage>,
    /// The branch each project's root folder is on, as last read.
    pub project_branches: Vec<ProjectBranch>,
}

/// The branch a project's root folder is on; `None` off any branch or outside git.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBranch {
    pub project_id: String,
    pub branch: Option<String>,
}

/// A message the user sent that waits for its thread's conversation to be compacted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeldMessage {
    pub thread_id: String,
    pub message_id: String,
    pub text: String,
    pub attachments: Vec<MessageAttachment>,
}

/// The message a thread holds until it is compacted; `None` once it went or was taken back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadHeldMessage {
    pub thread_id: String,
    pub message: Option<HeldMessage>,
}

/// Who sent a message waiting in a thread's queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageSender {
    User,
    /// News hoshi brought, such as a delegated task's answer or a pull request's update.
    Hoshi,
}

/// A message waiting in a thread's queue for a turn of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueEntry {
    pub message_id: String,
    pub text: String,
    pub attachments: Vec<MessageAttachment>,
    /// It waits for the user to go on with the thread, as after they stopped it.
    pub held: bool,
    pub sender: MessageSender,
}

/// The messages waiting in a thread's queue, in the order they go.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadQueue {
    pub thread_id: String,
    pub messages: Vec<QueueEntry>,
    /// The user can go on with the thread: its last turn was stopped, cut short or failed on
    /// a usage limit and nothing waits for an answer, or else messages wait held.
    pub can_resume: bool,
}

/// A delegated task's latest message, shortened to one line for its card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPreview {
    pub thread_id: String,
    pub text: String,
}

/// How many characters of a task's message its preview keeps.
pub const TASK_PREVIEW_CHARS: usize = 160;

/// `text` on one line, its whitespace collapsed, cut to [`TASK_PREVIEW_CHARS`].
pub fn task_preview(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= TASK_PREVIEW_CHARS {
        return line;
    }
    let cut: String = line.chars().take(TASK_PREVIEW_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
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

#[derive(Debug, Clone, Default)]
pub struct ReadModel {
    projects: BTreeMap<String, Project>,
    threads: BTreeMap<String, Thread>,
    /// Requests waiting for an answer, by id.
    requests: BTreeMap<String, ThreadItem>,
    /// Tool calls without output yet, of threads and their subagents.
    open_calls: BTreeSet<OpenToolCall>,
    /// The latest message of each delegated task, shortened.
    task_previews: BTreeMap<String, String>,
    /// How each running turn's provider retries, until it gets through.
    retries: BTreeMap<String, TurnRetry>,
    /// The MCP servers each thread already told of failing to start, since the app started.
    mcp_failures: BTreeMap<String, BTreeSet<String>>,
    /// The compaction each thread's running turn shows as running, by thread.
    compacting: BTreeMap<String, String>,
}

impl ReadModel {
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        Self {
            projects: snapshot
                .projects
                .iter()
                .map(|project| (project.id.clone(), project.clone()))
                .collect(),
            threads: snapshot
                .threads
                .iter()
                .map(|thread| (thread.id.clone(), thread.clone()))
                .collect(),
            requests: snapshot
                .pending_requests
                .iter()
                .map(|item| (item.id.clone(), item.clone()))
                .collect(),
            open_calls: snapshot.open_tool_calls.iter().cloned().collect(),
            task_previews: snapshot
                .task_previews
                .iter()
                .map(|preview| (preview.thread_id.clone(), preview.text.clone()))
                .collect(),
            retries: BTreeMap::new(),
            mcp_failures: BTreeMap::new(),
            compacting: BTreeMap::new(),
        }
    }

    /// How the thread's running turn retries to reach the model, while it does.
    pub fn retry(&self, thread_id: &str) -> Option<&TurnRetry> {
        self.retries.get(thread_id)
    }

    /// The latest message of each delegated task, shortened.
    pub fn task_previews(&self) -> Vec<TaskPreview> {
        self.task_previews
            .iter()
            .map(|(thread_id, text)| TaskPreview {
                thread_id: thread_id.clone(),
                text: text.clone(),
            })
            .collect()
    }

    pub fn open_tool_calls(&self) -> impl Iterator<Item = &OpenToolCall> {
        self.open_calls.iter()
    }

    /// Every request waiting for an answer, oldest first.
    pub fn all_pending_requests(&self) -> Vec<ThreadItem> {
        let mut pending: Vec<_> = self.requests.values().cloned().collect();
        pending.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        pending
    }

    /// The request `request_id`, while it waits for an answer.
    pub fn pending_request(&self, request_id: &str) -> Option<&ThreadItem> {
        self.requests.get(request_id)
    }

    /// The requests waiting for an answer in the thread's transcript and its subagents', in
    /// no particular order.
    pub fn pending_requests_within<'a>(
        &'a self,
        thread_id: &'a str,
    ) -> impl Iterator<Item = &'a ThreadItem> + 'a {
        self.requests
            .values()
            .filter(move |item| root_thread(&item.thread_id) == thread_id)
    }

    /// The thread's requests waiting for an answer, oldest first.
    pub fn pending_requests(&self, thread_id: &str) -> Vec<&ThreadItem> {
        let mut pending: Vec<_> = self
            .requests
            .values()
            .filter(|item| item.thread_id == thread_id)
            .collect();
        pending.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        pending
    }

    pub fn projects(&self) -> impl Iterator<Item = &Project> {
        self.projects.values()
    }

    pub fn threads(&self) -> impl Iterator<Item = &Thread> {
        self.threads.values()
    }

    pub fn thread(&self, thread_id: &str) -> Option<&Thread> {
        self.threads.get(thread_id)
    }

    pub fn project(&self, project_id: &str) -> Option<&Project> {
        self.projects.get(project_id)
    }

    /// The threads forked from `thread_id`, oldest first.
    pub fn forks_of(&self, thread_id: &str) -> Vec<&Thread> {
        let mut forks: Vec<&Thread> = self
            .threads
            .values()
            .filter(|thread| {
                thread
                    .forked_from
                    .as_ref()
                    .is_some_and(|origin| origin.thread_id == thread_id)
            })
            .collect();
        forks.sort_by(|a, b| (&a.created_at, &a.id).cmp(&(&b.created_at, &b.id)));
        forks
    }

    /// Where the thread's agent runs: its worktree when it has one, never the
    /// project root in that case. `None` for a chat, whose folder only the engine knows.
    pub fn thread_cwd(&self, thread_id: &str) -> Option<&str> {
        let thread = self.threads.get(thread_id)?;
        match &thread.worktree_path {
            Some(path) => Some(path),
            None => self
                .projects
                .get(thread.project_id.as_deref()?)
                .map(|project| project.workspace_root.as_str()),
        }
    }

    pub fn apply(&mut self, event: &Event) {
        match event {
            Event::ProjectCreated { project } => {
                self.projects.insert(project.id.clone(), project.clone());
            }
            Event::ProjectWorktreeSetupSet { project_id, setup } => {
                if let Some(project) = self.projects.get_mut(project_id) {
                    project.worktree_setup.clone_from(setup);
                }
            }
            Event::ProjectSettingsSet {
                project_id,
                settings,
            } => {
                if let Some(project) = self.projects.get_mut(project_id) {
                    project.settings.clone_from(settings);
                }
            }
            Event::ProjectRemoved { project_id } => {
                self.projects.remove(project_id);
                self.threads
                    .retain(|_, thread| thread.project_id.as_ref() != Some(project_id));
                let threads = &self.threads;
                self.requests
                    .retain(|_, item| threads.contains_key(&item.thread_id));
                self.open_calls
                    .retain(|call| threads.contains_key(root_thread(&call.thread_id)));
                self.task_previews
                    .retain(|thread_id, _| threads.contains_key(thread_id));
                self.retries
                    .retain(|thread_id, _| threads.contains_key(thread_id));
                self.mcp_failures
                    .retain(|thread_id, _| threads.contains_key(thread_id));
                self.compacting
                    .retain(|thread_id, _| threads.contains_key(thread_id));
            }
            Event::ThreadCreated { thread } => {
                self.threads
                    .insert(thread.id.clone(), Thread::clone(thread));
            }
            Event::ThreadDeleted { thread_id } => {
                self.threads.remove(thread_id);
                self.task_previews.remove(thread_id);
                self.retries.remove(thread_id);
                self.mcp_failures.remove(thread_id);
                self.compacting.remove(thread_id);
                self.requests.retain(|_, item| &item.thread_id != thread_id);
                self.open_calls
                    .retain(|call| root_thread(&call.thread_id) != thread_id);
            }
            Event::ThreadTitleSet { thread_id, title } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.title.clone_from(title);
                }
            }
            Event::ThreadBranchSet { thread_id, branch } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.branch.clone_from(branch);
                }
            }
            Event::ThreadWorktreeAttached {
                thread_id,
                worktree_path,
                branch,
                adopted,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.worktree_path = Some(worktree_path.clone());
                    thread.branch.clone_from(branch);
                    thread.worktree_released = false;
                    thread.worktree_adopted = *adopted;
                }
            }
            Event::ThreadWorktreeDetached { thread_id, branch } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.worktree_path = None;
                    thread.branch.clone_from(branch);
                    thread.worktree_released = false;
                    thread.worktree_adopted = false;
                }
            }
            Event::ThreadWorktreeReleased { thread_id } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.worktree_released = true;
                }
            }
            Event::ThreadWorktreeRestored { thread_id } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.worktree_released = false;
                }
            }
            Event::ThreadCleanedUp { thread_id } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.worktree_path = None;
                    thread.branch = None;
                    thread.worktree_released = false;
                    thread.worktree_adopted = false;
                    thread.session_id = None;
                    thread.provider_sessions.clear();
                }
            }
            Event::ThreadArchivedSet {
                thread_id,
                archived,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.archived = *archived;
                }
            }
            Event::ThreadPinnedSet { thread_id, pinned } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.pinned = *pinned;
                }
            }
            Event::ThreadSnoozed {
                thread_id,
                until,
                at,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.snoozed_until = Some(until.clone());
                    thread.snoozed_at = Some(at.clone());
                }
            }
            Event::ThreadUnsnoozed { thread_id, woke_at } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.snoozed_until = None;
                    thread.snoozed_at = None;
                    if woke_at.is_some() {
                        thread.last_snooze_wake_at.clone_from(woke_at);
                    }
                }
            }
            Event::ThreadVisited { thread_id, at } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.last_visited_at = Some(at.clone());
                }
            }
            Event::ThreadModelSet {
                thread_id,
                provider,
                model,
                effort,
                fast_mode,
                ultracode,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.provider.clone_from(provider);
                    thread.model.clone_from(model);
                    thread.effort = *effort;
                    thread.fast_mode = *fast_mode;
                    thread.ultracode = *ultracode;
                }
            }
            Event::ThreadAccessSet {
                thread_id,
                access_mode,
                plan_mode,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.access_mode = *access_mode;
                    thread.plan_mode = *plan_mode;
                }
            }
            Event::TurnStarted {
                thread_id,
                turn_id,
                at,
                model,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.status = ThreadStatus::Working;
                    thread.active_turn_id = Some(turn_id.clone());
                    thread.updated_at.clone_from(at);
                    if let Some(model) = model {
                        thread.last_turn_model = Some(Box::new(model.clone()));
                    }
                }
                self.retries.remove(thread_id);
            }
            Event::SessionAttached {
                thread_id,
                session_id,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.session_id = Some(session_id.clone());
                }
            }
            Event::ItemAdded { item } => {
                if let Some(thread) = self.threads.get_mut(&item.thread_id) {
                    thread.updated_at.clone_from(&item.created_at);
                    if let (Some(_), ItemContent::AssistantMessage { text }) =
                        (&thread.parent_thread_id, &item.content)
                    {
                        self.task_previews
                            .insert(thread.id.clone(), task_preview(text));
                    }
                }
                let call = OpenToolCall {
                    thread_id: item.thread_id.clone(),
                    item_id: item.id.clone(),
                };
                match &item.content {
                    ItemContent::ToolCall {
                        output: None,
                        interrupted: false,
                        ..
                    } => {
                        self.open_calls.insert(call);
                    }
                    ItemContent::ToolCall { .. } => {
                        self.open_calls.remove(&call);
                    }
                    ItemContent::McpServerFailed { name, .. } => {
                        self.mcp_failures
                            .entry(item.thread_id.clone())
                            .or_default()
                            .insert(name.clone());
                    }
                    ItemContent::Compaction {
                        status: CompactionStatus::Running,
                        ..
                    } => {
                        self.compacting
                            .insert(item.thread_id.clone(), item.id.clone());
                    }
                    ItemContent::Compaction { .. } => {
                        self.compacting.remove(&item.thread_id);
                    }
                    _ => {}
                }
                match item.content.request_status() {
                    Some(RequestStatus::Pending) => {
                        self.requests.insert(item.id.clone(), item.clone());
                    }
                    Some(_) => {
                        self.requests.remove(&item.id);
                    }
                    None => {}
                }
            }
            Event::ToolCallCompleted {
                thread_id, item_id, ..
            }
            | Event::ToolCallInterrupted { thread_id, item_id } => {
                self.open_calls.remove(&OpenToolCall {
                    thread_id: thread_id.clone(),
                    item_id: item_id.clone(),
                });
            }
            Event::ToolCallDetailSet { .. } | Event::MessagesDropped { .. } => {}
            Event::TurnRetrying { thread_id, retry } => match retry {
                Some(retry) => {
                    self.retries.insert(thread_id.clone(), retry.clone());
                }
                None => {
                    self.retries.remove(thread_id);
                }
            },
            Event::TurnCompleted {
                thread_id,
                outcome,
                at,
                ..
            } => {
                self.retries.remove(thread_id);
                self.compacting.remove(thread_id);
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.active_turn_id = None;
                    thread.status = match outcome {
                        TurnOutcome::Failed { .. } => ThreadStatus::Error,
                        TurnOutcome::Completed | TurnOutcome::Interrupted => ThreadStatus::Idle,
                    };
                    thread.updated_at.clone_from(at);
                    thread.last_turn_ended_at = Some(at.clone());
                }
            }
            Event::ThreadStatusSet { thread_id, status } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.status = *status;
                }
            }
            Event::ThreadDelegated {
                thread_id,
                parent_thread_id,
                shares_worktree,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.parent_thread_id = Some(parent_thread_id.clone());
                    thread.worktree_adopted |= *shares_worktree;
                }
            }
            Event::TaskCancelled { thread_id } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.task_cancelled = true;
                }
            }
            Event::TaskAnswered { thread_id } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.task_answered = true;
                }
            }
            Event::TaskReopened { thread_id } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.task_answered = false;
                }
            }
            Event::PullRequestWatchSet {
                thread_id,
                watching,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.watching_pull_request = *watching;
                }
            }
            Event::ConversationCursorSet { thread_id, cursor } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.conversation_cursor = Some(cursor.clone());
                    thread.resume_at = None;
                }
            }
            Event::ThreadRewound {
                thread_id,
                resume_at,
                last_turn_model,
                ..
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    if resume_at.is_none() {
                        thread.session_id = None;
                        thread.provider_sessions.clear();
                    }
                    thread.conversation_cursor.clone_from(resume_at);
                    thread.resume_at.clone_from(resume_at);
                    thread.last_turn_model = last_turn_model.clone().map(Box::new);
                }
                self.requests
                    .retain(|_, item| root_thread(&item.thread_id) != thread_id);
                self.open_calls
                    .retain(|call| root_thread(&call.thread_id) != thread_id);
            }
            Event::ThreadModelPending { thread_id, choice } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.pending_model = choice.clone().map(Box::new);
                }
            }
            Event::ProviderSwitched {
                thread_id,
                to,
                session_id,
                conversation_cursor,
                resume_at,
                provider_sessions,
                pending_handoffs,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.provider.clone_from(&to.provider);
                    thread.model.clone_from(&to.model);
                    thread.effort = to.effort;
                    thread.fast_mode = to.fast_mode;
                    thread.ultracode = to.ultracode;
                    thread.session_id.clone_from(session_id);
                    thread.conversation_cursor.clone_from(conversation_cursor);
                    thread.resume_at.clone_from(resume_at);
                    thread.provider_sessions.clone_from(provider_sessions);
                    thread.pending_handoffs.clone_from(pending_handoffs);
                    thread.pending_model = None;
                }
            }
            Event::ContextHandoffsSet {
                thread_id,
                handoffs,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.pending_handoffs.clone_from(handoffs);
                }
            }
        }
    }
}

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
            if thread.provider != provider && thread.active_turn_id.is_some() {
                return Err(CommandError::ThreadBusy { thread_id });
            }
            let has_conversation =
                thread.session_id.is_some() || !thread.provider_sessions.is_empty();
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
fn resolved(content: &ItemContent, response: &RequestResponse) -> Option<ItemContent> {
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

fn compaction_started(thread_id: &str, turn_id: &str, now: &str) -> Event {
    Event::ItemAdded {
        item: ThreadItem {
            id: format!("{turn_id}:compaction"),
            thread_id: thread_id.to_owned(),
            created_at: now.to_owned(),
            content: compaction(CompactionStatus::Running),
        },
    }
}

fn compaction(status: CompactionStatus) -> ItemContent {
    ItemContent::Compaction {
        status,
        before_tokens: None,
        after_tokens: None,
        error: None,
    }
}

/// The request as it stands once nobody waits for it any more.
fn cancelled(content: &ItemContent) -> ItemContent {
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
fn working_again(model: &ReadModel, thread: &Thread, closed: &str) -> Option<Event> {
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
enum Origin {
    User,
    /// The answer of the task child thread `0` worked on.
    TaskResult(String),
    PullRequest(PullRequestUpdate),
}

fn message_events(
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
    let switch = thread
        .active_turn_id
        .is_none()
        .then(|| switch_provider(thread, &message_id))
        .flatten();
    let rewind = match switch {
        Some(_) => None,
        None => rewind_point(thread, !matches!(origin, Origin::User)),
    };
    // Only a turn the transcript opens with the user's message records what it runs on, so a
    // change shows over that message rather than over one hoshi sent unseen.
    let compacts = crate::is_compact_command(&text);
    let shown = matches!(origin, Origin::User) && !compacts;
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

/// What a turn starting now runs on: the provider the user picked for the next message, else
/// the thread's own.
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
fn turn_switch(thread: &Thread, to: &ModelChoice) -> Option<Box<ModelSwitch>> {
    if to.provider != thread.provider {
        return None;
    }
    model_switch(thread.last_turn_model.as_deref()?, to)
}

/// The change from what the last turn ran on to `to`, when it stays on the same provider and
/// changes its model or effort. A provider switch shows as its handoff instead.
fn model_switch(from: &ModelChoice, to: &ModelChoice) -> Option<Box<ModelSwitch>> {
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
fn rewind_point(thread: &Thread, task_result: bool) -> Option<Rewind> {
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
fn forked(source: &Thread, id: String, turn: TurnPoint, now: &str) -> Thread {
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

/// Links a thread to the one whose agent delegated its task to it.
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
fn root_thread(transcript: &str) -> &str {
    transcript.split(':').next().unwrap_or(transcript)
}

/// A subagent's messages, reasoning and tool calls, in its own transcript. It has no turn
/// or status of its own: the call that started it stands for it in the thread.
fn record_subagent(transcript: &str, output: ProviderOutput, now: &str) -> Vec<Event> {
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
fn queued_turn(
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
fn start_idle_turn(
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

/// The message's first line of prose, without its markdown, or the name of its
/// first image or file when it has none. Stands until a generated title replaces it.
fn title_from(text: &str, attachments: &[MessageAttachment]) -> String {
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
fn same_path(a: &str, b: &str) -> bool {
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

#[cfg(test)]
mod tests;
