//! Tasks a thread's agent hands to child threads through hoshi's MCP server, and their
//! answers, which come back to the parent as messages.

use crate::{State, now};
use hoshi_core::{
    Command, ThreadStatus, ThreadWorkspace, TurnOutcome, record_delegation, send_task_result,
};
use serde_json::{Value, json};

/// What the agent reads with every running task: its answer comes on its own.
/// Kept short: a page of the transcript keeps only the start of a tool's output, which must
/// still hold the task id.
const NEXT: &str = "End your turn now; the answer arrives here as a message.";
/// An answer longer than this is cut; `task_status` still has it all.
const ANSWER_MAX_CHARS: usize = 16_000;
const TITLE_MAX_CHARS: usize = 60;

/// Added to the system prompt of agents that can delegate.
pub(crate) const INSTRUCTIONS: &str = "hoshi can run work in child threads the user can open, through the hoshi MCP server's tools (load them with ToolSearch, \"select:mcp__hoshi__delegate_task\").\n- Use your own Agent tool for quick help within this turn. Use delegate_task for work that should run as a thread of its own, or in its own checkout and branch (isolation: \"worktree\").\n- The child gets only `task`: include everything it needs.\n- delegate_task returns at once. The child's answer arrives later as a message in this thread, so end your turn after delegating; don't wait, sleep or poll with task_status.\n- Stop a task with task_cancel.";

/// A task a thread delegated, until its answer goes back.
pub(crate) struct Delegation {
    pub parent: String,
    pub title: String,
}

/// A fresh id in UUID form, which agents accept as a message id.
pub(crate) fn new_id() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        bytes = nanos.to_le_bytes();
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

impl State {
    /// A tool call from the agent of thread `caller`.
    pub(crate) fn call_tool(
        &mut self,
        caller: &str,
        tool: &str,
        arguments: &Value,
    ) -> Result<Value, String> {
        match tool {
            "delegate_task" => self.delegate_task(caller, arguments),
            "task_status" => self.task_status(caller, arguments),
            "task_cancel" => self.task_cancel(caller, arguments),
            _ => Err(format!("hoshi has no tool named {tool}")),
        }
    }

    fn delegate_task(&mut self, caller: &str, arguments: &Value) -> Result<Value, String> {
        let parent = self
            .model
            .thread(caller)
            .cloned()
            .ok_or("this thread no longer exists")?;
        if parent.active_turn_id.is_none() {
            return Err("delegate_task works only while this thread's turn runs".into());
        }
        let task = arguments["task"]
            .as_str()
            .map(str::trim)
            .filter(|task| !task.is_empty())
            .ok_or("`task` is empty")?;
        let title: String = arguments["title"]
            .as_str()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .or_else(|| task.lines().find(|line| !line.trim().is_empty()))
            .unwrap_or(task)
            .chars()
            .take(TITLE_MAX_CHARS)
            .collect();
        let isolated = arguments["isolation"] == "worktree";
        let workspace = match (&parent.worktree_path, isolated) {
            (_, true) => ThreadWorkspace::NewWorktree,
            (Some(path), false) => ThreadWorkspace::Worktree {
                path: path.clone(),
                branch: parent.branch.clone().unwrap_or_default(),
            },
            (None, false) => ThreadWorkspace::Local,
        };
        let shared = matches!(workspace, ThreadWorkspace::Worktree { .. });
        let child = new_id();
        let create = Command::ThreadCreate {
            thread_id: child.clone(),
            project_id: parent.project_id.clone(),
            title: title.clone(),
            workspace,
            model: parent.model.clone(),
            effort: parent.effort,
            fast_mode: parent.fast_mode,
            ultracode: parent.ultracode,
            access_mode: parent.access_mode,
            plan_mode: false,
        };
        // The parent's worktree is already there; only a new one goes through `prepare`.
        let created = if shared {
            self.run_prepared(create)
        } else {
            self.run_command(create)
        };
        created.map_err(|error| error.to_string())?;
        let now = now();
        let linked = record_delegation(&self.model, &child, caller);
        self.commit(&linked, &now)
            .map_err(|error| error.to_string())?;
        self.delegations.insert(
            child.clone(),
            Delegation {
                parent: caller.to_owned(),
                title: title.clone(),
            },
        );
        self.run_command(Command::MessageSend {
            thread_id: child.clone(),
            message_id: new_id(),
            text: task.to_owned(),
            attachments: Vec::new(),
        })
        .map_err(|error| error.to_string())?;
        Ok(json!({ "taskId": child, "status": "running", "next": NEXT }))
    }

    /// The child `taskId` names, when the caller delegated it.
    fn task(&self, caller: &str, arguments: &Value) -> Result<hoshi_core::Thread, String> {
        let task_id = arguments["taskId"].as_str().unwrap_or_default();
        self.model
            .thread(task_id)
            .filter(|thread| thread.parent_thread_id.as_deref() == Some(caller))
            .cloned()
            .ok_or_else(|| format!("this thread delegated no task {task_id}"))
    }

    fn task_status(&mut self, caller: &str, arguments: &Value) -> Result<Value, String> {
        let child = self.task(caller, arguments)?;
        let mut status = json!({
            "taskId": child.id,
            "title": child.title,
            "status": match child.status {
                ThreadStatus::Working => "running",
                ThreadStatus::WaitingForInput => "waiting_for_user",
                ThreadStatus::Idle => "completed",
                ThreadStatus::Error => "failed",
            },
        });
        if child.active_turn_id.is_some() {
            status["next"] = json!(NEXT);
        } else {
            // Read here, the answer needn't come again as a message.
            self.delegations.remove(&child.id);
            status["answer"] = json!(self.answer_of(&child.id));
        }
        Ok(status)
    }

    fn task_cancel(&mut self, caller: &str, arguments: &Value) -> Result<Value, String> {
        let child = self.task(caller, arguments)?;
        self.delegations.remove(&child.id);
        let running = self.turns.get(&child.id);
        if let Some(turn) = running {
            turn.running.interrupt();
        }
        let status = if running.is_some() {
            "cancel_requested"
        } else {
            "already_ended"
        };
        Ok(json!({ "taskId": child.id, "status": status }))
    }

    fn answer_of(&self, thread_id: &str) -> String {
        let answer = self
            .store
            .last_answer(thread_id)
            .ok()
            .flatten()
            .unwrap_or_default();
        match answer.char_indices().nth(ANSWER_MAX_CHARS) {
            Some((cut, _)) => format!("{}…", &answer[..cut]),
            None => answer,
        }
    }

    /// Once a delegated task's turn ends, its answer goes to the parent: into the turn it
    /// runs, or as the start of a new one.
    pub(crate) fn deliver_task_result(&mut self, child: &str, outcome: &TurnOutcome) {
        let Some(delegation) = self.delegations.remove(child) else {
            return;
        };
        let status = match outcome {
            TurnOutcome::Completed => "completed",
            TurnOutcome::Interrupted => "stopped",
            TurnOutcome::Failed { .. } => "failed",
        };
        let text = format!(
            "Delegated task {child} ({}) [{status}]:\n{}",
            delegation.title,
            self.answer_of(child)
        );
        let message_id = new_id();
        let now = now();
        let Ok(events) = send_task_result(
            &self.model,
            &delegation.parent,
            child,
            &message_id,
            &text,
            &now,
        ) else {
            return;
        };
        if let Err(error) = self.commit(&events, &now) {
            eprintln!("hoshi: failed to hand back a task's answer: {error}");
            return;
        }
        self.react(&events);
    }
}
