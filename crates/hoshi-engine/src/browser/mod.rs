//! The headless browser agents drive, a page per thread, as t3code's server-side preview:
//! real input events, the accessibility tree with refs to act on, screenshots and console.

mod cdp;
pub(crate) mod html;
mod launch;
pub(crate) mod tools;

pub use launch::find_browser;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use cdp::Connection;
use launch::BrowserProcess;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

pub const DEFAULT_SIZE: (u32, u32) = (1280, 800);
const CONSOLE_KEPT: usize = 200;
const READY_POLL: Duration = Duration::from_millis(100);

pub struct Browser {
    connection: Arc<Connection>,
    _process: BrowserProcess,
}

impl Browser {
    /// Starts `executable` headless, with its profile under `profile`.
    pub fn start(executable: &Path, profile: &Path) -> Result<Self, String> {
        let process = launch::launch(executable, profile)?;
        let connection = Arc::new(Connection::connect(&process.url)?);
        Ok(Self {
            connection,
            _process: process,
        })
    }

    /// A new blank page of `size` CSS pixels; `observer` hears what it shows.
    pub fn open_page(&self, size: (u32, u32), observer: Observer) -> Result<Page, String> {
        let target =
            self.connection
                .call("Target.createTarget", json!({ "url": "about:blank" }), None)?;
        let target_id = target["targetId"]
            .as_str()
            .ok_or("the browser opened no page")?
            .to_owned();
        let attached = self.connection.call(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
            None,
        )?;
        let session_id = attached["sessionId"]
            .as_str()
            .ok_or("the browser gave no session")?
            .to_owned();
        let console = Arc::new(Mutex::new(VecDeque::new()));
        listen(
            &self.connection,
            &session_id,
            Arc::downgrade(&console),
            observer,
        );
        let mut page = Page {
            connection: self.connection.clone(),
            target_id,
            session_id,
            refs: HashMap::new(),
            console,
            size,
            zoom: 1.0,
        };
        for domain in ["Page.enable", "Runtime.enable", "DOM.enable"] {
            page.call(domain, json!({}))?;
        }
        page.resize(size)?;
        Ok(page)
    }

    /// `false` once the browser quit or crashed: nothing it had answers any more.
    pub fn is_connected(&self) -> bool {
        self.connection.is_open()
    }
}

