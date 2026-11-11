//! The `preview_*` tools: each thread's agent drives a page of its own in the headless
//! browser, started on first use.

use super::{Browser, DEFAULT_SIZE, Observer, Page, PageEvent, find_browser};
use crate::{Notification, Notify};
use hoshi_core::{BrowserFrame, BrowserInput, BrowserPage, BrowserPoint};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

const LOAD_TIMEOUT: Duration = Duration::from_secs(30);
const WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_WAIT: Duration = Duration::from_secs(60);
const SNAPSHOT_CHARS: usize = 20_000;
const VALUE_CHARS: usize = 64_000;
const CONSOLE_LINES: usize = 20;
const MIN_SIZE: u64 = 240;
const MAX_SIZE: u64 = 3840;

/// The browser and the page of each thread that opened one.
pub(crate) struct Browsing {
    profile: PathBuf,
    browser: Option<Browser>,
    pages: HashMap<String, Page>,
    notify: Notify,
    /// Threads whose page the panel shows, which it streams to.
    viewing: HashSet<String>,
    /// The panel's size for threads whose page fills it, until a size is picked.
    fits: HashMap<String, (u32, u32)>,
    /// Threads whose page the user closed, until their agent opens one again.
    closed_by_user: HashSet<String>,
    /// Since when the browser has no page; it stays a while, keeping the sessions of the
    /// sites signed in to.
    idle_since: Option<Instant>,
}

/// How long a browser without pages is kept before it goes.
const IDLE_BROWSER: Duration = Duration::from_secs(10 * 60);
const NO_PAGE: &str = "this thread has no page open: call preview_open first";
const CLOSED_BY_USER: &str =
    "the user closed this thread's page: call preview_open if you still need it";

fn text<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments[key]
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn size(arguments: &Value, fallback: (u32, u32)) -> (u32, u32) {
    let side = |key: &str, fallback: u32| {
        arguments[key]
            .as_u64()
            .map_or(fallback, |value| value.clamp(MIN_SIZE, MAX_SIZE) as u32)
    };
    (side("width", fallback.0), side("height", fallback.1))
}

/// A bare host like `localhost:5173` gets `http://`; other addresses stay as written.
fn address(url: &str) -> String {
    let local = ["localhost", "127.", "0.0.0.0", "[::1]"]
        .iter()
        .any(|host| url.starts_with(host));
    if url.contains("://") || url.starts_with("about:") || url.starts_with("data:") {
        url.to_owned()
    } else if local {
        format!("http://{url}")
    } else {
        format!("https://{url}")
    }
}

impl Browsing {
    pub(crate) fn new(profile: PathBuf, notify: Notify) -> Self {
        Self {
            profile,
            browser: None,
            pages: HashMap::new(),
            notify,
            viewing: HashSet::new(),
            fits: HashMap::new(),
            closed_by_user: HashSet::new(),
            idle_since: None,
        }
    }

    /// The size a new page of the thread opens at: the panel's while it fills it.
    fn opening_size(&self, thread_id: &str) -> (u32, u32) {
        self.fits.get(thread_id).copied().unwrap_or(DEFAULT_SIZE)
    }

    /// The panel is `size` and the page fills it.
    pub(crate) fn user_fit(&mut self, thread_id: &str, wanted: (u32, u32)) -> Result<(), String> {
        let wanted = (
            wanted.0.clamp(MIN_SIZE as u32, MAX_SIZE as u32),
            wanted.1.clamp(MIN_SIZE as u32, MAX_SIZE as u32),
        );
        self.guarded(|this| {
            this.fits.insert(thread_id.to_owned(), wanted);
            if this.pages.contains_key(thread_id) {
                this.resize_page(thread_id, wanted)?;
            }
            this.announce(thread_id, false, None, false);
            Ok(())
        })
    }

    /// Runs `act`, dropping a browser that died before it and after a failure.
    fn guarded<T>(
        &mut self,
        act: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        self.drop_dead_browser();
        let result = act(self);
        if result.is_err() {
            self.drop_dead_browser();
        }
        result
    }

