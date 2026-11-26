//! Codex app-server notifications as provider-neutral output. A [`Conversation`] follows the
//! thread a turn runs in and the subagent threads it starts; a [`Translator`] turns one
//! thread's items into messages, reasoning and tool calls.

use crate::{truncate, truncate_json};
use hoshi_core::{
    BackgroundTask, ContextUsage, Effort, ProviderOutput, Task, TaskStatus, ToolDetail,
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

const TOOL_INPUT_MAX_CHARS: usize = 2_000;
const TOOL_OUTPUT_MAX_CHARS: usize = 20_000;
const DIFF_MAX_CHARS: usize = 60_000;
/// How much of a subagent's prompt names its call when the agent gave it no name.
const DESCRIPTION_MAX_CHARS: usize = 60;

/// A tool call as the transcript shows it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolCall {
    pub name: String,
    pub input: Value,
}

/// A command that runs, from its start.
#[derive(Debug, Clone)]
struct RunningCommand {
    item_id: String,
    process_id: Option<String>,
    description: String,
    /// Unix milliseconds.
    started_at: u64,
    /// Outlived the turn that started it.
    background: bool,
}

/// Which of a reasoning item's streams shows while it is written: its summary, or its raw
/// text with the index of the part written last.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ReasoningStream {
    Summary,
    Text(u64),
}

/// One thread's items.
#[derive(Debug, Default)]
pub(crate) struct Translator {
    tools: HashMap<String, ToolCall>,
    /// What running commands printed so far.
    outputs: HashMap<String, String>,
    commands: Vec<RunningCommand>,
    /// The commands left running in the background changed since last asked.
    background_changed: bool,
    reasoning: HashMap<String, ReasoningStream>,
    /// Web searches that started before their query was known.
    searches: HashSet<String>,
    /// The plan the agent proposed last, in plan mode.
    plan: Option<String>,
    /// The context after the last model call.
    used_tokens: Option<u64>,
    /// The context when the compaction running started.
    compacting: Option<u64>,
    last_message: Option<String>,
}

impl Translator {
    pub fn tool(&self, item_id: &str) -> Option<&ToolCall> {
        self.tools.get(item_id)
    }

    pub fn take_plan(&mut self) -> Option<String> {
        self.plan.take().filter(|plan| !plan.trim().is_empty())
    }

    /// The commands the agent left running in the background, when they changed since last
    /// asked.
    pub fn take_background_change(&mut self) -> Option<Vec<BackgroundTask>> {
        std::mem::take(&mut self.background_changed).then(|| self.background())
    }

    pub fn background(&self) -> Vec<BackgroundTask> {
        self.commands
            .iter()
            .filter(|command| command.background)
            .map(|command| BackgroundTask {
                task_id: command.item_id.clone(),
                tool_call_id: Some(command.item_id.clone()),
                description: command.description.clone(),
                started_at: command.started_at,
            })
            .collect()
    }

    /// The terminal a command runs in, by its call.
    pub fn process_id(&self, item_id: &str) -> Option<&str> {
        self.commands
            .iter()
            .find(|command| command.item_id == item_id)
            .and_then(|command| command.process_id.as_deref())
    }

    /// The turn completed with commands still running: they go on in the background, and
    /// their calls end with what they printed so far.
    pub fn settle(&mut self) -> Vec<ProviderOutput> {
        let mut outputs = Vec::new();
        for command in self
            .commands
            .iter_mut()
            .filter(|command| !command.background)
        {
            command.background = true;
            self.background_changed = true;
            let output = self.outputs.remove(&command.item_id).unwrap_or_default();
            outputs.push(ProviderOutput::ToolCallCompleted {
                item_id: command.item_id.clone(),
                output: truncate(&output, TOOL_OUTPUT_MAX_CHARS),
                is_error: false,
                detail: None,
            });
        }
        outputs
    }

    /// The thread's terminals closed, and the commands in them with them.
    pub fn close_terminals(&mut self) {
        let before = self.commands.len();
        self.commands.retain(|command| !command.background);
        self.background_changed |= self.commands.len() != before;
    }

