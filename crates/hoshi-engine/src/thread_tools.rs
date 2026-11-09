//! Tools an agent reaches other threads with, as t3code's `t3_thread_*`: find them, read
//! them, message them and wait for their turn to end.

use crate::{State, delegation, lock};
use hoshi_core::{Command, ItemContent, Thread, ThreadItem, ThreadStatus};
use serde_json::{Value, json};
use std::sync::{Mutex, Weak};
use std::time::{Duration, Instant};

const LIST_LIMIT: u64 = 20;
const MAX_LIST_LIMIT: u64 = 100;
const READ_LIMIT: u64 = 20;
const MAX_READ_LIMIT: u64 = 100;
const ITEM_CHARS: u64 = 2_000;
const MAX_ITEM_CHARS: u64 = 50_000;
const ANSWER_CHARS: usize = 4_000;
const DEFAULT_WAIT: Duration = Duration::from_secs(5 * 60);
const MAX_WAIT: Duration = Duration::from_secs(60 * 60);
const CHECK_EVERY: Duration = Duration::from_millis(500);

fn number(arguments: &Value, key: &str, default: u64, max: u64) -> usize {
    arguments[key].as_u64().unwrap_or(default).clamp(1, max) as usize
}

fn thread_argument<'a>(state: &'a State, arguments: &Value) -> Result<&'a Thread, String> {
    let thread_id = arguments["threadId"]
        .as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or("`threadId` is required")?;
    state
        .model
        .thread(thread_id)
        .ok_or_else(|| format!("no thread {thread_id}"))
}

fn status(thread: &Thread) -> &'static str {
    match thread.status {
        ThreadStatus::Idle => "idle",
        ThreadStatus::Working => "working",
        ThreadStatus::WaitingForInput => "waitingForInput",
        ThreadStatus::Error => "error",
    }
}

fn summary(state: &State, thread: &Thread) -> Value {
    let project = thread
        .project_id
        .as_deref()
        .and_then(|project_id| state.model.project(project_id))
        .map(|project| project.title.clone());
    json!({
        "threadId": thread.id,
        "title": thread.title,
        "status": status(thread),
        "project": project,
        "branch": thread.branch,
        "archived": thread.archived,
        "parentThreadId": thread.parent_thread_id,
        "updatedAt": thread.updated_at,
    })
}

fn cut(text: &str, max_chars: usize) -> (String, bool) {
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => (text[..index].to_owned(), true),
        None => (text.to_owned(), false),
    }
}

/// What a person would read of an item: their messages, the agent's answers and plans.
fn readable(item: &ThreadItem) -> Option<(&'static str, &str)> {
    match &item.content {
        ItemContent::UserMessage { text, .. } => Some(("user", text)),
        ItemContent::AssistantMessage { text } => Some(("assistant", text)),
        ItemContent::ProposedPlan { plan, .. } => Some(("plan", plan)),
        _ => None,
    }
}

impl State {
    /// `thread_list`: threads newest first, in the caller's project unless `allProjects`.
    pub(crate) fn list_threads(&self, caller: &str, arguments: &Value) -> Result<Value, String> {
        let project = self
            .model
            .thread(caller)
            .and_then(|thread| thread.project_id.clone());
        let all = arguments["allProjects"].as_bool().unwrap_or(false);
        let query = arguments["query"]
            .as_str()
            .map(|query| query.trim().to_lowercase())
            .filter(|query| !query.is_empty());
        let mut threads: Vec<&Thread> = self
            .model
            .threads()
            .filter(|thread| all || thread.project_id == project)
            .filter(|thread| {
                query.as_ref().is_none_or(|query| {
                    thread.title.to_lowercase().contains(query)
                        || thread
                            .branch
                            .as_ref()
                            .is_some_and(|branch| branch.to_lowercase().contains(query))
                })
            })
            .collect();
        threads.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        let limit = number(arguments, "limit", LIST_LIMIT, MAX_LIST_LIMIT);
        let listed: Vec<Value> = threads
            .iter()
            .take(limit)
            .map(|thread| summary(self, thread))
            .collect();
        Ok(json!({ "threads": listed, "more": threads.len() > limit }))
    }