impl Drop for Browser {
    /// Closes it through the protocol too: the process started may not be the browser.
    fn drop(&mut self) {
        let _ =
            self.connection
                .call_within("Browser.close", json!({}), None, Duration::from_secs(2));
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a page shows or became, for whoever displays it.
pub enum PageEvent {
    /// A JPEG of the page as it paints, while it is screencast.
    Frame {
        jpeg: Vec<u8>,
        width: u32,
        height: u32,
    },
    /// It went somewhere else, finished loading, or logged an error.
    Changed {
        url: String,
        title: String,
        errors: u32,
    },
}

pub type Observer = Arc<dyn Fn(PageEvent) + Send + Sync>;

fn is_error(line: &str) -> bool {
    line.starts_with("error:")
}

/// Follows the page's events: keeps its console, answers screencast frames, and tells
/// `observer` what it shows and where it went. Ends once the page is gone.
fn listen(
    connection: &Arc<Connection>,
    session: &str,
    console: Weak<Mutex<VecDeque<String>>>,
    observer: Observer,
) {
    let events = connection.subscribe();
    let connection = Arc::downgrade(connection);
    let session = session.to_owned();
    thread::spawn(move || {
        for event in events {
            let Some(console) = console.upgrade() else {
                return;
            };
            if event.session_id.as_deref() != Some(session.as_str()) {
                continue;
            }
            let Some(connection) = connection.upgrade() else {
                return;
            };
            let changed = match event.method.as_str() {
                "Page.screencastFrame" => {
                    let _ = connection.call(
                        "Page.screencastFrameAck",
                        json!({ "sessionId": event.params["sessionId"] }),
                        Some(&session),
                    );
                    let metadata = &event.params["metadata"];
                    let data = event.params["data"].as_str().unwrap_or_default();
                    let Ok(jpeg) = STANDARD.decode(data) else {
                        continue;
                    };
                    observer(PageEvent::Frame {
                        jpeg,
                        width: metadata["deviceWidth"].as_f64().unwrap_or_default() as u32,
                        height: metadata["deviceHeight"].as_f64().unwrap_or_default() as u32,
                    });
                    false
                }
                "Page.frameNavigated" => event.params["frame"]["parentId"].is_null(),
                "Page.loadEventFired" | "Page.navigatedWithinDocument" => true,
                "Runtime.consoleAPICalled" | "Runtime.exceptionThrown" => {
                    let line = console_line(&event.method, &event.params);
                    let error = is_error(&line);
                    let mut kept = lock(&console);
                    if kept.len() >= CONSOLE_KEPT {
                        kept.pop_front();
                    }
                    kept.push_back(line);
                    error
                }
                _ => false,
            };
            if changed {
                let errors = lock(&console).iter().filter(|line| is_error(line)).count() as u32;
                let place = connection
                    .call(
                        "Runtime.evaluate",
                        json!({ "expression": "[location.href, document.title]", "returnByValue": true }),
                        Some(&session),
                    )
                    .ok();
                let value = place.as_ref().map(|place| &place["result"]["value"]);
                let text = |index: usize| {
                    value
                        .and_then(|value| value[index].as_str())
                        .unwrap_or_default()
                        .to_owned()
                };
                observer(PageEvent::Changed {
                    url: text(0),
                    title: text(1),
                    errors,
                });
            }
        }
    });
}

fn console_line(method: &str, params: &Value) -> String {
    if method == "Runtime.exceptionThrown" {
        let details = &params["exceptionDetails"];
        let description = details["exception"]["description"]
            .as_str()
            .or_else(|| details["text"].as_str())
            .unwrap_or_default();
        return format!("error: {description}");
    }
    let kind = params["type"].as_str().unwrap_or("log");
    let args: Vec<String> = params["args"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|arg| match &arg["value"] {
            Value::String(text) => text.clone(),
            Value::Null => arg["description"].as_str().unwrap_or_default().to_owned(),
            value => value.to_string(),
        })
        .collect();
    format!("{kind}: {}", args.join(" "))
}

pub struct Page {
    connection: Arc<Connection>,
    target_id: String,
    session_id: String,
    /// The last snapshot's refs, to the DOM nodes they name.
    refs: HashMap<String, i64>,
    console: Arc<Mutex<VecDeque<String>>>,
    size: (u32, u32),
    /// Page zoom: the page lays out at `size / zoom` CSS pixels and paints at `size`.
    zoom: f64,
}

impl Page {
    fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        self.connection.call(method, params, Some(&self.session_id))
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Goes to `url` and waits until it loaded, or `timeout` passed.
    pub fn navigate(&mut self, url: &str, timeout: Duration) -> Result<(), String> {
        let result = self.call("Page.navigate", json!({ "url": url }))?;
        if let Some(error) = result["errorText"]
            .as_str()
            .filter(|error| !error.is_empty())
        {
            return Err(format!("couldn't open {url}: {error}"));
        }
        self.refs.clear();
        self.wait_ready(timeout)
    }

    /// The page's address and title.
    pub fn location(&self) -> Result<(String, String), String> {
        let value = self.evaluate("[location.href, document.title]")?;
        Ok((
            value[0].as_str().unwrap_or_default().to_owned(),
            value[1].as_str().unwrap_or_default().to_owned(),
        ))
    }

    /// A PNG of what the page shows, base64.
    pub fn screenshot(&self) -> Result<String, String> {
        let shot = self.call("Page.captureScreenshot", json!({ "format": "png" }))?;
        shot["data"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "the browser gave no screenshot".into())
    }

    /// The accessibility tree as indented lines, each element with a `ref` to act on.
    pub fn snapshot(&mut self, max_chars: usize) -> Result<String, String> {
        let tree = self.call("Accessibility.getFullAXTree", json!({}))?;
        let nodes = tree["nodes"].as_array().cloned().unwrap_or_default();
        let (text, refs) = format_tree(&nodes, max_chars);
        self.refs = refs;
        Ok(text)
    }

    fn node(&self, reference: &str) -> Result<i64, String> {
        self.refs.get(reference).copied().ok_or_else(|| {
            format!("no element {reference} in the last snapshot: take a new snapshot")
        })
    }

    fn center(&self, reference: &str) -> Result<(f64, f64), String> {
        let node = self.node(reference)?;
        let _ = self.call(
            "DOM.scrollIntoViewIfNeeded",
            json!({ "backendNodeId": node }),
        );
        let model = self.call("DOM.getBoxModel", json!({ "backendNodeId": node }))?;
        let quad: Vec<f64> = model["model"]["content"]
            .as_array()
            .ok_or("the element isn't shown")?
            .iter()
            .filter_map(Value::as_f64)
            .collect();
        if quad.len() < 8 {
            return Err("the element isn't shown".into());
        }
        let x = (quad[0] + quad[2] + quad[4] + quad[6]) / 4.0;
        let y = (quad[1] + quad[3] + quad[5] + quad[7]) / 4.0;
        Ok((x, y))
    }

    fn mouse(&self, kind: &str, (x, y): (f64, f64), extra: Value) -> Result<(), String> {
        let mut params = json!({ "type": kind, "x": x, "y": y });
        if let (Some(params), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
            params.extend(extra.clone());
        }
        self.call("Input.dispatchMouseEvent", params).map(|_| ())
    }

    pub fn click(&self, reference: &str) -> Result<(), String> {
        let point = self.center(reference)?;
        self.click_at(point)
    }

    pub fn click_at(&self, point: (f64, f64)) -> Result<(), String> {
        let press = json!({ "button": "left", "clickCount": 1 });
        self.mouse("mouseMoved", point, json!({}))?;
        self.mouse("mousePressed", point, press.clone())?;
        self.mouse("mouseReleased", point, press)
    }

    pub fn hover(&self, reference: &str) -> Result<(), String> {
        let point = self.center(reference)?;
        self.mouse("mouseMoved", point, json!({}))
    }

    /// Scrolls by `delta` pixels, at the element when given, else at the page's center.
    pub fn scroll(&self, reference: Option<&str>, delta: (f64, f64)) -> Result<(), String> {
        let point = match reference {
            Some(reference) => self.center(reference)?,
            None => (f64::from(self.size.0) / 2.0, f64::from(self.size.1) / 2.0),
        };
        self.mouse(
            "mouseWheel",
            point,
            json!({ "deltaX": delta.0, "deltaY": delta.1 }),
        )
    }

    fn call_on(&self, reference: &str, function: &str, arguments: Value) -> Result<Value, String> {
        let node = self.node(reference)?;
        let resolved = self.call("DOM.resolveNode", json!({ "backendNodeId": node }))?;
        let object = resolved["object"]["objectId"]
            .as_str()
            .ok_or("the element is gone")?;
        let result = self.call(
            "Runtime.callFunctionOn",
            json!({
                "objectId": object,
                "functionDeclaration": function,
                "arguments": arguments,
                "returnByValue": true,
                "awaitPromise": true,
            }),
        )?;
        script_result(&result)
    }

    /// Types `text` into the element, after emptying it when `clear`.
    pub fn type_text(&self, reference: &str, text: &str, clear: bool) -> Result<(), String> {
        let node = self.node(reference)?;
        self.call("DOM.focus", json!({ "backendNodeId": node }))?;
        if clear {
            self.call_on(
                reference,
                "function () { if ('value' in this) { this.value = ''; this.dispatchEvent(new Event('input', { bubbles: true })); } else if (this.isContentEditable) { this.textContent = ''; } }",
                json!([]),
            )?;
        }
        self.call("Input.insertText", json!({ "text": text }))
            .map(|_| ())
    }

    /// Picks the options of a `<select>` whose value or text is among `values`.
    pub fn select(&self, reference: &str, values: &[String]) -> Result<(), String> {
        self.call_on(
            reference,
            "function (values) { for (const option of this.options ?? []) { option.selected = values.includes(option.value) || values.includes(option.text); } this.dispatchEvent(new Event('input', { bubbles: true })); this.dispatchEvent(new Event('change', { bubbles: true })); }",
            json!([{ "value": values }]),
        )
        .map(|_| ())
    }

    /// Presses a key such as `Enter`, `Tab`, `Escape`, `ArrowDown` or a single character.
    pub fn press(&self, key: &str) -> Result<(), String> {
        let (code, text): (i64, Option<&str>) = match key {
            "Enter" => (13, Some("\r")),
            "Tab" => (9, None),
            "Escape" => (27, None),
            "Backspace" => (8, None),
            "Delete" => (46, None),
            "ArrowLeft" => (37, None),
            "ArrowUp" => (38, None),
            "ArrowRight" => (39, None),
            "ArrowDown" => (40, None),
            "Home" => (36, None),
            "End" => (35, None),
            "PageUp" => (33, None),
            "PageDown" => (34, None),
            " " | "Space" => (32, Some(" ")),
            single if single.chars().count() == 1 => (0, Some(single)),
            other => return Err(format!("hoshi can't press {other}")),
        };
        let key = if key == "Space" { " " } else { key };
        let mut down = json!({ "type": "keyDown", "key": key, "windowsVirtualKeyCode": code });
        if let Some(text) = text {
            down["text"] = json!(text);
        }
        self.call("Input.dispatchKeyEvent", down)?;
        self.call(
            "Input.dispatchKeyEvent",
            json!({ "type": "keyUp", "key": key, "windowsVirtualKeyCode": code }),
        )
        .map(|_| ())
    }

    /// Runs `expression` in the page and returns its value, awaiting a promise.
    pub fn evaluate(&self, expression: &str) -> Result<Value, String> {
        let result = self.call(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
        )?;
        script_result(&result)
    }

    /// Waits until the page shows `text` or has an element matching `selector`; `false`
    /// when `timeout` passed first.
    pub fn wait_for(
        &self,
        text: Option<&str>,
        selector: Option<&str>,
        timeout: Duration,
    ) -> Result<bool, String> {
        let check = format!(
            "(() => {{ const text = {}; const selector = {}; return (!text || (document.body?.innerText ?? '').includes(text)) && (!selector || document.querySelector(selector) !== null); }})()",
            json!(text),
            json!(selector)
        );
        let deadline = Instant::now() + timeout;
        loop {
            if self.evaluate(&check)? == json!(true) {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            thread::sleep(READY_POLL);
        }
    }

    pub fn resize(&mut self, size: (u32, u32)) -> Result<(), String> {
        let layout = |side: u32| (f64::from(side) / self.zoom).round().max(1.0);
        self.call(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": layout(size.0),
                "height": layout(size.1),
                "deviceScaleFactor": self.zoom,
                "mobile": false,
            }),
        )?;
        self.size = size;
        Ok(())
    }

    /// Zooms the page by `zoom` (1 is 100%), keeping its size.
    pub fn set_zoom(&mut self, zoom: f64) -> Result<(), String> {
        self.zoom = zoom.clamp(0.25, 5.0);
        self.resize(self.size)
    }

    /// `dark`, `light`, or `None` for the system's.
    pub fn set_color_scheme(&self, scheme: Option<&str>) -> Result<(), String> {
        let features = match scheme {
            Some(scheme) => json!([{ "name": "prefers-color-scheme", "value": scheme }]),
            None => json!([]),
        };
        self.call(
            "Emulation.setEmulatedMedia",
            json!({ "features": features }),
        )
        .map(|_| ())
    }

    /// The latest console messages and errors, oldest first.
    pub fn console(&self, last: usize) -> Vec<String> {
        let console = lock(&self.console);
        console
            .iter()
            .skip(console.len().saturating_sub(last))
            .cloned()
            .collect()
    }

    /// Streams what the page paints to its observer, at most `max` pixels.
    pub fn start_screencast(&self, max: (u32, u32)) -> Result<(), String> {
        self.call(
            "Page.startScreencast",
            json!({ "format": "jpeg", "quality": 70, "maxWidth": max.0, "maxHeight": max.1, "everyNthFrame": 1 }),
        )
        .map(|_| ())
    }

    pub fn stop_screencast(&self) {
        let _ = self.call("Page.stopScreencast", json!({}));
    }

    /// Where an element is, scrolled into view: to show what the agent acts on.
    pub fn point_of(&self, reference: &str) -> Result<(f64, f64), String> {
        self.center(reference)
    }

    /// Back (`-1`) or forward (`1`) in the page's history.
    pub fn go(&mut self, delta: i32, timeout: Duration) -> Result<(), String> {
        self.evaluate(&format!("history.go({delta})"))?;
        self.refs.clear();
        thread::sleep(READY_POLL);
        self.wait_ready(timeout)
    }

    pub fn reload(&mut self, timeout: Duration) -> Result<(), String> {
        self.call("Page.reload", json!({}))?;
        self.refs.clear();
        thread::sleep(READY_POLL);
        self.wait_ready(timeout)
    }

    fn wait_ready(&self, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.evaluate("document.readyState")? == json!("complete") {
                break;
            }
            thread::sleep(READY_POLL);
        }
        Ok(())
    }

