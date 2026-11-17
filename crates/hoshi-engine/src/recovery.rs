//! What a restart would lose of a thread, kept so hoshi picks it up again: the messages still
//! to reach its agent, the background work its agent ran, and how the thread goes on by
//! itself after a restart cut its turn short or once its usage limit reset, or waits for the
//! user after they stopped it.

use crate::{
    COMPACT_COMMAND, Notification, QueuedMessage, State, delegation, now, start_turn, unix_millis,
};
use hoshi_core::{
    BackgroundTask, CommandError, EffectiveSetting, Event, HeldMessage, ItemContent, MessageSender,
    ProjectSettings, QueueEntry, Resumption, ThreadQueue, TurnFailure, TurnOutcome,
    drop_queued_messages, may_continue_alone, restore_session, snooze_until_reset,
    start_resumed_turn,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};

/// Sent to a turn a restart cut short.
pub(crate) const CONTINUE_PROMPT: &str = "Continue where you left off. Your last turn was cut short, so a command or other action it started may have finished or may still be running. Before you repeat one that changes state, confirm it did not take effect and is not still running; if you cannot tell, say so instead of repeating it.";
/// Sent once the usage limit that failed a thread's turn reset, and when the user resumes a
/// thread whose turn was stopped or failed on its limit.
pub(crate) const LIMIT_RESET_PROMPT: &str = "Continue where you left off.";
pub(crate) const CUT_WORK_NOTE: &str = "Note: hoshi restarted, and this background work was cancelled before it finished. It will not report back on its own, so resume or restart what you still need and wait for its result:";
const MAX_NOTE_ENTRIES: usize = 10;
const MAX_LABEL_CHARS: usize = 160;
/// A thread whose turns keep being cut short by restarts goes on by itself this many times in
/// a row at most,
const MAX_CONTINUATION_CHAIN: usize = 3;
/// counting only continuations this recent.
const CONTINUATION_LOOP_WINDOW_MS: u64 = 5 * 60 * 1000;
/// A usage limit that reset longer ago than this no longer starts the thread again.
const MAX_OVERDUE_RESET_MS: u64 = 24 * 60 * 60 * 1000;

/// A thread's recovery as the store keeps it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Kept {
    queued: Vec<QueuedMessage>,
    /// What queues next waits for the user too.
    queue_held: bool,
    after_compaction: Option<QueuedMessage>,
    /// The commands its agent runs in the background, during a turn or after it.
    background: Vec<BackgroundTask>,
    /// Those of them the agent said it waits for.
    awaited: Vec<String>,
    /// Background work a restart cut short, which its agent hasn't been told of yet.
    cut_work: Vec<CutWork>,
    continuations: Vec<Continuation>,
    usage_limit: Option<LimitRecovery>,
    last_turn: Option<TurnEnd>,
}

/// How a thread's last turn ended, when the user may go on with it from there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum TurnEnd {
    /// Stopped, or cut short by a restart.
    Interrupted,
    UsageLimited,
}

/// Background work a restart ended before it finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CutWork {
    kind: WorkKind,
    label: String,
    id: String,
    /// The agent waited for it, so it gets to go on at once.
    awaited: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum WorkKind {
    Subagent,
    Shell,
}

impl WorkKind {
    fn name(self) -> &'static str {
        match self {
            Self::Subagent => "subagent",
            Self::Shell => "shell",
        }
    }
}

/// A turn hoshi started by itself to go on with one a restart cut short.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Continuation {
    turn_id: String,
    /// The turn it went on with; empty when it went on after the agent's turn had ended.
    of: String,
    /// Unix milliseconds.
    at: u64,
}

/// A thread whose turn failed on its usage limit, to start again once the limit resets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LimitRecovery {
    turn_id: String,
    /// RFC 3339.
    resets_at: String,
    /// It will go on by itself, as the settings said once it could.
    armed: bool,
    /// The thread's snooze is this limit's, until its reset.
    #[serde(default)]
    snoozed: bool,
    /// Whether the user chose that it goes on by itself, whatever the settings say; `None`
    /// follows them.
    #[serde(default)]
    resume: Option<bool>,
}

/// Background work the agent is still to be told a restart cut short.
struct CutNote {
    work: Vec<CutWork>,
    /// The turn whose prompt carries it; it is told for good once that turn completes.
    sent_in: Option<String>,
}

/// A thread a restart cut short, to go on once the providers are set up.
struct CutThread {
    thread_id: String,
    /// The turn the restart cut short; `None` when only its background work was.
    source: Option<String>,
}

