//! The MCP server hoshi gives its agents, so they can hand work to child threads. JSON-RPC
//! over HTTP on localhost; each thread's agent has a token of its own, which says who calls.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use tiny_http::{Header, Method, Request, Response, Server};

/// The name agents know the server by; its tools are `mcp__hoshi__<tool>`.
pub(crate) const SERVER_NAME: &str = "hoshi";
const PROTOCOL_VERSION: &str = "2025-06-18";

/// Answers a tool call made by the agent of a thread: its result, or why it failed.
pub(crate) type ToolHandler = dyn Fn(&str, &str, &Value) -> Result<Value, String> + Send + Sync;
/// Whether the agent of a thread is offered a tool, by its name. Only
/// [`CONDITIONAL_TOOLS`] are asked about.
pub(crate) type ToolFilter = dyn Fn(&str, &str) -> bool + Send + Sync;

/// Tools only some threads' agents see.
const CONDITIONAL_TOOLS: [&str; 1] = ["create_pull_request"];

pub(crate) struct McpServer {
    pub url: String,
    /// The thread each token speaks for.
    tokens: Arc<Mutex<HashMap<String, String>>>,
}

impl McpServer {
    pub fn start(handler: Arc<ToolHandler>, offered: Arc<ToolFilter>) -> std::io::Result<Self> {
        let server = Server::http("127.0.0.1:0").map_err(std::io::Error::other)?;
        let port = server
            .server_addr()
            .to_ip()
            .map(|address| address.port())
            .ok_or_else(|| std::io::Error::other("the MCP server has no port"))?;
        let tokens = Arc::<Mutex<HashMap<String, String>>>::default();
        let known = tokens.clone();
        thread::spawn(move || {
            for request in server.incoming_requests() {
                let (handler, offered, known) = (handler.clone(), offered.clone(), known.clone());
                thread::spawn(move || answer(request, &known, handler.as_ref(), offered.as_ref()));
            }
        });
        Ok(Self {
            url: format!("http://127.0.0.1:{port}/mcp"),
            tokens,
        })
    }

    fn tokens(&self) -> MutexGuard<'_, HashMap<String, String>> {
        self.tokens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The token the thread's agent calls with, the one it had if it had one.
    pub fn token_for(&self, thread_id: &str) -> Option<String> {
        let mut tokens = self.tokens();
        if let Some((token, _)) = tokens.iter().find(|(_, owner)| *owner == thread_id) {
            return Some(token.clone());
        }
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).ok()?;
        let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        tokens.insert(token.clone(), thread_id.to_owned());
        Some(token)
    }

    /// A deleted thread's agent can no longer call.
    pub fn forget(&self, thread_id: &str) {
        self.tokens().retain(|_, owner| owner != thread_id);
    }
}

fn answer(
    mut request: Request,
    tokens: &Mutex<HashMap<String, String>>,
    handler: &ToolHandler,
    offered: &ToolFilter,
) {
    if *request.method() != Method::Post {
        let _ = request.respond(Response::empty(405));
        return;
    }
    let caller = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Authorization"))
        .and_then(|header| header.value.as_str().strip_prefix("Bearer "))
        .and_then(|token| {
            tokens
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(token)
                .cloned()
        });
    let Some(caller) = caller else {
        let _ = request.respond(Response::empty(401));
        return;
    };
    let mut body = String::new();
    if request.as_reader().read_to_string(&mut body).is_err() {
        let _ = request.respond(Response::empty(400));
        return;
    }
    let Ok(message) = serde_json::from_str::<Value>(&body) else {
        let _ = request.respond(json_response(&rpc_error(
            &Value::Null,
            -32700,
            "parse error",
        )));
        return;
    };
    match reply(&caller, &message, handler, offered) {
        Some(reply) => {
            let _ = request.respond(json_response(&reply));
        }
        // A notification gets no answer.
        None => {
            let _ = request.respond(Response::empty(202));
        }
    }
}

fn json_response(body: &Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let header = Header::from_bytes("Content-Type", "application/json").expect("a valid header");
    Response::from_string(body.to_string()).with_header(header)
}

