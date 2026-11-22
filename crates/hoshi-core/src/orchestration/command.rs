use super::*;

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
