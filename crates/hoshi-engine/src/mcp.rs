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

pub(crate) struct McpServer {
    pub url: String,
    /// The thread each token speaks for.
    tokens: Arc<Mutex<HashMap<String, String>>>,
}

impl McpServer {
    pub fn start(handler: Arc<ToolHandler>) -> std::io::Result<Self> {
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
                let (handler, known) = (handler.clone(), known.clone());
                thread::spawn(move || answer(request, &known, handler.as_ref()));
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

fn answer(mut request: Request, tokens: &Mutex<HashMap<String, String>>, handler: &ToolHandler) {
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
    match reply(&caller, &message, handler) {
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
pub(crate) fn reply(caller: &str, message: &Value, handler: &ToolHandler) -> Option<Value> {
    let id = message.get("id")?.clone();
    let result = match message["method"].as_str() {
        Some("initialize") => json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
        }),
        Some("ping") => json!({}),
        Some("tools/list") => json!({ "tools": tools() }),
        Some("tools/call") => {
            let name = message["params"]["name"].as_str().unwrap_or_default();
            let arguments = &message["params"]["arguments"];
            match handler(caller, name, arguments) {
                Ok(value) => json!({
                    "content": [{ "type": "text", "text": value.to_string() }],
                    "structuredContent": value,
                }),
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

fn tools() -> Value {
    let task_id = json!({
        "type": "object",
        "properties": { "taskId": { "type": "string", "description": "The taskId delegate_task returned." } },
        "required": ["taskId"],
    });
    json!([
        {
            "name": "delegate_task",
            "description": "Start a child agent in a thread of its own, with only `task` and none of this conversation. Returns at once; the child's answer arrives later as a message in this thread, so end your turn instead of waiting or polling. Use isolation \"worktree\" for work that needs its own checkout and branch.",
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

    #[test]
    fn a_tool_call_reaches_the_handler_as_its_caller() {
        let call = json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": { "name": "task_status", "arguments": { "taskId": "c1" } } });

        let reply = reply("t1", &call, &echo).unwrap();

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
            reply("t1", &call, &echo).unwrap()["result"]["isError"],
            true
        );

        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert_eq!(reply("t1", &note, &echo), None);
    }

    #[test]
    fn the_server_lists_its_tools_and_turns_away_unknown_tokens() {
        let list = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
        let names: Vec<_> = reply("t1", &list, &echo).unwrap()["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["delegate_task", "task_status", "task_cancel"]);

        let server = McpServer::start(Arc::new(echo)).unwrap();
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
