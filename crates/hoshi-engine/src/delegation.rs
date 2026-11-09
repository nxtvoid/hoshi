//! Tasks a thread's agent hands to child threads through hoshi's MCP server, and their
//! answers, which come back to the parent as messages.

use crate::{State, WorkspaceOp, WorktreePlan, lock, now};
use hoshi_core::{
    Command, CommandError, Effort, ModelInfo, ThreadStatus, ThreadWorkspace, TurnOutcome,
    record_delegation, send_task_result,
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, Weak};
use std::thread;

/// What the agent reads with every running task: its answer comes on its own.
/// Kept short: a page of the transcript keeps only the start of a tool's output, which must
/// still hold the task id.
const NEXT: &str = "End your turn now; the answer arrives here as a message.";
/// What the agent reads of a task whose plan the user is still changing.
const PLANNING: &str =
    "The user is changing its plan in the task's thread; the answer arrives here as a message.";
/// An answer longer than this is cut; `task_status` still has it all.
const ANSWER_MAX_CHARS: usize = 16_000;
const TITLE_MAX_CHARS: usize = 60;

/// A `delegate_task` call checked under the lock, waiting for its worktree if it has one.
pub(crate) struct Delegating {
    child: String,
    title: String,
    task: String,
    create: Option<Command>,
    /// Works in its parent's worktree.
    shared: bool,
    /// Its own worktree, to create outside the lock.
    worktree: Option<(WorktreePlan, WorkspaceOp)>,
}

/// `delegate_task` from the MCP server: the task's own worktree is created outside the lock,
/// so other threads go on meanwhile.
pub(crate) fn delegate_task(
    state: &Weak<Mutex<State>>,
    caller: &str,
    arguments: &Value,
) -> Result<Value, String> {
    let shared = state.upgrade().ok_or("hoshi is closing")?;
    let mut delegating = lock(&shared).plan_delegation(caller, arguments)?;
    let created = delegating.worktree.as_ref().map(|(plan, _)| plan.create());
    lock(&shared).finish_delegation(caller, &mut delegating, created)
}

/// How many threads deep tasks can go: a thread delegates, and its tasks can delegate once
/// more.
pub(crate) const MAX_DEPTH: usize = 2;

/// Added to the system prompt of a delegated task's agent, after [`INSTRUCTIONS`].
pub(crate) const TASK_INSTRUCTIONS: &str =
    "You are a delegated task: do the work yourself. Delegate only what your task asks you to.";

/// Added to the system prompt of agents that can delegate.
pub(crate) const INSTRUCTIONS: &str = "hoshi, the app running you, gives you tools on its MCP server (load one with ToolSearch, \"select:mcp__hoshi__<name>\"). Ordinary coding needs none. Before you improvise around a missing credential, CLI or browser, check whether one of them covers it.\n- Subagents: your own Agent tool with run_in_background: false, and wait; start several in one message to run them in parallel. Never a background Agent: work that should go on in the background, as a thread of its own or in its own checkout and branch (isolation: \"worktree\"), goes to delegate_task.\n- delegate_task gives the child only `task`: include everything it needs. It returns at once and the answer arrives later as a message here, so end your turn; don't wait, sleep or poll task_status. task_cancel stops a task.\n- A token, password, PIN or pairing code only the user can give: request_secret, never a chat ask, and never search files or the environment for one. Use the file it returns only inside commands; never print it.\n- The user's other threads: thread_list finds them, thread_read reads one (always before you answer about it), thread_send messages one, thread_wait waits for its turn.\n- Asked to monitor or babysit a pull request, or when your work still waits on its checks or reviews: push, call watch_pull_request, end your turn; never sleep, loop or poll. Call unwatch_pull_request before you hand the work back. Delegated tasks never watch.\n- To check web work: preview_open in your thread's own browser, which the user sees in hoshi; preview_snapshot reads it, preview_act uses it. Don't start another browser.\n- Your turn ends without waiting for commands you run in the background (Bash run_in_background, Monitor), which keep running after it. To wait for one you started, such as tests or a build, call await_background_tasks with its task id before you end your turn; never for work you leave running, such as a dev server or a watcher.";

