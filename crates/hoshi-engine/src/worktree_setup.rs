//! The project's setup command, run in a terminal of its own in each worktree hoshi makes,
//! with the thread's messages held meanwhile when the project asks to wait for it.

use crate::terminal::{Launch, Terminals};
use crate::{Notification, Notify};
use hoshi_core::{Command, CommandError, TerminalEvent, WorktreeSetupRun, WorktreeSetupStatus};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Lines of output the thread shows while the command runs.
const OUTPUT_LINES: usize = 4;
const LINE_MAX_CHARS: usize = 300;
/// Output can stream fast, as from an install; the view needs only a glimpse of it.
const NOTIFY_EVERY: Duration = Duration::from_millis(200);

/// Where a setup runs, kept to run it again.
#[derive(Clone)]
pub struct SetupLaunch {
    pub thread_id: String,
    pub project_root: String,
    pub worktree_path: String,
    pub command: String,
    pub wait: bool,
}

struct Run {
    launch: SetupLaunch,
    status: WorktreeSetupStatus,
    exit_code: Option<u32>,
    output: Vec<String>,
    /// The line being written, until its newline comes.
    partial: String,
    held: Vec<Command>,
    notified_at: Option<Instant>,
    /// Tells this run from an earlier one in the same terminal, still exiting.
    generation: u64,
}

impl Run {
    fn view(&self) -> WorktreeSetupRun {
        let mut output = self.output.clone();
        let partial = clean_line(&self.partial);
        if !partial.is_empty() {
            output.push(partial);
        }
        let skip = output.len().saturating_sub(OUTPUT_LINES);
        WorktreeSetupRun {
            thread_id: self.launch.thread_id.clone(),
            terminal_id: terminal_id(&self.launch.thread_id),
            status: self.status,
            exit_code: self.exit_code,
            output: output.split_off(skip),
            held_messages: u32::try_from(self.held.len()).unwrap_or(u32::MAX),
        }
    }

    fn push_output(&mut self, data: &str) {
        self.partial.push_str(data);
        while let Some(newline) = self.partial.find('\n') {
            let line = clean_line(&self.partial[..newline]);
            self.partial.drain(..=newline);
            if !line.is_empty() {
                self.output.push(line);
            }
        }
        if self.output.len() > OUTPUT_LINES {
            self.output.drain(..self.output.len() - OUTPUT_LINES);
        }
        if self.partial.len() > 64 * 1024 {
            self.partial.clear();
        }
    }
}

/// The terminal a thread's setup runs in, which the UI opens to show all of its output.
pub fn terminal_id(thread_id: &str) -> String {
    format!("setup-{thread_id}")
}

#[derive(Clone)]
pub struct WorktreeSetups {
    terminals: Arc<Terminals>,
    runs: Arc<Mutex<HashMap<String, Run>>>,
    notify: Notify,
    generations: Arc<AtomicU64>,
}

impl WorktreeSetups {
    pub fn new(terminals: Arc<Terminals>, notify: Notify) -> Self {
        Self {
            terminals,
            runs: Arc::default(),
            notify,
            generations: Arc::default(),
        }
    }

    /// Runs the setup afresh; what an earlier run held stays held. `finished` gets the
    /// messages to send once it succeeds.
    pub fn start(
        &self,
        launch: SetupLaunch,
        finished: impl Fn(Vec<Command>) + Send + Sync + 'static,
    ) -> Result<(), CommandError> {
        let thread_id = launch.thread_id.clone();
        let generation = self.generations.fetch_add(1, Ordering::Relaxed);
        let held = guard(&self.runs)
            .remove(&thread_id)
            .map(|run| run.held)
            .unwrap_or_default();
        let run = Run {
            launch: launch.clone(),
            status: WorktreeSetupStatus::Running,
            exit_code: None,
            output: Vec::new(),
            partial: String::new(),
            held,
            notified_at: None,
            generation,
        };
        guard(&self.runs).insert(thread_id.clone(), run);
        self.send(&thread_id, true);
        let runs = self.runs.clone();
        let notify = self.notify.clone();
        let observed = thread_id.clone();
        let observer = Box::new(move |event: TerminalEvent| {
            let mut all = guard(&runs);
            let Some(run) = all
                .get_mut(&observed)
                .filter(|run| run.generation == generation)
            else {
                return;
            };
            match event {
                TerminalEvent::Output { data } => {
                    run.push_output(&data);
                    let due = run
                        .notified_at
                        .is_none_or(|at| at.elapsed() >= NOTIFY_EVERY);
                    if due {
                        run.notified_at = Some(Instant::now());
                        notify(Notification::WorktreeSetup(run.view()));
                    }
                }
                TerminalEvent::Exited { exit } => {
                    run.exit_code = exit.code;
                    let succeeded = exit.code == Some(0);
                    run.status = if succeeded {
                        WorktreeSetupStatus::Succeeded
                    } else {
                        WorktreeSetupStatus::Failed
                    };
                    let held = if succeeded {
                        std::mem::take(&mut run.held)
                    } else {
                        Vec::new()
                    };
                    notify(Notification::WorktreeSetup(run.view()));
                    drop(all);
                    if !held.is_empty() {
                        finished(held);
                    }
                }
            }
        });
        let started = self.terminals.run(
            &terminal_id(&thread_id),
            Launch {
                thread_id: &launch.thread_id,
                cwd: &launch.worktree_path,
                project_root: &launch.project_root,
                worktree_path: Some(&launch.worktree_path),
                command: Some(&launch.command),
            },
            observer,
        );
        if let Err(error) = &started {
            if let Some(run) = guard(&self.runs).get_mut(&thread_id) {
                run.status = WorktreeSetupStatus::Failed;
                run.output = vec![error.to_string()];
            }
            self.send(&thread_id, true);
        }
        started
    }

