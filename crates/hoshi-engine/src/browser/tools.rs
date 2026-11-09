//! The `preview_*` tools: each thread's agent drives a page of its own in the headless
//! browser, started on first use.

use super::{Browser, DEFAULT_SIZE, Page, find_browser};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

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
}

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
    pub(crate) fn new(profile: PathBuf) -> Self {
        Self {
            profile,
            browser: None,
            pages: HashMap::new(),
        }
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
        self.pages
            .get_mut(thread_id)
            .ok_or_else(|| "this thread has no page open: call preview_open first".into())
    }

    /// Closes the page of a thread that is gone.
    pub(crate) fn forget(&mut self, thread_id: &str) {
        if let Some(page) = self.pages.remove(thread_id) {
            page.close();
        }
        if self.pages.is_empty() {
            self.browser = None;
        }
    }

    pub(crate) fn call(
        &mut self,
        caller: &str,
        tool: &str,
        arguments: &Value,
    ) -> Result<Value, String> {
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
        if !self.pages.contains_key(caller) {
            let page = self.browser()?.open_page(size(arguments, DEFAULT_SIZE))?;
            self.pages.insert(caller.to_owned(), page);
        }
        let page = self.page(caller)?;
        let wanted = size(arguments, page.size());
        if wanted != page.size() {
            page.resize(wanted)?;
        }
        page.navigate(&url, LOAD_TIMEOUT)?;
        describe(page)
    }

    fn status(&mut self, caller: &str) -> Result<Value, String> {
        let Some(page) = self.pages.get(caller) else {
            return Ok(json!({ "open": false }));
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
                page.resize(wanted)?;
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
        let mut browsing = Browsing::new(profile.clone());
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
    fn bare_addresses_get_a_scheme() {
        assert_eq!(address("localhost:5173"), "http://localhost:5173");
        assert_eq!(address("127.0.0.1:3000/x"), "http://127.0.0.1:3000/x");
        assert_eq!(address("example.com"), "https://example.com");
        assert_eq!(address("file:///tmp/a.html"), "file:///tmp/a.html");
        assert_eq!(address("about:blank"), "about:blank");
    }
}