pub(crate) struct Recovery {
    /// Threads whose turn a restart cut short go on by themselves.
    pub continue_after_restart: bool,
    /// Threads whose usage limit reset go on by themselves.
    pub resume_after_usage_limit: bool,
    /// Threads whose turn failed on their usage limit are snoozed until it resets.
    pub snooze_until_limit_resets: bool,
    /// hoshi quits: the agent processes it ends are cut short like a restart cuts them, so
    /// what they ran stays kept.
    pub quitting: bool,
    /// Threads whose queue waits for the user, as after their turn failed or they stopped it.
    held: HashSet<String>,
    /// Threads whose pull request news, kept until their turn ended, waits for the user too.
    held_wakes: HashSet<String>,
    /// By thread, the turn the user stopped.
    stopped: HashMap<String, String>,
    /// By thread, how its last turn ended, while the user may resume it.
    ended: HashMap<String, TurnEnd>,
    /// By thread, its queue as the UI last heard of it, while not empty.
    shown: HashMap<String, ThreadQueue>,
    /// By thread, the background commands its agent waits for.
    awaited: HashMap<String, Vec<String>>,
    notes: HashMap<String, CutNote>,
    continuations: HashMap<String, Vec<Continuation>>,
    limits: HashMap<String, LimitRecovery>,
    cut: Vec<CutThread>,
    /// By thread, the session its running turn resumed.
    resumed: HashMap<String, String>,
}

impl Default for Recovery {
    fn default() -> Self {
        Self {
            continue_after_restart: true,
            resume_after_usage_limit: false,
            snooze_until_limit_resets: false,
            quitting: false,
            held: HashSet::new(),
            held_wakes: HashSet::new(),
            stopped: HashMap::new(),
            ended: HashMap::new(),
            shown: HashMap::new(),
            awaited: HashMap::new(),
            notes: HashMap::new(),
            continuations: HashMap::new(),
            limits: HashMap::new(),
            cut: Vec::new(),
            resumed: HashMap::new(),
        }
    }
}

impl Recovery {
    pub fn forget(&mut self, thread_id: &str) {
        self.held.remove(thread_id);
        self.held_wakes.remove(thread_id);
        self.stopped.remove(thread_id);
        self.ended.remove(thread_id);
        self.shown.remove(thread_id);
        self.awaited.remove(thread_id);
        self.notes.remove(thread_id);
        self.continuations.remove(thread_id);
        self.limits.remove(thread_id);
        self.resumed.remove(thread_id);
        self.cut.retain(|cut| cut.thread_id != thread_id);
    }
}

/// What the agent reads of the background work a restart cut short.
fn cut_work_note(work: &[CutWork]) -> String {
    let mut lines = vec![CUT_WORK_NOTE.to_owned()];
    lines.extend(
        work.iter()
            .take(MAX_NOTE_ENTRIES)
            .map(|entry| format!("- {}: {}", entry.kind.name(), entry.label)),
    );
    if work.len() > MAX_NOTE_ENTRIES {
        lines.push(format!("- and {} more", work.len() - MAX_NOTE_ENTRIES));
    }
    lines.join("\n")
}

/// `text` on one line, cut to `max` characters; `None` when blank.
fn compact_label(text: &str, max: usize) -> Option<String> {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return None;
    }
    if line.chars().count() <= max {
        return Some(line);
    }
    let cut: String = line.chars().take(max.saturating_sub(1)).collect();
    Some(format!("{cut}…"))
}

/// A label that keeps its id suffix whole.
fn label_with_id(name: Option<&str>, fallback: &str, id: &str) -> String {
    let suffix = format!(" (id {id})");
    let budget = MAX_LABEL_CHARS
        .saturating_sub(suffix.chars().count())
        .max(2);
    let name = name
        .and_then(|name| compact_label(name, budget))
        .unwrap_or_else(|| fallback.to_owned());
    format!("{name}{suffix}")
}

fn command_work(task: &BackgroundTask, awaited: bool) -> CutWork {
    CutWork {
        kind: WorkKind::Shell,
        label: label_with_id(Some(&task.description), "background command", &task.task_id),
        id: task.task_id.clone(),
        awaited,
    }
}

/// A subagent call left running, from its input.
fn subagent_work(item_id: &str, input: &str) -> CutWork {
    let input: Value = serde_json::from_str(input).unwrap_or_default();
    let name = ["description", "prompt"]
        .into_iter()
        .find_map(|key| input[key].as_str().filter(|text| !text.trim().is_empty()));
    CutWork {
        kind: WorkKind::Subagent,
        label: label_with_id(name, "subagent", item_id),
        id: item_id.to_owned(),
        awaited: true,
    }
}

fn merge(work: &mut Vec<CutWork>, added: CutWork) {
    if !work.iter().any(|entry| entry.id == added.id) {
        work.push(added);
    }
}

fn reset_millis(resets_at: &str) -> Option<u64> {
    let at = chrono::DateTime::parse_from_rfc3339(resets_at).ok()?;
    u64::try_from(at.timestamp_millis()).ok()
}

/// Whether a thread's latest continuations, counted back from `source`, ran too often in a row.
fn loops(continuations: &[Continuation], source: Option<&str>, now: u64) -> bool {
    let window_start = now.saturating_sub(CONTINUATION_LOOP_WINDOW_MS);
    let mut current = source
        .map(str::to_owned)
        .or_else(|| continuations.last().map(|last| last.turn_id.clone()));
    let mut chain = 0;
    while chain < MAX_CONTINUATION_CHAIN {
        let Some(record) = current
            .as_deref()
            .and_then(|turn| continuations.iter().find(|record| record.turn_id == turn))
        else {
            break;
        };
        if record.at < window_start {
            break;
        }
        chain += 1;
        current = Some(record.of.clone());
    }
    chain >= MAX_CONTINUATION_CHAIN
}