    /// The output one notification makes; those hoshi does not show make none.
    pub fn notification(&mut self, method: &str, params: &Value) -> Vec<ProviderOutput> {
        let item_id = || params["itemId"].as_str().unwrap_or_default().to_owned();
        let delta = || params["delta"].as_str().unwrap_or_default().to_owned();
        match method {
            "item/started" => {
                self.track(&params["item"], params["startedAtMs"].as_u64());
                self.started(&params["item"])
            }
            "item/completed" => self.completed(&params["item"]),
            "item/agentMessage/delta" if !delta().is_empty() => vec![ProviderOutput::TextDelta {
                item_id: item_id(),
                delta: delta(),
            }],
            "item/plan/delta" if !delta().is_empty() => vec![ProviderOutput::TextDelta {
                item_id: plan_request_id(params["turnId"].as_str().unwrap_or_default()),
                delta: delta(),
            }],
            "item/reasoning/summaryTextDelta" if !delta().is_empty() => {
                self.reasoning_delta(item_id(), ReasoningStream::Summary, delta())
            }
            "item/reasoning/textDelta" if !delta().is_empty() => {
                let index = params["contentIndex"].as_u64().unwrap_or_default();
                self.reasoning_delta(item_id(), ReasoningStream::Text(index), delta())
            }
            "item/reasoning/summaryPartAdded"
                if params["summaryIndex"].as_u64() > Some(0)
                    && self.reasoning.get(&item_id()) == Some(&ReasoningStream::Summary) =>
            {
                vec![ProviderOutput::ReasoningDelta {
                    item_id: item_id(),
                    delta: "\n\n".into(),
                }]
            }
            "item/commandExecution/outputDelta" => self.output_delta(&item_id(), &delta()),
            "turn/plan/updated" => vec![ProviderOutput::TaskList {
                tasks: tasks(&params["plan"]),
            }],
            "thread/tokenUsage/updated" => self.token_usage(&params["tokenUsage"]),
            _ => Vec::new(),
        }
    }

    /// Streams one of a reasoning item's streams: the first that writes to it.
    fn reasoning_delta(
        &mut self,
        item_id: String,
        stream: ReasoningStream,
        delta: String,
    ) -> Vec<ProviderOutput> {
        let shown = self.reasoning.entry(item_id.clone()).or_insert(stream);
        let delta = match (*shown, stream) {
            (ReasoningStream::Summary, ReasoningStream::Summary) => delta,
            (ReasoningStream::Text(last), ReasoningStream::Text(index)) => {
                *shown = stream;
                if index > last {
                    format!("\n\n{delta}")
                } else {
                    delta
                }
            }
            _ => return Vec::new(),
        };
        vec![ProviderOutput::ReasoningDelta { item_id, delta }]
    }

    /// Follows a command from its start until it ends.
    fn track(&mut self, item: &Value, started_at: Option<u64>) {
        let Some(id) = item["id"].as_str() else {
            return;
        };
        let running = item["type"] == "commandExecution"
            && matches!(item["status"].as_str(), None | Some("inProgress"));
        if !running || self.commands.iter().any(|command| command.item_id == id) {
            return;
        }
        self.commands.push(RunningCommand {
            item_id: id.to_owned(),
            process_id: item["processId"].as_str().map(str::to_owned),
            description: unwrap_shell(item["command"].as_str().unwrap_or_default()),
            started_at: started_at.unwrap_or_else(now_ms),
            background: false,
        });
    }

    /// Shows a call once; later starts of it make nothing.
    pub fn start(&mut self, item_id: &str, name: String, input: Value) -> Vec<ProviderOutput> {
        if self.tools.contains_key(item_id) {
            return Vec::new();
        }
        let started = ProviderOutput::ToolCallStarted {
            item_id: item_id.to_owned(),
            name: name.clone(),
            input: truncate_json(&input, TOOL_INPUT_MAX_CHARS),
        };
        self.tools
            .insert(item_id.to_owned(), ToolCall { name, input });
        vec![started]
    }

