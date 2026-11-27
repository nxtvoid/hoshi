use super::*;

/// The process's turn, the messages sent to it, and the background commands it may wait for.
#[derive(Debug)]
pub(super) struct QueueState {
    /// Sent, but not yet taken in by the agent, by message id.
    pub(super) waiting: HashSet<String>,
    /// The agent started answering what the turn began with. A message it read at once
    /// sooner than that would take that one's place: Claude Code drops what it had not yet
    /// begun.
    pub(super) answering: bool,
    /// The process is to end: its stdin closes, so nothing more can join.
    pub(super) closing: bool,
    /// The user asked the turn to stop.
    pub(super) interrupted: bool,
    /// A turn runs. Between turns, the process only keeps the commands the last one left
    /// running in the background.
    pub(super) turn_open: bool,
    /// The turn open is one the agent started by itself, as a task it left running ended:
    /// the next turn's message joins it, as the task's result tells its run apart.
    pub(super) woke: bool,
    /// The background commands running, by task id.
    pub(super) commands: HashSet<String>,
    /// The commands among them the agent said its turn waits for.
    pub(super) awaited: HashSet<String>,
    /// What the agent may do, as the latest turn asked.
    pub(super) access: Access,
    /// Where the session works, as last reported, or the worktree its thread moved to since.
    pub(super) cwd: Option<String>,
    /// The images and documents the conversation holds, as this process sent them.
    pub(super) media: MediaBudget,
    /// The turn's messages, by id: the one it began with and those sent to it since.
    pub(super) sent: HashSet<String>,
    /// The agent has not begun yet what the turn began with. A run that ends sooner is about
    /// something else, such as a background task that ended or an earlier process left.
    pub(super) unbegun: bool,
    /// Claude Code tells which of our messages its lines are about, which tells a run of
    /// ours from one of its own.
    pub(super) echoes: bool,
    /// The message the turn began with.
    pub(super) prompt: Option<String>,
    /// The agent began answering it.
    pub(super) prompt_started: bool,
    /// Messages sent to the turn that Claude Code let go of without the agent taking them in.
    pub(super) unread: Vec<String>,
    /// Claude Code reports each message's `command_lifecycle`.
    pub(super) lifecycles: bool,
    /// The model the process runs since a turn changed it; `None` while it is the one it was
    /// started with.
    pub(super) model: Option<TurnModel>,
    /// The model and effort a turn switched to, for the parser to follow.
    pub(super) retarget: Option<(Option<String>, Option<Effort>)>,
    /// Claude Code refused a model switch: what the process runs is no longer known.
    pub(super) model_unsure: bool,
    /// The agent scheduled a wakeup that has not run yet.
    pub(super) wakeup: bool,
}

impl QueueState {
    pub(super) fn awaits_begin(&self) -> bool {
        self.unbegun && self.echoes
    }

    /// Whether a message sent to the turn waits to be taken in. A CLI that tells neither
    /// when it takes one in nor which its lines are about never says it did.
    pub(super) fn steers_waiting(&self) -> bool {
        !self.waiting.is_empty() && (self.lifecycles || self.echoes)
    }

    /// The messages the agent never read: those Claude Code let go of, then, when `ended`
    /// cut them off, those still waiting. None is waiting after.
    pub(super) fn dropped(&mut self, ended: bool) -> Vec<String> {
        let mut dropped = std::mem::take(&mut self.unread);
        let mut waiting: Vec<String> = self.waiting.drain().collect();
        if ended {
            waiting.sort();
            dropped.extend(waiting);
        }
        dropped
    }
}

