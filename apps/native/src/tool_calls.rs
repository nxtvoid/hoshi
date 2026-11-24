//! How a tool call reads in the transcript: its action, a verb and its target, and what a
//! run of calls did, from the call's name and JSON input.

use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolAction {
    Command,
    Edit,
    Read,
    Search,
    Web,
    Hoshi(HoshiTool),
    Other,
}

/// What a call to one of hoshi's own tools counts as in a run's summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HoshiTool {
    HtmlPreview,
    HtmlRender,
    Delegate,
    TaskStatus,
    TaskCancel,
    ThreadList,
    ThreadRead,
    ThreadSend,
    ThreadWait,
    ThreadSnooze,
    OpenPullRequest,
    WatchPullRequest,
    UnwatchPullRequest,
    BackgroundWait,
    Browser,
}

const HOSHI_PREFIX: &str = "mcp__hoshi__";

/// One of hoshi's own tools, by its name without the server.
fn hoshi_tool(name: &str) -> Option<&str> {
    name.strip_prefix(HOSHI_PREFIX)
        .filter(|tool| hoshi_group(tool).is_some())
}

fn hoshi_group(tool: &str) -> Option<HoshiTool> {
    Some(match tool {
        "html_preview" => HoshiTool::HtmlPreview,
        "html_render" => HoshiTool::HtmlRender,
        "delegate_task" => HoshiTool::Delegate,
        "task_status" => HoshiTool::TaskStatus,
        "task_cancel" => HoshiTool::TaskCancel,
        "thread_list" => HoshiTool::ThreadList,
        "thread_read" => HoshiTool::ThreadRead,
        "thread_send" => HoshiTool::ThreadSend,
        "thread_wait" => HoshiTool::ThreadWait,
        "thread_snooze" => HoshiTool::ThreadSnooze,
        "create_pull_request" => HoshiTool::OpenPullRequest,
        "watch_pull_request" => HoshiTool::WatchPullRequest,
        "unwatch_pull_request" => HoshiTool::UnwatchPullRequest,
        "await_background_tasks" => HoshiTool::BackgroundWait,
        "preview_open"
        | "preview_navigate"
        | "preview_snapshot"
        | "preview_act"
        | "preview_evaluate"
        | "preview_wait_for"
        | "preview_resize"
        | "preview_set_appearance"
        | "preview_status"
        | "preview_close" => HoshiTool::Browser,
        _ => return None,
    })
}

/// What a call to one of hoshi's tools does while it runs, or did once done: "Previewed an
/// HTML page".
pub fn hoshi_label(name: &str, running: bool) -> Option<String> {
    use crate::m;
    let (live, done) = match hoshi_tool(name)? {
        "html_preview" => (
            m::hoshi_tool_live_html_preview(),
            m::hoshi_tool_done_html_preview(),
        ),
        "html_render" => (
            m::hoshi_tool_live_html_render(),
            m::hoshi_tool_done_html_render(),
        ),
        "delegate_task" => (m::hoshi_tool_live_delegate(), m::hoshi_tool_done_delegate()),
        "task_status" => (
            m::hoshi_tool_live_task_status(),
            m::hoshi_tool_done_task_status(),
        ),
        "task_cancel" => (
            m::hoshi_tool_live_task_cancel(),
            m::hoshi_tool_done_task_cancel(),
        ),
        "thread_list" => (
            m::hoshi_tool_live_thread_list(),
            m::hoshi_tool_done_thread_list(),
        ),
        "thread_read" => (
            m::hoshi_tool_live_thread_read(),
            m::hoshi_tool_done_thread_read(),
        ),
        "thread_send" => (
            m::hoshi_tool_live_thread_send(),
            m::hoshi_tool_done_thread_send(),
        ),
        "thread_wait" => (
            m::hoshi_tool_live_thread_wait(),
            m::hoshi_tool_done_thread_wait(),
        ),
        "thread_snooze" => (
            m::hoshi_tool_live_thread_snooze(),
            m::hoshi_tool_done_thread_snooze(),
        ),
        "create_pull_request" => (
            m::hoshi_tool_live_open_pull_request(),
            m::hoshi_tool_done_open_pull_request(),
        ),
        "watch_pull_request" => (
            m::hoshi_tool_live_watch_pull_request(),
            m::hoshi_tool_done_watch_pull_request(),
        ),
        "unwatch_pull_request" => (
            m::hoshi_tool_live_unwatch_pull_request(),
            m::hoshi_tool_done_unwatch_pull_request(),
        ),
        "await_background_tasks" => (
            m::hoshi_tool_live_background_wait(),
            m::hoshi_tool_done_background_wait(),
        ),
        "preview_open" => (
            m::hoshi_tool_live_browser_open(),
            m::hoshi_tool_done_browser_open(),
        ),
        "preview_navigate" => (
            m::hoshi_tool_live_browser_navigate(),
            m::hoshi_tool_done_browser_navigate(),
        ),
        "preview_snapshot" => (
            m::hoshi_tool_live_browser_snapshot(),
            m::hoshi_tool_done_browser_snapshot(),
        ),
        "preview_evaluate" => (
            m::hoshi_tool_live_browser_evaluate(),
            m::hoshi_tool_done_browser_evaluate(),
        ),
        "preview_wait_for" => (
            m::hoshi_tool_live_browser_wait(),
            m::hoshi_tool_done_browser_wait(),
        ),
        "preview_resize" => (
            m::hoshi_tool_live_browser_resize(),
            m::hoshi_tool_done_browser_resize(),
        ),
        "preview_set_appearance" => (
            m::hoshi_tool_live_browser_appearance(),
            m::hoshi_tool_done_browser_appearance(),
        ),
        "preview_status" => (
            m::hoshi_tool_live_browser_status(),
            m::hoshi_tool_done_browser_status(),
        ),
        "preview_close" => (
            m::hoshi_tool_live_browser_close(),
            m::hoshi_tool_done_browser_close(),
        ),
        _ => (
            m::hoshi_tool_live_browser_act(),
            m::hoshi_tool_done_browser_act(),
        ),
    };
    Some(if running { live } else { done })
}