    fn started(&mut self, item: &Value) -> Vec<ProviderOutput> {
        let Some(id) = item["id"].as_str() else {
            return Vec::new();
        };
        match item["type"].as_str().unwrap_or_default() {
            "commandExecution" => {
                let (name, input) = command_call(item);
                self.start(id, name, input)
            }
            "fileChange" => file_changes(item)
                .into_iter()
                .flat_map(|change| self.start(&change.id, change.name, change.input))
                .collect(),
            "mcpToolCall" => {
                let name = format!(
                    "mcp__{}__{}",
                    item["server"].as_str().unwrap_or_default(),
                    item["tool"].as_str().unwrap_or_default()
                );
                self.start(id, name, item["arguments"].clone())
            }
            "dynamicToolCall" => {
                let name = item["tool"].as_str().unwrap_or("tool").to_owned();
                self.start(id, name, item["arguments"].clone())
            }
            "webSearch" => match web_call(item) {
                Some((name, input)) => self.start(id, name, input),
                None => {
                    self.searches.insert(id.to_owned());
                    Vec::new()
                }
            },
            "imageView" => self.start(id, "Read".into(), json!({ "file_path": item["path"] })),
            "contextCompaction" => {
                self.compacting = Some(self.used_tokens.unwrap_or_default());
                vec![ProviderOutput::CompactionStarted]
            }
            _ => Vec::new(),
        }
    }

    fn completed(&mut self, item: &Value) -> Vec<ProviderOutput> {
        let Some(id) = item["id"].as_str() else {
            return Vec::new();
        };
        let status = item["status"].as_str().unwrap_or("completed");
        let failed = matches!(status, "failed" | "declined");
        match item["type"].as_str().unwrap_or_default() {
            "agentMessage" => {
                let text = item["text"].as_str().unwrap_or_default();
                if text.is_empty() {
                    return Vec::new();
                }
                self.last_message = Some(text.to_owned());
                vec![ProviderOutput::AssistantMessage {
                    item_id: id.to_owned(),
                    text: text.to_owned(),
                }]
            }
            "reasoning" => {
                self.reasoning.remove(id);
                let text = reasoning_text(item);
                if text.trim().is_empty() {
                    return Vec::new();
                }
                vec![ProviderOutput::Reasoning {
                    item_id: id.to_owned(),
                    text,
                }]
            }
            "plan" => {
                self.plan = item["text"].as_str().map(str::to_owned);
                Vec::new()
            }
            "commandExecution" => {
                let position = self
                    .commands
                    .iter()
                    .position(|command| command.item_id == id);
                if let Some(command) = position.map(|position| self.commands.remove(position))
                    && command.background
                {
                    self.background_changed = true;
                    return Vec::new();
                }
                let mut outputs = self.started(item);
                let streamed = self.outputs.remove(id).unwrap_or_default();
                let output = item["aggregatedOutput"]
                    .as_str()
                    .map_or(streamed, str::to_owned);
                let exit_code = item["exitCode"].as_i64();
                outputs.push(ProviderOutput::ToolCallCompleted {
                    item_id: id.to_owned(),
                    output: truncate(&output, TOOL_OUTPUT_MAX_CHARS),
                    is_error: failed || exit_code.is_some_and(|code| code != 0),
                    detail: None,
                });
                outputs
            }
            "fileChange" => {
                let mut outputs = self.started(item);
                for change in file_changes(item) {
                    outputs.push(ProviderOutput::ToolCallCompleted {
                        item_id: change.id,
                        output: String::new(),
                        is_error: status != "completed",
                        detail: (status == "completed").then_some(change.detail),
                    });
                }
                outputs
            }
            "mcpToolCall" | "dynamicToolCall" => {
                let mut outputs = self.started(item);
                let error = item["error"]["message"].as_str();
                let output = error.map_or_else(|| tool_result(item), str::to_owned);
                outputs.push(ProviderOutput::ToolCallCompleted {
                    item_id: id.to_owned(),
                    output: truncate(&output, TOOL_OUTPUT_MAX_CHARS),
                    is_error: failed || error.is_some() || item["success"] == false,
                    detail: None,
                });
                outputs
            }
            "webSearch" => {
                let mut outputs = Vec::new();
                if self.searches.remove(id) || !self.tools.contains_key(id) {
                    let (name, input) = web_call(item)
                        .unwrap_or_else(|| ("WebSearch".into(), json!({ "query": item["query"] })));
                    outputs.extend(self.start(id, name, input));
                }
                outputs.push(ProviderOutput::ToolCallCompleted {
                    item_id: id.to_owned(),
                    output: truncate(&search_results(item), TOOL_OUTPUT_MAX_CHARS),
                    is_error: false,
                    detail: None,
                });
                outputs
            }
            "imageView" => {
                let mut outputs = self.started(item);
                outputs.push(ProviderOutput::ToolCallCompleted {
                    item_id: id.to_owned(),
                    output: String::new(),
                    is_error: false,
                    detail: None,
                });
                outputs
            }
            "contextCompaction" => {
                let before = self.compacting.take().unwrap_or_default();
                vec![ProviderOutput::CompactionFinished {
                    before_tokens: before,
                    after_tokens: self.used_tokens.unwrap_or(before),
                }]
            }
            _ => Vec::new(),
        }
    }