/// The answer to one JSON-RPC message; `None` for a notification.
pub(crate) fn reply(
    caller: &str,
    message: &Value,
    handler: &ToolHandler,
    offered: &ToolFilter,
) -> Option<Value> {
    let id = message.get("id")?.clone();
    let result = match message["method"].as_str() {
        Some("initialize") => json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
        }),
        Some("ping") => json!({}),
        Some("tools/list") => json!({ "tools": tools_for(caller, offered) }),
        Some("tools/call") => {
            let name = message["params"]["name"].as_str().unwrap_or_default();
            let arguments = &message["params"]["arguments"];
            match handler(caller, name, arguments) {
                Ok(mut value) => {
                    let image = value
                        .as_object_mut()
                        .and_then(|fields| fields.remove("image"))
                        .and_then(|image| image.as_str().map(str::to_owned));
                    let mut content = vec![json!({ "type": "text", "text": value.to_string() })];
                    if let Some(data) = image {
                        content.push(
                            json!({ "type": "image", "data": data, "mimeType": "image/png" }),
                        );
                    }
                    json!({ "content": content, "structuredContent": value })
                }
                Err(error) => json!({
                    "content": [{ "type": "text", "text": error }],
                    "isError": true,
                }),
            }
        }
        _ => return Some(rpc_error(&id, -32601, "method not found")),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn tools_for(caller: &str, offered: &ToolFilter) -> Vec<Value> {
    let Value::Array(tools) = tools() else {
        return Vec::new();
    };
    tools
        .into_iter()
        .filter(|tool| {
            let name = tool["name"].as_str().unwrap_or_default();
            !CONDITIONAL_TOOLS.contains(&name) || offered(caller, name)
        })
        .collect()
}

