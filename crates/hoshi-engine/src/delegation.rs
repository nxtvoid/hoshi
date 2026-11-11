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
pub(crate) const NEXT: &str =
    "Its answer arrives here as a message, so end your turn. Don't wait on it or poll it.";
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

/// Added to the system prompt of a delegated task's agent, after the rest of [`instructions`].
pub(crate) const TASK_INSTRUCTIONS: &str = "You are a delegated task: do the work yourself; delegate only what it asks. Await or stop your background commands before your last message, unless told not to.";

/// When to use each of hoshi's tools; their descriptions say how.
const TOOLS: &str = "- Subagents: your Agent tool, run_in_background false. An agent or work in the background, however the user words it: delegate_task, never Agent with run_in_background. Work in a branch or checkout of its own, as each of several variants to compare: delegate_task with isolation \"worktree\", one per branch, however small; never git worktree or a second checkout yourself. Then end your turn; never poll task_status, sleep, Monitor or ScheduleWakeup.\n\
- A credential only the user has: request_secret.\n\
- The user's other hoshi threads: thread_list, thread_read (before you answer about one), thread_send. Not ListAgents or SendMessage.";

/// How answers show the files the agent made, which the user sees only through them.
const FILES: &str = "- The user sees your replies, not what your tools read. To show an image, embed it with Markdown and its absolute path: ![name](/abs/path.png). Link other files by their absolute path.";

/// For threads the user started, which own their pull request.
const PULL_REQUESTS: &str = "- Asked to babysit a pull request, or your work waits on its checks or reviews: watch_pull_request, then end your turn.\n\
- Asked to move to an existing worktree: EnterWorktree with its path, never cd.";

const BROWSER_AND_BACKGROUND: &str = "- Web previews when asked for browser work, or to see or reproduce something in a live app: preview_open, preview_snapshot, preview_act. Never to verify your own edit unasked. No other browser.\n\
- A visual the user asks for (chart, diagram, mockup, visualizer): an HTML file's path, or inline HTML, to html_preview, then html_render. A screenshot on disk or a text description doesn't show it; never start a server for it. An image to post or send elsewhere: a .png file and its path, never html_render.\n\
- Background Bash or Monitor commands don't hold your turn. To wait for one (tests, a build, a sleep, which only runs in the background): await_background_tasks with its id, then end your turn. Never for dev servers.";

/// Added after the rest of [`instructions`] for agents of threads on Bitbucket.
pub(crate) const BITBUCKET_INSTRUCTIONS: &str = "- On Bitbucket, open pull requests with create_pull_request, signed in as the user. Never look for or use a Bitbucket token, nor call its API yourself. If it fails, tell the user what it says.";

/// Added to the system prompt of agents that have hoshi's tools, of a thread `depth`
/// delegations deep.
/// `in_project` is false for a chat, which has no repository to open pull requests on.
pub(crate) fn instructions(depth: usize, in_project: bool, bitbucket: bool) -> String {
    let mut parts = vec![
        format!(
            "hoshi, the app running you, has tools on its MCP server{}. Ordinary coding needs none.",
            hoshi_providers::DEFERRED_TOOLS
        ),
        TOOLS.to_owned(),
        FILES.to_owned(),
    ];
    if depth == 0 && in_project {
        parts.push(PULL_REQUESTS.to_owned());
    }
    parts.push(BROWSER_AND_BACKGROUND.to_owned());
    if bitbucket {
        parts.push(BITBUCKET_INSTRUCTIONS.to_owned());
    }
    if depth > 0 {
        parts.push(TASK_INSTRUCTIONS.to_owned());
    }
    parts.join("\n")
}

/// A task a thread delegated, until its answer goes back.
pub(crate) struct Delegation {
    pub parent: String,
    pub title: String,
    /// For a task that works again after it answered: what it said then.
    pub previous_answer: Option<String>,
    /// Read back at startup: the restart cut it short, and it answers once the user has
    /// it finish.
    pub restored: bool,
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
                        previous_answer: None,
                        restored: true,
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