pub fn tool_action(name: &str) -> ToolAction {
    if let Some(tool) = name.strip_prefix(HOSHI_PREFIX).and_then(hoshi_group) {
        return ToolAction::Hoshi(tool);
    }
    match name {
        "Bash" | "PowerShell" | "BashOutput" => ToolAction::Command,
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => ToolAction::Edit,
        "Read" => ToolAction::Read,
        "Grep" | "Glob" => ToolAction::Search,
        "WebFetch" | "WebSearch" => ToolAction::Web,
        _ => ToolAction::Other,
    }
}

/// Calls that ask the user something, shown as requests rather than among the work.
pub fn is_request(name: &str) -> bool {
    matches!(
        name,
        "AskUserQuestion"
            | "ExitPlanMode"
            | "TodoWrite"
            | "TaskCreate"
            | "TaskUpdate"
            | "TaskList"
            | "TaskGet"
            | "mcp__hoshi__request_secret"
    )
}

/// Calls that hand work to another agent, which stand on their own row.
pub fn is_agent(name: &str) -> bool {
    is_subagent(name) || is_delegation(name)
}

pub fn is_subagent(name: &str) -> bool {
    matches!(name, "Agent" | "Task")
}

/// A workflow of agents the agent started in the background, which can be stopped as a
/// subagent can.
pub fn is_workflow(name: &str) -> bool {
    name == "Workflow"
}

/// A task delegated to a child thread, which runs on its own.
pub fn is_delegation(name: &str) -> bool {
    name == "mcp__hoshi__delegate_task"
}

/// A page the agent showed the user, on a row of its own.
pub fn is_html_render(name: &str) -> bool {
    name == "mcp__hoshi__html_render"
}

#[derive(Debug, PartialEq, Eq)]
pub struct ToolName {
    pub label: String,
    /// The MCP server an MCP tool comes from.
    pub source: Option<String>,
}