fn tools() -> Value {
    let task_id = json!({
        "type": "object",
        "properties": { "taskId": { "type": "string", "description": "The taskId delegate_task returned." } },
        "required": ["taskId"],
    });
    json!([
        {
            "name": "delegate_task",
            "description": "Start a child agent in a thread of its own the user can open, for work that should go on in the background or needs its own checkout and branch (isolation \"worktree\"). For help within this turn, use your own Agent tool instead. The child sees only `task`, none of this conversation. Returns at once; the answer arrives later as a message in this thread, so end your turn instead of waiting or polling.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task": { "type": "string", "description": "Everything the child needs to do the work: it sees nothing else." },
                    "title": { "type": "string", "description": "A short name for the child thread." },
                    "isolation": {
                        "type": "string",
                        "enum": ["shared", "worktree"],
                        "description": "shared (default) works in this thread's checkout; worktree in a new one from it.",
                    },
                    "model": { "type": "string", "description": "The child's model, by slug or name, only when the user asked for one; this thread's model otherwise." },
                    "effort": {
                        "type": "string",
                        "enum": ["low", "medium", "high", "xhigh", "max"],
                        "description": "The child's reasoning effort, only when the user asked for one; this thread's otherwise.",
                    },
                    "plan": { "type": "boolean", "description": "Start the child in plan mode: it proposes a plan for the user to approve before changing anything." },
                },
                "required": ["task"],
            },
        },
        {
            "name": "task_status",
            "description": "Where a delegated task stands, with its answer once it finished. Reading a finished answer means it won't arrive as a message.",
            "inputSchema": task_id,
        },
        {
            "name": "task_cancel",
            "description": "Stop a delegated task. Its answer won't arrive.",
            "inputSchema": task_id,
        },
        {
            "name": "request_secret",
            "description": "Get one secret you lack, such as a token, password, PIN or pairing code; one secret per call. Values that go together, such as the email and password of a login or a client id and its secret, are one secret: name them in `fields`, and the user fills one card and keeps them as one. Call vault_list first: a secret the user keeps in the vault comes by its `vaultId`, at once or once they approve it in a card. One the vault lacks, the user types in a card, where they can keep it in the vault for next time; when they ask you to save or create a secret, call it with `keep`. Use it when a command or a page needs a credential, instead of asking in chat or searching files, databases and the environment for one; not for access you already have. Waits for the answer and returns a file holding the value, or `files` with one per field: read one only inside the command that needs it, such as `curl -H \"Authorization: Bearer $(cat <file>)\"`, and never print it; to type a value into a page, pass its file to preview_act as secretFile. What the user already pasted in this thread goes in `value`, or in each field's `value`, with no card and only to a file; when they ask you to save it, add `keep` and they confirm in a card. The result's `inVault` tells whether the vault holds the secret now.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "vaultId": { "type": "string", "description": "The id vault_list gave for a secret the vault holds. `label` is then not needed." },
                    "label": { "type": "string", "description": "Card title, such as \"GitHub token\". Required without vaultId." },
                    "reason": { "type": "string", "description": "What it is for and, when the user must type it, where to get it." },
                    "placeholder": { "type": "string", "description": "Hint in the empty field, such as \"ghp_…\"." },
                    "fields": {
                        "type": "array",
                        "minItems": 2,
                        "maxItems": 6,
                        "description": "The values of a secret that has several, in the order to ask for them, such as a login's email and password. Leave out for a single key, token or code. Not with vaultId.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "label": { "type": "string", "description": "What the value is, such as \"Email\"." },
                                "secret": { "type": "boolean", "description": "False for a value the user may see as they type it, such as an email or a user name. True by default." },
                                "value": { "type": "string", "description": "The value itself, when the user pasted it in this thread: for every field or for none." },
                            },
                            "required": ["label"],
                        },
                    },
                    "keep": { "type": "boolean", "description": "The user asked to save this secret in the vault: the card opens set to keep it, named by `label`. With pasted values the card only asks them to confirm. Not with vaultId." },
                    "value": { "type": "string", "description": "A single secret the user pasted in this thread: written to a file with no card, and kept in the vault only with `keep`, once they confirm." },
                    "timeoutMs": { "type": "integer", "description": "How long to wait for the user; 10 minutes by default, at most an hour." },
                },
                "required": ["reason"],
            },
            "_meta": { "anthropic/alwaysLoad": true },
        },
        {
            "name": "vault_list",
            "description": "List the secrets the user keeps in hoshi's vault for this project, such as tokens, passwords and the logins of the apps they work on: each by its name and what it is for, never its value. Call it first whenever you need a credential, before you ask for one or go looking. Each has the id request_secret takes as vaultId, and its access: \"allowed\" comes at once, \"ask\" once the user approves.",
            "inputSchema": { "type": "object", "properties": {} },
            "_meta": { "anthropic/alwaysLoad": true },
        },
        {
            "name": "thread_list",
            "description": "List the user's hoshi threads, newest first, as when the user mentions another thread: in this thread's project, or every project with allProjects. Each has its threadId, title, status, project and branch.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Only threads whose title or branch contains this." },
                    "allProjects": { "type": "boolean", "description": "Every project, not only this thread's." },
                    "limit": { "type": "integer", "description": "At most this many, 1 to 100; 20 by default." },
                },
            },
        },
        {
            "name": "thread_read",
            "description": "Read another hoshi thread: its status and latest messages from the user and its agent, oldest first. Read a thread before you answer about it. Keep reads small with limit and maxCharsPerItem.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "threadId": { "type": "string" },
                    "limit": { "type": "integer", "description": "The latest messages to read, 1 to 100; 20 by default." },
                    "maxCharsPerItem": { "type": "integer", "description": "Longer messages are cut, 1 to 50000; 2000 by default." },
                },
                "required": ["threadId"],
            },
        },
        {
            "name": "thread_send",
            "description": "Send a message to another hoshi thread, as if the user wrote it there: it starts the thread's turn, or joins the turn that runs. Its answer stays in that thread; read it with thread_read, or thread_wait first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "threadId": { "type": "string" },
                    "message": { "type": "string", "description": "Everything that thread's agent needs: it doesn't see this conversation." },
                },
                "required": ["threadId", "message"],
            },
        },
        {
            "name": "thread_wait",
            "description": "Wait until another hoshi thread's turn ends, then get its status and latest answer. Returns at once when it is idle; on timedOut, call again only if still needed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "threadId": { "type": "string" },
                    "timeoutMs": { "type": "integer", "description": "How long to wait; 5 minutes by default, at most an hour." },
                },
                "required": ["threadId"],
            },
        },
        {
            "name": "create_pull_request",
            "description": "Open the pull request of this thread's branch on Bitbucket, signed in as the user through hoshi: never look for, ask for or use a Bitbucket token, nor call Bitbucket's API yourself. Commit first; it pushes the branch when origin lacks its commits, and the pull request then shows in the thread. Returns its number, url and state. If one is already open from the branch, the error gives its URL.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "description": { "type": "string", "description": "Markdown." },
                    "base": { "type": "string", "description": "The branch to merge into; by default the one this branch started from, else the repository's default branch." },
                    "draft": { "type": "boolean" },
                },
                "required": ["title"],
            },
        },
        {
            "name": "watch_pull_request",
            "description": "Watch this thread's open pull request, when asked to monitor or babysit it or when your work waits on its checks or reviews: hoshi wakes you with a message when checks fail, all checks pass, someone comments or reviews, or the branch conflicts. Handle what is already there first; only later news wakes you. After this call, end your turn. The watch stays on after each wake, so don't call this again on a wake. A wake is not a merge decision: check readiness yourself.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "unwatch_pull_request",
            "description": "Stop watching this thread's pull request. Call it before you hand the work back to the user.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "preview_open",
            "description": "Open a page in this thread's own headless browser, which the user sees in hoshi, such as a dev server (`localhost:5173`) or a file. Use it to check web work instead of a browser, Playwright or screenshot tool of your own; then preview_snapshot reads the page and preview_act uses it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string" },
                    "width": { "type": "integer", "description": "Viewport width in CSS pixels; 1280 by default." },
                    "height": { "type": "integer", "description": "Viewport height in CSS pixels; 800 by default." },
                },
                "required": ["url"],
            },
        },
        {
            "name": "preview_navigate",
            "description": "Go to another address in this thread's page.",
            "inputSchema": {
                "type": "object",
                "properties": { "url": { "type": "string" } },
                "required": ["url"],
            },
        },
        {
            "name": "preview_snapshot",
            "description": "Read the page as its accessibility tree, each element with a `ref` to pass to preview_act. Refs last until the next snapshot. includeImage adds a screenshot, to check how it looks.",
            "inputSchema": {
                "type": "object",
                "properties": { "includeImage": { "type": "boolean" } },
            },
        },
        {
            "name": "preview_act",
            "description": "Use the page as a person would, with real input: click (a ref, or x and y), hover, type (text into a ref; clear empties it first; secretFile in place of text types a secret you never read), press (a key such as Enter, Tab, Escape, ArrowDown), select (values of a <select> ref) or scroll (deltaX, deltaY, at a ref or the page). Snapshot again to see the result.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["click", "hover", "type", "press", "select", "scroll"] },
                    "ref": { "type": "string" },
                    "text": { "type": "string" },
                    "secretFile": { "type": "string", "description": "With type: the file request_secret returned, typed in place of `text`." },
                    "clear": { "type": "boolean" },
                    "key": { "type": "string" },
                    "values": { "type": "array", "items": { "type": "string" } },
                    "x": { "type": "number" },
                    "y": { "type": "number" },
                    "deltaX": { "type": "number" },
                    "deltaY": { "type": "number" },
                },
                "required": ["action"],
            },
        },
        {
            "name": "preview_evaluate",
            "description": "Run JavaScript in the page and get its value, awaiting a promise.",
            "inputSchema": {
                "type": "object",
                "properties": { "expression": { "type": "string" } },
                "required": ["expression"],
            },
        },
        {
            "name": "preview_wait_for",
            "description": "Wait until the page shows `text`, has an element matching `selector`, or both.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string" },
                    "selector": { "type": "string" },
                    "timeoutMs": { "type": "integer", "description": "10 seconds by default, at most a minute." },
                },
            },
        },
        {
            "name": "preview_resize",
            "description": "Change the page's viewport, as for a phone (390x844) or a wide screen.",
            "inputSchema": {
                "type": "object",
                "properties": { "width": { "type": "integer" }, "height": { "type": "integer" } },
            },
        },
        {
            "name": "preview_set_appearance",
            "description": "Show the page in dark or light mode, through prefers-color-scheme; system follows the browser's own.",
            "inputSchema": {
                "type": "object",
                "properties": { "colorScheme": { "type": "string", "enum": ["dark", "light", "system"] } },
                "required": ["colorScheme"],
            },
        },
        {
            "name": "preview_status",
            "description": "Whether this thread has a page open, its address, title and size, and its latest console messages and errors.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "preview_close",
            "description": "Close this thread's page.",
            "inputSchema": { "type": "object", "properties": {} },
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn echo(caller: &str, tool: &str, arguments: &Value) -> Result<Value, String> {
        if tool == "fail" {
            return Err("it failed".into());
        }
        Ok(json!({ "caller": caller, "tool": tool, "arguments": arguments }))
    }

    fn everything(_: &str, _: &str) -> bool {
        true
    }

    #[test]
    fn a_tool_call_reaches_the_handler_as_its_caller() {
        let call = json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": { "name": "task_status", "arguments": { "taskId": "c1" } } });

        let reply = reply("t1", &call, &echo, &everything).unwrap();

        assert_eq!(reply["id"], 7);
        assert_eq!(reply["result"]["structuredContent"]["caller"], "t1");
        assert_eq!(
            reply["result"]["structuredContent"]["arguments"]["taskId"],
            "c1"
        );
    }

    #[test]
    fn a_failed_call_is_a_tool_error_and_notifications_get_no_reply() {
        let call = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "fail", "arguments": {} } });
        assert_eq!(
            reply("t1", &call, &echo, &everything).unwrap()["result"]["isError"],
            true
        );

        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert_eq!(reply("t1", &note, &echo, &everything), None);
    }

    #[test]
    fn the_server_lists_its_tools_and_turns_away_unknown_tokens() {
        let list = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
        let names = |offered: &ToolFilter| -> Vec<String> {
            reply("t1", &list, &echo, offered).unwrap()["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tool| tool["name"].as_str().unwrap().to_owned())
                .collect()
        };
        let on_github = names(&|_, tool| tool != "create_pull_request");
        assert!(!on_github.iter().any(|name| name == "create_pull_request"));
        assert_eq!(on_github.len() + 1, names(&everything).len());
        let names = names(&everything);
        assert_eq!(
            names,
            [
                "delegate_task",
                "task_status",
                "task_cancel",
                "request_secret",
                "vault_list",
                "thread_list",
                "thread_read",
                "thread_send",
                "thread_wait",
                "create_pull_request",
                "watch_pull_request",
                "unwatch_pull_request",
                "preview_open",
                "preview_navigate",
                "preview_snapshot",
                "preview_act",
                "preview_evaluate",
                "preview_wait_for",
                "preview_resize",
                "preview_set_appearance",
                "preview_status",
                "preview_close"
            ]
        );

        let server = McpServer::start(Arc::new(echo), Arc::new(everything)).unwrap();
        let token = server.token_for("t1").unwrap();
        assert_eq!(server.token_for("t1"), Some(token.clone()));
        let agent = ureq_like_post(&server.url, Some(&token), &list);
        assert!(agent.contains("delegate_task"), "{agent}");
        assert_eq!(ureq_like_post(&server.url, Some("nope"), &list), "401");
    }

    /// A bare HTTP POST, to keep an HTTP client out of the crate's dependencies.
    fn ureq_like_post(url: &str, token: Option<&str>, body: &Value) -> String {
        use std::io::Write;
        let address = url
            .trim_start_matches("http://")
            .trim_end_matches("/mcp")
            .to_owned();
        let mut stream = std::net::TcpStream::connect(&address).unwrap();
        let body = body.to_string();
        let auth = token
            .map(|token| format!("Authorization: Bearer {token}\r\n"))
            .unwrap_or_default();
        write!(
            stream,
            "POST /mcp HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\n{auth}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        if response.starts_with("HTTP/1.1 401") {
            return "401".into();
        }
        response
            .split("\r\n\r\n")
            .nth(1)
            .unwrap_or_default()
            .to_owned()
    }
}
