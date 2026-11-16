//! Tools an agent reaches other threads with: find them, read them, message them, wait for
//! their turn to end and snooze them.

use crate::{State, delegation, lock};
use chrono::{SecondsFormat, Utc};
use hoshi_core::{Command, CommandError, ItemContent, Thread, ThreadItem, ThreadStatus, item_text};
use serde_json::{Value, json};
use std::sync::{Mutex, Weak};
use std::time::{Duration, Instant};

const LIST_LIMIT: u64 = 20;
const MAX_LIST_LIMIT: u64 = 100;
const READ_LIMIT: u64 = 10;
const MAX_READ_LIMIT: u64 = 100;
const ITEM_CHARS: u64 = 1_500;
const MAX_ITEM_CHARS: u64 = 50_000;
const ANSWER_CHARS: usize = 4_000;
const DEFAULT_WAIT: Duration = Duration::from_secs(5 * 60);
const MAX_WAIT: Duration = Duration::from_secs(60 * 60);
const CHECK_EVERY: Duration = Duration::from_millis(500);
const SENT_NEXT: &str = "Its answer stays in that thread; read it later with thread_read.";
const WAIT_NEXT: &str = "Still running. End your turn unless you need its answer now.";

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

fn snooze_error(error: CommandError) -> String {
    match error {
        CommandError::SnoozeTimePassed { .. } => format!(
            "`until` must be an RFC 3339 time after now, {}",
            Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
        ),
        CommandError::ThreadArchived { thread_id } => {
            format!("thread {thread_id} is archived, so it can't be snoozed")
        }
        CommandError::ThreadWaitingForUser { thread_id } => format!(
            "thread {thread_id}, or a task it delegated, waits for the user to answer an approval or a request; it can't be snoozed until they do"
        ),
        CommandError::MessageQueued { thread_id } => format!(
            "thread {thread_id} has a message from the user waiting to be sent; it can't be snoozed until that message starts"
        ),
        other => other.to_string(),
    }
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
    let mut summary = json!({
        "threadId": thread.id,
        "title": thread.title,
        "status": status(thread),
    });
    let optional = [
        ("project", project),
        ("branch", thread.branch.clone()),
        ("parentThreadId", thread.parent_thread_id.clone()),
    ];
    for (key, value) in optional {
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            summary[key] = json!(value);
        }
    }
    if thread.archived {
        summary["archived"] = json!(true);
    }
    summary["updatedAt"] = json!(thread.updated_at);
    summary
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

    /// `thread_read`: a thread's latest messages, oldest first; or one item, from `textOffset`.
    pub(crate) fn read_thread(&self, arguments: &Value) -> Result<Value, String> {
        let thread = thread_argument(self, arguments)?;
        let max_chars = number(arguments, "maxCharsPerItem", ITEM_CHARS, MAX_ITEM_CHARS);
        if let Some(item_id) = arguments["itemId"]
            .as_str()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            return self.read_item(thread, item_id, arguments, max_chars);
        }
        if !arguments["textOffset"].is_null() {
            return Err("`textOffset` goes with `itemId`".into());
        }
        let limit = number(arguments, "limit", READ_LIMIT, MAX_READ_LIMIT);
        let (items, earlier) = self
            .store
            .latest_messages(&thread.id, limit)
            .map_err(|error| error.to_string())?;
        let read: Vec<Value> = items
            .iter()
            .filter_map(|item| Some((item, readable(item)?)))
            .map(|(item, (role, text))| {
                let (text, truncated) = cut(text, max_chars);
                let mut message = json!({ "id": item.id, "role": role, "text": text });
                if truncated {
                    message["truncated"] = json!(true);
                }
                message
            })
            .collect();
        let mut detail = summary(self, thread);
        detail["messages"] = Value::Array(read);
        if earlier > 0 {
            detail["earlierMessages"] = json!(earlier);
        }
        Ok(detail)
    }

    /// One item of `thread`, or of a thread it was forked from, as a handoff names it: its
    /// text from `textOffset`, and where the rest goes on.
    fn read_item(
        &self,
        thread: &Thread,
        item_id: &str,
        arguments: &Value,
        max_chars: usize,
    ) -> Result<Value, String> {
        let mut lineage = vec![thread.id.clone()];
        let mut origin = thread.forked_from.as_deref();
        while let Some(fork) = origin.filter(|fork| !lineage.contains(&fork.thread_id)) {
            lineage.push(fork.thread_id.clone());
            origin = self
                .model
                .thread(&fork.thread_id)
                .and_then(|source| source.forked_from.as_deref());
        }
        let mut found = None;
        for thread_id in &lineage {
            found = self
                .store
                .thread_item(thread_id, item_id)
                .map_err(|error| error.to_string())?;
            if found.is_some() {
                break;
            }
        }
        let item = found.ok_or_else(|| format!("thread {} has no item {item_id}", thread.id))?;
        let (kind, text) =
            item_text(&item).ok_or_else(|| format!("item {item_id} has no text to read"))?;
        let offset = arguments["textOffset"].as_u64().unwrap_or(0) as usize;
        let rest = text
            .char_indices()
            .nth(offset)
            .map_or("", |(index, _)| &text[index..]);
        let (page, cut_off) = cut(rest, max_chars);
        let mut read = json!({
            "id": item.id,
            "kind": kind,
            "text": page,
            "nextTextOffset": cut_off.then_some(offset + max_chars),
        });
        if item.thread_id != thread.id {
            read["sourceThreadId"] = json!(item.thread_id);
        }
        let mut detail = summary(self, thread);
        detail["item"] = read;
        Ok(detail)
    }

    /// `thread_snooze`: snoozes a thread, the caller's own by default, until `until`; or
    /// wakes it.
    pub(crate) fn snooze_thread(
        &mut self,
        caller: &str,
        arguments: &Value,
    ) -> Result<Value, String> {
        let named = arguments["threadId"]
            .as_str()
            .is_some_and(|id| !id.trim().is_empty());
        let thread_id = if named {
            thread_argument(self, arguments)?.id.clone()
        } else {
            caller.to_owned()
        };
        let until = arguments["until"]
            .as_str()
            .map(str::trim)
            .filter(|until| !until.is_empty());
        let command = match (arguments["wake"].as_bool().unwrap_or(false), until) {
            (true, None) => Command::ThreadWake {
                thread_id: thread_id.clone(),
            },
            (true, Some(_)) => return Err("pass `until` or `wake`, not both".into()),
            (false, Some(until)) => Command::ThreadSnooze {
                thread_id: thread_id.clone(),
                until: until.to_owned(),
            },
            (false, None) => return Err("`until` is required, or `wake`: true".into()),
        };
        self.run_command(command).map_err(snooze_error)?;
        let until = self
            .model
            .thread(&thread_id)
            .and_then(|thread| thread.snoozed_until.clone());
        Ok(json!({ "threadId": thread_id, "snoozedUntil": until }))
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
            "next": SENT_NEXT,
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
                .last_answer(&thread.id)
                .ok()
                .flatten()
                .map(|text| cut(&text, ANSWER_CHARS).0);
            let mut result = summary(&state, thread);
            if done {
                result["lastAnswer"] = json!(answer);
            } else {
                result["timedOut"] = json!(true);
                result["next"] = json!(WAIT_NEXT);
            }
            return Ok(result);
        }
        drop(state);
        drop(shared);
        std::thread::sleep(CHECK_EVERY);
    }
}

