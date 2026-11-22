//! The `preview_*` tools: each thread's agent drives a page of its own in the headless
//! browser, started on first use.

use super::{Browser, DEFAULT_SIZE, Observer, Page, PageEvent, find_browser};
use crate::throttle::Throttle;
use crate::{Notification, Notify};
use hoshi_core::{
    BrowserFrame, BrowserInput, BrowserPage, BrowserPoint, BrowserSettings, PageColorScheme,
    PageSize, SearchEngine,
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

const LOAD_TIMEOUT: Duration = Duration::from_secs(30);
const WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_WAIT: Duration = Duration::from_secs(60);
const SNAPSHOT_CHARS: usize = 20_000;
const VALUE_CHARS: usize = 64_000;
const CONSOLE_LINES: usize = 20;
const MIN_SIZE: u64 = 240;
const MAX_SIZE: u64 = 3840;

/// The browser and the page of each thread that opened one. Each page is used by one
/// caller at a time, apart from the others, so a slow page keeps no other waiting.
pub(crate) struct Browsing {
    profile: PathBuf,
    notify: Notify,
    /// Apart from the rest, so they change while a page is busy.
    settings: Arc<Mutex<BrowserSettings>>,
    /// Held only for a moment, never while the browser works.
    state: Mutex<State>,
    /// Held while the browser starts, so it starts once.
    launching: Mutex<()>,
}

#[derive(Default)]
struct State {
    browser: Option<Arc<Browser>>,
    pages: HashMap<String, OpenPage>,
    /// Threads whose page the panel shows, which it streams to.
    viewing: HashSet<String>,
    /// The panel's size for threads whose page fills it, until a size is picked.
    fits: HashMap<String, (u32, u32)>,
    /// Device pixels per pixel of the screen showing each thread's page.
    scales: HashMap<String, f64>,
    /// Threads whose page the user closed, until their agent opens one again.
    closed_by_user: HashSet<String>,
    /// Since when the browser has no page; it stays a while, keeping the sessions of the
    /// sites signed in to.
    idle_since: Option<Instant>,
    /// The panel's size when a page last filled it, for new pages that open filling it.
    last_fit: Option<(u32, u32)>,
}

struct OpenPage {
    page: Arc<Mutex<Page>>,
    /// To close it while it is busy.
    target_id: String,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The least time between two frames of a page sent to the UI: about 30 a second.
const FRAME_EVERY: Duration = Duration::from_millis(33);
/// How long a browser without pages is kept before it goes.
const IDLE_BROWSER: Duration = Duration::from_secs(10 * 60);
const NO_PAGE: &str = "this thread has no page open: call preview_open first";
const CLOSED_BY_USER: &str =
    "the user closed this thread's page: call preview_open if you still need it";
const NO_AGENT_ACCESS: &str =
    "the user turned off the browser for agents in hoshi's settings, so it can't be used";

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

/// Whether the address bar's `input` is somewhere to go rather than words to search for.
fn is_address(input: &str) -> bool {
    is_local_path(input)
        || !input.contains(char::is_whitespace)
            && (input.contains("://")
                || input.contains('.')
                || input.contains(':')
                || input.starts_with('/')
                || input.starts_with("localhost"))
}

fn is_local_path(input: &str) -> bool {
    let bytes = input.as_bytes();
    let windows_path = bytes.len() > 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    windows_path
        || input.starts_with("file://")
        || (input.starts_with('/') && !input.starts_with("//"))
}

/// `text` as a URL query value.
fn encode_query(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(byte).to_string()
            }
            b' ' => "+".to_owned(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// A custom search URL hoshi can use: http(s), with `%s` for the terms.
fn usable_search_url(template: &str) -> bool {
    (template.starts_with("https://") || template.starts_with("http://")) && template.contains("%s")
}

/// The search for `terms` with the engine the user chose; DuckDuckGo while a custom one
/// isn't usable.
fn search_url(terms: &str, settings: &BrowserSettings) -> String {
    let template = match settings.search_engine {
        SearchEngine::Custom if usable_search_url(&settings.custom_search_url) => {
            settings.custom_search_url.as_str()
        }
        SearchEngine::DuckDuckGo | SearchEngine::Custom => "https://duckduckgo.com/?q=%s",
        SearchEngine::Google => "https://www.google.com/search?q=%s",
        SearchEngine::Bing => "https://www.bing.com/search?q=%s",
        SearchEngine::Brave => "https://search.brave.com/search?q=%s",
        SearchEngine::Kagi => "https://kagi.com/search?q=%s",
        SearchEngine::Ecosia => "https://www.ecosia.org/search?q=%s",
    };
    template.replacen("%s", &encode_query(terms), 1)
}

/// What the address bar opens for `input`: the address, or a search for the words.
fn address_or_search(input: &str, settings: &BrowserSettings) -> String {
    let input = input.trim();
    if is_address(input) {
        address(input)
    } else {
        search_url(input, settings)
    }
}

/// A bare host like `localhost:5173` gets `http://`, an absolute file path `file://`; other
/// addresses stay as written.
fn address(url: &str) -> String {
    let local = ["localhost", "127.", "0.0.0.0", "[::1]"]
        .iter()
        .any(|host| url.starts_with(host));
    let bytes = url.as_bytes();
    let windows_path = bytes.len() > 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    if windows_path {
        format!("file:///{}", url.replace('\\', "/").replace(' ', "%20"))
    } else if url.starts_with('/') && !url.starts_with("//") {
        format!("file://{}", url.replace(' ', "%20"))
    } else if url.starts_with("file://") {
        url.replace(' ', "%20")
    } else if url.contains("://") || url.starts_with("about:") || url.starts_with("data:") {
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
            notify,
            settings: Arc::default(),
            state: Mutex::default(),
            launching: Mutex::new(()),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// Where the settings are kept, to change them without waiting for a busy page.
    pub(crate) fn settings_handle(&self) -> Arc<Mutex<BrowserSettings>> {
        self.settings.clone()
    }

    /// How new pages open, and whether agents may use the browser.
    #[cfg(test)]
    pub(crate) fn configure(&self, settings: BrowserSettings) {
        *lock(&self.settings) = settings;
    }

    /// Holds the browser's start, as a slow start does.
    #[cfg(test)]
    pub(crate) fn hold_launch(&self) -> MutexGuard<'_, ()> {
        lock(&self.launching)
    }

    fn settings(&self) -> BrowserSettings {
        lock(&self.settings).clone()
    }

    /// The size a new page of the thread opens at: the panel's while it fills it, else the
    /// one the user picked in the settings.
    fn opening_size(&self, thread_id: &str) -> (u32, u32) {
        let (fit, last_fit) = {
            let state = self.state();
            (state.fits.get(thread_id).copied(), state.last_fit)
        };
        if let Some(fit) = fit {
            return fit;
        }
        match self.settings().page_size {
            PageSize::Fill => last_fit.unwrap_or(DEFAULT_SIZE),
            PageSize::Desktop => DEFAULT_SIZE,
            PageSize::Tablet => (768, 1024),
            PageSize::Phone => (390, 844),
        }
    }

    /// A page that just opened at its default size fills the panel when the settings say so.
    fn fill_when_default(&self, thread_id: &str, size: (u32, u32)) {
        if self.settings().page_size == PageSize::Fill {
            self.state()
                .fits
                .entry(thread_id.to_owned())
                .or_insert(size);
        }
    }

    /// The panel is `size` and the page fills it.
    pub(crate) fn user_fit(&self, thread_id: &str, wanted: (u32, u32)) -> Result<(), String> {
        let wanted = (
            wanted.0.clamp(MIN_SIZE as u32, MAX_SIZE as u32),
            wanted.1.clamp(MIN_SIZE as u32, MAX_SIZE as u32),
        );
        self.guarded(|| {
            let page = {
                let mut state = self.state();
                state.fits.insert(thread_id.to_owned(), wanted);
                state.last_fit = Some(wanted);
                state.pages.get(thread_id).map(|open| open.page.clone())
            };
            let mut page = page.as_deref().map(lock);
            if let Some(page) = page.as_deref_mut() {
                self.resize_page(thread_id, page, wanted)?;
            }
            self.announce(thread_id, page.as_deref(), false, None, false);
            Ok(())
        })
    }

    /// The screen showing the thread's page has `scale` device pixels per pixel.
    pub(crate) fn user_scale(&self, thread_id: &str, scale: f64) -> Result<(), String> {
        if !scale.is_finite() || scale <= 0.0 {
            return Ok(());
        }
        self.guarded(|| {
            let (page, viewing) = {
                let mut state = self.state();
                state.scales.insert(thread_id.to_owned(), scale);
                let page = state.pages.get(thread_id).map(|open| open.page.clone());
                (page, state.viewing.contains(thread_id))
            };
            let Some(page) = page else {
                return Ok(());
            };
            let mut page = lock(&page);
            page.set_scale(scale)?;
            if viewing {
                page.stop_screencast();
                page.start_screencast()?;
            }
            Ok(())
        })
    }

    /// Runs `act`, dropping a browser that died before it and after a failure.
    fn guarded<T>(&self, act: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        self.drop_dead_browser();
        let result = act();
        if result.is_err() {
            self.drop_dead_browser();
        }
        result
    }

    /// A browser that quit or crashed goes with its pages, so the next page starts a new one.
    fn drop_dead_browser(&self) {
        let (browser, closed) = {
            let mut state = self.state();
            if state.browser.as_deref().is_none_or(Browser::is_connected) {
                return;
            }
            state.idle_since = None;
            let closed: Vec<String> = state
                .pages
                .drain()
                .map(|(thread_id, _)| thread_id)
                .collect();
            (state.browser.take(), closed)
        };
        drop(browser);
        for thread_id in closed {
            self.announce(&thread_id, None, false, None, false);
        }
    }

    /// Closes every page and the browser, as when the app quits.
    pub(crate) fn close(&self) {
        let (pages, browser) = {
            let mut state = self.state();
            state.idle_since = None;
            (std::mem::take(&mut state.pages), state.browser.take())
        };
        drop(pages);
        drop(browser);
    }

    fn resize_page(
        &self,
        thread_id: &str,
        page: &mut Page,
        wanted: (u32, u32),
    ) -> Result<(), String> {
        if page.size() == wanted {
            return Ok(());
        }
        page.resize(wanted)?;
        if self.state().viewing.contains(thread_id) {
            page.stop_screencast();
            page.start_screencast()?;
        }
        Ok(())
    }

    /// What the page shows and where it went reach the UI as notifications.
    fn observer(&self, thread_id: &str) -> Observer {
        let (notify, thread_id) = (self.notify.clone(), thread_id.to_owned());
        let frames = {
            let notify = notify.clone();
            Throttle::new(
                FRAME_EVERY,
                |shown, frame| *shown = frame,
                move |frame| {
                    notify(Notification::BrowserFrame(frame));
                },
            )
        };
        Arc::new(move |event| {
            let notification = match event {
                PageEvent::Frame {
                    jpeg,
                    width,
                    height,
                } => {
                    frames.offer(BrowserFrame {
                        thread_id: thread_id.clone(),
                        jpeg,
                        width,
                        height,
                    });
                    return;
                }
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

    /// Opens the thread's page if it has none, streaming it when the panel shows it. Gives
    /// the size it opened at, or `None` when it was open.
    fn ensure_page(
        &self,
        thread_id: &str,
        wanted: (u32, u32),
    ) -> Result<Option<(u32, u32)>, String> {
        let scale = {
            let state = self.state();
            if state.pages.contains_key(thread_id) {
                return Ok(None);
            }
            state.scales.get(thread_id).copied()
        };
        let observer = self.observer(thread_id);
        let settings = self.settings();
        let browser = self.browser()?;
        let mut page = browser.open_page(wanted, observer)?;
        let configured = (|| {
            if let Some(scale) = scale {
                page.set_scale(scale)?;
            }
            if settings.zoom != 100 {
                page.set_zoom(f64::from(settings.zoom) / 100.0)?;
            }
            match settings.color_scheme {
                PageColorScheme::System => Ok(()),
                PageColorScheme::Light => page.set_color_scheme(Some("light")),
                PageColorScheme::Dark => page.set_color_scheme(Some("dark")),
            }
        })();
        if let Err(error) = configured {
            page.close();
            return Err(error);
        }
        let size = page.size();
        let target_id = page.target_id.clone();
        let page = Arc::new(Mutex::new(page));
        let viewing = {
            let mut state = self.state();
            if state.pages.contains_key(thread_id) {
                drop(state);
                browser.close_target(&target_id);
                return Ok(None);
            }
            state.pages.insert(
                thread_id.to_owned(),
                OpenPage {
                    page: page.clone(),
                    target_id,
                },
            );
            state.closed_by_user.remove(thread_id);
            state.idle_since = None;
            state.viewing.contains(thread_id)
        };
        if viewing {
            lock(&page).start_screencast()?;
        }
        Ok(Some(size))
    }

    /// Tells the UI where the page is now; `by_agent` when the agent touched it.
    fn announce(
        &self,
        thread_id: &str,
        page: Option<&Page>,
        by_agent: bool,
        agent_point: Option<(f64, f64)>,
        opened: bool,
    ) {
        let fitted = self.state().fits.contains_key(thread_id);
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
            fitted: Some(fitted),
        }));
    }

    /// The thread's page if it has one, which only the caller then uses.
    fn page_if_open(&self, thread_id: &str) -> Option<Arc<Mutex<Page>>> {
        self.state()
            .pages
            .get(thread_id)
            .map(|open| open.page.clone())
    }

    /// The panel shows the thread's page, or stopped showing it.
    pub(crate) fn view(&self, thread_id: &str, viewing: bool) -> Result<(), String> {
        self.guarded(|| {
            let page = {
                let mut state = self.state();
                if viewing {
                    state.viewing.insert(thread_id.to_owned());
                } else {
                    state.viewing.remove(thread_id);
                }
                state.pages.get(thread_id).map(|open| open.page.clone())
            };
            let page = page.as_deref().map(lock);
            if let Some(page) = page.as_deref() {
                if viewing {
                    page.start_screencast()?;
                } else {
                    page.stop_screencast();
                }
            }
            self.announce(thread_id, page.as_deref(), false, None, false);
            Ok(())
        })
    }

    /// The user opens an address from the panel, with a page of their own when the thread
    /// had none.
    pub(crate) fn user_open(&self, thread_id: &str, url: &str) -> Result<(), String> {
        self.guarded(|| {
            if let Some(size) = self.ensure_page(thread_id, self.opening_size(thread_id))? {
                self.fill_when_default(thread_id, size);
            }
            let target = address_or_search(url, &self.settings());
            let page = self.page(thread_id)?;
            let mut page = lock(&page);
            page.navigate(&target, LOAD_TIMEOUT)?;
            self.announce(thread_id, Some(&page), false, None, false);
            Ok(())
        })
    }

    /// Back (`-1`), forward (`1`) or reload (`0`).
    pub(crate) fn user_go(&self, thread_id: &str, delta: i32) -> Result<(), String> {
        self.guarded(|| {
            let page = self.page(thread_id)?;
            let mut page = lock(&page);
            if delta == 0 {
                page.reload(LOAD_TIMEOUT)?;
            } else {
                page.go(delta, LOAD_TIMEOUT)?;
            }
            self.announce(thread_id, Some(&page), false, None, false);
            Ok(())
        })
    }

    /// The user picked a size, which stops the page filling the panel.
    pub(crate) fn user_resize(&self, thread_id: &str, wanted: (u32, u32)) -> Result<(), String> {
        self.guarded(|| {
            self.state().fits.remove(thread_id);
            let page = self.page(thread_id)?;
            let mut page = lock(&page);
            self.resize_page(thread_id, &mut page, wanted)?;
            self.announce(thread_id, Some(&page), false, None, false);
            Ok(())
        })
    }

    pub(crate) fn user_input(&self, thread_id: &str, input: &BrowserInput) -> Result<(), String> {
        self.guarded(|| {
            let page = self.page(thread_id)?;
            let page = lock(&page);
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

    /// Runs `expression` in the thread's open page and gives its value.
    pub(crate) fn user_evaluate(&self, thread_id: &str, expression: &str) -> Result<Value, String> {
        self.guarded(|| {
            let page = self.page(thread_id)?;
            let page = lock(&page);
            page.evaluate(expression)
        })
    }

    /// The running browser, started once however many pages ask for it at a time.
    fn browser(&self) -> Result<Arc<Browser>, String> {
        let _launching = lock(&self.launching);
        if let Some(browser) = self.state().browser.clone() {
            return Ok(browser);
        }
        let executable = find_browser().ok_or(
            "no Chrome or Edge is installed here, so there is no browser to open pages in",
        )?;
        let browser = Arc::new(Browser::start(&executable, &self.profile)?);
        self.state().browser = Some(browser.clone());
        Ok(browser)
    }

    fn page(&self, thread_id: &str) -> Result<Arc<Mutex<Page>>, String> {
        let state = self.state();
        match state.pages.get(thread_id) {
            Some(open) => Ok(open.page.clone()),
            None if state.closed_by_user.contains(thread_id) => Err(CLOSED_BY_USER.into()),
            None => Err(NO_PAGE.into()),
        }
    }

    /// Closes the thread's page, even while it is busy. The browser stays a while after its
    /// last page.
    pub(crate) fn forget(&self, thread_id: &str) {
        let closing = {
            let mut state = self.state();
            let closing = state.pages.remove(thread_id).map(|open| open.target_id);
            state.viewing.remove(thread_id);
            state.fits.remove(thread_id);
            state.closed_by_user.remove(thread_id);
            if state.pages.is_empty() && state.browser.is_some() && state.idle_since.is_none() {
                state.idle_since = Some(Instant::now());
            }
            closing.zip(state.browser.clone())
        };
        if let Some((target_id, browser)) = closing {
            browser.close_target(&target_id);
        }
        self.announce(thread_id, None, false, None, false);
    }

    /// The user closed the thread's page from the panel, which its agent is told the next
    /// time it reaches for it.
    pub(crate) fn user_close(&self, thread_id: &str) {
        let open = self.state().pages.contains_key(thread_id);
        self.forget(thread_id);
        if open {
            self.state().closed_by_user.insert(thread_id.to_owned());
        }
    }

    /// Ends a browser that has had no page for a while.
    pub(crate) fn end_idle_browser(&self) {
        let ended = {
            let mut state = self.state();
            let idle = state
                .idle_since
                .is_some_and(|since| since.elapsed() >= IDLE_BROWSER);
            if !idle || !state.pages.is_empty() {
                return;
            }
            state.idle_since = None;
            state.browser.take()
        };
        drop(ended);
    }

    pub(crate) fn call(
        &self,
        caller: &str,
        tool: &str,
        arguments: &Value,
    ) -> Result<Value, String> {
        if !self.settings().agent_access {
            return Err(NO_AGENT_ACCESS.into());
        }
        self.drop_dead_browser();
        let point = self.agent_point(caller, tool, arguments);
        let result = self.dispatch(caller, tool, arguments);
        if result.is_err() {
            self.drop_dead_browser();
        }
        if result.is_ok() && !matches!(tool, "preview_close" | "preview_status") {
            let opened = tool == "preview_open"
                && result.as_ref().is_ok_and(|value| value["opened"] == true);
            let page = self.page_if_open(caller);
            let page = page.as_deref().map(lock);
            self.announce(caller, page.as_deref(), true, point, opened);
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
        let page = self.page_if_open(caller)?;
        let page = lock(&page);
        page.point_of(text(arguments, "ref")?).ok()
    }

    fn dispatch(&self, caller: &str, tool: &str, arguments: &Value) -> Result<Value, String> {
        match tool {
            "preview_open" => self.open(caller, arguments),
            "preview_close" => {
                let open = self.state().pages.contains_key(caller);
                self.forget(caller);
                Ok(json!({ "closed": open }))
            }
            "preview_status" => self.status(caller),
            _ => self.act_on(caller, tool, arguments),
        }
    }

    fn open(&self, caller: &str, arguments: &Value) -> Result<Value, String> {
        let url = address(text(arguments, "url").ok_or("`url` is required")?);
        // A size the agent asks for wins over the panel's.
        let sized = !arguments["width"].is_null() || !arguments["height"].is_null();
        if sized {
            self.state().fits.remove(caller);
        }
        let opened = self.ensure_page(caller, size(arguments, self.opening_size(caller)))?;
        if let Some(size) = opened.filter(|_| !sized) {
            self.fill_when_default(caller, size);
        }
        let page = self.page(caller)?;
        let mut page = lock(&page);
        if sized {
            let wanted = size(arguments, page.size());
            self.resize_page(caller, &mut page, wanted)?;
        }
        page.navigate(&url, LOAD_TIMEOUT)?;
        let mut result = describe(&page)?;
        result["opened"] = json!(opened.is_some());
        Ok(result)
    }

    fn status(&self, caller: &str) -> Result<Value, String> {
        let Some(page) = self.page_if_open(caller) else {
            let closed_by_user = self.state().closed_by_user.contains(caller);
            return Ok(json!({ "open": false, "closedByUser": closed_by_user }));
        };
        let page = lock(&page);
        let mut status = describe(&page)?;
        status["open"] = json!(true);
        status["console"] = json!(page.console(CONSOLE_LINES));
        Ok(status)
    }

    fn act_on(&self, caller: &str, tool: &str, arguments: &Value) -> Result<Value, String> {
        let page = self.page(caller)?;
        let mut page = lock(&page);
        let page = &mut *page;
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
                self.state().fits.remove(caller);
                self.resize_page(caller, page, wanted)?;
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
mod tests;