    /// Holds a message for the agent while the thread's setup is to be waited for and
    /// hasn't succeeded; hands it back otherwise.
    pub fn hold(&self, command: Command) -> Option<Command> {
        let Command::MessageSend { thread_id, .. } = &command else {
            return Some(command);
        };
        let thread_id = thread_id.clone();
        {
            let mut runs = guard(&self.runs);
            let Some(run) = runs.get_mut(&thread_id) else {
                return Some(command);
            };
            if !run.launch.wait || run.status == WorktreeSetupStatus::Succeeded {
                return Some(command);
            }
            run.held.push(command);
        }
        self.send(&thread_id, true);
        None
    }

    /// Stops waiting for the thread's setup, and gives what it held to send now.
    pub fn stop_waiting(&self, thread_id: &str) -> Vec<Command> {
        let held = guard(&self.runs).get_mut(thread_id).map(|run| {
            run.launch.wait = false;
            std::mem::take(&mut run.held)
        });
        self.send(thread_id, true);
        held.unwrap_or_default()
    }

    pub fn launch(&self, thread_id: &str) -> Option<SetupLaunch> {
        guard(&self.runs)
            .get(thread_id)
            .map(|run| run.launch.clone())
    }

    pub fn runs(&self) -> Vec<WorktreeSetupRun> {
        guard(&self.runs).values().map(Run::view).collect()
    }

    pub fn retain_threads(&self, keep: impl Fn(&str) -> bool) {
        guard(&self.runs).retain(|thread_id, _| keep(thread_id));
    }

    fn send(&self, thread_id: &str, now: bool) {
        let view = {
            let mut runs = guard(&self.runs);
            let Some(run) = runs.get_mut(thread_id) else {
                return;
            };
            if now {
                run.notified_at = Some(Instant::now());
            }
            run.view()
        };
        (self.notify)(Notification::WorktreeSetup(view));
    }
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A line as it reads on screen: escape sequences removed, and only what the last
/// carriage return left, as a progress bar redraws itself.
fn clean_line(raw: &str) -> String {
    let mut text = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(char) = chars.next() {
        match char {
            '\u{1b}' => match chars.next() {
                Some('[') => {
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' {
                            break;
                        }
                        if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' => {
                if chars.peek().is_some() {
                    text.clear();
                }
            }
            char if char.is_control() && char != '\t' => {}
            char => text.push(char),
        }
    }
    let line = text.trim_end();
    line.chars().take(LINE_MAX_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_keeps_what_the_screen_shows() {
        assert_eq!(clean_line("\u{1b}[32mok\u{1b}[0m done\r"), "ok done");
        assert_eq!(clean_line("10%\r50%\r100%"), "100%");
        assert_eq!(clean_line("\u{1b}]0;title\u{7}bun install"), "bun install");
    }

    #[test]
    fn the_output_keeps_its_last_lines() {
        let mut run = Run {
            launch: SetupLaunch {
                thread_id: "t".into(),
                project_root: String::new(),
                worktree_path: String::new(),
                command: String::new(),
                wait: false,
            },
            status: WorktreeSetupStatus::Running,
            exit_code: None,
            output: Vec::new(),
            partial: String::new(),
            held: Vec::new(),
            notified_at: None,
            generation: 0,
        };
        run.push_output("one\ntwo\r\n\nthree\nfour\nfive\nsi");
        run.push_output("x");
        assert_eq!(run.view().output, ["three", "four", "five", "six"]);
    }
}
