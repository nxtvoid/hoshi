//! The headless browser agents drive, a page per thread: real input events, the accessibility
//! tree with refs to act on, screenshots and console.

mod act;
mod cdp;
pub(crate) mod html;
mod keys;
mod launch;
mod outline;
mod refs;
pub(crate) mod tools;

pub use launch::find_browser;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use cdp::{CALL_TIMEOUT, Connection};
use launch::BrowserProcess;
use refs::{Place, Refs, Tree, text_of};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

pub const DEFAULT_SIZE: (u32, u32) = (1280, 800);
const CONSOLE_KEPT: usize = 200;
/// Dialogs in a row dismissed for a page to go elsewhere.
const MAX_DISMISSED: usize = 20;
/// The longest side of a screenshot, in pixels.
const MAX_SCREENSHOT_SIDE: f64 = 1600.0;
const READY_POLL: Duration = Duration::from_millis(100);
const SCREENCAST_QUALITY: u8 = 85;
/// The most device pixels per pixel a page paints at. The browser's own screen has it, as a
/// screencast frame is never sharper than that screen.
const MAX_SCALE: f64 = 3.0;

/// `size` in device pixels at `scale`.
fn device_pixels(size: (u32, u32), scale: f64) -> (u32, u32) {
    let side = |side: u32| (f64::from(side) * scale).round().max(1.0) as u32;
    (side(size.0), side(size.1))
}

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
        let dialog = Arc::new(Mutex::new(None));
        follow_dialogs(&self.connection, &session_id, Arc::downgrade(&dialog));
        let mut page = Page {
            connection: self.connection.clone(),
            target_id,
            session_id,
            refs: Refs::default(),
            dialog,
            acted_at: None,
            acted: false,
            console,
            size,
            zoom: 1.0,
            scale: 1.0,
        };
        for domain in ["Page.enable", "Runtime.enable", "DOM.enable"] {
            page.call(domain, json!({}))?;
        }
        // Behind another thread's page it would count as hidden: its timers slow down and
        // pointing at it takes seconds.
        page.call(
            "Emulation.setFocusEmulationEnabled",
            json!({ "enabled": true }),
        )?;
        page.resize(size)?;
        Ok(page)
    }

    /// Closes a page by its target, whatever it is busy with.
    pub fn close_target(&self, target_id: &str) {
        let _ = self
            .connection
            .call("Target.closeTarget", json!({ "targetId": target_id }), None);
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

/// A dialog of the page's own: an alert, a confirm or a prompt.
struct Dialog {
    /// Tells it from the next one, which may open as it closes.
    serial: u64,
    kind: String,
    message: String,
    /// What a prompt offers as its answer.
    offered: String,
}

impl Dialog {
    fn says(&self) -> String {
        format!(
            "the page waits on a dialog ({}) that says {:?}: press Enter to accept it or Escape to dismiss it",
            self.kind, self.message
        )
    }
}

/// Keeps `dialog` with the one the page shows. Leaving a page is accepted on its own, as
/// whoever asked to leave meant it.
fn follow_dialogs(
    connection: &Arc<Connection>,
    session: &str,
    dialog: Weak<Mutex<Option<Dialog>>>,
) {
    let events = connection.subscribe();
    let connection = Arc::downgrade(connection);
    let session = session.to_owned();
    thread::spawn(move || {
        let mut serial = 0;
        for event in events {
            let Some(dialog) = dialog.upgrade() else {
                return;
            };
            if event.session_id.as_deref() != Some(session.as_str()) {
                continue;
            }
            match event.method.as_str() {
                "Page.javascriptDialogOpening" if event.params["type"] == "beforeunload" => {
                    let Some(connection) = connection.upgrade() else {
                        return;
                    };
                    let _ = connection.call(
                        "Page.handleJavaScriptDialog",
                        json!({ "accept": true }),
                        Some(&session),
                    );
                }
                "Page.javascriptDialogOpening" => {
                    serial += 1;
                    *lock(&dialog) = Some(Dialog {
                        serial,
                        offered: text_of(&event.params["defaultPrompt"]),
                        kind: text_of(&event.params["type"]),
                        message: text_of(&event.params["message"]),
                    });
                }
                "Page.javascriptDialogClosed" => *lock(&dialog) = None,
                _ => {}
            }
        }
    });
}

pub struct Page {
    connection: Arc<Connection>,
    target_id: String,
    session_id: String,
    refs: Refs,
    /// The dialog the page shows, which holds it until answered.
    dialog: Arc<Mutex<Option<Dialog>>>,
    /// Where the agent last acted on an element.
    acted_at: Option<(f64, f64)>,
    /// Whether the agent's action reached the page.
    acted: bool,
    console: Arc<Mutex<VecDeque<String>>>,
    size: (u32, u32),
    /// Page zoom: the page lays out at `size / zoom` CSS pixels and paints at `size`.
    zoom: f64,
    /// Device pixels per pixel of `size` on the screen that shows it.
    scale: f64,
}

impl Page {
    fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        self.call_within(method, params, CALL_TIMEOUT)
    }

    /// Gives up at once while a dialog holds the page, which answers nothing until it closes.
    fn call_within(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let held = || lock(&self.dialog).as_ref().map(Dialog::says);
        if let Some(reason) = held() {
            return Err(reason);
        }
        self.connection
            .call_unless(method, params, Some(&self.session_id), timeout, held)
            .map_err(|error| match error.as_str() {
                "Session with given id not found." => "the page was closed".into(),
                _ => error,
            })
    }

    /// Accepts or dismisses the dialog the page shows; `false` when it shows none.
    fn answer_dialog(&self, accept: bool) -> Result<bool, String> {
        let Some((serial, offered)) = lock(&self.dialog)
            .as_ref()
            .map(|dialog| (dialog.serial, dialog.offered.clone()))
        else {
            return Ok(false);
        };
        self.connection.call(
            "Page.handleJavaScriptDialog",
            json!({ "accept": accept, "promptText": offered }),
            Some(&self.session_id),
        )?;
        lock(&self.dialog).take_if(|dialog| dialog.serial == serial);
        Ok(true)
    }

    /// Dismisses the dialogs that would hold the page back from going elsewhere.
    fn dismiss_dialogs(&self) -> Result<(), String> {
        for _ in 0..MAX_DISMISSED {
            if !self.answer_dialog(false)? {
                break;
            }
            thread::sleep(READY_POLL);
        }
        Ok(())
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Goes to `url` and waits until it loaded, or `timeout` passed.
    pub fn navigate(&mut self, url: &str, timeout: Duration) -> Result<(), String> {
        self.dismiss_dialogs()?;
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

    /// How much smaller than the page its screenshots are, as large ones cost agents dearly.
    fn screenshot_shrink(&self) -> f64 {
        (MAX_SCREENSHOT_SIDE / f64::from(self.size.0.max(self.size.1))).min(1.0)
    }

    /// Pixels of a screenshot per CSS pixel.
    pub fn screenshot_scale(&self) -> f64 {
        self.screenshot_shrink() * self.zoom
    }

    /// A PNG of what the page shows, base64, at `size` whatever screen shows it, up to a limit.
    pub fn screenshot(&self) -> Result<String, String> {
        let mut params = json!({ "format": "png" });
        let shrunk = self.screenshot_shrink();
        if self.scale != 1.0 || shrunk != 1.0 {
            let metrics = self.call("Page.getLayoutMetrics", json!({}))?;
            let viewport = &metrics["cssVisualViewport"];
            let side = |key: &str| viewport[key].as_f64().unwrap_or_default();
            params["clip"] = json!({
                "x": side("pageX"),
                "y": side("pageY"),
                "width": side("clientWidth"),
                "height": side("clientHeight"),
                "scale": shrunk / self.scale,
            });
        }
        let shot = self.call("Page.captureScreenshot", params)?;
        shot["data"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "the browser gave no screenshot".into())
    }

    /// The page as an outline, with a ref for what can be acted on.
    pub fn snapshot(&mut self, max_chars: usize) -> Result<String, String> {
        let place = self.place()?;
        let nodes = self.nodes()?;
        let tree = Tree::new(&nodes);
        self.refs.begin(&tree, place);
        let text = outline::outline(&tree, max_chars, &mut self.refs);
        self.refs.trim();
        Ok(text)
    }

    /// The page's accessibility nodes, the root first.
    fn nodes(&self) -> Result<Vec<Value>, String> {
        let mut tree = self.call("Accessibility.getFullAXTree", json!({}))?;
        match tree["nodes"].take() {
            Value::Array(nodes) => Ok(nodes),
            _ => Ok(Vec::new()),
        }
    }

    fn place(&self) -> Result<Place, String> {
        let value = self.evaluate("[String(performance.timeOrigin), location.href]")?;
        Ok(Place {
            document: text_of(&value[0]),
            url: text_of(&value[1]),
        })
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
                "deviceScaleFactor": self.zoom * self.scale,
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

    /// Paints `scale` device pixels per pixel of its size, as the screen showing it has.
    pub fn set_scale(&mut self, scale: f64) -> Result<(), String> {
        self.scale = scale.clamp(1.0, MAX_SCALE);
        self.resize(self.size)
    }

    /// Its size in the device pixels it paints.
    pub fn device_size(&self) -> (u32, u32) {
        device_pixels(self.size, self.scale)
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

    /// Streams what the page paints to its observer, one device pixel per pixel.
    pub fn start_screencast(&self) -> Result<(), String> {
        let (width, height) = self.device_size();
        self.call(
            "Page.startScreencast",
            json!({ "format": "jpeg", "quality": SCREENCAST_QUALITY, "maxWidth": width, "maxHeight": height, "everyNthFrame": 1 }),
        )
        .map(|_| ())
    }

    pub fn stop_screencast(&self) {
        let _ = self.call("Page.stopScreencast", json!({}));
    }

    /// Back (`-1`) or forward (`1`) in the page's history.
    pub fn go(&mut self, delta: i32, timeout: Duration) -> Result<(), String> {
        self.dismiss_dialogs()?;
        self.evaluate(&format!("history.go({delta})"))?;
        self.refs.clear();
        thread::sleep(READY_POLL);
        self.wait_ready(timeout)
    }

    pub fn reload(&mut self, timeout: Duration) -> Result<(), String> {
        self.dismiss_dialogs()?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
                .and_then(|line| line.rsplit_once(" [e"))
                .map(|(_, rest)| format!("e{}", rest.trim_end_matches(']')))
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

    #[test]
    fn a_page_s_device_pixels_follow_the_screen_s_scale() {
        assert_eq!(device_pixels((800, 600), 1.0), (800, 600));
        assert_eq!(device_pixels((800, 600), 1.5), (1200, 900));
        assert_eq!(device_pixels((801, 333), 1.25), (1001, 416));
        assert_eq!(device_pixels((0, 0), 2.0), (1, 1));
    }

    /// The width and height of a JPEG, from its frame header.
    fn jpeg_size(jpeg: &[u8]) -> Option<(u32, u32)> {
        let mut at = 2;
        while at + 9 < jpeg.len() {
            let marker = jpeg[at + 1];
            let length = usize::from(u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]));
            if matches!(marker, 0xc0 | 0xc2) {
                let side =
                    |from: usize| u32::from(u16::from_be_bytes([jpeg[from], jpeg[from + 1]]));
                return Some((side(at + 7), side(at + 5)));
            }
            at += 2 + length;
        }
        None
    }

    #[test]
    fn a_scaled_page_streams_device_pixels_and_screenshots_at_its_size() {
        let Some(executable) = find_browser() else {
            eprintln!("no Chromium-based browser here; skipping");
            return;
        };
        let profile = std::env::temp_dir().join(format!("hoshi-scale-{}", std::process::id()));
        let browser = Browser::start(&executable, &profile).unwrap();
        let (sender, frames) = std::sync::mpsc::channel();
        let sender = Mutex::new(sender);
        let mut page = browser
            .open_page(
                (400, 300),
                Arc::new(move |event| {
                    if let PageEvent::Frame {
                        jpeg,
                        width,
                        height,
                    } = event
                    {
                        let _ = lock(&sender).send((jpeg_size(&jpeg), width, height));
                    }
                }),
            )
            .unwrap();
        page.navigate(
            "data:text/html,<body style='height:3000px'>Sharp</body>",
            Duration::from_secs(10),
        )
        .unwrap();

        page.set_scale(1.5).unwrap();
        page.evaluate("scrollTo(0, 500)").unwrap();
        let ratio = page.evaluate("devicePixelRatio").unwrap();
        let laid_out = page.evaluate("innerWidth").unwrap();
        let shot = STANDARD.decode(page.screenshot().unwrap()).unwrap();
        page.start_screencast().unwrap();
        let frame = frames.recv_timeout(Duration::from_secs(30)).unwrap();
        page.close();
        drop(browser);
        let _ = std::fs::remove_dir_all(&profile);

        assert!((ratio.as_f64().unwrap() - 1.5).abs() < 0.001);
        assert_eq!(laid_out, json!(400));
        let side = |from: usize| u32::from_be_bytes(shot[from..from + 4].try_into().unwrap());
        assert_eq!((side(16), side(20)), (400, 300));
        assert_eq!(page.device_size(), (600, 450));
        assert_eq!(frame, (Some((600, 450)), 400, 300));
    }
}