    /// A browser that quit or crashed goes with its pages, so the next page starts a new one.
    fn drop_dead_browser(&mut self) {
        if self.browser.as_ref().is_none_or(Browser::is_connected) {
            return;
        }
        self.browser = None;
        self.idle_since = None;
        let closed: Vec<String> = self.pages.drain().map(|(thread_id, _)| thread_id).collect();
        for thread_id in closed {
            self.announce(&thread_id, false, None, false);
        }
    }

    /// Closes every page and the browser, as when the app quits.
    pub(crate) fn close(&mut self) {
        self.pages.clear();
        self.browser = None;
        self.idle_since = None;
    }

    fn resize_page(&mut self, thread_id: &str, wanted: (u32, u32)) -> Result<(), String> {
        let viewing = self.viewing.contains(thread_id);
        let page = self.page(thread_id)?;
        if page.size() == wanted {
            return Ok(());
        }
        page.resize(wanted)?;
        if viewing {
            page.start_screencast(wanted)?;
        }
        Ok(())
    }

    /// What the page shows and where it went reach the UI as notifications.
    fn observer(&self, thread_id: &str) -> Observer {
        let (notify, thread_id) = (self.notify.clone(), thread_id.to_owned());
        Arc::new(move |event| {
            let notification = match event {
                PageEvent::Frame {
                    data,
                    width,
                    height,
                } => Notification::BrowserFrame(BrowserFrame {
                    thread_id: thread_id.clone(),
                    data,
                    width,
                    height,
                }),
                PageEvent::Changed { url, title, errors } => {
                    Notification::BrowserPage(BrowserPage {
                        thread_id: thread_id.clone(),
                        open: true,
                        url,
                        title,
                        errors,
                        by_agent: false,
                        agent_point: None,
                        opened: false,
                        fitted: None,
                    })
                }
            };
            notify(notification);
        })
    }

    /// Opens the thread's page if it has none, streaming it when the panel shows it.
    fn ensure_page(&mut self, thread_id: &str, wanted: (u32, u32)) -> Result<bool, String> {
        if self.pages.contains_key(thread_id) {
            return Ok(false);
        }
        let observer = self.observer(thread_id);
        let page = self.browser()?.open_page(wanted, observer)?;
        if self.viewing.contains(thread_id) {
            page.start_screencast(page.size())?;
        }
        self.pages.insert(thread_id.to_owned(), page);
        self.closed_by_user.remove(thread_id);
        self.idle_since = None;
        Ok(true)
    }

    /// Tells the UI where the page is now; `by_agent` when the agent touched it.
    fn announce(
        &self,
        thread_id: &str,
        by_agent: bool,
        agent_point: Option<(f64, f64)>,
        opened: bool,
    ) {
        let page = self.pages.get(thread_id);
        let (url, title) = page
            .and_then(|page| page.location().ok())
            .unwrap_or_default();
        (self.notify)(Notification::BrowserPage(BrowserPage {
            thread_id: thread_id.to_owned(),
            open: page.is_some(),
            url,
            title,
            errors: page.map_or(0, Page::errors),
            by_agent,
            agent_point: agent_point.map(|(x, y)| BrowserPoint { x, y }),
            opened,
            fitted: Some(self.fits.contains_key(thread_id)),
        }));
    }

    /// The panel shows the thread's page, or stopped showing it.
    pub(crate) fn view(&mut self, thread_id: &str, viewing: bool) -> Result<(), String> {
        self.guarded(|this| {
            if viewing {
                this.viewing.insert(thread_id.to_owned());
            } else {
                this.viewing.remove(thread_id);
            }
            if let Some(page) = this.pages.get(thread_id) {
                if viewing {
                    page.start_screencast(page.size())?;
                } else {
                    page.stop_screencast();
                }
            }
            this.announce(thread_id, false, None, false);
            Ok(())
        })
    }

    /// The user opens an address from the panel, with a page of their own when the thread
    /// had none.
    pub(crate) fn user_open(&mut self, thread_id: &str, url: &str) -> Result<(), String> {
        self.guarded(|this| {
            this.ensure_page(thread_id, this.opening_size(thread_id))?;
            this.page(thread_id)?
                .navigate(&address(url), LOAD_TIMEOUT)?;
            this.announce(thread_id, false, None, false);
            Ok(())
        })
    }