impl State {
    /// Writes down what a restart would lose of the thread.
    pub(crate) fn save_recovery(&self, thread_id: &str) {
        if self.recovery.quitting {
            return;
        }
        let kept = self.model.thread(thread_id).map(|_| {
            let background = self.background_commands(thread_id).to_vec();
            let awaited = self
                .recovery
                .awaited
                .get(thread_id)
                .into_iter()
                .flatten()
                .filter(|task_id| background.iter().any(|task| &task.task_id == *task_id))
                .cloned()
                .collect();
            Kept {
                queued: self
                    .queued
                    .get(thread_id)
                    .map(|queued| queued.iter().cloned().collect())
                    .unwrap_or_default(),
                queue_held: self.recovery.held.contains(thread_id),
                after_compaction: self.after_compaction.get(thread_id).cloned(),
                background,
                awaited,
                cut_work: self
                    .recovery
                    .notes
                    .get(thread_id)
                    .map(|note| note.work.clone())
                    .unwrap_or_default(),
                continuations: self
                    .recovery
                    .continuations
                    .get(thread_id)
                    .cloned()
                    .unwrap_or_default(),
                usage_limit: self.recovery.limits.get(thread_id).cloned(),
                last_turn: self.recovery.ended.get(thread_id).copied(),
            }
        });
        let kept = kept.filter(|kept| *kept != Kept::default());
        if let Err(error) = self.store.save_thread_recovery(thread_id, kept.as_ref()) {
            eprintln!("hoshi: failed to keep what a restart would lose of a thread: {error}");
        }
    }

    /// Reads back what the last run kept: queued messages wait for the user, and threads a
    /// restart cut short are noted to go on. Runs before the turns it cut short are closed.
    pub(crate) fn recover_from_restart(&mut self) {
        let mut kept: HashMap<String, Kept> = self
            .store
            .thread_recoveries()
            .unwrap_or_else(|error| {
                eprintln!("hoshi: failed to read what threads kept over the restart: {error}");
                Vec::new()
            })
            .into_iter()
            .collect();
        let running: HashMap<String, String> = self
            .model
            .threads()
            .filter_map(|thread| Some((thread.id.clone(), thread.active_turn_id.clone()?)))
            .collect();
        for thread_id in running.keys() {
            kept.entry(thread_id.clone()).or_default();
        }
        let now = unix_millis();
        for (thread_id, kept) in kept {
            if self.model.thread(&thread_id).is_none() {
                continue;
            }
            let source = running.get(&thread_id).cloned();
            let mut work = kept.cut_work;
            let mut awaited = false;
            if source.is_some() {
                for subagent in self.open_subagents(&thread_id) {
                    awaited = true;
                    merge(&mut work, subagent);
                }
            }
            for task in &kept.background {
                let command = command_work(task, kept.awaited.contains(&task.task_id));
                awaited |= command.awaited;
                merge(&mut work, command);
            }
            if kept.queue_held || !kept.queued.is_empty() {
                let queued = kept
                    .queued
                    .into_iter()
                    .map(|message| QueuedMessage {
                        held: true,
                        ..message
                    })
                    .collect();
                self.queued.insert(thread_id.clone(), queued);
                self.recovery.held.insert(thread_id.clone());
            }
            if let Some(message) = kept.after_compaction {
                self.after_compaction.insert(thread_id.clone(), message);
            }
            if let Some(limit) = kept.usage_limit {
                self.recovery.limits.insert(thread_id.clone(), limit);
            }
            if let Some(end) = kept.last_turn {
                self.recovery.ended.insert(thread_id.clone(), end);
            }
            if !work.is_empty() {
                let note = CutNote {
                    work,
                    sent_in: None,
                };
                self.recovery.notes.insert(thread_id.clone(), note);
            }
            let continues = match &source {
                Some(turn_id) => !self.is_compaction(&thread_id, turn_id),
                None => awaited,
            } && may_continue_alone(&self.model, &thread_id);
            if continues && loops(&kept.continuations, source.as_deref(), now) {
                eprintln!("hoshi: not continuing thread {thread_id}, cut short too often in a row");
            } else if continues {
                self.recovery.cut.push(CutThread {
                    thread_id: thread_id.clone(),
                    source,
                });
            }
            if !kept.continuations.is_empty() {
                self.recovery
                    .continuations
                    .insert(thread_id.clone(), kept.continuations);
            }
            self.save_recovery(&thread_id);
            self.show_queue(&thread_id);
        }
    }

    /// The subagents the thread's running turn started and still waits for.
    fn open_subagents(&self, thread_id: &str) -> Vec<CutWork> {
        self.model
            .open_tool_calls()
            .filter(|call| call.thread_id == thread_id)
            .filter_map(|call| {
                let item = self.store.thread_item(thread_id, &call.item_id).ok()??;
                match item.content {
                    ItemContent::ToolCall { name, input, .. }
                        if matches!(name.as_str(), "Agent" | "Task") =>
                    {
                        Some(subagent_work(&item.id, &input))
                    }
                    _ => None,
                }
            })
            .collect()
    }