/// An MCP tool's own name, with its server: `mcp__claude_ai_Gmail__send_email` → `send email`
/// from `Gmail`.
pub fn tool_name(name: &str) -> ToolName {
    let words = |text: &str| {
        text.strip_prefix("claude_ai_")
            .unwrap_or(text)
            .replace('_', " ")
            .trim()
            .to_owned()
    };
    let Some((server, tool)) = name
        .strip_prefix("mcp__")
        .and_then(|rest| rest.split_once("__"))
        .filter(|(server, tool)| !server.is_empty() && !tool.is_empty())
    else {
        return ToolName {
            label: name.to_owned(),
            source: None,
        };
    };
    ToolName {
        label: words(tool),
        source: Some(words(server)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallStatus {
    Running,
    Failed,
    Stopped,
    Done,
}

pub fn call_status(has_output: bool, is_error: bool, interrupted: bool) -> CallStatus {
    if interrupted {
        CallStatus::Stopped
    } else if is_error {
        CallStatus::Failed
    } else if has_output {
        CallStatus::Done
    } else {
        CallStatus::Running
    }
}

/// What a call does while it runs, or did once done; `None` for a tool known only by name.
pub fn verb(action: ToolAction, running: bool) -> Option<String> {
    Some(match (action, running) {
        (ToolAction::Command, true) => crate::m::tool_live_command(),
        (ToolAction::Edit, true) => crate::m::tool_live_edit(),
        (ToolAction::Read, true) => crate::m::tool_live_read(),
        (ToolAction::Search, true) => crate::m::tool_live_search(),
        (ToolAction::Web, true) => crate::m::tool_live_web(),
        (ToolAction::Command, false) => crate::m::tool_done_command(),
        (ToolAction::Edit, false) => crate::m::tool_done_edit(),
        (ToolAction::Read, false) => crate::m::tool_done_read(),
        (ToolAction::Search, false) => crate::m::tool_done_search(),
        (ToolAction::Web, false) => crate::m::tool_done_web(),
        (ToolAction::Hoshi(_) | ToolAction::Other, _) => return None,
    })
}

const PATH_KEYS: [&str; 3] = ["file_path", "notebook_path", "path"];
const TARGET_KEYS: [&str; 7] = [
    "command",
    "file_path",
    "notebook_path",
    "path",
    "pattern",
    "url",
    "query",
];

fn parse_input(input: &str) -> Option<Map<String, Value>> {
    match serde_json::from_str(input) {
        Ok(Value::Object(map)) => Some(map),
        _ => None,
    }
}

/// The one argument that says what a call did, with paths made relative to `cwd`.
pub fn tool_target(name: &str, input: &str, cwd: Option<&str>) -> String {
    if name == "ToolSearch" {
        return String::new();
    }
    let Some(value) = parse_input(input) else {
        return input.to_owned();
    };
    for key in TARGET_KEYS {
        if let Some(Value::String(target)) = value.get(key) {
            return if PATH_KEYS.contains(&key) {
                relative_path(target, cwd)
            } else {
                target.clone()
            };
        }
    }
    if tool_action(name) == ToolAction::Other {
        String::new()
    } else {
        input.to_owned()
    }
}

/// A call's arguments one per line, `key  value`, as an opened row shows them; the input as
/// it came when it isn't a JSON object.
pub fn input_lines(input: &str) -> String {
    let Some(value) = parse_input(input) else {
        return input.to_owned();
    };
    let width = value
        .keys()
        .map(|key| key.chars().count())
        .max()
        .unwrap_or(0);
    value
        .iter()
        .map(|(key, value)| {
            let text = match value {
                Value::String(text) if !text.is_empty() => text.clone(),
                other => other.to_string(),
            };
            format!("{key:width$}  {text}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn relative_path(path: &str, cwd: Option<&str>) -> String {
    let Some(cwd) = cwd else {
        return path.to_owned();
    };
    let normalize = |value: &str| value.replace('\\', "/").trim_end_matches('/').to_owned();
    let (target, root) = (normalize(path), normalize(cwd));
    let windows = root.as_bytes().get(1) == Some(&b':')
        && root.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && root.as_bytes().get(2) == Some(&b'/');
    let compare = |value: &str| {
        if windows {
            value.to_lowercase()
        } else {
            value.to_owned()
        }
    };
    if compare(&target) == compare(&root) {
        return ".".into();
    }
    if compare(&target).starts_with(&format!("{}/", compare(&root))) {
        return target[root.len() + 1..].to_owned();
    }
    path.to_owned()
}

/// What a run of calls did, per action; `None` stands for the actions left unnamed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SummaryPart {
    pub action: Option<ToolAction>,
    pub count: usize,
}

const MAX_SUMMARY_ACTIONS: usize = 2;

fn priority(action: ToolAction) -> u8 {
    match action {
        ToolAction::Command | ToolAction::Edit => 0,
        ToolAction::Read | ToolAction::Search | ToolAction::Web | ToolAction::Hoshi(_) => 1,
        ToolAction::Other => 2,
    }
}

/// Counts per action, in the order they first ran: the two most telling are named, the rest
/// counted together. Edits count the files they touched. Takes `(name, input)` per call.
pub fn summarize<'a>(calls: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<SummaryPart> {
    let mut groups: Vec<(ToolAction, Vec<&str>)> = Vec::new();
    let mut total = 0;
    for (name, input) in calls {
        total += 1;
        let action = tool_action(name);
        match groups.iter_mut().find(|(known, _)| *known == action) {
            Some((_, inputs)) => inputs.push(input),
            None => groups.push((action, vec![input])),
        }
    }
    let mut named: Vec<_> = groups.iter().enumerate().collect();
    named.sort_by_key(|(index, (action, _))| (priority(*action), *index));
    named.truncate(MAX_SUMMARY_ACTIONS);
    named.sort_by_key(|(index, _)| *index);
    let calls: usize = named.iter().map(|(_, (_, inputs))| inputs.len()).sum();
    let mut parts: Vec<_> = named
        .into_iter()
        .map(|(_, (action, inputs))| SummaryPart {
            action: Some(*action),
            count: if *action == ToolAction::Edit {
                edited_files(inputs)
            } else {
                inputs.len()
            },
        })
        .collect();
    if total > calls {
        parts.push(SummaryPart {
            action: None,
            count: total - calls,
        });
    }
    parts
}

fn edited_files(inputs: &[&str]) -> usize {
    let mut files = std::collections::HashSet::new();
    let mut unnamed = 0;
    for input in inputs {
        let path = parse_input(input).and_then(|value| {
            PATH_KEYS.iter().find_map(|key| {
                value
                    .get(*key)?
                    .as_str()
                    .map(|path| path.replace('\\', "/"))
            })
        });
        match path {
            Some(path) => {
                files.insert(path);
            }
            None => unnamed += 1,
        }
    }
    files.len() + unnamed
}

fn hoshi_summary(tool: HoshiTool, count: usize) -> String {
    use crate::m;
    match tool {
        HoshiTool::HtmlPreview => m::hoshi_summary_html_preview(count),
        HoshiTool::HtmlRender => m::hoshi_summary_html_render(count),
        HoshiTool::Delegate => m::hoshi_summary_delegate(count),
        HoshiTool::TaskStatus => m::hoshi_summary_task_status(count),
        HoshiTool::TaskCancel => m::hoshi_summary_task_cancel(count),
        HoshiTool::ThreadList => m::hoshi_summary_thread_list(count),
        HoshiTool::ThreadRead => m::hoshi_summary_thread_read(count),
        HoshiTool::ThreadSend => m::hoshi_summary_thread_send(count),
        HoshiTool::ThreadWait => m::hoshi_summary_thread_wait(count),
        HoshiTool::ThreadSnooze => m::hoshi_summary_thread_snooze(count),
        HoshiTool::OpenPullRequest => m::hoshi_summary_open_pull_request(count),
        HoshiTool::WatchPullRequest => m::hoshi_summary_watch_pull_request(count),
        HoshiTool::UnwatchPullRequest => m::hoshi_summary_unwatch_pull_request(count),
        HoshiTool::BackgroundWait => m::hoshi_summary_background_wait(count),
        HoshiTool::Browser => m::hoshi_summary_browser(count),
    }
}

/// "Searched code 2 times and read 3 files".
pub fn summary_text(parts: &[SummaryPart]) -> String {
    let phrases: Vec<String> = parts
        .iter()
        .map(|part| match part.action {
            Some(ToolAction::Command) => crate::m::tool_summary_command(part.count),
            Some(ToolAction::Edit) => crate::m::tool_summary_edit(part.count),
            Some(ToolAction::Read) => crate::m::tool_summary_read(part.count),
            Some(ToolAction::Search) => crate::m::tool_summary_search(part.count),
            Some(ToolAction::Web) => crate::m::tool_summary_web(part.count),
            Some(ToolAction::Hoshi(tool)) => hoshi_summary(tool, part.count),
            Some(ToolAction::Other) => crate::m::tool_summary_other(part.count),
            None => crate::m::tool_summary_others(part.count),
        })
        .collect();
    let sentence = match phrases.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [first, second] => crate::m::list_of_two(first, second),
        [rest @ .., last] => crate::m::list_of_many(last, rest.join(", ")),
    };
    let mut chars = sentence.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => sentence,
    }
}

/// One line of reasoning to show while it is folded: the latest while it streams, the first
/// once it is done, without markdown marks.
pub fn reasoning_preview(text: &str, streaming: bool) -> String {
    let mut lines = text
        .lines()
        .map(crate::markdown::plain_text)
        .filter(|line| !line.is_empty());
    let line = if streaming {
        lines.next_back()
    } else {
        lines.next()
    };
    line.unwrap_or_default()
}

const THOUGHT_SOURCE_CHARS: usize = 4000;

/// What a thought is about, as a running group shows it above its action: its bold heading
/// when it opens with one, else its first sentence, without markdown marks.
pub fn thought_line(thought: &str) -> String {
    let source: String = thought.chars().take(THOUGHT_SOURCE_CHARS).collect();
    let first = source.trim_start().lines().next().unwrap_or_default();
    if let Some(heading) = first
        .trim_end()
        .strip_prefix("**")
        .and_then(|rest| rest.strip_suffix("**"))
        .filter(|heading| !heading.is_empty() && !heading.contains('*'))
    {
        return crate::markdown::plain_text(heading);
    }
    let text = crate::markdown::plain_text(&source);
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (at, &(_, char)) in chars.iter().enumerate() {
        if !matches!(char, '.' | '?' | '!') {
            continue;
        }
        let mut end = at + 1;
        if chars
            .get(end)
            .is_some_and(|(_, next)| matches!(next, '"' | '\'' | '\u{201d}' | '\u{2019}' | ')'))
        {
            end += 1;
        }
        if chars.get(end).is_some_and(|(_, next)| next.is_whitespace()) {
            return text[..chars[end].0].to_owned();
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{
        CallStatus, HoshiTool, SummaryPart, ToolAction, ToolName, call_status, hoshi_label,
        input_lines, reasoning_preview, relative_path, summarize, summary_text, thought_line,
        tool_action, tool_name, tool_target, verb,
    };

    #[test]
    fn hoshi_tools_read_as_what_they_did() {
        assert_eq!(
            hoshi_label("mcp__hoshi__html_preview", false).as_deref(),
            Some("Previewed an HTML page")
        );
        assert_eq!(
            hoshi_label("mcp__hoshi__preview_open", true).as_deref(),
            Some("Opening a page in the preview browser")
        );
        assert_eq!(hoshi_label("mcp__hoshi__unknown", false), None);
        assert_eq!(hoshi_label("mcp__other__html_preview", false), None);
        assert_eq!(
            tool_action("mcp__hoshi__preview_snapshot"),
            ToolAction::Hoshi(HoshiTool::Browser)
        );
    }

    #[test]
    fn a_run_names_hoshi_tools_before_the_rest() {
        let calls = [
            (
                "ToolSearch",
                r#"{"query":"select:mcp__hoshi__html_preview"}"#,
            ),
            ("Write", r#"{"file_path":"/tmp/page.html"}"#),
            ("mcp__hoshi__html_preview", r#"{"path":"/tmp/page.html"}"#),
        ];
        assert_eq!(
            summary_text(&summarize(calls)),
            "Changed 1 file, previewed 1 HTML page, and performed 1 other action"
        );
        assert_eq!(tool_target("ToolSearch", calls[0].1, None), "");
    }

    #[test]
    fn an_opened_call_lists_its_arguments() {
        assert_eq!(
            input_lines(r#"{"query":"select:a,b","max_results":2}"#),
            "query        select:a,b\nmax_results  2"
        );
        assert_eq!(input_lines("not json"), "not json");
    }

    #[test]
    fn names_a_call_by_its_verb_and_target() {
        let cwd = Some("C:\\work\\hoshi");
        let read = r#"{"file_path":"C:\\work\\hoshi\\src\\foo.ts"}"#;
        assert_eq!(tool_target("Read", read, cwd), "src/foo.ts");
        assert_eq!(verb(ToolAction::Read, false).as_deref(), Some("Read"));
        let bash = r#"{"command":"cargo test","description":"Run the tests"}"#;
        assert_eq!(tool_target("Bash", bash, cwd), "cargo test");
        assert_eq!(verb(ToolAction::Command, false).as_deref(), Some("Ran"));
        assert_eq!(verb(ToolAction::Command, true).as_deref(), Some("Running"));
        assert_eq!(
            tool_target("Grep", r#"{"pattern":"fn main"}"#, cwd),
            "fn main"
        );
        assert_eq!(verb(ToolAction::Other, false), None);
    }

    #[test]
    fn a_tool_without_a_known_argument_has_no_target() {
        assert_eq!(tool_target("mcp__x__y", r#"{"limit":3}"#, None), "");
        assert_eq!(
            tool_target("Read", r#"{"limit":3}"#, None),
            r#"{"limit":3}"#
        );
        assert_eq!(tool_target("Bash", "not json", None), "not json");
    }

    #[test]
    fn keeps_paths_outside_the_working_directory_whole() {
        assert_eq!(
            relative_path("/etc/hosts", Some("/home/me/repo")),
            "/etc/hosts"
        );
        assert_eq!(relative_path("/home/me/repo/", Some("/home/me/repo")), ".");
        assert_eq!(relative_path("C:/Work/a.rs", Some("c:\\work")), "a.rs");
        assert_eq!(relative_path("/Home/a.rs", Some("/home")), "/Home/a.rs");
    }

    #[test]
    fn names_an_mcp_tool_and_its_server() {
        assert_eq!(
            tool_name("mcp__claude_ai_Gmail__send_email"),
            ToolName {
                label: "send email".into(),
                source: Some("Gmail".into()),
            }
        );
        assert_eq!(tool_name("Bash").source, None);
    }

    #[test]
    fn tells_the_status_of_a_call() {
        assert_eq!(call_status(false, false, false), CallStatus::Running);
        assert_eq!(call_status(true, true, false), CallStatus::Failed);
        assert_eq!(call_status(false, false, true), CallStatus::Stopped);
        assert_eq!(call_status(true, false, false), CallStatus::Done);
    }

    #[test]
    fn sums_up_a_run_of_calls() {
        let calls = [
            ("Grep", r#"{"pattern":"a"}"#),
            ("Read", r#"{"file_path":"a"}"#),
            ("Grep", r#"{"pattern":"b"}"#),
            ("Read", r#"{"file_path":"b"}"#),
            ("Read", r#"{"file_path":"c"}"#),
        ];
        let parts = summarize(calls);
        assert_eq!(
            summary_text(&parts),
            "Searched code 2 times and read 3 files"
        );
    }

    #[test]
    fn names_the_two_most_telling_actions_and_counts_the_rest() {
        let calls = [
            ("Read", "{}"),
            ("Edit", r#"{"file_path":"a.rs"}"#),
            ("Edit", r#"{"file_path":"a.rs"}"#),
            ("WebFetch", "{}"),
            ("Bash", r#"{"command":"ls"}"#),
        ];
        let parts = summarize(calls);
        assert_eq!(
            parts,
            [
                SummaryPart {
                    action: Some(ToolAction::Edit),
                    count: 1,
                },
                SummaryPart {
                    action: Some(ToolAction::Command),
                    count: 1,
                },
                SummaryPart {
                    action: None,
                    count: 2,
                },
            ]
        );
        assert_eq!(
            summary_text(&parts),
            "Changed 1 file, ran 1 command, and performed 2 other actions"
        );
    }

    #[test]
    fn previews_reasoning_without_markdown() {
        let text = "**Planning**\n\nI'll read `main.rs` first.\n";
        assert_eq!(reasoning_preview(text, false), "Planning");
        assert_eq!(reasoning_preview(text, true), "I'll read main.rs first.");
        assert_eq!(
            reasoning_preview("Calling html_preview on file_name.rs", false),
            "Calling html_preview on file_name.rs"
        );
    }

    #[test]
    fn a_thought_shows_its_heading_or_first_sentence() {
        assert_eq!(
            thought_line("**Checking html_preview**\n\nMore here."),
            "Checking html_preview"
        );
        assert_eq!(
            thought_line("I'll call `html_render` now. Then *check* it."),
            "I'll call html_render now."
        );
        assert_eq!(thought_line("No end yet"), "No end yet");
    }
}