    /// Back (`-1`), forward (`1`) or reload (`0`).
    pub(crate) fn user_go(&mut self, thread_id: &str, delta: i32) -> Result<(), String> {
        self.guarded(|this| {
            let page = this.page(thread_id)?;
            if delta == 0 {
                page.reload(LOAD_TIMEOUT)?;
            } else {
                page.go(delta, LOAD_TIMEOUT)?;
            }
            this.announce(thread_id, false, None, false);
            Ok(())
        })
    }

    /// The user picked a size, which stops the page filling the panel.
    pub(crate) fn user_resize(
        &mut self,
        thread_id: &str,
        wanted: (u32, u32),
    ) -> Result<(), String> {
        self.guarded(|this| {
            this.fits.remove(thread_id);
            this.resize_page(thread_id, wanted)?;
            this.announce(thread_id, false, None, false);
            Ok(())
        })
    }

    pub(crate) fn user_input(
        &mut self,
        thread_id: &str,
        input: &BrowserInput,
    ) -> Result<(), String> {
        self.guarded(|this| {
            let page = this.page(thread_id)?;
            match input {
                BrowserInput::Click { x, y } => page.click_at((*x, *y)),
                BrowserInput::Scroll {
                    x,
                    y,
                    delta_x,
                    delta_y,
                } => page.scroll_at((*x, *y), (*delta_x, *delta_y)),
                BrowserInput::Key { key } => page.press(key),
                BrowserInput::Text { text } => page.insert_text(text),
            }
        })
    }

    fn browser(&mut self) -> Result<&Browser, String> {
        if self.browser.is_none() {
            let executable = find_browser().ok_or(
                "no Chrome or Edge is installed here, so there is no browser to open pages in",
            )?;
            self.browser = Some(Browser::start(&executable, &self.profile)?);
        }
        self.browser.as_ref().ok_or_else(|| "no browser".into())
    }

    fn page(&mut self, thread_id: &str) -> Result<&mut Page, String> {
        let closed = self.closed_by_user.contains(thread_id);
        self.pages
            .get_mut(thread_id)
            .ok_or_else(|| if closed { CLOSED_BY_USER } else { NO_PAGE }.into())
    }

    /// Closes the thread's page. The browser stays a while after its last page.
    pub(crate) fn forget(&mut self, thread_id: &str) {
        if let Some(page) = self.pages.remove(thread_id) {
            page.close();
        }
        self.viewing.remove(thread_id);
        self.fits.remove(thread_id);
        self.closed_by_user.remove(thread_id);
        if self.pages.is_empty() && self.browser.is_some() && self.idle_since.is_none() {
            self.idle_since = Some(Instant::now());
        }
        self.announce(thread_id, false, None, false);
    }

    /// The user closed the thread's page from the panel, which its agent is told the next
    /// time it reaches for it.
    pub(crate) fn user_close(&mut self, thread_id: &str) {
        let open = self.pages.contains_key(thread_id);
        self.forget(thread_id);
        if open {
            self.closed_by_user.insert(thread_id.to_owned());
        }
    }

    /// Ends a browser that has had no page for a while.
    pub(crate) fn end_idle_browser(&mut self) {
        let idle = self
            .idle_since
            .is_some_and(|since| since.elapsed() >= IDLE_BROWSER);
        if idle && self.pages.is_empty() {
            self.browser = None;
            self.idle_since = None;
        }
    }

    pub(crate) fn call(
        &mut self,
        caller: &str,
        tool: &str,
        arguments: &Value,
    ) -> Result<Value, String> {
        self.drop_dead_browser();
        let point = self.agent_point(caller, tool, arguments);
        let result = self.dispatch(caller, tool, arguments);
        if result.is_err() {
            self.drop_dead_browser();
        }
        if result.is_ok() && !matches!(tool, "preview_close" | "preview_status") {
            let opened = tool == "preview_open"
                && result.as_ref().is_ok_and(|value| value["opened"] == true);
            self.announce(caller, true, point, opened);
        }
        result
    }