    fn is_compaction(&self, thread_id: &str, turn_id: &str) -> bool {
        matches!(
            self.store.thread_item(thread_id, turn_id),
            Ok(Some(item)) if matches!(
                &item.content,
                ItemContent::UserMessage { text, .. } if text.trim() == COMPACT_COMMAND
            )
        )
    }

    /// Has the threads a restart cut short go on, as the settings allow: told to continue
    /// where they left off, and of the background work that ended with the restart.
    pub(crate) fn continue_cut_threads(&mut self) {
        for cut in std::mem::take(&mut self.recovery.cut) {
            let thread_id = cut.thread_id;
            if !self.continue_after_restart(&thread_id).value {
                continue;
            }
            let ready = self.model.thread(&thread_id).is_some_and(|thread| {
                thread.active_turn_id.is_none() && self.adapter_for(&thread.provider).is_some()
            });
            if !ready || self.snoozed(&thread_id) || !may_continue_alone(&self.model, &thread_id) {
                continue;
            }
            let note = self
                .recovery
                .notes
                .get(&thread_id)
                .map(|note| cut_work_note(&note.work));
            let (text, resumed) = match (&cut.source, note) {
                (_, None) => (CONTINUE_PROMPT.to_owned(), Resumption::Restart),
                (Some(_), Some(note)) => {
                    (format!("{note}\n\n{CONTINUE_PROMPT}"), Resumption::Restart)
                }
                (None, Some(note)) => (note, Resumption::RestartEndedWork),
            };
            let turn_id = delegation::new_id();
            let now = now();
            let events =
                start_resumed_turn(&self.model, &thread_id, &turn_id, &text, resumed, &now);
            if events.is_empty() {
                continue;
            }
            if let Err(error) = self.commit(&events, &now) {
                eprintln!("hoshi: failed to continue a thread a restart cut short: {error}");
                continue;
            }
            if let Some(note) = self.recovery.notes.get_mut(&thread_id) {
                note.sent_in = Some(turn_id.clone());
            }
            let continuations = self
                .recovery
                .continuations
                .entry(thread_id.clone())
                .or_default();
            let of = cut.source.unwrap_or_default();
            continuations.push(Continuation {
                turn_id: turn_id.clone(),
                of,
                at: unix_millis(),
            });
            let excess = continuations.len().saturating_sub(MAX_CONTINUATION_CHAIN);
            continuations.drain(..excess);
            self.save_recovery(&thread_id);
            start_turn(self, &thread_id, &turn_id, text, Vec::new(), None);
        }
    }

    /// The prompt of turn `turn_id` with the background work a restart cut short before it,
    /// until a turn that carried it completes. A compaction carries nothing.
    pub(crate) fn carry_cut_work(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        prompt: String,
    ) -> String {
        if prompt.trim() == COMPACT_COMMAND {
            return prompt;
        }
        let Some(note) = self.recovery.notes.get_mut(thread_id) else {
            return prompt;
        };
        if note.sent_in.as_deref() == Some(turn_id) {
            return prompt;
        }
        note.sent_in = Some(turn_id.to_owned());
        format!("{}\n\nUser message:\n{prompt}", cut_work_note(&note.work))
    }

    /// Remembers the session a starting turn resumes, which a failed resume must not lose.
    pub(crate) fn note_resumed_session(&mut self, thread_id: &str, session_id: Option<&str>) {
        match session_id {
            Some(session_id) => {
                self.recovery
                    .resumed
                    .insert(thread_id.to_owned(), session_id.to_owned());
            }
            None => {
                self.recovery.resumed.remove(thread_id);
            }
        }
    }

    /// Whether the project's threads a restart cut short go on by themselves, and whether the
    /// project or the app's settings say so. A chat follows the app's settings.
    pub(crate) fn continue_after_restart_in(
        &self,
        project_id: Option<&str>,
    ) -> EffectiveSetting<bool> {
        let global = self.recovery.continue_after_restart;
        match project_id.and_then(|project_id| self.model.project(project_id)) {
            Some(project) => project.settings.continue_after_restart(global),
            None => ProjectSettings::default().continue_after_restart(global),
        }
    }

    fn continue_after_restart(&self, thread_id: &str) -> EffectiveSetting<bool> {
        let project_id = self
            .model
            .thread(thread_id)
            .and_then(|thread| thread.project_id.as_deref());
        self.continue_after_restart_in(project_id)
    }