    /// `thread_read`: a thread's latest messages, oldest first.
    pub(crate) fn read_thread(&self, arguments: &Value) -> Result<Value, String> {
        let thread = thread_argument(self, arguments)?;
        let items = self
            .store
            .thread_items(&thread.id)
            .map_err(|error| error.to_string())?;
        let limit = number(arguments, "limit", READ_LIMIT, MAX_READ_LIMIT);
        let max_chars = number(arguments, "maxCharsPerItem", ITEM_CHARS, MAX_ITEM_CHARS);
        let messages: Vec<_> = items
            .iter()
            .filter_map(|item| Some((item, readable(item)?)))
            .collect();
        let start = messages.len().saturating_sub(limit);
        let read: Vec<Value> = messages[start..]
            .iter()
            .map(|(item, (role, text))| {
                let (text, truncated) = cut(text, max_chars);
                json!({ "id": item.id, "role": role, "text": text, "truncated": truncated })
            })
            .collect();
        let mut detail = summary(self, thread);
        detail["messages"] = Value::Array(read);
        detail["earlierMessages"] = json!(start);
        Ok(detail)
    }

    /// `thread_send`: a message to another thread, which starts its turn or joins the one
    /// running.
    pub(crate) fn send_to_thread(
        &mut self,
        caller: &str,
        arguments: &Value,
    ) -> Result<Value, String> {
        let thread = thread_argument(self, arguments)?;
        if thread.id == caller {
            return Err("this is your own thread: answer in your turn instead".into());
        }
        let message = arguments["message"]
            .as_str()
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .ok_or("`message` is empty")?
            .to_owned();
        let (thread_id, steered) = (thread.id.clone(), thread.active_turn_id.is_some());
        let message_id = delegation::new_id();
        self.run_command(Command::MessageSend {
            thread_id: thread_id.clone(),
            message_id: message_id.clone(),
            text: message,
            attachments: Vec::new(),
        })
        .map_err(|error| error.to_string())?;
        Ok(json!({
            "threadId": thread_id,
            "messageId": message_id,
            "delivery": if steered { "steered" } else { "started" },
        }))
    }
}

/// `thread_wait`: blocks, outside the engine's lock, until the thread's turn ends or the
/// wait runs out; then says how it stands, with its latest answer.
pub(crate) fn wait_for_thread(
    state: &Weak<Mutex<State>>,
    caller: &str,
    arguments: &Value,
) -> Result<Value, String> {
    let wait = arguments["timeoutMs"]
        .as_u64()
        .map_or(DEFAULT_WAIT, Duration::from_millis)
        .clamp(CHECK_EVERY, MAX_WAIT);
    let deadline = Instant::now() + wait;
    loop {
        let shared = state.upgrade().ok_or("hoshi is closing")?;
        let state = lock(&shared);
        let thread = thread_argument(&state, arguments)?;
        if thread.id == caller {
            return Err("this is your own thread: it can't wait for itself".into());
        }
        let done = thread.active_turn_id.is_none();
        let timed_out = Instant::now() >= deadline;
        if done || timed_out {
            let answer = state
                .store
                .thread_items(&thread.id)
                .unwrap_or_default()
                .iter()
                .rev()
                .find_map(|item| match &item.content {
                    ItemContent::AssistantMessage { text } => Some(cut(text, ANSWER_CHARS).0),
                    _ => None,
                });
            let mut result = summary(&state, thread);
            result["timedOut"] = json!(!done);
            result["lastAnswer"] = json!(answer);
            return Ok(result);
        }
        drop(state);
        drop(shared);
        std::thread::sleep(CHECK_EVERY);
    }
}