/// Added after [`INSTRUCTIONS`] for agents of threads on Bitbucket.
pub(crate) const BITBUCKET_INSTRUCTIONS: &str = "- This repository is on Bitbucket: open its pull requests with create_pull_request, which signs in as the user. Never look for, ask for or use Bitbucket tokens, nor call its API yourself. If it fails, tell the user what it says.";

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

/// The slug of the model `wanted` names among `models`: by slug or name, or as a family,
/// such as "haiku", whose newest model the catalog lists first.
fn pick_model(models: &[ModelInfo], wanted: &str) -> Result<String, String> {
    if models.is_empty() {
        return Ok(wanted.to_owned());
    }
    let family = wanted.to_lowercase();
    models
        .iter()
        .find(|model| {
            [&model.slug, &model.name, &model.short_name]
                .iter()
                .any(|name| name.eq_ignore_ascii_case(wanted))
        })
        .or_else(|| {
            models.iter().find(|model| {
                !model.legacy
                    && !model.update_required
                    && model.slug.to_lowercase().contains(&family)
            })
        })
        .map(|model| model.slug.clone())
        .ok_or_else(|| {
            let known: Vec<_> = models.iter().map(|model| model.slug.as_str()).collect();
            format!(
                "there is no model {wanted}; use one of {}",
                known.join(", ")
            )
        })
}

impl State {
    /// How many delegations led to `thread_id`: 0 for a thread the user started.
    pub(crate) fn depth(&self, thread_id: &str) -> usize {
        let mut depth = 0;
        let mut current = self.model.thread(thread_id);
        while let Some(parent) = current
            .and_then(|thread| thread.parent_thread_id.as_deref())
            .and_then(|parent| self.model.thread(parent))
        {
            depth += 1;
            current = Some(parent);
        }
        depth
    }

    /// The tasks whose answer hasn't gone back yet, as the store has them after a restart.
    pub(crate) fn pending_delegations(&self) -> HashMap<String, Delegation> {
        let answered: HashSet<String> = self
            .store
            .answered_tasks()
            .unwrap_or_else(|error| {
                eprintln!("hoshi: failed to read the answered tasks: {error}");
                Vec::new()
            })
            .into_iter()
            .collect();
        self.model
            .threads()
            .filter(|thread| {
                !answered.contains(&thread.id) && !thread.task_cancelled && !thread.task_answered
            })
            .filter_map(|thread| {
                let parent = thread.parent_thread_id.clone()?;
                self.model.thread(&parent)?;
                Some((
                    thread.id.clone(),
                    Delegation {
                        parent,
                        title: thread.title.clone(),
                    },
                ))
            })
            .collect()
    }