    /// What a turn's end means for what waits on the thread: a failed turn holds its queue
    /// and, on a usage limit, arms its resumption; a completed one tells the agent of the
    /// work a restart cut for good and, unless the user stopped it, lets hoshi's own news go.
    pub(crate) fn turn_ended(&mut self, thread_id: &str, turn_id: &str, outcome: &TurnOutcome) {
        let stopped = self.recovery.stopped.remove(thread_id).as_deref() == Some(turn_id);
        let resumed = self.recovery.resumed.remove(thread_id);
        if let (
            Some(session_id),
            TurnOutcome::Failed {
                reason: TurnFailure::ResumeFailed { .. },
            },
        ) = (resumed, outcome)
        {
            let events = restore_session(&self.model, thread_id, &session_id);
            if let Err(error) = self.commit(&events, &now()) {
                eprintln!("hoshi: failed to keep the session a resume lost: {error}");
            }
        }
        match outcome {
            TurnOutcome::Completed => {
                let told = self
                    .recovery
                    .notes
                    .get(thread_id)
                    .is_some_and(|note| note.sent_in.as_deref() == Some(turn_id));
                if told {
                    self.recovery.notes.remove(thread_id);
                }
                if !stopped {
                    self.release_unattended(thread_id);
                }
            }
            TurnOutcome::Failed { reason } => {
                self.hold_queue(thread_id);
                if let TurnFailure::UsageLimited {
                    resets_at: Some(resets_at),
                    ..
                } = reason
                    && reset_millis(resets_at).is_some_and(|reset| reset > unix_millis())
                {
                    let limit = LimitRecovery {
                        turn_id: turn_id.to_owned(),
                        resets_at: resets_at.clone(),
                        armed: false,
                        snoozed: false,
                        resume: None,
                    };
                    self.recovery.limits.insert(thread_id.to_owned(), limit);
                    if self.recovery.snooze_until_limit_resets {
                        self.snooze_until_limit_resets(thread_id);
                    }
                }
            }
            TurnOutcome::Interrupted => {}
        }
        self.save_recovery(thread_id);
        self.show_queue(thread_id);
    }

    /// Drops what recorded events make stale: a new turn supersedes a usage limit's
    /// resumption and the way the last one ended, and a rewind takes the queued messages out
    /// of the transcript. Notes how a turn ended, for the user to resume it.
    pub(crate) fn recovery_after(&mut self, events: &[Event]) {
        let mut touched: Vec<&str> = Vec::new();
        for event in events {
            match event {
                Event::TurnStarted { thread_id, .. } => {
                    let limited = self.recovery.limits.remove(thread_id).is_some();
                    if self.recovery.ended.remove(thread_id).is_some() || limited {
                        self.save_recovery(thread_id);
                    }
                    touched.push(thread_id);
                }
                Event::TurnCompleted {
                    thread_id, outcome, ..
                } => {
                    let end = match outcome {
                        TurnOutcome::Interrupted => Some(TurnEnd::Interrupted),
                        TurnOutcome::Failed {
                            reason: TurnFailure::UsageLimited { .. },
                        } => Some(TurnEnd::UsageLimited),
                        _ => None,
                    };
                    let changed = match end {
                        Some(end) => {
                            self.recovery.ended.insert(thread_id.clone(), end) != Some(end)
                        }
                        None => self.recovery.ended.remove(thread_id).is_some(),
                    };
                    if changed {
                        self.save_recovery(thread_id);
                    }
                    touched.push(thread_id);
                }
                Event::ThreadRewound { thread_id, .. } => {
                    self.queued.remove(thread_id);
                    self.recovery.held.remove(thread_id);
                    self.recovery.held_wakes.remove(thread_id);
                    self.recovery.limits.remove(thread_id);
                    self.recovery.ended.remove(thread_id);
                    self.save_recovery(thread_id);
                    touched.push(thread_id);
                }
                Event::ThreadSnoozed { thread_id, .. }
                | Event::ThreadUnsnoozed { thread_id, .. } => {
                    if let Some(limit) = self.recovery.limits.get_mut(thread_id)
                        && limit.snoozed
                    {
                        limit.snoozed = false;
                        self.save_recovery(thread_id);
                    }
                }
                Event::ItemAdded { item } => touched.push(&item.thread_id),
                _ => {}
            }
        }
        touched.dedup();
        for thread_id in touched {
            self.show_queue(thread_id);
        }
    }

    /// Whether what queues for the thread waits for the user.
    pub(crate) fn queue_holds(&self, thread_id: &str) -> bool {
        self.recovery.held.contains(thread_id)
    }

    /// Whether the thread's pull request news, kept while its turn ran, waits for the user.
    pub(crate) fn wakes_held(&self, thread_id: &str) -> bool {
        self.recovery.held_wakes.contains(thread_id) || self.limit_holds(thread_id)
    }

    /// News kept for the thread waits for the user too, while its queue does.
    pub(crate) fn hold_wakes_if_held(&mut self, thread_id: &str) {
        if self.queue_holds(thread_id) {
            self.recovery.held_wakes.insert(thread_id.to_owned());
        }
    }

    /// Whether the thread's last turn failed on its usage limit, so nothing hoshi sends starts
    /// it until the user goes on.
    pub(crate) fn limit_holds(&self, thread_id: &str) -> bool {
        self.recovery.ended.get(thread_id) == Some(&TurnEnd::UsageLimited)
    }

    /// The messages queued for the thread wait for the user, and so does what comes next.
    fn hold_queue(&mut self, thread_id: &str) {
        self.recovery.held.insert(thread_id.to_owned());
        for message in self.queued.get_mut(thread_id).into_iter().flatten() {
            message.held = true;
        }
        if self
            .pending_wakes
            .get(thread_id)
            .is_some_and(|wakes| !wakes.is_empty())
        {
            self.recovery.held_wakes.insert(thread_id.to_owned());
        }
    }