    /// Where an action is about to happen, to show the user what the agent acts on.
    fn agent_point(&self, caller: &str, tool: &str, arguments: &Value) -> Option<(f64, f64)> {
        if tool != "preview_act" {
            return None;
        }
        if let (Some(x), Some(y)) = (arguments["x"].as_f64(), arguments["y"].as_f64()) {
            return Some((x, y));
        }
        let page = self.pages.get(caller)?;
        page.point_of(text(arguments, "ref")?).ok()
    }

    fn dispatch(&mut self, caller: &str, tool: &str, arguments: &Value) -> Result<Value, String> {
        match tool {
            "preview_open" => self.open(caller, arguments),
            "preview_close" => {
                let open = self.pages.contains_key(caller);
                self.forget(caller);
                Ok(json!({ "closed": open }))
            }
            "preview_status" => self.status(caller),
            _ => self.act_on(caller, tool, arguments),
        }
    }

    fn open(&mut self, caller: &str, arguments: &Value) -> Result<Value, String> {
        let url = address(text(arguments, "url").ok_or("`url` is required")?);
        // A size the agent asks for wins over the panel's.
        let sized = !arguments["width"].is_null() || !arguments["height"].is_null();
        if sized {
            self.fits.remove(caller);
        }
        let opened = self.ensure_page(caller, size(arguments, self.opening_size(caller)))?;
        if sized {
            let wanted = size(arguments, self.page(caller)?.size());
            self.resize_page(caller, wanted)?;
        }
        let page = self.page(caller)?;
        page.navigate(&url, LOAD_TIMEOUT)?;
        let mut result = describe(page)?;
        result["opened"] = json!(opened);
        Ok(result)
    }

    fn status(&mut self, caller: &str) -> Result<Value, String> {
        let Some(page) = self.pages.get(caller) else {
            let closed_by_user = self.closed_by_user.contains(caller);
            return Ok(json!({ "open": false, "closedByUser": closed_by_user }));
        };
        let mut status = describe(page)?;
        status["open"] = json!(true);
        status["console"] = json!(page.console(CONSOLE_LINES));
        Ok(status)
    }

    fn act_on(&mut self, caller: &str, tool: &str, arguments: &Value) -> Result<Value, String> {
        let page = self.page(caller)?;
        let reference = text(arguments, "ref");
        match tool {
            "preview_navigate" => {
                let url = address(text(arguments, "url").ok_or("`url` is required")?);
                page.navigate(&url, LOAD_TIMEOUT)?;
                describe(page)
            }
            "preview_snapshot" => {
                let snapshot = page.snapshot(SNAPSHOT_CHARS)?;
                let mut result = describe(page)?;
                result["snapshot"] = json!(snapshot);
                if arguments["includeImage"] == true {
                    result["image"] = json!(page.screenshot()?);
                }
                Ok(result)
            }
            "preview_act" => {
                let action = text(arguments, "action").ok_or("`action` is required")?;
                let needs_ref = || reference.ok_or(format!("`ref` is required to {action}"));
                match action {
                    "click" => match (arguments["x"].as_f64(), arguments["y"].as_f64()) {
                        (Some(x), Some(y)) => page.click_at((x, y))?,
                        _ => page.click(needs_ref()?)?,
                    },
                    "hover" => page.hover(needs_ref()?)?,
                    "type" => page.type_text(
                        needs_ref()?,
                        arguments["text"].as_str().unwrap_or_default(),
                        arguments["clear"].as_bool().unwrap_or(false),
                    )?,
                    "press" => page.press(text(arguments, "key").ok_or("`key` is required")?)?,
                    "select" => {
                        let values: Vec<String> = arguments["values"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|value| value.as_str().map(str::to_owned))
                            .collect();
                        page.select(needs_ref()?, &values)?;
                    }
                    "scroll" => page.scroll(
                        reference,
                        (
                            arguments["deltaX"].as_f64().unwrap_or(0.0),
                            arguments["deltaY"].as_f64().unwrap_or(0.0),
                        ),
                    )?,
                    other => return Err(format!("unknown action {other}")),
                }
                let mut result = describe(page)?;
                result["done"] = json!(action);
                Ok(result)
            }
            "preview_evaluate" => {
                let expression = text(arguments, "expression").ok_or("`expression` is required")?;
                let value = page.evaluate(expression)?;
                let shown = value.to_string();
                if shown.len() > VALUE_CHARS {
                    return Ok(
                        json!({ "value": format!("{}…", &shown[..shown.floor_char_boundary(VALUE_CHARS)]), "truncated": true }),
                    );
                }
                Ok(json!({ "value": value }))
            }
            "preview_wait_for" => {
                let timeout = arguments["timeoutMs"]
                    .as_u64()
                    .map_or(WAIT_TIMEOUT, Duration::from_millis)
                    .min(MAX_WAIT);
                let found = page.wait_for(
                    text(arguments, "text"),
                    text(arguments, "selector"),
                    timeout,
                )?;
                Ok(json!({ "found": found }))
            }
            "preview_resize" => {
                let wanted = size(arguments, page.size());
                self.fits.remove(caller);
                self.resize_page(caller, wanted)?;
                Ok(json!({ "width": wanted.0, "height": wanted.1 }))
            }
            "preview_set_appearance" => {
                let scheme = match text(arguments, "colorScheme") {
                    Some("dark") => Some("dark"),
                    Some("light") => Some("light"),
                    _ => None,
                };
                page.set_color_scheme(scheme)?;
                Ok(json!({ "colorScheme": scheme.unwrap_or("system") }))
            }
            _ => Err(format!("hoshi has no tool named {tool}")),
        }
    }
}

