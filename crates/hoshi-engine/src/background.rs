//! Agent processes kept between turns for the commands they left running in the background,
//! such as a dev server, and `await_background_tasks`, by which the agent says which of its
//! commands its turn waits for instead.

use crate::{Notification, State, Turn, delegation, now};
use hoshi_core::{Event, ProviderOutput, ThreadBackgroundTasks, start_woken_turn};
use hoshi_providers::RunningTurn;
use serde_json::{Value, json};

pub(crate) const TOOL: &str = "await_background_tasks";
const WAITING_NEXT: &str = "End your turn; it stays open until they finish.";

/// A process whose turn completed, kept for the commands it left running.
pub(crate) struct Lingering {
    pub process: u64,
    pub running: Box<dyn RunningTurn>,
    /// The commands it runs, once it reported any.
    pub background: Option<ThreadBackgroundTasks>,
    /// Its turn owed a delegated task's answer, so the turn it wakes for answers again.
    pub owes_answer: bool,
    /// Its commands all ended, which shows once it wakes or ends: a task waiting on them
    /// would otherwise read as done for a moment.
    pub ended: bool,
}

impl State {
    /// What agent process `process` reports: for the turn it runs, or for its thread between
    /// turns. A process the thread no longer has is not heard.
    pub(crate) fn receive_from(&mut self, thread_id: &str, process: u64, output: ProviderOutput) {
        let turn_id = self
            .turns
            .get(thread_id)
            .filter(|turn| turn.process == process)
            .map(|turn| turn.id.clone());
        if let Some(turn_id) = turn_id {
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
        match output {
            ProviderOutput::BackgroundTasks { tasks, waiting } => {
                if tasks.is_empty() {
                    if let Some(lingering) = self.lingering.get_mut(thread_id) {
                        lingering.ended = true;
                    }
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
            }
            ProviderOutput::Woke => self.wake(thread_id),
            ProviderOutput::ProcessEnded => {
                if let Some(lingering) = self.lingering.remove(thread_id) {
                    self.clear_background(thread_id, lingering.background.as_ref());
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
            lingering.background
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
                ended: false,
            },
        );
    }

    /// Ends the process a thread's completed turn kept, and the commands running in it.
    pub(crate) fn close_lingering(&mut self, thread_id: &str) {
        if let Some(lingering) = self.lingering.remove(thread_id) {
            self.end_lingering(thread_id, lingering);
        }
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

    pub(crate) fn close_all_lingering(&mut self) {
        let threads: Vec<String> = self.lingering.keys().cloned().collect();
        for thread_id in threads {
            self.close_lingering(&thread_id);
        }
    }

    fn clear_background(&self, thread_id: &str, background: Option<&ThreadBackgroundTasks>) {
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
