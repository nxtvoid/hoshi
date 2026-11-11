use crate::{
    AccessMode, ApprovalDecision, CompactionStatus, DiffFile, Effort, ItemContent,
    MessageAttachment, OpenToolCall, Project, ProviderOutput, PullRequestUpdate, RequestResponse,
    RequestStatus, Rewind, Thread, ThreadBackgroundTasks, ThreadContextUsage, ThreadItem,
    ThreadPullRequest, ThreadStatus, ThreadWorkspace, ToolDetail, TurnFailure, TurnOutcome,
    WorktreeSetup,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use ts_rs::TS;

const TITLE_MAX_CHARS: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
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
    ThreadCreate {
        thread_id: String,
        /// `None` starts a chat, which always works locally in its own folder.
        project_id: Option<String>,
        title: String,
        workspace: ThreadWorkspace,
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
    /// Applies from the next turn.
    ThreadModelSet {
        thread_id: String,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
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
    ThreadCreated {
        thread: Thread,
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
    ThreadModelSet {
        thread_id: String,
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
    /// the session after `resume_at`, or starts a new one when it is `None`.
    ThreadRewound {
        thread_id: String,
        message_id: String,
        resume_at: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
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
    #[error("thread {thread_id} is already running a turn")]
    ThreadBusy { thread_id: String },
    #[error("thread {thread_id} is not running a turn")]
    ThreadNotWorking { thread_id: String },
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
    /// Storage or other failures outside the domain; `detail` is for logs, not the UI.
    #[error("internal error: {detail}")]
    Internal { detail: String },
}

/// What the UI reads at startup: every project and thread, without transcripts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
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
}

/// A message the user sent that waits for its thread's conversation to be compacted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HeldMessage {
    pub thread_id: String,
    pub message_id: String,
    pub text: String,
    pub attachments: Vec<MessageAttachment>,
}

/// The message a thread holds until it is compacted; `None` once it went or was taken back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThreadHeldMessage {
    pub thread_id: String,
    pub message: Option<HeldMessage>,
}

/// A delegated task's latest message, shortened to one line for its card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EventEnvelope {
    pub sequence: u64,
    pub event: Event,
}

/// Which item a [`StreamDelta`] is writing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum StreamKind {
    Text,
    Reasoning,
    /// A running tool's output so far: each delta replaces the previous one.
    ToolOutput,
}