#[cfg(test)]
mod tests {
    use crate::{Engine, Notify, QueuedMessage, lock};
    use chrono::{Duration, Utc};
    use hoshi_core::{
        AccessMode, Command, Event, ItemContent, RequestStatus, ThreadItem, ThreadWorkspace,
        TurnPoint, TurnStatus,
    };
    use hoshi_store::Store;
    use serde_json::{Value, json};
    use std::sync::Arc;

    fn engine(name: &str) -> Engine {
        let data =
            std::env::temp_dir().join(format!("hoshi-thread-tools-{name}-{}", std::process::id()));
        let notify: Notify = Arc::new(|_| {});
        let engine = Engine::from_store(Store::open_in_memory().unwrap(), &data, notify).unwrap();
        for thread_id in ["t", "o"] {
            engine
                .dispatch(Command::ThreadCreate {
                    thread_id: thread_id.into(),
                    project_id: None,
                    title: String::new(),
                    workspace: ThreadWorkspace::Local,
                    provider: hoshi_core::default_provider(),
                    model: None,
                    effort: None,
                    fast_mode: false,
                    ultracode: false,
                    access_mode: AccessMode::AcceptEdits,
                    plan_mode: false,
                })
                .unwrap();
        }
        engine
    }

    fn add(engine: &Engine, thread_id: &str, id: &str, content: ItemContent) {
        let item = ThreadItem {
            id: id.into(),
            thread_id: thread_id.into(),
            created_at: "2026-11-15T10:00:00Z".into(),
            content,
        };
        lock(&engine.state)
            .commit(&[Event::ItemAdded { item }], "2026-11-15T10:00:00Z")
            .unwrap();
    }

    fn call(engine: &Engine, tool: &str, arguments: Value) -> Result<Value, String> {
        lock(&engine.state).call_tool("t", tool, &arguments)
    }

