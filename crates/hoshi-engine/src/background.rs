//! Agent processes kept between turns for the commands they left running in the background,
//! such as a dev server, and `await_background_tasks`, by which the agent says which of its
//! commands its turn waits for instead.

use crate::{Notification, State, Turn, delegation, now};
use hoshi_core::{
    BackgroundTask, Event, ProviderOutput, ThreadBackgroundTasks, TurnOutcome, start_woken_turn,
};
use hoshi_providers::RunningTurn;
use serde_json::{Value, json};
use std::time::{Duration, SystemTime};

pub(crate) const TOOL: &str = "await_background_tasks";
const WAITING_NEXT: &str = "End your turn; it stays open until they finish.";
/// How long a kept process may go without reporting anything,
const IDLE_LIMIT: Duration = Duration::from_secs(30 * 60);
/// and while commands it runs in the background have not ended.
const BACKGROUND_IDLE_LIMIT: Duration = Duration::from_secs(4 * 60 * 60);
/// How long after its commands ended its agent may still wake for them.
const WAKE_GRACE: Duration = Duration::from_secs(5);

/// A process whose turn completed, kept for the commands it left running.
pub(crate) struct Lingering {
    pub process: u64,
    pub running: Box<dyn RunningTurn>,
    /// The commands it runs, once it reported any.
    pub background: Option<ThreadBackgroundTasks>,
    /// Its turn owed a delegated task's answer, so the turn it wakes for answers again.
    pub owes_answer: bool,
    /// As a delegated task, it holds back its answer until it wakes for the commands it
    /// awaits.
    pub holds_answer: bool,
    /// Its commands all ended, which shows once it wakes or ends: a task waiting on them
    /// would otherwise read as done for a moment.
    pub ended: bool,
    /// When it last reported anything, by the wall clock: a monotonic one stops while the
    /// computer sleeps on Linux and macOS, and the process would outlive its sign-in.
    pub heard_at: SystemTime,
}

impl Lingering {
    /// The agent awaits commands it runs: its turn ended, but its work did not.
    pub fn waits(&self) -> bool {
        self.background
            .as_ref()
            .is_some_and(|background| background.waiting)
    }

    fn quiet_too_long(&self, now: SystemTime) -> bool {
        let running = (!self.ended
            && self
                .background
                .as_ref()
                .is_some_and(|background| !background.tasks.is_empty()))
            || self.running.awaits_work();
        let limit = if running {
            BACKGROUND_IDLE_LIMIT
        } else {
            IDLE_LIMIT
        };
        now.duration_since(self.heard_at).unwrap_or_default() >= limit
    }
}

impl Lingering {
    /// Commands still run in it, which ending it would end too.
    pub fn runs_background_work(&self) -> bool {
        !self.ended
            && self
                .background
                .as_ref()
                .is_some_and(|background| !background.tasks.is_empty())
    }

    /// The commands it ran just ended, so its agent may wake for them.
    fn may_wake(&self) -> bool {
        self.ended
            && self
                .heard_at
                .elapsed()
                .is_ok_and(|silent| silent < WAKE_GRACE)
    }
}

impl State {
    /// Whether moving the thread to another provider would end what its agent left running, or
    /// cut the turn it is about to wake for.
    pub(crate) fn holds_off_switch(&self, thread_id: &str) -> bool {
        self.lingering
            .get(thread_id)
            .is_some_and(|lingering| lingering.runs_background_work() || lingering.may_wake())
    }

    fn lingering_of(&self, provider: &str) -> impl Iterator<Item = (&String, &Lingering)> {
        self.lingering.iter().filter(move |(thread_id, _)| {
            self.model
                .thread(thread_id)
                .is_some_and(|thread| thread.provider == provider)
        })
    }