    /// The user stopped the thread: what waits in its queue, and what queues next, waits for
    /// them, and a usage limit's resumption no longer starts it. `false` when it runs no turn
    /// and waits for nothing, so there is nothing to stop.
    pub(crate) fn hold_for_stop(&mut self, thread_id: &str) -> bool {
        let Some(thread) = self.model.thread(thread_id) else {
            return false;
        };
        let running = thread.active_turn_id.clone();
        let waiting = self.recovery.limits.contains_key(thread_id)
            || self
                .queued
                .get(thread_id)
                .is_some_and(|queued| !queued.is_empty());
        if running.is_none() && !waiting {
            return false;
        }
        if let Some(turn_id) = running {
            self.recovery.stopped.insert(thread_id.to_owned(), turn_id);
        }
        self.recovery.limits.remove(thread_id);
        self.hold_queue(thread_id);
        self.save_recovery(thread_id);
        self.show_queue(thread_id);
        true
    }

    /// The user goes on with the thread: what waited in its queue follows.
    pub(crate) fn release_queue(&mut self, thread_id: &str) {
        self.release_queue_after(thread_id, None);
    }

    /// The user goes on with the thread: what waited in its queue follows `first`, a message
    /// of theirs that may wait in it too.
    pub(crate) fn release_queue_after(&mut self, thread_id: &str, first: Option<&str>) {
        let held = self.recovery.held.remove(thread_id);
        let wakes = self.recovery.held_wakes.remove(thread_id);
        if !held && !wakes {
            return;
        }
        if let Some(queued) = self.queued.get_mut(thread_id) {
            for message in queued.iter_mut() {
                message.held = false;
            }
            let at = first.and_then(|first| queued.iter().position(|message| message.id == first));
            if let Some(message) = at.and_then(|at| queued.remove(at)) {
                queued.push_front(message);
            }
        }
        self.save_recovery(thread_id);
        self.show_queue(thread_id);
        self.start_next_queued(thread_id);
        self.deliver_pending_wakes(thread_id);
    }

    /// A turn completed past the hold: messages hoshi queued go on, as nobody reviews them;
    /// the user's still wait for the user.
    fn release_unattended(&mut self, thread_id: &str) {
        self.recovery.held_wakes.remove(thread_id);
        if !self.recovery.held.contains(thread_id) {
            return;
        }
        let mut still_held = false;
        for message in self.queued.get_mut(thread_id).into_iter().flatten() {
            message.held &= message.from_user;
            still_held |= message.held;
        }
        if !still_held {
            self.recovery.held.remove(thread_id);
        }
    }

    /// The messages queued for the thread that wait for the user.
    pub(crate) fn held_queue(&self, thread_id: &str) -> Vec<HeldMessage> {
        self.queued
            .get(thread_id)
            .into_iter()
            .flatten()
            .filter(|message| message.held)
            .map(|message| crate::held_message(thread_id, message))
            .collect()
    }

    /// Hoshi's own message to a thread its usage limit stopped waits in the queue instead of
    /// starting a turn. Otherwise it starts the idle thread, though the user's messages wait.
    pub(crate) fn queue_if_held(&self, thread_id: &str, events: &mut Vec<Event>) {
        if self.limit_holds(thread_id) {
            events.retain(|event| {
                !matches!(event, Event::TurnStarted { thread_id: started, .. } if started == thread_id)
            });
        }
    }

    /// The thread's queue in the order it goes, and whether the user can resume the thread.
    pub(crate) fn thread_queue(&self, thread_id: &str) -> ThreadQueue {
        let messages = self
            .queued
            .get(thread_id)
            .into_iter()
            .flatten()
            .map(|message| QueueEntry {
                message_id: message.id.clone(),
                text: message.text.clone(),
                attachments: message.attachments.clone(),
                held: message.held,
                sender: if message.from_user {
                    MessageSender::User
                } else {
                    MessageSender::Hoshi
                },
            })
            .collect::<Vec<_>>();
        let can_resume = if self.resumable(thread_id) {
            self.model.pending_requests(thread_id).is_empty()
        } else {
            messages.iter().any(|message| message.held)
        };
        ThreadQueue {
            thread_id: thread_id.to_owned(),
            messages,
            can_resume,
        }
    }

    /// Whether the idle thread's last turn was stopped, cut short or failed on its usage
    /// limit, so resuming it tells its agent to continue.
    pub(crate) fn resumable(&self, thread_id: &str) -> bool {
        self.recovery.ended.contains_key(thread_id)
            && self
                .model
                .thread(thread_id)
                .is_some_and(|thread| thread.active_turn_id.is_none())
    }

    /// Tells the UI of the thread's queue, when it changed.
    pub(crate) fn show_queue(&mut self, thread_id: &str) {
        if self.model.thread(thread_id).is_none() {
            self.recovery.shown.remove(thread_id);
            return;
        }
        let queue = self.thread_queue(thread_id);
        let empty = queue.messages.is_empty() && !queue.can_resume;
        let changed = match self.recovery.shown.get(thread_id) {
            Some(shown) => *shown != queue,
            None => !empty,
        };
        if !changed {
            return;
        }
        if empty {
            self.recovery.shown.remove(thread_id);
        } else {
            self.recovery
                .shown
                .insert(thread_id.to_owned(), queue.clone());
        }
        (self.notify)(Notification::Queue(queue));
    }