    /// The slug of the model the agent named, by slug, name or family.
    fn model_slug(&self, wanted: &str) -> Result<String, String> {
        match &self.adapter {
            Some(adapter) => pick_model(&adapter.models().models, wanted),
            None => Ok(wanted.to_owned()),
        }
    }

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
            "thread_list" => self.list_threads(caller, arguments),
            "thread_read" => self.read_thread(arguments),
            "thread_send" => self.send_to_thread(caller, arguments),
            "watch_pull_request" => self.watch_pull_request(caller),
            "unwatch_pull_request" => self.unwatch_pull_request(caller),
            crate::background::TOOL => self.await_background_tasks(caller, arguments),
            _ => Err(format!("hoshi has no tool named {tool}")),
        }
    }

    /// `delegate_task` under the lock throughout; [`delegate_task`] creates a task's own
    /// worktree outside it.
    fn delegate_task(&mut self, caller: &str, arguments: &Value) -> Result<Value, String> {
        let mut delegating = self.plan_delegation(caller, arguments)?;
        let created = delegating.worktree.as_ref().map(|(plan, _)| plan.create());
        self.finish_delegation(caller, &mut delegating, created)
    }

    /// Checks a `delegate_task` call and settles what its task is; a task in a worktree of
    /// its own also gets where that goes, reserved.
    fn plan_delegation(&mut self, caller: &str, arguments: &Value) -> Result<Delegating, String> {
        let parent = self
            .model
            .thread(caller)
            .cloned()
            .ok_or("this thread no longer exists")?;
        if parent.active_turn_id.is_none() {
            return Err("delegate_task works only while this thread's turn runs".into());
        }
        if self.depth(caller) >= MAX_DEPTH {
            return Err("this task is too deep to delegate further: do the work yourself".into());
        }
        let model = match arguments["model"]
            .as_str()
            .map(str::trim)
            .filter(|model| !model.is_empty())
        {
            Some(wanted) => Some(self.model_slug(wanted)?),
            None => parent.model.clone(),
        };
        let effort = match arguments.get("effort").filter(|effort| !effort.is_null()) {
            Some(effort) => Some(
                serde_json::from_value::<Effort>(effort.clone())
                    .map_err(|_| "`effort` is one of low, medium, high, xhigh, max")?,
            ),
            None => parent.effort,
        };
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
        let worktree = match (&workspace, parent.project_id.as_deref()) {
            (ThreadWorkspace::NewWorktree, Some(project)) => {
                let plan = self
                    .plan_worktree(&child, project, None)
                    .map_err(|error| error.to_string())?;
                let path = plan.path.to_string_lossy().into_owned();
                let op = self
                    .begin_op(&child, &[&path])
                    .map_err(|error| error.to_string())?;
                Some((plan, op))
            }
            _ => None,
        };
        let create = Command::ThreadCreate {
            thread_id: child.clone(),
            project_id: parent.project_id.clone(),
            title: title.clone(),
            workspace,
            model,
            effort,
            fast_mode: parent.fast_mode,
            ultracode: parent.ultracode,
            access_mode: parent.access_mode,
            plan_mode: arguments["plan"].as_bool().unwrap_or(false),
        };
        Ok(Delegating {
            child,
            title,
            task: task.to_owned(),
            create: Some(create),
            shared,
            worktree,
        })
    }

    /// Creates the task's thread, in the worktree `created` made if it has its own, and
    /// sends it its task.
    fn finish_delegation(
        &mut self,
        caller: &str,
        delegating: &mut Delegating,
        created: Option<Result<ThreadWorkspace, CommandError>>,
    ) -> Result<Value, String> {
        let Delegating {
            child,
            title,
            task,
            shared,
            ..
        } = delegating;
        let (child, title, task) = (child.clone(), title.clone(), task.clone());
        let Some(mut create) = delegating.create.take() else {
            return Err("this task was already delegated".into());
        };
        let worktree = delegating.worktree.take();
        let op = match (worktree, created) {
            (Some((plan, op)), Some(Ok(made))) => {
                if let Command::ThreadCreate { workspace, .. } = &mut create {
                    *workspace = made;
                }
                Some((plan, op))
            }
            (Some((_, op)), Some(Err(error))) => {
                self.end_op(op, None).map_err(|error| error.to_string())?;
                return Err(error.to_string());
            }
            _ => None,
        };
        // The parent's worktree, or the one made above, is already there.
        let created = if *shared || op.is_some() {
            self.run_prepared(create)
        } else {
            self.run_command(create)
        };
        let op = match (created, op) {
            (Ok(()), Some((_, op))) => {
                self.start_worktree_setup(&child);
                Some(op)
            }
            (Ok(()), None) => None,
            (Err(error), op) => {
                if let Some((plan, op)) = op {
                    thread::spawn(move || plan.discard());
                    self.end_op(op, None).map_err(|error| error.to_string())?;
                }
                return Err(error.to_string());
            }
        };
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
        let send = Command::MessageSend {
            thread_id: child.clone(),
            message_id: new_id(),
            text: task,
            attachments: Vec::new(),
        };
        match op {
            Some(op) => self.end_op(op, Some(send)),
            None => self.run_command(send),
        }
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
        let planning = child.plan_mode && self.delegations.contains_key(&child.id);
        if child.active_turn_id.is_some() {
            status["next"] = json!(NEXT);
        } else if planning {
            status["status"] = json!("waiting_for_user");
            status["next"] = json!(PLANNING);
        } else {
            // Read here, the answer needn't come again as a message.
            self.delegations.remove(&child.id);
            let now = now();
            let answered = hoshi_core::answer_task(&self.model, &child.id);
            if let Err(error) = self.commit(&answered, &now) {
                eprintln!("hoshi: failed to record a read answer: {error}");
            }
            status["answer"] = json!(self.answer_of(&child.id));
        }
        Ok(status)
    }

    fn task_cancel(&mut self, caller: &str, arguments: &Value) -> Result<Value, String> {
        let child = self.task(caller, arguments)?;
        let status = if self.cancel_task(&child.id) {
            "cancel_requested"
        } else {
            "already_ended"
        };
        Ok(json!({ "taskId": child.id, "status": status }))
    }

    /// Cancels a delegated task: it stops if it runs, and its answer never goes back.
    /// `true` when it was running.
    pub(crate) fn cancel_task(&mut self, child: &str) -> bool {
        self.delegations.remove(child);
        let now = now();
        let events = hoshi_core::cancel_task(&self.model, child);
        if let Err(error) = self.commit(&events, &now) {
            eprintln!("hoshi: failed to record a cancelled task: {error}");
        }
        let running = self.turns.get(child);
        if let Some(turn) = running {
            turn.running.interrupt();
        }
        running.is_some()
    }

    fn answer_of(&self, thread_id: &str) -> String {
        let said = self
            .store
            .last_answer(thread_id)
            .ok()
            .flatten()
            .unwrap_or_default();
        let planning = self
            .model
            .thread(thread_id)
            .is_some_and(|thread| thread.plan_mode);
        let plan = planning
            .then(|| self.store.last_plan(thread_id).ok().flatten())
            .flatten();
        let answer = match plan {
            Some(plan) => format!("{said}\n\nThe plan, not approved yet:\n{plan}"),
            None => said,
        };
        match answer.char_indices().nth(ANSWER_MAX_CHARS) {
            Some((cut, _)) => format!("{}…", &answer[..cut]),
            None => answer,
        }
    }

    /// Once a delegated task's turn ends, its answer goes to the parent: into the turn it
    /// runs, or as the start of a new one.
    pub(crate) fn deliver_task_result(&mut self, child: &str, outcome: &TurnOutcome) {
        // The user sent the task's plan back: it answers once they approve one or stop it.
        if *outcome == TurnOutcome::Completed
            && self.store.plan_declined_last(child).unwrap_or(false)
        {
            return;
        }
        self.hand_back(child, outcome);
    }

    /// Gives a delegated task's answer to its parent: into the turn it runs, or as the start
    /// of a new one.
    pub(crate) fn hand_back(&mut self, child: &str, outcome: &TurnOutcome) {
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
        let Ok(mut events) = send_task_result(
            &self.model,
            &delegation.parent,
            child,
            &message_id,
            &text,
            &now,
        ) else {
            return;
        };
        events.extend(hoshi_core::answer_task(&self.model, child));
        if let Err(error) = self.commit(&events, &now) {
            eprintln!("hoshi: failed to hand back a task's answer: {error}");
            return;
        }
        self.react(&events);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(slug: &str, name: &str, legacy: bool) -> ModelInfo {
        ModelInfo {
            slug: slug.into(),
            name: name.into(),
            short_name: name.into(),
            legacy,
            is_new: false,
            update_required: false,
            efforts: Vec::new(),
            default_effort: None,
            fast_mode: false,
            ultracode: false,
        }
    }

    #[test]
    fn a_model_is_found_by_slug_name_or_family() {
        let models = [
            model("claude-opus-5-5", "Opus 5.5", false),
            model("claude-haiku-5-5", "Haiku 5.5", false),
            model("claude-haiku-4-5", "Haiku 4.5", true),
        ];

        assert_eq!(
            pick_model(&models, "Haiku 4.5").unwrap(),
            "claude-haiku-4-5"
        );
        assert_eq!(
            pick_model(&models, "claude-opus-5-5").unwrap(),
            "claude-opus-5-5"
        );
        assert_eq!(pick_model(&models, "haiku").unwrap(), "claude-haiku-5-5");
        assert!(
            pick_model(&models, "gpt")
                .unwrap_err()
                .contains("claude-opus-5-5")
        );
    }
}