/// Text an assistant is still writing. Sent to the UI as it streams, never stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
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
        }
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
            }
            Event::ThreadCreated { thread } => {
                self.threads.insert(thread.id.clone(), thread.clone());
            }
            Event::ThreadDeleted { thread_id } => {
                self.threads.remove(thread_id);
                self.task_previews.remove(thread_id);
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
            Event::ThreadModelSet {
                thread_id,
                model,
                effort,
                fast_mode,
                ultracode,
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
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
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.status = ThreadStatus::Working;
                    thread.active_turn_id = Some(turn_id.clone());
                    thread.updated_at.clone_from(at);
                }
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
            Event::ToolCallDetailSet { .. } => {}
            Event::TurnCompleted {
                thread_id,
                outcome,
                at,
                ..
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    thread.active_turn_id = None;
                    thread.status = match outcome {
                        TurnOutcome::Failed { .. } => ThreadStatus::Error,
                        TurnOutcome::Completed | TurnOutcome::Interrupted => ThreadStatus::Idle,
                    };
                    thread.updated_at.clone_from(at);
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
                ..
            } => {
                if let Some(thread) = self.threads.get_mut(thread_id) {
                    if resume_at.is_none() {
                        thread.session_id = None;
                    }
                    thread.conversation_cursor.clone_from(resume_at);
                    thread.resume_at.clone_from(resume_at);
                }
                self.requests
                    .retain(|_, item| root_thread(&item.thread_id) != thread_id);
                self.open_calls
                    .retain(|call| root_thread(&call.thread_id) != thread_id);
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
                thread: Thread {
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
                    created_at: now.to_owned(),
                    updated_at: now.to_owned(),
                },
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
                Rewind::After { cursor } => Some(cursor),
            };
            Ok(vec![Event::ThreadRewound {
                thread_id,
                message_id,
                resume_at,
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
            let mut events = Vec::with_capacity(2);
            if archived && thread.pinned {
                events.push(Event::ThreadPinnedSet {
                    thread_id: thread_id.clone(),
                    pinned: false,
                });
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
            let mut events = Vec::with_capacity(2);
            if pinned && thread.archived {
                events.push(Event::ThreadArchivedSet {
                    thread_id: thread_id.clone(),
                    archived: false,
                });
            }
            if thread.pinned != pinned {
                events.push(Event::ThreadPinnedSet { thread_id, pinned });
            }
            Ok(events)
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
            model: thread_model,
            effort,
            fast_mode,
            ultracode,
        } => {
            if !model.threads.contains_key(&thread_id) {
                return Err(CommandError::ThreadNotFound { thread_id });
            }
            Ok(vec![Event::ThreadModelSet {
                thread_id,
                model: thread_model,
                effort,
                fast_mode,
                ultracode,
            }])
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
            let mut events = vec![Event::ItemAdded {
                item: ThreadItem {
                    id: request_id.clone(),
                    thread_id: thread_id.clone(),
                    created_at: now.to_owned(),
                    content,
                },
            }];
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
    let images = attachments
        .iter()
        .filter(|attachment| matches!(attachment, MessageAttachment::Image { .. }));
    if text.is_empty() && images.count() == 0 {
        return Err(CommandError::EmptyMessage);
    }
    let mut events = Vec::with_capacity(5);
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
    if thread.title.is_empty() {
        events.push(Event::ThreadTitleSet {
            thread_id: thread_id.clone(),
            title: title_from(&text, &attachments),
        });
    }
    events.push(Event::ItemAdded {
        item: ThreadItem {
            id: message_id.clone(),
            thread_id: thread_id.clone(),
            created_at: now.to_owned(),
            content: ItemContent::UserMessage {
                rewind: rewind_point(thread, !matches!(origin, Origin::User)),
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
            },
        },
    });
    // A running turn takes the message in; the engine passes it on.
    if thread.active_turn_id.is_none() {
        events.push(Event::TurnStarted {
            thread_id,
            turn_id: message_id,
            at: now.to_owned(),
        });
    }
    Ok(events)
}

/// Where a message that starts a turn lets the conversation come back to.
fn rewind_point(thread: &Thread, task_result: bool) -> Option<Rewind> {
    if thread.active_turn_id.is_some() || task_result {
        return None;
    }
    match (&thread.session_id, &thread.conversation_cursor) {
        (None, _) => Some(Rewind::Start),
        (Some(_), Some(cursor)) => Some(Rewind::After {
            cursor: cursor.clone(),
        }),
        (Some(_), None) => None,
    }
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
    .collect()
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
        | ProviderOutput::Woke
        | ProviderOutput::ProcessEnded => Vec::new(),
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
            ItemContent::Compaction {
                status: CompactionStatus::Running,
                before_tokens: None,
                after_tokens: None,
                error: None,
            },
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
/// it. Nothing when the thread is gone or busy again.
pub fn start_queued_turn(
    model: &ReadModel,
    thread_id: &str,
    message_id: &str,
    now: &str,
) -> Vec<Event> {
    start_idle_turn(model, thread_id, message_id, now)
}

/// Starts a turn the agent began by itself after its own turn completed, as a command it
/// left running in the background ended. Nothing when the thread is gone or busy.
pub fn start_woken_turn(
    model: &ReadModel,
    thread_id: &str,
    turn_id: &str,
    now: &str,
) -> Vec<Event> {
    start_idle_turn(model, thread_id, turn_id, now)
}

fn start_idle_turn(model: &ReadModel, thread_id: &str, message_id: &str, now: &str) -> Vec<Event> {
    match model.threads.get(thread_id) {
        Some(thread) if thread.active_turn_id.is_none() => vec![Event::TurnStarted {
            thread_id: thread_id.to_owned(),
            turn_id: message_id.to_owned(),
            at: now.to_owned(),
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
/// first image when it has none. Stands until a generated title replaces it.
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
    let image = attachments.iter().find_map(|attachment| match attachment {
        MessageAttachment::Image { name, .. } => Some(name.clone()),
        _ => None,
    });
    prose
        .or(image)
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
mod tests {
    use super::*;
    use crate::Question;

    const NOW: &str = "2026-11-07T12:00:00.000Z";

    fn run(model: &mut ReadModel, command: Command) -> Result<Vec<Event>, CommandError> {
        let events = decide(model, command, NOW)?;
        for event in &events {
            model.apply(event);
        }
        Ok(events)
    }

    fn record(model: &mut ReadModel, turn_id: &str, output: ProviderOutput) -> Vec<Event> {
        let events = record_output(model, "t", turn_id, output, NOW);
        for event in &events {
            model.apply(event);
        }
        events
    }

    fn create_project(id: &str, root: &str) -> Command {
        Command::ProjectCreate {
            project_id: id.into(),
            title: id.into(),
            workspace_root: root.into(),
        }
    }

    fn create_thread(id: &str, project_id: &str) -> Command {
        create_thread_in(id, Some(project_id))
    }

    fn create_thread_in(id: &str, project_id: Option<&str>) -> Command {
        Command::ThreadCreate {
            thread_id: id.into(),
            project_id: project_id.map(str::to_owned),
            title: String::new(),
            workspace: ThreadWorkspace::Local,
            model: None,
            effort: None,
            fast_mode: false,
            ultracode: false,
            access_mode: AccessMode::AcceptEdits,
            plan_mode: false,
        }
    }

    fn send(message_id: &str, text: &str) -> Command {
        Command::MessageSend {
            thread_id: "t".into(),
            message_id: message_id.into(),
            text: text.into(),
            attachments: Vec::new(),
        }
    }

    #[test]
    fn task_preview_keeps_one_short_line() {
        assert_eq!(
            task_preview(
                "  Found it.

- one
- two  "
            ),
            "Found it. - one - two"
        );
        let long = "word ".repeat(100);
        let preview = task_preview(&long);
        assert_eq!(preview.chars().count(), TASK_PREVIEW_CHARS);
        assert!(preview.ends_with("word…"));
    }

    #[test]
    fn keeps_the_latest_message_of_delegated_tasks_only() {
        let mut model = model_with_thread();
        run(&mut model, create_thread("task", "p")).unwrap();
        model.apply(&Event::ThreadDelegated {
            thread_id: "task".into(),
            parent_thread_id: "t".into(),
            shares_worktree: false,
        });
        let say = |thread_id: &str, id: &str, text: &str| Event::ItemAdded {
            item: ThreadItem {
                id: id.into(),
                thread_id: thread_id.into(),
                created_at: NOW.into(),
                content: ItemContent::AssistantMessage { text: text.into() },
            },
        };

        model.apply(&say("t", "m1", "the parent talks"));
        model.apply(&say("task", "m2", "Looking"));
        model.apply(&say(
            "task",
            "m3",
            "Counted
42 files",
        ));

        assert_eq!(
            model.task_previews(),
            [TaskPreview {
                thread_id: "task".into(),
                text: "Counted 42 files".into(),
            }]
        );
        model.apply(&Event::ThreadDeleted {
            thread_id: "task".into(),
        });
        assert!(model.task_previews().is_empty());
    }

    fn model_with_thread() -> ReadModel {
        let mut model = ReadModel::default();
        run(&mut model, create_project("p", "/repo")).unwrap();
        run(&mut model, create_thread("t", "p")).unwrap();
        model
    }

    #[test]
    fn rejects_a_second_project_on_the_same_folder() {
        let mut model = ReadModel::default();
        run(&mut model, create_project("a", "/repo/hoshi")).unwrap();

        let error = run(&mut model, create_project("b", "/repo/hoshi/")).unwrap_err();

        assert_eq!(
            error,
            CommandError::DuplicateWorkspaceRoot {
                project_id: "a".into()
            }
        );
    }

    #[test]
    fn rejects_a_thread_for_a_missing_project() {
        let mut model = ReadModel::default();

        let error = run(&mut model, create_thread("t", "nope")).unwrap_err();

        assert_eq!(
            error,
            CommandError::ProjectNotFound {
                project_id: "nope".into()
            }
        );
    }

    #[test]
    fn a_chat_belongs_to_no_project_and_works_only_locally() {
        let mut model = ReadModel::default();
        run(&mut model, create_project("p", "/repo")).unwrap();
        run(&mut model, create_thread_in("chat", None)).unwrap();

        assert_eq!(model.thread("chat").unwrap().project_id, None);
        assert_eq!(model.thread_cwd("chat"), None);

        let mut worktree = create_thread_in("c2", None);
        if let Command::ThreadCreate { workspace, .. } = &mut worktree {
            *workspace = ThreadWorkspace::Worktree {
                path: "/wt".into(),
                branch: "b".into(),
            };
        }
        assert_eq!(
            run(&mut model, worktree).unwrap_err(),
            CommandError::ChatNotLocal {
                thread_id: "c2".into()
            }
        );

        run(
            &mut model,
            Command::ProjectRemove {
                project_id: "p".into(),
            },
        )
        .unwrap();
        assert!(model.thread("chat").is_some());
    }

    #[test]
    fn removing_a_project_removes_its_threads_and_frees_the_folder() {
        let mut model = ReadModel::default();
        run(&mut model, create_project("p", "/repo")).unwrap();
        run(&mut model, create_project("q", "/other")).unwrap();
        run(&mut model, create_thread("t1", "p")).unwrap();
        run(&mut model, create_thread("t2", "q")).unwrap();

        run(
            &mut model,
            Command::ProjectRemove {
                project_id: "p".into(),
            },
        )
        .unwrap();

        let thread_ids: Vec<_> = model.threads().map(|thread| thread.id.as_str()).collect();
        assert_eq!(thread_ids, ["t2"]);
        run(&mut model, create_project("p2", "/repo")).unwrap();
    }

    #[test]
    fn first_message_titles_the_thread_and_starts_a_turn() {
        let mut model = model_with_thread();

        run(&mut model, send("m1", "  Fix the login bug\nwith details")).unwrap();

        let thread = model.thread("t").unwrap();
        assert_eq!(thread.title, "Fix the login bug");
        assert_eq!(thread.status, ThreadStatus::Working);
        assert_eq!(thread.active_turn_id.as_deref(), Some("m1"));
    }

    #[test]
    fn the_first_title_skips_markdown_syntax_and_code() {
        let title = |text: &str| title_from(text, &[]);

        assert_eq!(
            title(
                "```tsx
const a = 1
```

## Fix the *login* `bug`"
            ),
            "Fix the login bug"
        );
        assert_eq!(title("> quoted ask"), "quoted ask");
        assert_eq!(
            title(
                "1. first step
2. second"
            ),
            "first step"
        );
        assert_eq!(
            title(
                "```tsx
```"
            ),
            ""
        );
    }

    #[test]
    fn a_generated_title_replaces_only_the_title_it_started_from() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "fix the login bug")).unwrap();

        assert!(record_title(&model, "t", "something else", "Login Bug").is_empty());
        assert!(record_title(&model, "t", "fix the login bug", "  ").is_empty());
        let events = record_title(&model, "t", "fix the login bug", " Fix Login Redirect ");

        assert_eq!(
            events,
            [Event::ThreadTitleSet {
                thread_id: "t".into(),
                title: "Fix Login Redirect".into()
            }]
        );
    }

    #[test]
    fn a_message_to_a_busy_thread_joins_its_turn() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "first")).unwrap();

        let events = run(&mut model, send("m2", "second")).unwrap();

        assert!(matches!(
            events.as_slice(),
            [Event::ItemAdded { item }] if item.id == "m2"
                && matches!(item.content, ItemContent::UserMessage { steer: true, .. })
        ));
        assert_eq!(
            model.thread("t").unwrap().active_turn_id.as_deref(),
            Some("m1")
        );
        assert!(start_queued_turn(&model, "t", "m2", NOW).is_empty());

        record(
            &mut model,
            "m1",
            ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Completed,
            },
        );
        assert!(matches!(
            start_queued_turn(&model, "t", "m2", NOW).as_slice(),
            [Event::TurnStarted { turn_id, .. }] if turn_id == "m2"
        ));
    }

    #[test]
    fn rejects_an_empty_message() {
        let mut model = model_with_thread();

        assert_eq!(
            run(&mut model, send("m1", "   ")).unwrap_err(),
            CommandError::EmptyMessage
        );
    }

    fn image(name: &str) -> MessageAttachment {
        MessageAttachment::Image {
            id: "i1".into(),
            name: name.into(),
            mime_type: "image/png".into(),
            size: 10,
        }
    }

    #[test]
    fn an_image_alone_is_a_message_named_after_it() {
        let mut model = model_with_thread();
        let command = Command::MessageSend {
            thread_id: "t".into(),
            message_id: "m1".into(),
            text: " ".into(),
            attachments: vec![image("screenshot.png")],
        };

        let events = run(&mut model, command).unwrap();

        assert_eq!(model.thread("t").unwrap().title, "screenshot.png");
        assert!(events.iter().any(|event| matches!(
            event,
            Event::ItemAdded { item } if item.content == ItemContent::UserMessage {
                text: String::new(),
                attachments: vec![image("screenshot.png")],
                steer: false,
                task_result: None,
                pull_request_update: None,
                rewind: Some(Rewind::Start),
            }
        )));
    }

    fn rewind_of(events: &[Event]) -> Option<Rewind> {
        events.iter().find_map(|event| match event {
            Event::ItemAdded { item } => match &item.content {
                ItemContent::UserMessage { rewind, .. } => Some(rewind.clone()),
                _ => None,
            },
            _ => None,
        })?
    }

    fn finish_turn(model: &mut ReadModel, turn_id: &str) {
        record(
            model,
            turn_id,
            ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Completed,
            },
        );
    }

    #[test]
    fn each_turn_s_message_knows_where_the_conversation_goes_back_to() {
        let mut model = model_with_thread();

        let first = run(&mut model, send("m1", "one")).unwrap();
        record(
            &mut model,
            "m1",
            ProviderOutput::SessionStarted {
                session_id: "s".into(),
            },
        );
        let steer = run(&mut model, send("m1b", "also")).unwrap();
        finish_turn(&mut model, "m1");
        let unknown = run(&mut model, send("m2", "two")).unwrap();
        finish_turn(&mut model, "m2");
        let cursor = record_conversation_cursor(&model, "t", "u2".into());
        model.apply(&cursor[0]);
        let second = run(&mut model, send("m3", "three")).unwrap();

        assert_eq!(rewind_of(&first), Some(Rewind::Start));
        assert_eq!(rewind_of(&steer), None);
        assert_eq!(rewind_of(&unknown), None);
        assert_eq!(
            rewind_of(&second),
            Some(Rewind::After {
                cursor: "u2".into()
            })
        );
    }

    #[test]
    fn a_rewind_pins_the_session_or_drops_it_and_a_new_cursor_unpins_it() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "one")).unwrap();
        record(
            &mut model,
            "m1",
            ProviderOutput::SessionStarted {
                session_id: "s".into(),
            },
        );
        let rewind = |rewind: Rewind| Command::ThreadRewind {
            thread_id: "t".into(),
            message_id: "m1".into(),
            rewind,
        };

        assert_eq!(
            run(&mut model, rewind(Rewind::Start)).unwrap_err(),
            CommandError::ThreadBusy {
                thread_id: "t".into()
            }
        );
        finish_turn(&mut model, "m1");
        run(
            &mut model,
            rewind(Rewind::After {
                cursor: "u1".into(),
            }),
        )
        .unwrap();
        let thread = model.thread("t").unwrap();
        assert_eq!(thread.session_id.as_deref(), Some("s"));
        assert_eq!(thread.resume_at.as_deref(), Some("u1"));
        let cursor = record_conversation_cursor(&model, "t", "u9".into());
        model.apply(&cursor[0]);
        assert_eq!(model.thread("t").unwrap().resume_at, None);

        run(&mut model, rewind(Rewind::Start)).unwrap();
        let thread = model.thread("t").unwrap();
        assert_eq!(thread.session_id, None);
        assert_eq!(thread.conversation_cursor, None);
    }

    #[test]
    fn a_mention_without_text_or_image_is_empty() {
        let mut model = model_with_thread();
        let command = Command::MessageSend {
            thread_id: "t".into(),
            message_id: "m1".into(),
            text: String::new(),
            attachments: vec![MessageAttachment::Skill { name: "pdf".into() }],
        };

        assert_eq!(
            run(&mut model, command).unwrap_err(),
            CommandError::EmptyMessage
        );
    }

    #[test]
    fn interrupting_an_idle_thread_fails() {
        let mut model = model_with_thread();

        let error = run(
            &mut model,
            Command::TurnInterrupt {
                thread_id: "t".into(),
            },
        )
        .unwrap_err();

        assert_eq!(
            error,
            CommandError::ThreadNotWorking {
                thread_id: "t".into()
            }
        );
    }

    #[test]
    fn a_command_left_running_can_be_stopped_once_the_turn_ended_but_a_subagent_cannot() {
        let mut model = model_with_thread();
        let stop_command = Command::BackgroundTaskStop {
            thread_id: "t".into(),
            task_id: "b1".into(),
        };
        let stop_subagent = Command::SubagentStop {
            thread_id: "t".into(),
            tool_use_id: "toolu_1".into(),
        };

        assert_eq!(run(&mut model, stop_command), Ok(Vec::new()));
        assert!(run(&mut model, stop_subagent).is_err());
    }

    #[test]
    fn the_agent_wakes_into_a_turn_of_its_own_only_on_an_idle_thread() {
        let mut model = model_with_thread();

        let woken = start_woken_turn(&model, "t", "w1", NOW);
        assert!(matches!(
            woken.as_slice(),
            [Event::TurnStarted { turn_id, .. }] if turn_id == "w1"
        ));
        apply_all(&mut model, &woken);
        assert!(start_woken_turn(&model, "t", "w2", NOW).is_empty());
        assert!(start_woken_turn(&model, "gone", "w3", NOW).is_empty());
    }

    #[test]
    fn reasoning_is_kept_as_an_item_and_its_deltas_are_not() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "hi")).unwrap();

        let streamed = record(
            &mut model,
            "m1",
            ProviderOutput::ReasoningDelta {
                item_id: "r".into(),
                delta: "Let me".into(),
            },
        );
        let finished = record(
            &mut model,
            "m1",
            ProviderOutput::Reasoning {
                item_id: "r".into(),
                text: "Let me check.".into(),
            },
        );

        assert!(streamed.is_empty());
        assert!(matches!(
            &finished[..],
            [Event::ItemAdded { item }]
                if item.id == "r" && item.content == ItemContent::Reasoning { text: "Let me check.".into() }
        ));
    }

    #[test]
    fn output_from_a_finished_turn_is_dropped() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "hi")).unwrap();
        record(
            &mut model,
            "m1",
            ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Interrupted,
            },
        );

        let late = record(
            &mut model,
            "m1",
            ProviderOutput::AssistantMessage {
                item_id: "a".into(),
                text: "too late".into(),
            },
        );

        assert!(late.is_empty());
        assert_eq!(model.thread("t").unwrap().status, ThreadStatus::Idle);
    }

    #[test]
    fn a_failed_turn_records_the_reason_and_marks_the_thread() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "hi")).unwrap();

        let events = record(
            &mut model,
            "m1",
            turn_failed(TurnFailure::SpawnFailed {
                detail: "not found".into(),
            }),
        );

        assert!(matches!(
            &events[0],
            Event::ItemAdded { item } if matches!(item.content, ItemContent::TurnFailed { .. })
        ));
        let thread = model.thread("t").unwrap();
        assert_eq!(thread.status, ThreadStatus::Error);
        assert_eq!(thread.active_turn_id, None);
    }

    fn apply_all(model: &mut ReadModel, events: &[Event]) {
        for event in events {
            model.apply(event);
        }
    }

    #[test]
    fn an_attached_worktree_becomes_the_thread_cwd() {
        let mut model = model_with_thread();

        let events = record_worktree(&model, "t", "/wt/fix".into(), Some("fix".into()), false);
        apply_all(&mut model, &events);

        assert_eq!(model.thread_cwd("t"), Some("/wt/fix"));
        assert_eq!(model.thread("t").unwrap().branch.as_deref(), Some("fix"));
        assert!(!model.thread("t").unwrap().worktree_adopted);
        assert!(record_worktree(&model, "t", "/wt/fix".into(), None, false).is_empty());
    }

    #[test]
    fn an_adopted_worktree_is_never_removed_by_the_thread() {
        let mut model = model_with_thread();
        let events = record_worktree(&model, "t", "/wt/pr".into(), Some("feat/pr".into()), true);
        apply_all(&mut model, &events);

        assert!(model.thread("t").unwrap().worktree_adopted);
        for command in [
            Command::ThreadWorktreeRelease {
                thread_id: "t".into(),
            },
            Command::ThreadCleanUp {
                thread_id: "t".into(),
            },
        ] {
            assert_eq!(
                decide(&model, command, NOW).unwrap_err(),
                CommandError::WorktreeAdopted {
                    thread_id: "t".into()
                }
            );
        }
    }

    #[test]
    fn leaving_the_worktree_returns_the_thread_to_the_project_root() {
        let mut model = model_with_thread();
        let events = record_worktree(&model, "t", "/wt/pr".into(), Some("feat/pr".into()), true);
        apply_all(&mut model, &events);

        let events = record_worktree_left(&model, "t", Some("main".into()));
        apply_all(&mut model, &events);

        let thread = model.thread("t").unwrap();
        assert_eq!(model.thread_cwd("t"), Some("/repo"));
        assert_eq!(thread.branch.as_deref(), Some("main"));
        assert!(!thread.worktree_adopted);
        assert!(record_worktree_left(&model, "t", None).is_empty());
    }

    fn archive(archived: bool) -> Command {
        Command::ThreadArchive {
            thread_id: "t".into(),
            archived,
        }
    }

    fn pin(pinned: bool) -> Command {
        Command::ThreadPin {
            thread_id: "t".into(),
            pinned,
        }
    }

    #[test]
    fn archiving_unpins_and_pinning_unarchives() {
        let mut model = model_with_thread();

        run(&mut model, pin(true)).unwrap();
        run(&mut model, archive(true)).unwrap();
        let thread = model.thread("t").unwrap();
        assert!(thread.archived && !thread.pinned);

        run(&mut model, pin(true)).unwrap();
        let thread = model.thread("t").unwrap();
        assert!(!thread.archived && thread.pinned);
    }

    #[test]
    fn archiving_twice_changes_nothing() {
        let mut model = model_with_thread();
        run(&mut model, archive(true)).unwrap();

        assert!(run(&mut model, archive(true)).unwrap().is_empty());
    }

    #[test]
    fn a_new_message_brings_an_archived_thread_back() {
        let mut model = model_with_thread();
        run(&mut model, archive(true)).unwrap();

        run(&mut model, send("m1", "one more thing")).unwrap();

        assert!(!model.thread("t").unwrap().archived);
    }

    #[test]
    fn the_session_is_kept_for_the_next_turn() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "hi")).unwrap();

        record(
            &mut model,
            "m1",
            ProviderOutput::SessionStarted {
                session_id: "s1".into(),
            },
        );

        assert_eq!(model.thread("t").unwrap().session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn a_worktree_thread_runs_in_its_worktree_and_a_local_one_in_the_root() {
        let mut model = model_with_thread();
        let mut command = create_thread("w", "p");
        if let Command::ThreadCreate { workspace, .. } = &mut command {
            *workspace = ThreadWorkspace::Worktree {
                path: "/worktrees/w".into(),
                branch: "hoshi/w".into(),
            };
        }

        run(&mut model, command).unwrap();

        assert_eq!(model.thread_cwd("t"), Some("/repo"));
        assert_eq!(model.thread_cwd("w"), Some("/worktrees/w"));
        assert_eq!(
            model.thread("w").unwrap().branch.as_deref(),
            Some("hoshi/w")
        );
    }

    #[test]
    fn an_unprepared_worktree_is_never_created() {
        let mut model = model_with_thread();
        let mut command = create_thread("w", "p");
        if let Command::ThreadCreate { workspace, .. } = &mut command {
            *workspace = ThreadWorkspace::NewWorktree;
        }

        let error = run(&mut model, command).unwrap_err();

        assert!(matches!(error, CommandError::Internal { .. }));
        assert!(model.thread("w").is_none());
    }

    #[test]
    fn a_branch_is_recorded_only_when_it_changes() {
        let mut model = model_with_thread();

        let first = record_branch(&model, "t", Some("main".into()));
        first.iter().for_each(|event| model.apply(event));
        let again = record_branch(&model, "t", Some("main".into()));

        assert_eq!(first.len(), 1);
        assert!(again.is_empty());
        assert_eq!(model.thread("t").unwrap().branch.as_deref(), Some("main"));
    }

    #[test]
    fn the_model_choice_is_kept_on_the_thread() {
        let mut model = model_with_thread();

        run(
            &mut model,
            Command::ThreadModelSet {
                thread_id: "t".into(),
                model: Some("opus".into()),
                effort: Some(Effort::High),
                fast_mode: true,
                ultracode: true,
            },
        )
        .unwrap();

        let thread = model.thread("t").unwrap();
        assert_eq!(thread.model.as_deref(), Some("opus"));
        assert_eq!(thread.effort, Some(Effort::High));
        assert!(thread.fast_mode && thread.ultracode);
    }

    #[test]
    fn the_access_choice_is_kept_on_the_thread() {
        let mut model = model_with_thread();
        assert_eq!(
            model.thread("t").unwrap().access_mode,
            AccessMode::AcceptEdits
        );

        run(
            &mut model,
            Command::ThreadAccessSet {
                thread_id: "t".into(),
                access_mode: AccessMode::FullAccess,
                plan_mode: true,
            },
        )
        .unwrap();

        let thread = model.thread("t").unwrap();
        assert_eq!(thread.access_mode, AccessMode::FullAccess);
        assert!(thread.plan_mode);
    }

    fn approval() -> ItemContent {
        ItemContent::Approval {
            tool_call_id: Some("toolu_1".into()),
            tool_name: "Bash".into(),
            input: r#"{"command":"npm test"}"#.into(),
            reason: None,
            can_accept_for_session: false,
            status: RequestStatus::Pending,
            decision: None,
            feedback: None,
        }
    }

    fn open(model: &mut ReadModel, request_id: &str, content: ItemContent) -> Vec<Event> {
        record(
            model,
            "m1",
            ProviderOutput::RequestOpened {
                request_id: request_id.into(),
                content,
            },
        )
    }

    fn respond(request_id: &str, response: RequestResponse) -> Command {
        Command::RequestRespond {
            thread_id: "t".into(),
            request_id: request_id.into(),
            response,
        }
    }

    fn decision(decision: ApprovalDecision) -> RequestResponse {
        RequestResponse::Decision {
            decision,
            feedback: None,
        }
    }

    fn status(model: &ReadModel) -> ThreadStatus {
        model.thread("t").unwrap().status
    }

    #[test]
    fn a_request_waits_for_the_user_until_answered() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "test it")).unwrap();
        open(&mut model, "r1", approval());
        assert_eq!(status(&model), ThreadStatus::WaitingForInput);
        assert_eq!(model.pending_requests("t").len(), 1);

        let events = run(
            &mut model,
            respond(
                "r1",
                RequestResponse::Decision {
                    decision: ApprovalDecision::Decline,
                    feedback: Some("  run the unit tests only ".into()),
                },
            ),
        )
        .unwrap();

        let Event::ItemAdded { item } = &events[0] else {
            panic!("expected the resolved request, got {events:?}");
        };
        assert!(matches!(
            &item.content,
            ItemContent::Approval {
                status: RequestStatus::Resolved,
                decision: Some(ApprovalDecision::Decline),
                feedback: Some(feedback),
                ..
            } if feedback == "run the unit tests only"
        ));
        assert_eq!(status(&model), ThreadStatus::Working);
        assert!(model.pending_requests("t").is_empty());
    }

    #[test]
    fn the_thread_waits_while_any_request_is_open() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "test it")).unwrap();
        open(&mut model, "r1", approval());
        let again = open(&mut model, "r2", approval());
        assert_eq!(again.len(), 1, "already waiting: {again:?}");

        run(
            &mut model,
            respond("r1", decision(ApprovalDecision::Accept)),
        )
        .unwrap();
        assert_eq!(status(&model), ThreadStatus::WaitingForInput);

        record(
            &mut model,
            "m1",
            ProviderOutput::RequestWithdrawn {
                request_id: "r2".into(),
            },
        );
        assert_eq!(status(&model), ThreadStatus::Working);
    }

    #[test]
    fn a_request_is_answered_once_and_only_as_it_asks() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "test it")).unwrap();
        open(&mut model, "r1", approval());

        let answers = RequestResponse::Answers {
            answers: vec!["yes".into()],
        };
        assert_eq!(
            run(&mut model, respond("r1", answers)),
            Err(CommandError::RequestResponseInvalid {
                request_id: "r1".into()
            })
        );
        assert_eq!(
            run(
                &mut model,
                respond("r1", decision(ApprovalDecision::AcceptForSession))
            ),
            Err(CommandError::RequestResponseInvalid {
                request_id: "r1".into()
            }),
            "this approval offers no session grant"
        );
        run(
            &mut model,
            respond("r1", decision(ApprovalDecision::Accept)),
        )
        .unwrap();
        assert_eq!(
            run(
                &mut model,
                respond("r1", decision(ApprovalDecision::Accept))
            ),
            Err(CommandError::RequestNotPending {
                request_id: "r1".into()
            })
        );
    }

    #[test]
    fn questions_take_one_answer_each() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "ask me")).unwrap();
        let question = Question {
            header: "Color".into(),
            question: "Which color?".into(),
            options: Vec::new(),
            multi_select: false,
        };
        open(
            &mut model,
            "r1",
            ItemContent::Questions {
                questions: vec![question.clone(), question],
                status: RequestStatus::Pending,
                answers: None,
            },
        );

        let one = RequestResponse::Answers {
            answers: vec!["Red".into()],
        };
        assert!(run(&mut model, respond("r1", one)).is_err());
        let both = RequestResponse::Answers {
            answers: vec!["Red".into(), "Blue".into()],
        };
        let events = run(&mut model, respond("r1", both)).unwrap();
        assert!(matches!(
            &events[0],
            Event::ItemAdded { item } if matches!(
                &item.content,
                ItemContent::Questions { answers: Some(answers), status: RequestStatus::Resolved, .. }
                    if answers.len() == 2
            )
        ));
    }

    #[test]
    fn an_accepted_plan_takes_the_thread_out_of_plan_mode() {
        let mut model = model_with_thread();
        run(
            &mut model,
            Command::ThreadAccessSet {
                thread_id: "t".into(),
                access_mode: AccessMode::Ask,
                plan_mode: true,
            },
        )
        .unwrap();
        run(&mut model, send("m1", "plan it")).unwrap();
        open(
            &mut model,
            "r1",
            ItemContent::ProposedPlan {
                plan: "1. Do it".into(),
                status: RequestStatus::Pending,
                decision: None,
                feedback: None,
            },
        );

        run(
            &mut model,
            respond("r1", decision(ApprovalDecision::Accept)),
        )
        .unwrap();

        let thread = model.thread("t").unwrap();
        assert!(!thread.plan_mode);
        assert_eq!(thread.access_mode, AccessMode::Ask);
        assert_eq!(thread.status, ThreadStatus::Working);
    }

    #[test]
    fn requests_left_open_are_cancelled_when_the_turn_ends() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "test it")).unwrap();
        open(&mut model, "r1", approval());

        let events = record(
            &mut model,
            "m1",
            ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Interrupted,
            },
        );

        assert!(matches!(
            &events[0],
            Event::ItemAdded { item } if item.content.request_status() == Some(RequestStatus::Cancelled)
        ));
        assert!(model.pending_requests("t").is_empty());
        assert_eq!(status(&model), ThreadStatus::Idle);
        assert_eq!(
            run(
                &mut model,
                respond("r1", decision(ApprovalDecision::Accept))
            ),
            Err(CommandError::RequestNotPending {
                request_id: "r1".into()
            })
        );
    }

    #[test]
    fn a_turn_the_app_cut_short_keeps_its_questions_for_an_answer_by_message() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "test it")).unwrap();
        open(&mut model, "r1", approval());
        let asked = ItemContent::Questions {
            questions: vec![Question {
                header: "Color".into(),
                question: "Which color?".into(),
                options: Vec::new(),
                multi_select: false,
            }],
            status: RequestStatus::Pending,
            answers: None,
        };
        open(&mut model, "r2", asked.clone());

        let events = close_stale_turn(&model, "t", "m1", NOW);
        for event in &events {
            model.apply(event);
        }

        let pending: Vec<_> = model
            .pending_requests("t")
            .iter()
            .map(|item| item.id.clone())
            .collect();
        assert_eq!(pending, ["r2"]);
        assert_eq!(status(&model), ThreadStatus::Idle);
        let answers = RequestResponse::Answers {
            answers: vec![" Blue ".into()],
        };
        assert_eq!(
            answers_message(&asked, &answers).as_deref(),
            Some(
                "Which color?
Blue"
            )
        );
    }

    #[test]
    fn a_rewind_drops_the_questions_it_takes_out_of_the_transcript() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "test it")).unwrap();
        let asked = ItemContent::Questions {
            questions: vec![Question {
                header: "Color".into(),
                question: "Which color?".into(),
                options: Vec::new(),
                multi_select: false,
            }],
            status: RequestStatus::Pending,
            answers: None,
        };
        open(&mut model, "r1", asked);
        for event in close_stale_turn(&model, "t", "m1", NOW) {
            model.apply(&event);
        }
        assert_eq!(model.pending_requests("t").len(), 1);

        run(
            &mut model,
            Command::ThreadRewind {
                thread_id: "t".into(),
                message_id: "m1".into(),
                rewind: Rewind::Start,
            },
        )
        .unwrap();

        assert!(model.pending_requests("t").is_empty());
        assert!(model.pending_request("r1").is_none());
        let answers = RequestResponse::Answers {
            answers: vec!["Blue".into()],
        };
        assert!(run(&mut model, respond("r1", answers)).is_err());
    }

    #[test]
    fn a_page_keeps_only_the_start_of_long_tool_output() {
        let call = |output: &str| ItemContent::ToolCall {
            name: "Bash".into(),
            input: "{}".into(),
            output: Some(output.into()),
            is_error: false,
            detail: None,
            output_omitted: false,
            interrupted: false,
        };
        let long = "x".repeat(1_000);

        let ItemContent::ToolCall {
            output,
            output_omitted,
            ..
        } = call(&long).summary()
        else {
            unreachable!()
        };
        assert_eq!(output.map(|text| text.len()), Some(200));
        assert!(output_omitted);
        assert_eq!(call("short").summary(), call("short"));
    }

    #[test]
    fn a_subagent_writes_its_own_transcript_beside_the_thread() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "count the files")).unwrap();
        let nested = |tool_use_id: &str, output| ProviderOutput::Subagent {
            tool_use_id: tool_use_id.into(),
            output: Box::new(output),
        };

        let started = record(
            &mut model,
            "m1",
            nested(
                "toolu_agent",
                ProviderOutput::ToolCallStarted {
                    item_id: "toolu_ls".into(),
                    name: "Bash".into(),
                    input: "{}".into(),
                },
            ),
        );
        let ignored = record(
            &mut model,
            "m1",
            nested(
                "toolu_agent",
                ProviderOutput::TextDelta {
                    item_id: "x".into(),
                    delta: "hi".into(),
                },
            ),
        );

        assert!(matches!(
            started.as_slice(),
            [Event::ItemAdded { item }] if item.thread_id == "t:toolu_agent" && item.id == "toolu_ls"
        ));
        assert!(ignored.is_empty());
        assert_eq!(
            subagent_thread_id("t:toolu_agent", "toolu_inner"),
            "t:toolu_inner"
        );
        assert_eq!(model.thread("t").unwrap().status, ThreadStatus::Working);
    }

    #[test]
    fn calls_left_running_stop_with_their_turn_subagents_included() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "run it")).unwrap();
        let start = |item_id: &str| ProviderOutput::ToolCallStarted {
            item_id: item_id.into(),
            name: "Bash".into(),
            input: "{}".into(),
        };
        record(&mut model, "m1", start("done"));
        record(&mut model, "m1", start("long"));
        record(
            &mut model,
            "m1",
            ProviderOutput::Subagent {
                tool_use_id: "toolu_agent".into(),
                output: Box::new(start("inner")),
            },
        );
        record(
            &mut model,
            "m1",
            ProviderOutput::ToolCallCompleted {
                item_id: "done".into(),
                output: "ok".into(),
                is_error: false,
                detail: None,
            },
        );

        let events = record(
            &mut model,
            "m1",
            ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Interrupted,
            },
        );

        let interrupted: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                Event::ToolCallInterrupted { thread_id, item_id } => {
                    Some(format!("{thread_id}/{item_id}"))
                }
                _ => None,
            })
            .collect();
        assert_eq!(interrupted, ["t/long", "t:toolu_agent/inner"]);
        assert_eq!(model.open_tool_calls().count(), 0);
    }

    #[test]
    fn a_delegated_thread_knows_its_parent_and_gets_its_answer_back_as_hoshi() {
        let mut model = model_with_thread();
        run(&mut model, create_thread("c", "p")).unwrap();
        for event in record_delegation(&model, "c", "t") {
            model.apply(&event);
        }
        assert_eq!(
            model.thread("c").unwrap().parent_thread_id.as_deref(),
            Some("t")
        );
        assert!(record_delegation(&model, "c", "missing").is_empty());

        let events = send_task_result(&model, "t", "c", "r1", "done", NOW).unwrap();

        assert!(events.iter().any(|event| matches!(
            event,
            Event::ItemAdded { item } if matches!(
                &item.content,
                ItemContent::UserMessage { task_result: Some(child), .. } if child == "c"
            )
        )));
        assert!(
            events.iter().any(
                |event| matches!(event, Event::TurnStarted { turn_id, .. } if turn_id == "r1")
            )
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_compare_case_and_separator_insensitively() {
        assert!(same_path(r"C:\Repo\Hoshi", "c:/repo/hoshi/"));
    }

    fn set_setup(files: &[&str], command: &str) -> Command {
        Command::ProjectWorktreeSetupSet {
            project_id: "p".into(),
            setup: Some(WorktreeSetup {
                files: files.iter().map(|&file| file.to_owned()).collect(),
                command: command.into(),
                wait: true,
            }),
        }
    }

    fn setup_of(model: &ReadModel) -> Option<WorktreeSetup> {
        model.project("p").and_then(|p| p.worktree_setup.clone())
    }

    #[test]
    fn a_worktree_setup_keeps_its_files_relative_and_once_each() {
        let mut model = ReadModel::default();
        run(&mut model, create_project("p", "/repo")).unwrap();

        run(
            &mut model,
            set_setup(
                &[
                    " .env ",
                    "",
                    r"config\local.properties",
                    "./.env",
                    "secrets//*.env/",
                ],
                "  ",
            ),
        )
        .unwrap();

        let setup = setup_of(&model).unwrap();
        assert_eq!(
            setup.files,
            [".env", "config/local.properties", "secrets/*.env"]
        );
        assert_eq!(setup.command, "");
    }

    #[test]
    fn a_worktree_setup_without_files_or_command_is_none() {
        let mut model = ReadModel::default();
        run(&mut model, create_project("p", "/repo")).unwrap();
        run(&mut model, set_setup(&["x"], "bun install")).unwrap();

        run(&mut model, set_setup(&[" ", ""], " ")).unwrap();

        assert_eq!(setup_of(&model), None);
    }

    #[test]
    fn a_worktree_setup_refuses_files_outside_the_project() {
        let mut model = ReadModel::default();
        run(&mut model, create_project("p", "/repo")).unwrap();

        for path in [
            "/etc/passwd",
            r"\\server\share",
            r"C:\Users\nxtvoid\.env",
            "d:secrets",
            "../other/.env",
            "config/../../.env",
            ".git/config",
            ".GIT",
        ] {
            let error = run(&mut model, set_setup(&[".env", path], "")).unwrap_err();
            assert_eq!(
                error,
                CommandError::SetupFileInvalid {
                    path: path.to_owned()
                },
                "{path}"
            );
        }
        assert_eq!(setup_of(&model), None);
    }
}