    fn output_delta(&mut self, item_id: &str, delta: &str) -> Vec<ProviderOutput> {
        let backgrounded = self
            .commands
            .iter()
            .any(|command| command.item_id == item_id && command.background);
        if backgrounded {
            return Vec::new();
        }
        let output = self.outputs.entry(item_id.to_owned()).or_default();
        if output.len() > TOOL_OUTPUT_MAX_CHARS * 4 {
            return Vec::new();
        }
        output.push_str(delta);
        vec![ProviderOutput::ToolOutput {
            item_id: item_id.to_owned(),
            output: truncate(output, TOOL_OUTPUT_MAX_CHARS),
        }]
    }

    fn token_usage(&mut self, usage: &Value) -> Vec<ProviderOutput> {
        let Some(used) = usage["last"]["totalTokens"].as_u64() else {
            return Vec::new();
        };
        self.used_tokens = Some(used);
        vec![ProviderOutput::ContextUsage(ContextUsage {
            used_tokens: used,
            max_tokens: usage["modelContextWindow"].as_u64(),
            cache_ttl_secs: None,
            cache_expires_at: None,
        })]
    }
}

/// hoshi's id for the plan a turn proposes, which its streamed text shows under until then.
pub(crate) fn plan_request_id(turn_id: &str) -> String {
    format!("plan-{turn_id}")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

fn reasoning_text(item: &Value) -> String {
    let parts = |key: &str| -> Vec<String> {
        item[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|part| part.as_str().map(str::to_owned))
            .collect()
    };
    let summary = parts("summary");
    if summary.is_empty() {
        parts("content").join("\n\n")
    } else {
        summary.join("\n\n")
    }
}

fn tasks(plan: &Value) -> Vec<Task> {
    plan.as_array()
        .into_iter()
        .flatten()
        .map(|step| Task {
            subject: step["step"].as_str().unwrap_or_default().to_owned(),
            active_form: None,
            status: match step["status"].as_str() {
                Some("inProgress") => TaskStatus::InProgress,
                Some("completed") => TaskStatus::Completed,
                _ => TaskStatus::Pending,
            },
        })
        .collect()
}

/// A shell command as the transcript names it: a file it only reads as `Read`, a search as
/// `Grep`, anything else as `Bash` with the command the shell runs, without its wrapper.
pub(crate) fn command_call(item: &Value) -> (String, Value) {
    let actions = item["commandActions"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    if let [action] = actions {
        match action["type"].as_str() {
            Some("read") if action["path"].is_string() => {
                return ("Read".into(), json!({ "file_path": action["path"] }));
            }
            Some("search") if action["query"].is_string() => {
                return (
                    "Grep".into(),
                    json!({ "pattern": action["query"], "path": action["path"] }),
                );
            }
            _ => {
                if let Some(command) = action["command"].as_str() {
                    return ("Bash".into(), json!({ "command": command }));
                }
            }
        }
    }
    let command = item["command"].as_str().unwrap_or_default();
    ("Bash".into(), json!({ "command": unwrap_shell(command) }))
}

/// The command a shell wrapper runs, as in `bash -lc "ls"` or `powershell.exe -Command 'ls'`;
/// anything else as it is.
pub(crate) fn unwrap_shell(command: &str) -> String {
    let trimmed = command.trim();
    let (program, rest) = first_word(trimmed);
    let name = program
        .trim_matches(['"', '\''])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    let flags: &[&str] = match name {
        "bash" | "sh" | "zsh" | "dash" | "ksh" | "fish" => &["-c", "-lc", "-ic"],
        "powershell" | "pwsh" => &["-command", "-c"],
        "cmd" => &["/c", "/s/c"],
        _ => return trimmed.to_owned(),
    };
    let mut rest = rest;
    loop {
        let (word, after) = first_word(rest);
        if word.is_empty() {
            return trimmed.to_owned();
        }
        if flags.contains(&word.to_ascii_lowercase().as_str()) {
            return unquote(after.trim());
        }
        if !word.starts_with(['-', '/']) {
            return trimmed.to_owned();
        }
        rest = after;
    }
}

/// The first word of `text`, quotes kept, and what follows it.
fn first_word(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    let end = match text.chars().next() {
        Some(quote @ ('"' | '\'')) => text[1..].find(quote).map_or(text.len(), |end| end + 2),
        _ => text.find(char::is_whitespace).unwrap_or(text.len()),
    };
    (&text[..end], &text[end..])
}

fn unquote(text: &str) -> String {
    let inner = |quote: char| {
        text.strip_prefix(quote)
            .and_then(|text| text.strip_suffix(quote))
    };
    if let Some(inner) = inner('\'') {
        return inner.replace("''", "'");
    }
    if let Some(inner) = inner('"') {
        return inner
            .replace("\\\"", "\"")
            .replace("\\$", "$")
            .replace("\\`", "`")
            .replace("\\\\", "\\");
    }
    text.to_owned()
}

/// One file of a patch.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FileChange {
    /// The patch's item id for its first file, then `<id>#<n>`.
    pub id: String,
    pub name: String,
    pub input: Value,
    pub detail: ToolDetail,
}

/// A patch's files as calls the transcript knows: a new file as `Write`, a changed or
/// removed one as `Edit`, each with its change as unified diff hunks.
pub(crate) fn file_changes(item: &Value) -> Vec<FileChange> {
    let id = item["id"].as_str().unwrap_or_default();
    item["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(index, change)| {
            let path = change["kind"]["move_path"]
                .as_str()
                .or_else(|| change["path"].as_str())
                .unwrap_or_default()
                .to_owned();
            let diff = change["diff"].as_str().unwrap_or_default();
            let lines = |sign: char| -> Vec<&str> {
                diff.lines()
                    .filter(|line| line.starts_with(sign))
                    .map(|line| &line[1..])
                    .collect()
            };
            let (name, input, hunks) = match change["kind"]["type"].as_str() {
                Some("add") => (
                    "Write",
                    json!({ "file_path": path, "content": diff }),
                    whole_file_hunk(diff, '+'),
                ),
                Some("delete") => (
                    "Edit",
                    json!({ "file_path": path, "old_string": diff, "new_string": "" }),
                    whole_file_hunk(diff, '-'),
                ),
                _ => {
                    let hunks = diff.find("@@").map_or(diff, |start| &diff[start..]);
                    (
                        "Edit",
                        json!({
                            "file_path": path,
                            "old_string": lines('-').join("\n"),
                            "new_string": lines('+').join("\n"),
                        }),
                        hunks.to_owned(),
                    )
                }
            };
            let count = |sign: char| {
                hunks
                    .lines()
                    .filter(|line| line.starts_with(sign))
                    .filter(|line| !line.starts_with("+++") && !line.starts_with("---"))
                    .count() as u32
            };
            FileChange {
                id: if index == 0 {
                    id.to_owned()
                } else {
                    format!("{id}#{index}")
                },
                name: name.into(),
                input,
                detail: ToolDetail::FileChange {
                    additions: count('+'),
                    deletions: count('-'),
                    diff: Some(truncate(&hunks, DIFF_MAX_CHARS)),
                    path,
                },
            }
        })
        .collect()
}