    /// Whether the agent of thread `caller` is offered a tool only some threads have.
    pub(crate) fn offers_tool(&self, caller: &str, tool: &str) -> bool {
        match tool {
            crate::pull_request_tool::TOOL => self.opens_pull_requests(caller),
            "watch_pull_request" | "unwatch_pull_request" => {
                self.model.thread(caller).is_some_and(|thread| {
                    thread.parent_thread_id.is_none() && thread.project_id.is_some()
                })
            }
            _ => true,
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
        if isolated && parent.project_id.is_none() {
            return Err("a chat has no repository: leave `isolation` out".into());
        }
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
                previous_answer: None,
                restored: false,
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

    /// Whether a task's answer is on its way: owed, and not left behind by a restart with
    /// nothing running to give it.
    pub(crate) fn awaited(&self, thread_id: &str) -> bool {
        self.delegations.get(thread_id).is_some_and(|delegation| {
            !delegation.restored
                || self.turns.contains_key(thread_id)
                || self
                    .model
                    .thread(thread_id)
                    .is_some_and(|thread| thread.plan_mode)
        })
    }

    /// Whether a task that answered will answer again: a command its answer left running
    /// still goes on, or a task of its own is in flight.
    fn answers_again(&self, thread_id: &str) -> bool {
        let cancelled = self
            .model
            .thread(thread_id)
            .is_none_or(|thread| thread.task_cancelled);
        !cancelled
            && (self
                .lingering
                .get(thread_id)
                .is_some_and(|lingering| lingering.owes_answer)
                || self.model.threads().any(|task| {
                    task.parent_thread_id.as_deref() == Some(thread_id) && self.in_flight(&task.id)
                }))
    }

    /// Whether a task is still to answer the thread that delegated it.
    fn in_flight(&self, thread_id: &str) -> bool {
        self.awaited(thread_id) || self.answers_again(thread_id)
    }

    /// The user stopped a task: it is cancelled, and the thread still waiting for its answer
    /// hears so, to go on without it.
    pub(crate) fn stop_task(&mut self, child: &str) {
        let awaited = self.in_flight(child);
        let delegated = self
            .model
            .thread(child)
            .and_then(|thread| Some((thread.parent_thread_id.clone()?, thread.title.clone())));
        self.cancel_task(child);
        let Some((parent, title)) = delegated.filter(|_| awaited) else {
            return;
        };
        let text = format!(
            "Delegated task {child} ({title}) [stopped]:\n{STOPPED_BY_USER}\n\n{}",
            self.what_next(child, &parent)
        );
        self.reopen_task(&parent);
        let now = now();
        let Ok(events) = send_task_result(&self.model, &parent, child, &new_id(), &text, &now)
        else {
            return;
        };
        if let Err(error) = self.commit(&events, &now) {
            eprintln!("hoshi: failed to tell of a stopped task: {error}");
            return;
        }
        self.react(&events);
    }

    /// Cancels a delegated task: it stops if it runs, with the commands it left running and
    /// the tasks of its own in flight, and its answer never goes back. `true` when anything
    /// was running.
    pub(crate) fn cancel_task(&mut self, child: &str) -> bool {
        let own: Vec<String> = self
            .model
            .threads()
            .filter(|task| {
                task.parent_thread_id.as_deref() == Some(child) && self.in_flight(&task.id)
            })
            .map(|task| task.id.clone())
            .collect();
        let mut stopped = false;
        for task in own {
            stopped |= self.cancel_task(&task);
        }
        self.delegations.remove(child);
        let now = now();
        let events = hoshi_core::cancel_task(&self.model, child);
        if let Err(error) = self.commit(&events, &now) {
            eprintln!("hoshi: failed to record a cancelled task: {error}");
        }
        let kept = self.lingering.contains_key(child);
        self.close_lingering(child);
        let running = self.turns.get(child);
        if let Some(turn) = running {
            turn.running.interrupt();
        }
        running.is_some() || kept || stopped
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

    /// An answered task that works again, as its agent woke for a command its answer left
    /// running or its own task answered it: the turn it runs answers its parent again.
    pub(crate) fn reopen_task(&mut self, child: &str) {
        if self.delegations.contains_key(child) {
            return;
        }
        let events = hoshi_core::reopen_task(&self.model, child);
        let Some(thread) = self.model.thread(child).filter(|_| !events.is_empty()) else {
            return;
        };
        let delegation = Delegation {
            parent: thread.parent_thread_id.clone().unwrap_or_default(),
            title: thread.title.clone(),
            previous_answer: self.store.last_answer(child).ok().flatten(),
            restored: false,
        };
        if let Err(error) = self.commit(&events, &now()) {
            eprintln!("hoshi: failed to reopen a task: {error}");
            return;
        }
        self.delegations.insert(child.to_owned(), delegation);
    }

    /// Gives a delegated task's answer to its parent: into the turn it runs, or as the start
    /// of a new one.
    pub(crate) fn hand_back(&mut self, child: &str, outcome: &TurnOutcome) {
        let Some(delegation) = self.delegations.remove(child) else {
            return;
        };
        let now = now();
        let said = self.store.last_answer(child).ok().flatten();
        // A reopened task that woke and said nothing new has nothing more to tell.
        if *outcome == TurnOutcome::Completed
            && delegation.previous_answer.is_some()
            && said == delegation.previous_answer
        {
            let answered = hoshi_core::answer_task(&self.model, child);
            if let Err(error) = self.commit(&answered, &now) {
                eprintln!("hoshi: failed to record a task's answer: {error}");
            }
            return;
        }
        let status = match outcome {
            TurnOutcome::Completed => "completed",
            TurnOutcome::Interrupted => "stopped",
            TurnOutcome::Failed { .. } => "failed",
        };
        let text = format!(
            "Delegated task {child} ({}) [{status}]:\n{}\n\n{}",
            delegation.title,
            self.answer_of(child),
            self.what_next(child, &delegation.parent)
        );
        // The parent is itself a task that already answered: what it says now goes up too.
        self.reopen_task(&delegation.parent);
        let message_id = new_id();
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

    /// What the parent should do with a task's answer, from how its other tasks stand.
    fn what_next(&self, child: &str, parent: &str) -> String {
        let others = self
            .model
            .threads()
            .filter(|thread| {
                thread.id != child
                    && thread.parent_thread_id.as_deref() == Some(parent)
                    && self.in_flight(&thread.id)
            })
            .count();
        let mut next = Vec::new();
        if self.answers_again(child) {
            next.push(ANSWERS_AGAIN.to_owned());
        }
        next.push(match others {
            0 if next.is_empty() => ALL_ANSWERED.to_owned(),
            0 => ONE_LEFT.to_owned(),
            1 => format!("1 other task still runs. {ONE_LINE}"),
            n => format!("{n} other tasks still run. {ONE_LINE}"),
        });
        next.join(" ")
    }
}

/// After a task's answer, for its parent.
pub(crate) const ANSWERS_AGAIN: &str =
    "It isn't finished: it answers again once the command or task it still waits on ends.";
const ONE_LINE: &str = "Reply in one line about this task only, in the user's language; summarize once all have answered.";
const ONE_LEFT: &str =
    "Reply in one line, in the user's language; summarize once it answers again.";
pub(crate) const ALL_ANSWERED: &str = "None of the tasks you delegated is still running.";
pub(crate) const STOPPED_BY_USER: &str =
    "The user stopped this task before it answered. Don't start it again unless they ask.";

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
    fn the_instructions_name_only_the_tools_a_thread_has() {
        assert!(instructions(0, true, false).contains("watch_pull_request"));
        assert!(!instructions(0, false, false).contains("watch_pull_request"));
        assert!(!instructions(1, true, false).contains("watch_pull_request"));
        assert!(instructions(1, true, false).ends_with(TASK_INSTRUCTIONS));
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