impl Default for QueueState {
    fn default() -> Self {
        Self {
            waiting: HashSet::new(),
            answering: false,
            closing: false,
            interrupted: false,
            turn_open: true,
            woke: false,
            commands: HashSet::new(),
            awaited: HashSet::new(),
            access: Access::default(),
            cwd: None,
            media: MediaBudget::default(),
            sent: HashSet::new(),
            unbegun: false,
            echoes: false,
            prompt: None,
            prompt_started: false,
            unread: Vec::new(),
            lifecycles: false,
            model: None,
            retarget: None,
            model_unsure: false,
            wakeup: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct Queue(Arc<Mutex<QueueState>>);

impl Queue {
    pub(super) fn state(&self) -> MutexGuard<'_, QueueState> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Marks the running commands among `task_ids` as awaited, or no longer; returns them.
    pub(super) fn await_tasks(&self, task_ids: &[String], awaited: bool) -> Vec<String> {
        let mut state = self.state();
        let running: Vec<String> = task_ids
            .iter()
            .filter(|task_id| state.commands.contains(*task_id))
            .cloned()
            .collect();
        for task_id in &running {
            if awaited {
                state.awaited.insert(task_id.clone());
            } else {
                state.awaited.remove(task_id);
            }
        }
        running
    }
}

/// What a process was started with, which a later turn must share to run in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Launch {
    pub(super) cwd: String,
    pub(super) model: TurnModel,
    /// Added to the system prompt.
    pub(super) instructions: Option<String>,
    pub(super) tool_search: ToolSearch,
    pub(super) mcp: Option<McpServer>,
    /// Allowed to bypass permissions, which a later turn can only switch to if so.
    pub(super) full_access: bool,
}

/// Why a process can't take a next turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Misfit {
    Folder,
    Session,
    ConversationPoint,
    Model,
    Instructions,
    Mcp,
    FullAccess,
    Closing,
    TurnOpen,
}

/// The lines that start `request` as the next turn of a process started as `launch`, whose
/// session is `session`, and open that turn. Refused when the request asks for what the
/// process can't change: another folder or point of the conversation, or a model it can't
/// switch to in place.
pub(super) fn next_turn(
    launch: &Launch,
    default_model: Option<&str>,
    session: Option<&str>,
    queue: &mut QueueState,
    request: &TurnRequest,
) -> Result<Vec<String>, Misfit> {
    let access = access(request);
    let here = same_folder(&request.cwd, &launch.cwd)
        || queue
            .cwd
            .as_deref()
            .is_some_and(|cwd| same_folder(&request.cwd, cwd));
    let model = turn_model(request, default_model);
    let switch = model_switch(queue.model.as_ref().unwrap_or(&launch.model), &model);
    let misfit = if !here {
        Some(Misfit::Folder)
    } else if session.is_none() || request.resume_session_id.as_deref() != session {
        Some(Misfit::Session)
    } else if request.resume_session_at.is_some() || request.fork_session {
        Some(Misfit::ConversationPoint)
    } else if queue.model_unsure || switch.is_none() {
        Some(Misfit::Model)
    } else if system_prompt(request, default_model, launch.tool_search) != launch.instructions {
        Some(Misfit::Instructions)
    } else if request.mcp != launch.mcp {
        Some(Misfit::Mcp)
    } else if access.mode == AccessMode::FullAccess && !launch.full_access {
        Some(Misfit::FullAccess)
    } else if queue.closing {
        Some(Misfit::Closing)
    } else if queue.turn_open && (!queue.woke || queue.interrupted) {
        Some(Misfit::TurnOpen)
    } else {
        None
    };
    if let Some(misfit) = misfit {
        return Err(misfit);
    }
    queue.woke = false;
    let mut lines = switch.unwrap_or_default();
    if !lines.is_empty() {
        queue.model = Some(model);
        queue.retarget = Some((
            request.model.clone().or(default_model.map(str::to_owned)),
            request.effort,
        ));
    }
    if access != queue.access {
        let mode = json!({
            "type": "control_request",
            "request_id": MODE_REQUEST_ID,
            "request": { "subtype": "set_permission_mode", "mode": access.permission_mode() },
        });
        lines.push(mode.to_string());
        queue.access = access;
    }
    let mut media = MediaBudget::after(&request.earlier_media);
    lines.push(user_message(
        &request.prompt,
        &request.attachments,
        &mut media,
        queue,
    ));
    queue.media = media;
    queue.turn_open = true;
    queue.answering = false;
    queue.interrupted = false;
    Ok(lines)
}

/// The control requests that switch a process running `live` to `wanted`; `None` when that
/// needs a process of its own: an effort set can't be cleared back to what the user's
/// settings say.
pub(super) fn model_switch(live: &TurnModel, wanted: &TurnModel) -> Option<Vec<String>> {
    if wanted.effort.is_none() && live.effort.is_some() {
        return None;
    }
    let mut lines = Vec::new();
    if wanted.slug != live.slug {
        let model = json!({
            "type": "control_request",
            "request_id": MODEL_REQUEST_ID,
            "request": { "subtype": "set_model", "model": wanted.slug },
        });
        lines.push(model.to_string());
    }
    if (&wanted.effort, wanted.fast_mode, wanted.ultracode)
        != (&live.effort, live.fast_mode, live.ultracode)
    {
        let mut settings = json!({ "fastMode": wanted.fast_mode, "ultracode": wanted.ultracode });
        if let Some(effort) = &wanted.effort {
            settings["effortLevel"] = json!(effort);
        }
        if wanted.ultracode {
            settings["permissions"] = json!({ "allow": ["Workflow"] });
        }
        let flags = json!({
            "type": "control_request",
            "request_id": FLAGS_REQUEST_ID,
            "request": { "subtype": "apply_flag_settings", "settings": settings },
        });
        lines.push(flags.to_string());
    }
    Some(lines)
}

pub(super) fn same_folder(one: &str, other: &str) -> bool {
    let normal = |path: &str| {
        let path = path.replace('\\', "/");
        let path = path.trim_end_matches('/');
        if cfg!(windows) {
            path.to_lowercase()
        } else {
            path.to_owned()
        }
    };
    normal(one) == normal(other)
}