    #[test]
    fn an_item_reads_by_id_in_pages_also_from_the_thread_a_fork_came_from() {
        let engine = engine("read-item");
        let text = format!("{}é{}", "a".repeat(5), "b".repeat(4));
        add(
            &engine,
            "o",
            "o-answer",
            ItemContent::AssistantMessage { text: text.clone() },
        );
        lock(&engine.state)
            .decide_and_commit(Command::ThreadFork {
                thread_id: "o".into(),
                fork_thread_id: "f".into(),
                turn: TurnPoint {
                    turn_id: "o-answer".into(),
                    through_item_id: "o-answer".into(),
                    ordinal: 1,
                    status: TurnStatus::Completed,
                    provider: hoshi_core::default_provider(),
                    session_id: None,
                    cursor: None,
                },
            })
            .unwrap();
        let read = |offset: Option<u64>| {
            let mut arguments =
                json!({ "threadId": "f", "itemId": "o-answer", "maxCharsPerItem": 4 });
            if let Some(offset) = offset {
                arguments["textOffset"] = json!(offset);
            }
            call(&engine, "thread_read", arguments).unwrap()["item"].clone()
        };

        let mut pages = Vec::new();
        let mut offset = None;
        loop {
            let item = read(offset);
            pages.push(item["text"].as_str().unwrap().to_owned());
            assert_eq!(item["kind"], "assistant_message");
            assert_eq!(item["sourceThreadId"], "o");
            match item["nextTextOffset"].as_u64() {
                Some(next) => offset = Some(next),
                None => break,
            }
        }

        assert_eq!(pages, ["aaaa", "aébb", "bb"]);
        assert_eq!(pages.concat(), text);
        assert_eq!(read(Some(50))["text"], "");
        assert_eq!(read(Some(50))["nextTextOffset"], Value::Null);
        assert!(
            call(
                &engine,
                "thread_read",
                json!({ "threadId": "o", "itemId": "nope" })
            )
            .is_err()
        );
        assert!(
            call(
                &engine,
                "thread_read",
                json!({ "threadId": "o", "textOffset": 4 })
            )
            .is_err()
        );
    }

    #[test]
    fn an_agent_snoozes_and_wakes_a_thread_and_hears_why_it_cannot() {
        let engine = engine("snooze");
        let later = (Utc::now() + Duration::hours(2)).to_rfc3339();
        let snoozed_until = |thread_id: &str| {
            lock(&engine.state)
                .model
                .thread(thread_id)
                .unwrap()
                .snoozed_until
                .clone()
        };

        let own = call(&engine, "thread_snooze", json!({ "until": later })).unwrap();
        assert_eq!(own["threadId"], "t");
        assert!(own["snoozedUntil"].is_string());
        assert!(snoozed_until("t").is_some());
        let woken = call(&engine, "thread_snooze", json!({ "wake": true })).unwrap();
        assert_eq!(woken["snoozedUntil"], Value::Null);
        assert_eq!(snoozed_until("t"), None);
        call(
            &engine,
            "thread_snooze",
            json!({ "threadId": "o", "until": later }),
        )
        .unwrap();
        assert!(snoozed_until("o").is_some());

        let refused = |arguments: Value| call(&engine, "thread_snooze", arguments).unwrap_err();
        assert!(refused(json!({})).contains("`until`"));
        assert!(refused(json!({ "until": later, "wake": true })).contains("not both"));
        let passed = (Utc::now() - Duration::hours(1)).to_rfc3339();
        assert!(refused(json!({ "until": passed })).contains("after now"));
        assert!(refused(json!({ "until": "tomorrow" })).contains("RFC 3339"));

        lock(&engine.state)
            .queued
            .entry("t".into())
            .or_default()
            .push_back(QueuedMessage {
                id: "m2".into(),
                text: "also this".into(),
                attachments: Vec::new(),
                from_user: true,
                held: false,
            });
        assert!(refused(json!({ "until": later })).contains("message from the user waiting"));
        lock(&engine.state).queued.clear();

        add(
            &engine,
            "t",
            "approval",
            ItemContent::Approval {
                tool_call_id: None,
                tool_name: "Bash".into(),
                input: "{}".into(),
                reason: None,
                can_accept_for_session: false,
                status: RequestStatus::Pending,
                decision: None,
                feedback: None,
            },
        );
        assert!(
            refused(json!({ "until": later })).contains("waits for the user to answer an approval")
        );

        engine
            .dispatch(Command::ThreadArchive {
                thread_id: "o".into(),
                archived: true,
            })
            .unwrap();
        assert!(refused(json!({ "threadId": "o", "until": later })).contains("is archived"));
    }
}
