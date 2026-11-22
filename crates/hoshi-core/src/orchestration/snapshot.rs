use super::*;

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
