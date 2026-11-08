use crate::{
    AccessMode, Effort, ItemContent, MessageAttachment, Project, ProviderOutput, Thread,
    ThreadContextUsage, ThreadItem, ThreadPullRequest, ThreadStatus, ThreadWorkspace, TurnFailure,
    TurnOutcome,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    },
    TurnCompleted {
        thread_id: String,
        turn_id: String,
        outcome: TurnOutcome,
        at: String,
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
    #[error("message is empty")]
    EmptyMessage,
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
    /// A chat has no repository, so it can't work in a worktree.
    #[error("chat {thread_id} can only work in its own folder")]
    ChatNotLocal { thread_id: String },
    #[error("folder {path} cannot be read or created")]
    FolderUnavailable { path: String },
    /// A new project or a clone would mix with files already there.
    #[error("folder {path} is not empty")]
    FolderNotEmpty { path: String },
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
        }
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
            Event::ProjectRemoved { project_id } => {
                self.projects.remove(project_id);
                self.threads
                    .retain(|_, thread| thread.project_id.as_ref() != Some(project_id));
            }
            Event::ThreadCreated { thread } => {
                self.threads.insert(thread.id.clone(), thread.clone());
            }
            Event::ThreadDeleted { thread_id } => {
                self.threads.remove(thread_id);
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
                }
            }
            Event::ToolCallCompleted { .. } => {}
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
                },
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
        } => {
            let Some(thread) = model.threads.get(&thread_id) else {
                return Err(CommandError::ThreadNotFound { thread_id });
            };
            if thread.active_turn_id.is_some() {
                return Err(CommandError::ThreadBusy { thread_id });
            }
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
                    content: ItemContent::UserMessage { text, attachments },
                },
            });
            events.push(Event::TurnStarted {
                thread_id,
                turn_id: message_id,
                at: now.to_owned(),
            });
            Ok(events)
        }
        Command::TurnInterrupt { thread_id } => {
            let Some(thread) = model.threads.get(&thread_id) else {
                return Err(CommandError::ThreadNotFound { thread_id });
            };
            if thread.active_turn_id.is_none() {
                return Err(CommandError::ThreadNotWorking { thread_id });
            }
            Ok(Vec::new())
        }
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
    let active = model
        .threads
        .get(thread_id)
        .is_some_and(|thread| thread.active_turn_id.as_deref() == Some(turn_id));
    if !active {
        return Vec::new();
    }
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
        ProviderOutput::TextDelta { .. }
        | ProviderOutput::ReasoningDelta { .. }
        | ProviderOutput::ContextUsage(_)
        | ProviderOutput::UsageLimits(_)
        | ProviderOutput::WorkingDirectory { .. } => Vec::new(),
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
            },
        )],
        ProviderOutput::ToolCallCompleted {
            item_id,
            output,
            is_error,
        } => vec![Event::ToolCallCompleted {
            thread_id: thread_id.to_owned(),
            item_id,
            output,
            is_error,
        }],
        ProviderOutput::TurnCompleted { outcome } => {
            let mut events = Vec::with_capacity(2);
            if let TurnOutcome::Failed { reason } = &outcome {
                events.push(item(
                    format!("{turn_id}:failed"),
                    ItemContent::TurnFailed {
                        reason: reason.clone(),
                    },
                ));
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
    fn a_busy_thread_rejects_another_message() {
        let mut model = model_with_thread();
        run(&mut model, send("m1", "first")).unwrap();

        let error = run(&mut model, send("m2", "second")).unwrap_err();

        assert_eq!(
            error,
            CommandError::ThreadBusy {
                thread_id: "t".into()
            }
        );
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
            }
        )));
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

    #[cfg(windows)]
    #[test]
    fn windows_paths_compare_case_and_separator_insensitively() {
        assert!(same_path(r"C:\Repo\Hoshi", "c:/repo/hoshi/"));
    }
}