    /// Starts a turn with one of the user's queued messages while the thread is stopped; the
    /// rest of its queue follows.
    pub(crate) fn send_queued_now(
        &mut self,
        thread_id: &str,
        message_id: &str,
    ) -> Result<(), CommandError> {
        let thread = self
            .model
            .thread(thread_id)
            .ok_or_else(|| CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            })?;
        if thread.active_turn_id.is_some() {
            return Err(CommandError::ThreadBusy {
                thread_id: thread_id.to_owned(),
            });
        }
        let at = self.queued.get(thread_id).and_then(|queued| {
            queued
                .iter()
                .position(|message| message.id == message_id && message.from_user)
        });
        let Some(message) = at.and_then(|at| self.queued.get_mut(thread_id)?.remove(at)) else {
            return Err(CommandError::QueuedMessageNotFound {
                thread_id: thread_id.to_owned(),
                message_id: message_id.to_owned(),
            });
        };
        self.recovery.held.remove(thread_id);
        self.recovery.held_wakes.remove(thread_id);
        for message in self.queued.get_mut(thread_id).into_iter().flatten() {
            message.held = false;
        }
        self.save_recovery(thread_id);
        self.start_queued(thread_id, message);
        self.show_queue(thread_id);
        Ok(())
    }

    /// Takes messages out of the thread's queue, `None` for all of them; the transcript shows
    /// them never read.
    pub(crate) fn remove_queued(
        &mut self,
        thread_id: &str,
        message_id: Option<&str>,
    ) -> Result<(), CommandError> {
        if self.model.thread(thread_id).is_none() {
            return Err(CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            });
        }
        let removed: Vec<String> = match message_id {
            Some(message_id) => {
                let queued = self.queued.get_mut(thread_id);
                let message = queued.and_then(|queued| {
                    let at = queued.iter().position(|message| message.id == message_id)?;
                    queued.remove(at)
                });
                let Some(message) = message else {
                    return Err(CommandError::QueuedMessageNotFound {
                        thread_id: thread_id.to_owned(),
                        message_id: message_id.to_owned(),
                    });
                };
                vec![message.id]
            }
            None => self
                .queued
                .remove(thread_id)
                .into_iter()
                .flatten()
                .map(|message| message.id)
                .collect(),
        };
        if self.queued.get(thread_id).is_some_and(VecDeque::is_empty) {
            self.queued.remove(thread_id);
        }
        self.save_recovery(thread_id);
        let events = drop_queued_messages(&self.model, thread_id, removed);
        self.commit(&events, &now())?;
        self.show_queue(thread_id);
        Ok(())
    }

    /// `await_background_tasks` answered: which commands the agent now waits for.
    pub(crate) fn note_awaited(&mut self, thread_id: &str, answer: &Value) {
        let waiting = answer["waiting"].as_bool().unwrap_or(true);
        let task_ids = answer["backgroundTaskIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str);
        let awaited = self
            .recovery
            .awaited
            .entry(thread_id.to_owned())
            .or_default();
        for task_id in task_ids {
            awaited.retain(|known| known != task_id);
            if waiting {
                awaited.push(task_id.to_owned());
            }
        }
        self.save_recovery(thread_id);
    }

    /// The thread no longer waits for its usage limit to reset, until its turn fails on a
    /// limit again.
    pub(crate) fn cancel_limit_resume(&mut self, thread_id: &str) {
        if self.recovery.limits.remove(thread_id).is_some() {
            self.save_recovery(thread_id);
        }
    }

    /// The user chose whether the thread goes on by itself once its usage limit resets,
    /// whatever the settings say. Whether it has a limit to choose for.
    pub(crate) fn choose_limit_resume(&mut self, thread_id: &str, resume: bool) -> bool {
        let Some(limit) = self.recovery.limits.get_mut(thread_id) else {
            return false;
        };
        limit.resume = Some(resume);
        limit.armed = resume;
        self.save_recovery(thread_id);
        true
    }

    /// Snoozes the thread until the usage limit that failed its turn resets, a snooze the limit
    /// owns until something wakes it or snoozes it again. Whether it snoozed.
    pub(crate) fn snooze_until_limit_resets(&mut self, thread_id: &str) -> bool {
        let Some(resets_at) = self
            .recovery
            .limits
            .get(thread_id)
            .map(|limit| limit.resets_at.clone())
        else {
            return false;
        };
        let now = now();
        let events = snooze_until_reset(&self.model, thread_id, &resets_at, &now);
        if events.is_empty() {
            return false;
        }
        if let Err(error) = self.commit(&events, &now) {
            eprintln!("hoshi: failed to snooze a thread until its usage limit resets: {error}");
            return false;
        }
        if let Some(limit) = self.recovery.limits.get_mut(thread_id) {
            limit.snoozed = true;
        }
        self.save_recovery(thread_id);
        true
    }

    /// The thread's snooze is the one its usage limit put it in, until the reset.
    pub(crate) fn limit_snoozed(&self, thread_id: &str) -> bool {
        self.recovery
            .limits
            .get(thread_id)
            .is_some_and(|limit| limit.snoozed)
    }

    /// When the thread's usage limit resets, as RFC 3339, while it goes on by itself then.
    pub(crate) fn limit_resume(&self, thread_id: &str) -> Option<String> {
        self.recovery
            .limits
            .get(thread_id)
            .filter(|limit| limit.armed)
            .map(|limit| limit.resets_at.clone())
    }

    /// Has the threads whose usage limit reset by `now`, in Unix milliseconds, go on where
    /// they left off, as the settings allow.
    pub(crate) fn resume_limited_threads(&mut self, now: u64) {
        let auto = self.recovery.resume_after_usage_limit;
        let thread_ids: Vec<String> = self.recovery.limits.keys().cloned().collect();
        for thread_id in thread_ids {
            let Some(mut limit) = self.recovery.limits.get(&thread_id).cloned() else {
                continue;
            };
            let Some(reset) = reset_millis(&limit.resets_at) else {
                self.cancel_limit_resume(&thread_id);
                continue;
            };
            let Some(thread) = self.model.thread(&thread_id) else {
                self.recovery.limits.remove(&thread_id);
                continue;
            };
            let enabled = self.adapter_for(&thread.provider).is_some();
            if thread.active_turn_id.is_some() || !may_continue_alone(&self.model, &thread_id) {
                continue;
            }
            let wanted = limit.resume.unwrap_or(auto);
            if !limit.armed {
                if now.saturating_sub(reset) > MAX_OVERDUE_RESET_MS {
                    self.cancel_limit_resume(&thread_id);
                    continue;
                }
                if !wanted {
                    continue;
                }
                limit.armed = true;
                self.recovery.limits.insert(thread_id.clone(), limit);
                self.save_recovery(&thread_id);
            }
            if !wanted || reset > now || !enabled || self.snoozed_at_millis(&thread_id, now) {
                continue;
            }
            let turn_id = delegation::new_id();
            let at = crate::now();
            let events = start_resumed_turn(
                &self.model,
                &thread_id,
                &turn_id,
                LIMIT_RESET_PROMPT,
                Resumption::UsageLimitReset,
                &at,
            );
            if let Err(error) = self.commit(&events, &at) {
                eprintln!("hoshi: failed to resume a thread after its usage limit: {error}");
                continue;
            }
            start_turn(
                self,
                &thread_id,
                &turn_id,
                LIMIT_RESET_PROMPT.to_owned(),
                Vec::new(),
                None,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn continuation(turn_id: &str, of: &str, at: u64) -> Continuation {
        Continuation {
            turn_id: turn_id.into(),
            of: of.into(),
            at,
        }
    }

    #[test]
    fn the_note_names_each_cut_work_and_how_many_more() {
        let work: Vec<CutWork> = (0..12)
            .map(|index| CutWork {
                kind: if index == 0 {
                    WorkKind::Subagent
                } else {
                    WorkKind::Shell
                },
                label: format!("work {index}"),
                id: index.to_string(),
                awaited: false,
            })
            .collect();

        let note = cut_work_note(&work);

        let lines: Vec<&str> = note.lines().collect();
        assert_eq!(lines[0], CUT_WORK_NOTE);
        assert_eq!(lines[1], "- subagent: work 0");
        assert_eq!(lines[2], "- shell: work 1");
        assert_eq!(lines[10], "- shell: work 9");
        assert_eq!(lines[11], "- and 2 more");
        assert_eq!(lines.len(), 12);
    }

    #[test]
    fn cut_work_is_labelled_from_what_the_agent_said_with_its_id_whole() {
        let task = BackgroundTask {
            task_id: "b7".into(),
            tool_call_id: None,
            description: "  Start   the dev\nserver ".into(),
            started_at: 1,
        };
        assert_eq!(
            command_work(&task, false).label,
            "Start the dev server (id b7)"
        );
        let unnamed = BackgroundTask {
            description: String::new(),
            ..task
        };
        assert_eq!(
            command_work(&unnamed, true).label,
            "background command (id b7)"
        );

        let long = "word ".repeat(100);
        let input = serde_json::json!({ "prompt": long }).to_string();
        let subagent = subagent_work("toolu_1", &input);
        assert!(subagent.awaited);
        assert!(subagent.label.ends_with("… (id toolu_1)"));
        assert_eq!(subagent.label.chars().count(), MAX_LABEL_CHARS);
        let input = serde_json::json!({ "description": "Count files", "prompt": "x" });
        assert_eq!(
            subagent_work("toolu_2", &input.to_string()).label,
            "Count files (id toolu_2)"
        );
    }

    #[test]
    fn continuing_stops_after_three_restarts_in_a_row_within_five_minutes() {
        let now = 10 * 60 * 1000;
        let recent = [
            continuation("c1", "m1", now - 60_000),
            continuation("c2", "c1", now - 40_000),
            continuation("c3", "c2", now - 20_000),
        ];
        assert!(!loops(&recent, Some("m1"), now));
        assert!(!loops(&recent, Some("c2"), now));
        assert!(loops(&recent, Some("c3"), now));
        assert!(loops(&recent, None, now));

        let older = [
            continuation("c1", "m1", now - CONTINUATION_LOOP_WINDOW_MS - 1),
            continuation("c2", "c1", now - 40_000),
            continuation("c3", "c2", now - 20_000),
        ];
        assert!(!loops(&older, Some("c3"), now));
    }
}