    /// Types `text` where the focus is.
    pub fn insert_text(&self, text: &str) -> Result<(), String> {
        self.call("Input.insertText", json!({ "text": text }))
            .map(|_| ())
    }

    pub fn scroll_at(&self, point: (f64, f64), delta: (f64, f64)) -> Result<(), String> {
        self.mouse(
            "mouseWheel",
            point,
            json!({ "deltaX": delta.0, "deltaY": delta.1 }),
        )
    }

    /// Console errors kept so far.
    pub fn errors(&self) -> u32 {
        lock(&self.console)
            .iter()
            .filter(|line| is_error(line))
            .count() as u32
    }

    pub fn close(&self) {
        let _ = self.connection.call(
            "Target.closeTarget",
            json!({ "targetId": self.target_id }),
            None,
        );
    }
}

fn script_result(result: &Value) -> Result<Value, String> {
    if let Some(details) = result.get("exceptionDetails") {
        let description = details["exception"]["description"]
            .as_str()
            .or_else(|| details["text"].as_str())
            .unwrap_or("the script failed");
        return Err(description.to_owned());
    }
    Ok(result["result"]["value"].clone())
}

/// Roles that only group others, left out unless they carry a name.
const PLAIN_ROLES: [&str; 6] = [
    "none",
    "generic",
    "InlineTextBox",
    "LineBreak",
    "RootWebArea",
    "WebArea",
];
const FLAGS: [&str; 6] = [
    "checked", "disabled", "expanded", "selected", "pressed", "focused",
];

fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// `Accessibility.getFullAXTree` as an outline like
/// `- button "Save" [ref=e3]`, and the DOM node each ref names.
fn format_tree(nodes: &[Value], max_chars: usize) -> (String, HashMap<String, i64>) {
    let by_id: HashMap<&str, &Value> = nodes
        .iter()
        .filter_map(|node| Some((node["nodeId"].as_str()?, node)))
        .collect();
    let mut lines = String::new();
    let mut refs = HashMap::new();
    let mut truncated = false;
    let mut stack: Vec<(&Value, usize)> = nodes
        .first()
        .map(|root| vec![(root, 0)])
        .unwrap_or_default();
    while let Some((node, depth)) = stack.pop() {
        if truncated {
            break;
        }
        let role = text_of(&node["role"]["value"]);
        let name = text_of(&node["name"]["value"]).trim().to_owned();
        let ignored = node["ignored"] == true;
        let shown = !ignored && !(PLAIN_ROLES.contains(&role.as_str()) && name.is_empty());
        let child_depth = if shown { depth + 1 } else { depth };
        if shown {
            let mut line = format!("{}- ", "  ".repeat(depth));
            if role == "StaticText" {
                line.push_str(&format!("text: {name}"));
            } else {
                line.push_str(&role);
                if !name.is_empty() {
                    line.push_str(&format!(" {}", json!(name)));
                }
                let value = text_of(&node["value"]["value"]);
                if !value.is_empty() {
                    line.push_str(&format!(" value={}", json!(value)));
                }
                for property in node["properties"].as_array().into_iter().flatten() {
                    let flag = property["name"].as_str().unwrap_or_default();
                    if FLAGS.contains(&flag) && property["value"]["value"] == true {
                        line.push_str(&format!(" [{flag}]"));
                    }
                }
                if let Some(backend) = node["backendDOMNodeId"].as_i64() {
                    let reference = format!("e{}", refs.len() + 1);
                    line.push_str(&format!(" [ref={reference}]"));
                    refs.insert(reference, backend);
                }
            }
            line.push('\n');
            if lines.len() + line.len() > max_chars {
                truncated = true;
                continue;
            }
            lines.push_str(&line);
        }
        let children: Vec<&Value> = node["childIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|id| by_id.get(id.as_str()?).copied())
            .collect();
        for child in children.into_iter().rev() {
            stack.push((child, child_depth));
        }
    }
    if truncated {
        lines.push_str("[truncated: the page has more]\n");
    }
    (lines, refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, role: &str, name: &str, children: &[&str], backend: Option<i64>) -> Value {
        json!({
            "nodeId": id,
            "ignored": false,
            "role": { "value": role },
            "name": { "value": name },
            "childIds": children,
            "backendDOMNodeId": backend,
        })
    }

    #[test]
    fn the_tree_reads_as_an_outline_with_refs_to_act_on() {
        let mut checkbox = node("5", "checkbox", "Remember me", &[], Some(15));
        checkbox["properties"] = json!([{ "name": "checked", "value": { "value": true } }]);
        let nodes = vec![
            node("1", "RootWebArea", "", &["2"], Some(11)),
            node("2", "generic", "", &["3", "4", "5"], Some(12)),
            node("3", "heading", "Sign in", &["6"], Some(13)),
            node("4", "button", "Save", &[], Some(14)),
            checkbox,
            node("6", "StaticText", "Sign in", &[], None),
        ];

        let (text, refs) = format_tree(&nodes, 10_000);

        assert_eq!(
            text,
            "- heading \"Sign in\" [ref=e1]\n  - text: Sign in\n- button \"Save\" [ref=e2]\n- checkbox \"Remember me\" [checked] [ref=e3]\n"
        );
        assert_eq!(refs["e2"], 14);
        let (cut, _) = format_tree(&nodes, 40);
        assert!(cut.ends_with("[truncated: the page has more]\n"));
    }

    #[test]
    fn a_page_s_listener_stops_once_the_page_is_gone() {
        let connection = Arc::new(Connection::offline());
        let console = Arc::new(Mutex::new(VecDeque::new()));
        listen(
            &connection,
            "page",
            Arc::downgrade(&console),
            Arc::new(|_| {}),
        );
        let elsewhere = cdp::CdpEvent {
            method: "Page.screencastFrame".into(),
            params: json!({}),
            session_id: Some("other".into()),
        };
        assert_eq!(connection.report(&elsewhere), 1);

        drop(console);

        let deadline = Instant::now() + Duration::from_secs(5);
        while connection.report(&elsewhere) > 0 {
            assert!(Instant::now() < deadline, "the listener kept running");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_real_page_can_be_read_clicked_and_typed_into() {
        let Some(executable) = find_browser() else {
            eprintln!("no Chromium-based browser here; skipping");
            return;
        };
        let profile = std::env::temp_dir().join(format!("hoshi-browser-{}", std::process::id()));
        let browser = Browser::start(&executable, &profile).unwrap();
        let mut page = browser.open_page(DEFAULT_SIZE, Arc::new(|_| {})).unwrap();
        let html = "<h1>Hi</h1><input aria-label='Name'><button onclick=\"document.querySelector('h1').textContent = 'Clicked ' + document.querySelector('input').value; console.log('done')\">Go</button>";
        let url = format!(
            "data:text/html,{}",
            html.replace(' ', "%20").replace('#', "%23")
        );

        page.navigate(&url, Duration::from_secs(10)).unwrap();
        let snapshot = page.snapshot(10_000).unwrap();
        let reference = |label: &str| {
            snapshot
                .lines()
                .find(|line| line.contains(label))
                .and_then(|line| line.split("[ref=").nth(1))
                .map(|rest| rest.trim_end_matches(']').to_owned())
                .unwrap()
        };
        page.type_text(&reference("textbox"), "Ana", true).unwrap();
        page.click(&reference("button \"Go\"")).unwrap();

        assert!(
            page.wait_for(Some("Clicked Ana"), None, Duration::from_secs(5))
                .unwrap()
        );
        assert_eq!(page.evaluate("document.title === ''").unwrap(), json!(true));
        assert!(!page.screenshot().unwrap().is_empty());
        thread::sleep(Duration::from_millis(200));
        assert!(page.console(10).iter().any(|line| line == "log: done"));
        page.close();
        drop(browser);
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[test]
    fn a_zoomed_page_lays_out_narrower_at_the_same_size() {
        let Some(executable) = find_browser() else {
            eprintln!("no Chromium-based browser here; skipping");
            return;
        };
        let profile = std::env::temp_dir().join(format!("hoshi-zoom-{}", std::process::id()));
        let browser = Browser::start(&executable, &profile).unwrap();
        let mut page = browser.open_page((1000, 600), Arc::new(|_| {})).unwrap();

        page.set_zoom(1.25).unwrap();
        page.set_color_scheme(Some("dark")).unwrap();
        let laid_out = page.evaluate("window.innerWidth").unwrap();
        let dark = page
            .evaluate("matchMedia('(prefers-color-scheme: dark)').matches")
            .unwrap();
        page.close();
        drop(browser);
        let _ = std::fs::remove_dir_all(&profile);

        assert_eq!(laid_out, json!(800));
        assert_eq!(dark, json!(true));
        assert_eq!(page.size(), (1000, 600));
    }
}