fn describe(page: &Page) -> Result<Value, String> {
    let (url, title) = page.location()?;
    let (width, height) = page.size();
    Ok(json!({ "url": url, "title": title, "width": width, "height": height }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_opens_reads_uses_and_closes_its_page() {
        if find_browser().is_none() {
            eprintln!("no Chromium-based browser here; skipping");
            return;
        }
        let profile = std::env::temp_dir().join(format!("hoshi-preview-{}", std::process::id()));
        let notify: Notify = Arc::new(|_| {});
        let mut browsing = Browsing::new(profile.clone(), notify);
        let page = "data:text/html,<button onclick=\"console.error('boom');this.textContent='Done'\">Run</button>";

        let opened = browsing
            .call(
                "t",
                "preview_open",
                &json!({ "url": page, "width": 390, "height": 844 }),
            )
            .unwrap();
        let snapshot = browsing
            .call("t", "preview_snapshot", &json!({ "includeImage": true }))
            .unwrap();
        let text = snapshot["snapshot"].as_str().unwrap();
        let reference = text
            .lines()
            .find(|line| line.contains("button"))
            .and_then(|line| line.split("[ref=").nth(1))
            .map(|rest| rest.trim_end_matches(']'))
            .unwrap();
        browsing
            .call(
                "t",
                "preview_act",
                &json!({ "action": "click", "ref": reference }),
            )
            .unwrap();
        let label = browsing
            .call(
                "t",
                "preview_evaluate",
                &json!({ "expression": "document.querySelector('button').textContent" }),
            )
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let status = browsing.call("t", "preview_status", &json!({})).unwrap();
        let other = browsing.call("other", "preview_snapshot", &json!({}));
        browsing.call("t", "preview_close", &json!({})).unwrap();

        assert_eq!(
            (opened["width"].as_u64(), opened["height"].as_u64()),
            (Some(390), Some(844))
        );
        assert!(!snapshot["image"].as_str().unwrap().is_empty());
        assert_eq!(label["value"], "Done");
        assert!(
            status["console"]
                .as_array()
                .unwrap()
                .iter()
                .any(|line| line == "error: boom")
        );
        assert!(other.is_err());
        assert_eq!(
            browsing.call("t", "preview_status", &json!({})).unwrap()["open"],
            false
        );
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[test]
    fn a_page_the_user_closed_says_so_and_opens_again_in_the_same_browser() {
        if find_browser().is_none() {
            eprintln!("no Chromium-based browser here; skipping");
            return;
        }
        let profile = std::env::temp_dir().join(format!("hoshi-closed-{}", std::process::id()));
        let mut browsing = Browsing::new(profile.clone(), Arc::new(|_| {}));
        let page = "data:text/html,<p>Signed in</p>";
        browsing
            .call("t", "preview_open", &json!({ "url": page }))
            .unwrap();

        browsing.user_close("t");
        let refused = browsing.call("t", "preview_snapshot", &json!({}));
        let status = browsing.call("t", "preview_status", &json!({})).unwrap();
        let kept = browsing.browser.is_some();
        let again = browsing
            .call("t", "preview_open", &json!({ "url": page }))
            .unwrap();
        let after = browsing.call("t", "preview_status", &json!({})).unwrap();

        assert_eq!(refused.unwrap_err(), CLOSED_BY_USER);
        assert_eq!(status["closedByUser"], true);
        assert!(kept);
        assert_eq!(again["opened"], true);
        assert_eq!(after["open"], true);
        browsing.close();
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[test]
    fn the_panel_sees_the_page_live_and_the_user_can_use_it() {
        if find_browser().is_none() {
            eprintln!("no Chromium-based browser here; skipping");
            return;
        }
        let profile = std::env::temp_dir().join(format!("hoshi-view-{}", std::process::id()));
        let (sender, received) = std::sync::mpsc::channel();
        let sender = std::sync::Mutex::new(sender);
        let notify: Notify = Arc::new(move |notification| {
            let _ = sender.lock().unwrap().send(notification);
        });
        let mut browsing = Browsing::new(profile.clone(), notify);
        let page = "data:text/html,<button style=\"width:400px;height:200px\" onclick=\"this.textContent='Clicked'\">Press</button>";

        let opened = browsing
            .call("t", "preview_open", &json!({ "url": page }))
            .unwrap();
        browsing.view("t", true).unwrap();
        let frame = received
            .iter()
            .find_map(|notification| match notification {
                Notification::BrowserFrame(frame) => Some(frame),
                _ => None,
            })
            .unwrap();
        browsing
            .user_input("t", &BrowserInput::Click { x: 100.0, y: 100.0 })
            .unwrap();
        let label = browsing
            .call(
                "t",
                "preview_evaluate",
                &json!({ "expression": "document.querySelector('button').textContent" }),
            )
            .unwrap();
        browsing.view("t", false).unwrap();
        browsing.forget("t");

        assert_eq!(opened["opened"], true);
        assert_eq!((frame.thread_id.as_str(), frame.width), ("t", 1280));
        assert!(!frame.data.is_empty());
        assert_eq!(label["value"], "Clicked");
        let pages: Vec<BrowserPage> = received
            .try_iter()
            .filter_map(|notification| match notification {
                Notification::BrowserPage(page) => Some(page),
                _ => None,
            })
            .collect();
        assert!(pages.iter().any(|page| page.by_agent));
        assert!(pages.last().is_some_and(|page| !page.open));
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[test]
    fn a_browser_that_went_away_starts_again_on_the_next_open() {
        if find_browser().is_none() {
            eprintln!("no Chromium-based browser here; skipping");
            return;
        }
        let profile = std::env::temp_dir().join(format!("hoshi-relaunch-{}", std::process::id()));
        let mut browsing = Browsing::new(profile.clone(), Arc::new(|_| {}));
        let page = json!({ "url": "about:blank" });
        browsing.call("t", "preview_open", &page).unwrap();
        let browser = browsing.browser.as_ref().unwrap();
        let _ = browser.connection.call_within(
            "Browser.close",
            json!({}),
            None,
            Duration::from_secs(5),
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while browser.is_connected() {
            assert!(std::time::Instant::now() < deadline, "the browser stayed");
            std::thread::sleep(Duration::from_millis(20));
        }

        let lost = browsing.call("t", "preview_status", &json!({})).unwrap();
        let reopened = browsing.call("t", "preview_open", &page).unwrap();
        let status = browsing.call("t", "preview_status", &json!({})).unwrap();
        browsing.close();

        assert_eq!(lost["open"], false);
        assert_eq!(reopened["opened"], true);
        assert_eq!(status["open"], true);
        assert!(browsing.browser.is_none());
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[test]
    fn bare_addresses_get_a_scheme() {
        assert_eq!(address("localhost:5173"), "http://localhost:5173");
        assert_eq!(address("127.0.0.1:3000/x"), "http://127.0.0.1:3000/x");
        assert_eq!(address("example.com"), "https://example.com");
        assert_eq!(address("file:///tmp/a.html"), "file:///tmp/a.html");
        assert_eq!(address("about:blank"), "about:blank");
    }
}