/// A whole file added (`+`) or removed (`-`) as one hunk.
fn whole_file_hunk(content: &str, sign: char) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let range = format!("1,{}", lines.len());
    let header = if sign == '+' {
        format!("@@ -0,0 +{range} @@")
    } else {
        format!("@@ -{range} +0,0 @@")
    };
    let mut hunk = header;
    for line in lines {
        hunk.push('\n');
        hunk.push(sign);
        hunk.push_str(line);
    }
    hunk.push('\n');
    hunk
}

/// A page the search opened as `WebFetch`, else the search as `WebSearch`; `None` while
/// nothing says what it looks for.
fn web_call(item: &Value) -> Option<(String, Value)> {
    let action = &item["action"];
    if action["type"] == "openPage"
        && let Some(url) = action["url"].as_str()
    {
        return Some(("WebFetch".into(), json!({ "url": url })));
    }
    let query = item["query"]
        .as_str()
        .filter(|query| !query.is_empty())
        .or_else(|| action["query"].as_str())
        .or_else(|| action["pattern"].as_str())?;
    Some(("WebSearch".into(), json!({ "query": query })))
}

fn search_results(item: &Value) -> String {
    item["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|result| {
            [&result["title"], &result["url"], &result["snippet"]]
                .into_iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|result| !result.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The text an MCP or dynamic tool answered with.
fn tool_result(item: &Value) -> String {
    let blocks = item["result"]["content"]
        .as_array()
        .or_else(|| item["contentItems"].as_array());
    let texts: Vec<&str> = blocks
        .into_iter()
        .flatten()
        .filter_map(|block| block["text"].as_str())
        .collect();
    if !texts.is_empty() {
        return texts.join("\n");
    }
    match &item["result"]["structuredContent"] {
        Value::Null => String::new(),
        structured => structured.to_string(),
    }
}

/// A subagent thread.
#[derive(Debug)]
struct Agent {
    /// The call in the turn's own thread that started it, or the subagent that did.
    call: String,
    /// Started by the turn's own thread, so its end ends the call.
    direct: bool,
    translator: Translator,
    turn_id: Option<String>,
    done: bool,
}

/// The thread a turn runs in and the subagent threads it starts.
#[derive(Debug)]
pub(crate) struct Conversation {
    thread_id: String,
    turn_id: Option<String>,
    translator: Translator,
    agents: HashMap<String, Agent>,
    /// The MCP servers whose failure to start was reported.
    failed_servers: HashSet<String>,
}

impl Conversation {
    pub fn new(thread_id: String) -> Self {
        Self {
            thread_id,
            turn_id: None,
            translator: Translator::default(),
            agents: HashMap::new(),
            failed_servers: HashSet::new(),
        }
    }

    pub fn settle(&mut self) -> Vec<ProviderOutput> {
        self.translator.settle()
    }

    pub fn background(&self) -> Vec<BackgroundTask> {
        self.translator.background()
    }

    pub fn take_background_change(&mut self) -> Option<Vec<BackgroundTask>> {
        self.translator.take_background_change()
    }

    pub fn process_id(&self, item_id: &str) -> Option<&str> {
        self.translator.process_id(item_id)
    }

    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    /// The turn running in the thread, once it started.
    pub fn turn_id(&self) -> Option<&str> {
        self.turn_id.as_deref()
    }

    pub fn set_turn(&mut self, turn_id: Option<String>) {
        self.turn_id = turn_id;
    }

    pub fn take_plan(&mut self) -> Option<String> {
        self.translator.take_plan()
    }

    /// The call `item_id` in `thread_id`, the turn's own or a subagent's.
    pub fn tool(&self, thread_id: &str, item_id: &str) -> Option<ToolCall> {
        let translator = if thread_id == self.thread_id {
            &self.translator
        } else {
            &self.agents.get(thread_id)?.translator
        };
        translator.tool(item_id).cloned()
    }

    /// The running turns of the subagents call `tool_use_id` started, as `(thread, turn)`.
    pub fn subagent_turns(&self, tool_use_id: &str) -> Vec<(String, String)> {
        self.agents
            .iter()
            .filter(|(_, agent)| agent.call == tool_use_id && !agent.done)
            .filter_map(|(thread, agent)| Some((thread.clone(), agent.turn_id.clone()?)))
            .collect()
    }

    /// The output one notification makes, as the turn's own or a subagent's.
    pub fn notification(&mut self, method: &str, params: &Value) -> Vec<ProviderOutput> {
        let Some(thread) = params["threadId"].as_str() else {
            return Vec::new();
        };
        if thread == self.thread_id {
            return self.own(method, params);
        }
        if self.agents.contains_key(thread) {
            return self.subagent(thread, method, params);
        }
        Vec::new()
    }

    /// A hook that stopped the running turn, with what it said.
    fn hook_stopped(&self, params: &Value) -> Option<ProviderOutput> {
        let run = &params["run"];
        let turn = params["turnId"].as_str()?;
        if run["status"] != "stopped" || self.turn_id.as_deref().is_some_and(|own| own != turn) {
            return None;
        }
        let reason = run["entries"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|entry| entry["kind"] == "stop")
            .filter_map(|entry| entry["text"].as_str().map(str::trim))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(
                "
",
            );
        if reason.is_empty() {
            return None;
        }
        Some(ProviderOutput::HookStopped {
            event: run["eventName"].as_str().unwrap_or("stop").to_owned(),
            source: run["source"]
                .as_str()
                .filter(|source| *source != "unknown")
                .map(str::to_owned),
            reason,
        })
    }

    fn own(&mut self, method: &str, params: &Value) -> Vec<ProviderOutput> {
        let item = &params["item"];
        match (method, item["type"].as_str()) {
            ("turn/started", _) => {
                self.turn_id = params["turn"]["id"].as_str().map(str::to_owned);
                Vec::new()
            }
            ("item/started" | "item/completed", Some("collabAgentToolCall")) => {
                self.collab(method, item, None)
            }
            ("item/started" | "item/completed", Some("subAgentActivity")) => {
                self.activity(method, item, None)
            }
            ("model/rerouted", _) => params["toModel"]
                .as_str()
                .map(|model| ProviderOutput::ModelRerouted {
                    model: model.to_owned(),
                    reason: params["reason"].as_str().map(str::to_owned),
                })
                .into_iter()
                .collect(),
            ("mcpServer/startupStatus/updated", _) => self.server_status(params),
            ("hook/completed", _) => self.hook_stopped(params).into_iter().collect(),
            ("thread/closed", _) => {
                self.translator.close_terminals();
                Vec::new()
            }
            _ => self.translator.notification(method, params),
        }
    }

    /// An MCP server that failed to start, reported once until it starts again.
    fn server_status(&mut self, params: &Value) -> Vec<ProviderOutput> {
        let Some(name) = params["name"].as_str() else {
            return Vec::new();
        };
        if params["status"] != "failed" {
            self.failed_servers.remove(name);
            return Vec::new();
        }
        if !self.failed_servers.insert(name.to_owned()) {
            return Vec::new();
        }
        vec![ProviderOutput::McpServerFailed {
            name: name.to_owned(),
            error: params["error"]
                .as_str()
                .map(str::trim)
                .unwrap_or_default()
                .to_owned(),
        }]
    }

    fn subagent(&mut self, thread: &str, method: &str, params: &Value) -> Vec<ProviderOutput> {
        let item = &params["item"];
        let call = self.agents[thread].call.clone();
        let mut outputs = Vec::new();
        match (method, item["type"].as_str()) {
            ("turn/started", _) => {
                let agent = self.agents.get_mut(thread).expect("known above");
                agent.turn_id = params["turn"]["id"].as_str().map(str::to_owned);
                return Vec::new();
            }
            ("turn/completed", _) => return self.subagent_ended(thread, false),
            ("item/started" | "item/completed", Some("collabAgentToolCall")) => {
                self.collab(method, item, Some(&call));
            }
            ("item/started" | "item/completed", Some("subAgentActivity")) => {
                self.activity(method, item, Some(&call));
            }
            _ => {}
        }
        let own = self
            .agents
            .get_mut(thread)
            .expect("known above")
            .translator
            .notification(method, params);
        outputs.extend(within(&call, own));
        outputs
    }

    /// Ends the call a subagent ran for, with its last message.
    fn subagent_ended(&mut self, thread: &str, stopped: bool) -> Vec<ProviderOutput> {
        let mut outputs = Vec::new();
        let Some(agent) = self.agents.get_mut(thread) else {
            return outputs;
        };
        agent.turn_id = None;
        if !agent.direct || agent.done {
            return outputs;
        }
        agent.done = true;
        outputs.push(ProviderOutput::ToolCallCompleted {
            item_id: agent.call.clone(),
            output: truncate(
                agent.translator.last_message.as_deref().unwrap_or_default(),
                TOOL_OUTPUT_MAX_CHARS,
            ),
            is_error: false,
            detail: None,
        });
        if stopped {
            outputs.push(ProviderOutput::ToolCallStopped {
                item_id: agent.call.clone(),
            });
        }
        outputs
    }

    fn register(&mut self, thread: &str, call: String, direct: bool) {
        self.agents.entry(thread.to_owned()).or_insert(Agent {
            call,
            direct,
            translator: Translator::default(),
            turn_id: None,
            done: false,
        });
    }

    /// The model a subagent runs on, once known.
    fn subagent_model(call: &str, item: &Value) -> Option<ProviderOutput> {
        let model = item["model"].as_str().filter(|model| !model.is_empty())?;
        Some(ProviderOutput::ToolCallDetail {
            item_id: call.to_owned(),
            detail: ToolDetail::Subagent {
                model: model.to_owned(),
                effort: item["reasoningEffort"].as_str().and_then(effort),
                resume_id: None,
            },
        })
    }

    /// An agent tool call: one that spawns subagents shows as `Task`; waiting for them and
    /// messaging them show nothing of their own. `owner` is the call of the subagent that
    /// made it, when one did.
    fn collab(&mut self, method: &str, item: &Value, owner: Option<&str>) -> Vec<ProviderOutput> {
        if item["tool"] != "spawnAgent" {
            return Vec::new();
        }
        let id = item["id"].as_str().unwrap_or_default();
        let prompt = item["prompt"].as_str().unwrap_or_default();
        let mut outputs = Vec::new();
        if owner.is_none() {
            outputs.extend(self.translator.start(
                id,
                "Task".into(),
                json!({ "description": describe(prompt), "prompt": prompt }),
            ));
        }
        if method != "item/completed" {
            return outputs;
        }
        for thread in item["receiverThreadIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let call = owner.unwrap_or(id).to_owned();
            self.register(thread, call, owner.is_none());
        }
        if owner.is_some() {
            return outputs;
        }
        if item["status"] == "failed" {
            outputs.push(ProviderOutput::ToolCallCompleted {
                item_id: id.to_owned(),
                output: String::new(),
                is_error: true,
                detail: None,
            });
        } else {
            outputs.extend(Self::subagent_model(id, item));
        }
        outputs
    }

    /// A subagent started or ended, as Codex reports agents it runs on its own.
    fn activity(&mut self, method: &str, item: &Value, owner: Option<&str>) -> Vec<ProviderOutput> {
        let Some(thread) = item["agentThreadId"].as_str() else {
            return Vec::new();
        };
        match item["kind"].as_str() {
            Some("started") if method == "item/started" => {
                let id = item["id"].as_str().unwrap_or_default();
                self.register(thread, owner.unwrap_or(id).to_owned(), owner.is_none());
                if owner.is_some() {
                    return Vec::new();
                }
                let name = item["agentPath"]
                    .as_str()
                    .and_then(|path| path.rsplit('/').next())
                    .unwrap_or_default()
                    .replace('_', " ");
                let mut outputs =
                    self.translator
                        .start(id, "Task".into(), json!({ "description": name }));
                outputs.extend(Self::subagent_model(id, item));
                outputs
            }
            Some(kind @ ("completed" | "interrupted")) if method == "item/completed" => {
                self.subagent_ended(thread, kind == "interrupted")
            }
            _ => Vec::new(),
        }
    }
}

/// A short name for a subagent's call, from its prompt.
fn describe(prompt: &str) -> String {
    let line = prompt.lines().next().unwrap_or_default().trim();
    let sentence = line
        .split_inclusive(". ")
        .next()
        .unwrap_or(line)
        .trim()
        .trim_end_matches('.');
    truncate(sentence, DESCRIPTION_MAX_CHARS)
}

/// `outputs` as what the subagent `call` started did.
fn within(call: &str, outputs: Vec<ProviderOutput>) -> Vec<ProviderOutput> {
    outputs
        .into_iter()
        .filter(|output| {
            !matches!(
                output,
                ProviderOutput::ContextUsage(_)
                    | ProviderOutput::TaskList { .. }
                    | ProviderOutput::CompactionStarted
                    | ProviderOutput::CompactionFinished { .. }
            )
        })
        .map(|output| ProviderOutput::Subagent {
            tool_use_id: call.to_owned(),
            output: Box::new(output),
        })
        .collect()
}

pub(crate) fn effort(value: &str) -> Option<Effort> {
    match value {
        "low" => Some(Effort::Low),
        "medium" => Some(Effort::Medium),
        "high" => Some(Effort::High),
        "xhigh" => Some(Effort::Xhigh),
        "max" => Some(Effort::Max),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