    /// Whether agents of `provider` kept between turns still have work to end or answers to
    /// give.
    pub(crate) fn keeps_agents_busy(&self, provider: &str) -> bool {
        self.lingering_of(provider).any(|(_, lingering)| {
            lingering.runs_background_work()
                || lingering.may_wake()
                || lingering.owes_answer
                || lingering.holds_answer
        })
    }

    pub(crate) fn close_lingering_of(&mut self, provider: &str) {
        let threads: Vec<String> = self
            .lingering_of(provider)
            .map(|(thread_id, _)| thread_id.clone())
            .collect();
        for thread_id in threads {
            self.close_lingering(&thread_id);
        }
    }

    /// The agent process of the thread: its turn's, or the one kept between turns.
    fn agent_process(&self, thread_id: &str) -> Option<&dyn RunningTurn> {
        match self.turns.get(thread_id) {
            Some(turn) => Some(turn.running.as_ref()),
            None => self
                .lingering
                .get(thread_id)
                .map(|lingering| lingering.running.as_ref()),
        }
    }

    /// The thread moved to a worktree its agent made: its process may run the next turns
    /// there.
    pub(crate) fn agent_moved(&self, thread_id: &str, cwd: &str) {
        if let Some(process) = self.agent_process(thread_id) {
            process.moved_to(cwd);
        }
    }

    /// The commands the thread's agent still runs in the background.
    pub(crate) fn background_commands(&self, thread_id: &str) -> &[BackgroundTask] {
        let background = match self.turns.get(thread_id) {
            Some(turn) => turn.background.as_ref(),
            None => self
                .lingering
                .get(thread_id)
                .filter(|lingering| !lingering.ended)
                .and_then(|lingering| lingering.background.as_ref()),
        };
        background.map_or(&[], |background| background.tasks.as_slice())
    }

    /// When `task_id` started, in Unix milliseconds, if it is the only command the thread's
    /// agent runs in the background: whatever servers the agent started since are its.
    pub(crate) fn sole_background_command(&self, thread_id: &str, task_id: &str) -> Option<u64> {
        match self.background_commands(thread_id) {
            [only] if only.task_id == task_id => Some(only.started_at),
            _ => None,
        }
    }

    pub(crate) fn runs_background_command(&self, thread_id: &str, task_id: &str) -> bool {
        self.background_commands(thread_id)
            .iter()
            .any(|task| task.task_id == task_id)
    }

    /// Has the thread's agent end a command it runs in the background, which it then takes
    /// as stopped by the user and not as failed.
    pub(crate) fn stop_background_command(&self, thread_id: &str, task_id: &str) {
        if let Some(process) = self.agent_process(thread_id) {
            process.stop_background_task(task_id);
        }
    }

    /// What agent process `process` reports: for the turn it runs, or for its thread between
    /// turns. A process the thread no longer has is not heard.
    pub(crate) fn receive_from(&mut self, thread_id: &str, process: u64, output: ProviderOutput) {
        let turn_id = self
            .turns
            .get(thread_id)
            .filter(|turn| turn.process == process)
            .map(|turn| turn.id.clone());
        if let Some(turn_id) = turn_id {
            if output == ProviderOutput::Woke
                && let Some(turn) = self.turns.get_mut(thread_id)
                && std::mem::take(&mut turn.owes_answer)
            {
                self.reopen_task(thread_id);
            }
            self.receive(thread_id, &turn_id, output);
        } else if self
            .lingering
            .get(thread_id)
            .is_some_and(|lingering| lingering.process == process)
        {
            self.receive_between_turns(thread_id, output);
        }
    }

