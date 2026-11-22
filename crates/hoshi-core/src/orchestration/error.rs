use super::*;

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