    fn receive_between_turns(&mut self, thread_id: &str, output: ProviderOutput) {
        if let Some(lingering) = self.lingering.get_mut(thread_id) {
            lingering.heard_at = SystemTime::now();
        }
        match output {
            ProviderOutput::BackgroundTasks { tasks, waiting } => {
                if tasks.is_empty() {
                    if let Some(lingering) = self.lingering.get_mut(thread_id) {
                        lingering.ended = true;
                    }
                    self.save_recovery(thread_id);
                    return;
                }
                let background = ThreadBackgroundTasks {
                    thread_id: thread_id.to_owned(),
                    tasks,
                    waiting,
                };
                if let Some(lingering) = self.lingering.get_mut(thread_id) {
                    lingering.background = Some(background.clone());
                }
                (self.notify)(Notification::BackgroundTasks(background));
                self.save_recovery(thread_id);
            }
            ProviderOutput::Woke => self.wake(thread_id),
            ProviderOutput::ProcessEnded => {
                if let Some(lingering) = self.lingering.remove(thread_id) {
                    self.clear_background(thread_id, lingering.background.as_ref());
                    self.answer_held(thread_id, &lingering);
                }
            }
            _ => {}
        }
    }

    /// The agent went on by itself after its turn, as a command it left running ended: a
    /// turn of its own, in the process it kept.
    fn wake(&mut self, thread_id: &str) {
        let turn_id = delegation::new_id();
        let now = now();
        let events = start_woken_turn(&self.model, thread_id, &turn_id, &now);
        if events.is_empty() {
            return;
        }
        if let Err(error) = self.commit(&events, &now) {
            eprintln!("hoshi: failed to start the turn the agent woke for: {error}");
            return;
        }
        let Some(lingering) = self.lingering.remove(thread_id) else {
            return;
        };
        if lingering.owes_answer {
            self.reopen_task(thread_id);
        }
        let background = if lingering.ended {
            self.clear_background(thread_id, lingering.background.as_ref());
            None
        } else {
            self.resumed_background(lingering.background)
        };
        let (workspace, checkpointed) = self
            .cwd(thread_id)
            .map(|cwd| self.turn_workspace(thread_id, &turn_id, &cwd))
            .unwrap_or_default();
        self.turns.insert(
            thread_id.to_owned(),
            Turn {
                id: turn_id,
                process: lingering.process,
                running: lingering.running,
                workspace,
                checkpointed,
                arrivals: 0,
                cursor: None,
                background,
                compacting: false,
                handoff: None,
                owes_answer: false,
            },
        );
    }

    /// Keeps the process of a turn that just completed while it runs commands in the
    /// background; otherwise clears what the turn showed running.
    pub(crate) fn keep_between_turns(&mut self, thread_id: &str, turn: Turn) {
        let Turn {
            process,
            running,
            background,
            ..
        } = turn;
        if !running.lingers() {
            self.clear_background(thread_id, background.as_ref());
            return;
        }
        let shown = self
            .model
            .thread(thread_id)
            .is_some_and(|thread| !thread.archived);
        if !shown {
            running.close();
            self.clear_background(thread_id, background.as_ref());
            return;
        }
        let owes_answer = self.delegations.contains_key(thread_id);
        self.lingering.insert(
            thread_id.to_owned(),
            Lingering {
                process,
                running,
                background,
                owes_answer,
                holds_answer: false,
                ended: false,
                heard_at: SystemTime::now(),
            },
        );
    }

    /// Ends the process a thread's completed turn kept, and the commands running in it.
    pub(crate) fn close_lingering(&mut self, thread_id: &str) {
        if let Some(lingering) = self.lingering.remove(thread_id) {
            self.answer_held(thread_id, &lingering);
            self.end_lingering(thread_id, lingering);
        }
    }

    /// A delegated task whose agent awaited commands answers once it woke for them; when its
    /// process ends before that, it answers as stopped with what it said last. Quitting
    /// leaves the answer to the turn that goes on after the restart.
    fn answer_held(&mut self, thread_id: &str, lingering: &Lingering) {
        if lingering.holds_answer && !self.turns.contains_key(thread_id) && !self.recovery.quitting
        {
            self.hand_back(thread_id, &TurnOutcome::Interrupted);
        }
    }

    /// The commands a kept process runs, as the turn that now runs in it shows them: no
    /// longer only waited for.
    pub(crate) fn resumed_background(
        &self,
        background: Option<ThreadBackgroundTasks>,
    ) -> Option<ThreadBackgroundTasks> {
        background.map(|mut background| {
            if background.waiting {
                background.waiting = false;
                (self.notify)(Notification::BackgroundTasks(background.clone()));
            }
            background
        })
    }

    pub(crate) fn end_lingering(&self, thread_id: &str, lingering: Lingering) {
        lingering.running.close();
        self.clear_background(thread_id, lingering.background.as_ref());
    }

    /// Ends the processes of threads that are gone, archived or moved to another session.
    pub(crate) fn close_lingering_after(&mut self, events: &[Event]) {
        for event in events {
            match event {
                Event::ThreadDeleted { thread_id }
                | Event::ThreadArchivedSet {
                    thread_id,
                    archived: true,
                }
                | Event::ThreadRewound { thread_id, .. }
                | Event::ThreadWorktreeReleased { thread_id }
                | Event::ThreadCleanedUp { thread_id } => self.close_lingering(thread_id),
                _ => {}
            }
        }
    }

    /// Ends the process kept for `thread_id` when it went quiet for longer than it may.
    pub(crate) fn close_lingering_if_idle(&mut self, thread_id: &str, now: SystemTime) {
        if self
            .lingering
            .get(thread_id)
            .is_some_and(|lingering| lingering.quiet_too_long(now))
        {
            self.close_lingering(thread_id);
        }
    }

    /// Ends the processes kept between turns that went quiet for longer than they may.
    pub(crate) fn close_idle_lingering(&mut self, now: SystemTime) {
        let quiet: Vec<String> = self
            .lingering
            .iter()
            .filter(|(_, lingering)| lingering.quiet_too_long(now))
            .map(|(thread_id, _)| thread_id.clone())
            .collect();
        for thread_id in quiet {
            self.close_lingering(&thread_id);
        }
    }

    pub(crate) fn close_all_lingering(&mut self) {
        let threads: Vec<String> = self.lingering.keys().cloned().collect();
        for thread_id in threads {
            self.close_lingering(&thread_id);
        }
    }

    pub(crate) fn clear_background(
        &self,
        thread_id: &str,
        background: Option<&ThreadBackgroundTasks>,
    ) {
        self.save_recovery(thread_id);
        if background.is_some_and(|background| !background.tasks.is_empty()) {
            (self.notify)(Notification::BackgroundTasks(ThreadBackgroundTasks {
                thread_id: thread_id.to_owned(),
                tasks: Vec::new(),
                waiting: false,
            }));
        }
    }

    /// `await_background_tasks`: the turn waits for these commands, or no longer does.
    pub(crate) fn await_background_tasks(
        &self,
        caller: &str,
        arguments: &Value,
    ) -> Result<Value, String> {
        let task_ids: Vec<String> = arguments["backgroundTaskIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|task_id| !task_id.is_empty())
            .map(str::to_owned)
            .collect();
        if task_ids.is_empty() {
            return Err(
                "`backgroundTaskIds` is empty: pass the id each background Bash or Monitor command reported"
                    .into(),
            );
        }
        let waiting = arguments["waiting"].as_bool().unwrap_or(true);
        let turn = self
            .turns
            .get(caller)
            .ok_or("this thread's turn isn't running")?;
        let running = turn
            .running
            .await_background_tasks(&task_ids, waiting)
            .ok_or("this thread's agent can't wait for background commands")?;
        let not_running: Vec<&String> = task_ids
            .iter()
            .filter(|task_id| !running.contains(task_id))
            .collect();
        let mut result = json!({ "waiting": waiting, "backgroundTaskIds": running });
        if !not_running.is_empty() {
            result["notRunning"] = json!(not_running);
        }
        if waiting && !running.is_empty() {
            result["next"] = json!(WAITING_NEXT);
        }
        Ok(result)
    }
}
