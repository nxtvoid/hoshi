//! What agents and the user do on a page. An agent's action on an element first checks that a
//! person could do it: that the element is the one it read, shown, enabled and not covered.

use super::keys::{self, Chord, Key};
use super::refs::{Lost, Tree};
use super::{Page, lock, script_result};
use serde_json::{Value, json};
use std::thread;
use std::time::{Duration, Instant};

/// How long an element has to become one a person could act on.
const ACTIONABLE_TIMEOUT: Duration = Duration::from_secs(2);
const ACTIONABLE_POLL: Duration = Duration::from_millis(40);
/// Looking an element up again reads the whole page.
const FIND_POLL: Duration = Duration::from_millis(200);
/// How far an element may move between two looks and still be at rest, in CSS pixels.
const AT_REST: f64 = 0.5;
/// A page is settled once nothing changed for this long, in milliseconds.
const QUIET_MS: u64 = 100;
/// How long a page that keeps changing, or that went elsewhere, is waited for.
const MAX_SETTLE: Duration = Duration::from_millis(600);
const MAX_LOAD: Duration = Duration::from_secs(5);
/// Where an element is scrolled to in turn while something covers it, as a sticky header.
const ALIGNMENTS: [&str; 3] = ["center", "end", "start"];
/// Longer text is put in at once rather than key by key.
const MAX_TYPED: usize = 64;

/// Where to point at an element, or why nobody could: `{ x, y }` or `{ why, by, x, y }`, with
/// where it would be.
/// `align` scrolls it there first, `enabled` refuses a disabled one, and `any` takes
/// the place whatever covers it.
const AIM: &str = r#"function (align, enabled, any) {
  const el = this.nodeType === 9 ? this.documentElement : this;
  if (!el.isConnected) return { why: 'gone' };
  const CONTROLS = 'a[href],button,input,select,textarea,summary,[role=button],[role=link],[role=checkbox],[role=radio],[role=switch],[role=tab],[role=option],[role=combobox],[role^=menuitem]';
  const FIELDS = 'input,textarea,select,[role=checkbox],[role=radio],[role=switch],[role=combobox],[role=slider],[role=textbox]';
  const control = el.closest('button,input,select,textarea,option');
  const said = (el.closest(CONTROLS) ?? el).getAttribute('aria-disabled');
  if (enabled && (control?.matches(':disabled') || said === 'true')) return { why: 'disabled' };
  if (align) el.scrollIntoView({ block: align, inline: align, behavior: 'instant' });
  const shown = (boxes) => [...boxes].filter((box) => box.width > 0 && box.height > 0);
  // One without a box of its own shows what it holds.
  const boxes = (node) => {
    const own = shown(node.getClientRects());
    if (own.length || !node.firstChild) return own;
    const held = document.createRange();
    held.selectNodeContents(node);
    return shown(held.getClientRects());
  };
  // The middle first, then places around it, as something may cover only a part.
  const places = (box) => {
    const left = Math.max(box.left, 0), right = Math.min(box.right, innerWidth);
    const top = Math.max(box.top, 0), bottom = Math.min(box.bottom, innerHeight);
    if (right <= left || bottom <= top) return [];
    return [[0.5, 0.5], [0.2, 0.5], [0.8, 0.5], [0.5, 0.2], [0.5, 0.8], [0.05, 0.5], [0.95, 0.5], [0.5, 0.05], [0.5, 0.95], [0.1, 0.1], [0.9, 0.9]].map(([x, y]) => ({ x: left + (right - left) * x, y: top + (bottom - top) * y }));
  };
  const hitAt = ({ x, y }) => {
    let hit = null;
    for (let root = document; root; root = hit.shadowRoot) {
      const next = root.elementFromPoint(x, y);
      if (!next || next === hit) return hit;
      hit = next;
    }
    return hit;
  };
  const within = (node, holder) => {
    for (; node; node = node.assignedSlot ?? node.parentNode ?? node.host) if (node === holder) return true;
    return false;
  };
  const acts = el.matches(CONTROLS);
  const field = el.matches(FIELDS);
  const viaLabel = (hit, label) => {
    const other = hit.closest(CONTROLS);
    return within(hit, label) && (!other || other === el || !within(other, label));
  };
  const takes = (hit) => {
    if (within(hit, el)) return true;
    // What a shadow root's slot or a closed one hides from the page counts as its host. The
    // browser's own roots, as of a date, crash the page when asked their mode.
    for (let root = el.getRootNode(); root instanceof ShadowRoot; root = root.host.getRootNode()) {
      if (root.host === hit && (!hit.shadowRoot || root === el.getRootNode())) return true;
    }
    const label = hit.closest('label');
    if (label && label.control === el && viaLabel(hit, label)) return true;
    const apart = (node) => node && node !== document.body && node !== document.documentElement;
    // Fields are often drawn by what holds them, or by what lies over them next to them.
    if (field && apart(el.parentElement) && within(hit, el.parentElement)) return true;
    if (within(el, hit)) return apart(hit) && (!acts || getComputedStyle(hit).cursor === 'pointer');
    const around = el.closest('a[href],button,summary,[role=button],[role=link],[role=option],[role=tab],[role^=menuitem]');
    if (around && within(hit, around)) return true;
    if (acts) return false;
    // What is stretched over a whole card takes its clicks.
    let over = hit.closest('a[href],button,[role=button],[role=link]');
    if (!over && getComputedStyle(hit).cursor === 'pointer') over = hit;
    if (!over || getComputedStyle(over).position === 'fixed') return false;
    const card = over.offsetParent ?? over.parentElement;
    return apart(card) && within(el, card) && boxes(el).flatMap(places).every((place) => within(hitAt(place), over));
  };
  const own = boxes(el);
  const visible = getComputedStyle(el).visibility === 'visible';
  let cover = null, first = null;
  for (const box of own) {
    for (const place of places(box)) {
      first ??= place;
      if (any) return place;
      const hit = visible ? hitAt(place) : null;
      if (hit && takes(hit)) return place;
      cover ??= hit;
    }
  }
  if (any) {
    const box = el.getBoundingClientRect();
    return { x: Math.min(Math.max(box.left + box.width / 2, 0), innerWidth - 1), y: Math.min(Math.max(box.top + box.height / 2, 0), innerHeight - 1) };
  }
  for (const label of el.labels ?? []) {
    for (const place of boxes(label).flatMap(places)) {
      const hit = hitAt(place);
      if (hit && viaLabel(hit, label)) return place;
    }
  }
  // One too small to see stands for what holds it.
  if (field && !own.some((box) => box.width >= 4 && box.height >= 4)) {
    let holder = el.parentElement;
    for (let up = 0; up < 3 && holder && holder !== document.body; up++, holder = holder.parentElement) {
      const large = boxes(holder).filter((box) => box.width >= 8 && box.height >= 8);
      for (const place of large.flatMap(places)) {
        const hit = hitAt(place);
        if (hit && viaLabel(hit, holder)) return place;
      }
      if (large.length) break;
    }
  }
  if (!own.length) return { why: el.localName === 'option' ? 'option' : 'hidden' };
  if (!visible) return { why: 'hidden', ...first };
  if (!cover) return { why: 'outside' };
  const words = (cover.getAttribute('aria-label') ?? cover.innerText ?? '').replace(/\s+/g, ' ').trim().slice(0, 60);
  const classes = typeof cover.className === 'string' && cover.className.trim() ? '.' + cover.className.trim().split(/\s+/).slice(0, 2).join('.') : '';
  const by = '<' + cover.localName + (cover.id ? '#' + cover.id : '') + classes + '>' + (words ? ' ' + JSON.stringify(words) : '');
  return { why: within(el, cover) ? 'passed' : 'covered', by, ...first };
}"#;

/// Focuses the element, or a label's control, to type into: `{ why }` when it takes no text,
/// else whether its value is picked rather than typed, whether it holds anything and whether
/// it takes keys.
const FOCUS: &str = r#"function (clear) {
  const el = this.nodeType === 9 ? this.documentElement : this;
  if (!el.isConnected) return { why: 'gone' };
  const typed = el.matches('input,textarea') || el.isContentEditable;
  const into = typed ? el : el.closest('label')?.control ?? el;
  if (into.matches(':disabled')) return { why: 'disabled' };
  if (into.readOnly) return { why: 'read-only' };
  // Asked of its own root, as a closed one shows only its host outside.
  const focused = () => {
    let active = into.getRootNode().activeElement ?? document.activeElement;
    while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement;
    return active;
  };
  const within = (node) => {
    for (; node; node = node.parentNode ?? node.host) if (node === into) return true;
    return false;
  };
  const before = focused();
  into.focus();
  const after = focused();
  if (after === before && !within(after)) return { why: 'unfocusable' };
  const picked = into.localName === 'input' && /^(color|date|datetime-local|month|range|time|week)$/.test(into.type);
  let holds = false;
  if (clear && !picked) {
    if (into.localName === 'input' || into.localName === 'textarea') {
      holds = into.value !== '';
      if (holds) into.select();
    } else if (into.isContentEditable) {
      holds = into.textContent !== '' || into.childElementCount > 0;
      if (holds) getSelection().selectAllChildren(into);
    }
  }
  const keyed = into.localName === 'textarea' || (into.localName === 'input' && !/^(button|checkbox|file|hidden|image|radio|reset|submit)$/.test(into.type));
  return { picked, holds, keyed };
}"#;

/// Empties a field that isn't after its text was selected and deleted, as one that moves its
/// caret when focused.
const EMPTY: &str = r#"function () {
  const el = this.nodeType === 9 ? this.documentElement : this;
  const field = el.matches('input,textarea') ? el : el.closest('label')?.control ?? el;
  if (typeof field.value !== 'string' || field.value === '' || field.isContentEditable) return;
  field.select?.();
  document.execCommand('delete');
}"#;

/// Sets the value of an input whose value is picked, as a date; `false` when it can't have it.
const PICK: &str = r#"function (value) {
  const el = this.localName === 'input' ? this : this.closest('label')?.control ?? this;
  const before = el.value;
  const set = (value) => Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(el, value);
  if (el.type === 'datetime-local') value = value.replace(' ', 'T');
  if (el.type === 'range' && (value === '' || Number.isNaN(Number(value)))) return false;
  set(value);
  if (el.type !== 'range' && !el.value.toLowerCase().startsWith(value.toLowerCase()) || (value !== '' && el.value === '')) {
    set(before);
    return false;
  }
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return true;
}"#;

/// Picks the options of a `<select>` whose value or text is among `values`: `{ why }` when
/// there is none, with what it has.
const SELECT: &str = r#"function (values) {
  const el = this.localName === 'select' ? this : this.closest('label')?.control ?? this;
  if (el.localName !== 'select') return { why: 'other' };
  if (el.matches(':disabled')) return { why: 'disabled' };
  const options = [...el.options];
  const plain = (text) => text.replace(/\s+/g, ' ').trim();
  const named = (value) => {
    const byValue = options.filter((option) => option.value === value);
    return byValue.length ? byValue : options.filter((option) => [option.text, option.label].some((text) => plain(text) === plain(value)));
  };
  const missing = values.filter((value) => !named(value).length);
  if (missing.length) return { why: 'missing', missing, has: options.slice(0, 20).map((option) => option.label) };
  const found = values.flatMap(named);
  if (found.some((option) => option.disabled || option.parentElement.disabled)) return { why: 'disabled' };
  const chosen = el.multiple ? found : found.slice(0, 1);
  for (const option of options) option.selected = chosen.includes(option);
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return {};
}"#;

impl Page {
    /// The agent is about to act.
    pub fn begin_act(&mut self) {
        self.acted = false;
    }

    /// What follows reaches the page, unless a dialog holds it.
    fn reaches_page(&mut self) {
        self.acted = self.dialog_says().is_none();
    }

    /// Whether the agent's action reached the page.
    pub fn has_acted(&self) -> bool {
        self.acted
    }

    /// Where the agent last acted on an element, once.
    pub fn take_acted_at(&mut self) -> Option<(f64, f64)> {
        self.acted_at.take()
    }

    /// The node the ref names now.
    fn resolve(&mut self, reference: &str) -> Result<i64, Lost> {
        let changed = Lost::Changed;
        let place = self.place().map_err(changed)?;
        let alone = self.refs.find_alone(reference, &place, |node| {
            let mut read = self
                .call(
                    "Accessibility.getPartialAXTree",
                    json!({ "backendNodeId": node, "fetchRelatives": false }),
                )
                .ok()?;
            Some(read["nodes"][0].take())
        })?;
        if let Some(node) = alone {
            return Ok(node);
        }
        let nodes = self.nodes().map_err(changed)?;
        self.refs.find(reference, &Tree::new(&nodes), &place)
    }

    fn call_on(&self, node: i64, function: &str, arguments: Value) -> Result<Value, String> {
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
        );
        let _ = self.call("Runtime.releaseObject", json!({ "objectId": object }));
        script_result(&result?)
    }

    /// Where to point at what the ref names, once a person could.
    fn aim(&mut self, reference: &str, enabled: bool) -> Result<(f64, f64), String> {
        let deadline = Instant::now() + ACTIONABLE_TIMEOUT;
        let mut node = None;
        let mut last = None;
        let mut align = None;
        let mut alignments = ALIGNMENTS.iter().cycle();
        let mut pointed = false;
        loop {
            let late = Instant::now() >= deadline;
            let current = match node {
                Some(node) => node,
                None => match self.resolve(reference) {
                    Ok(found) => {
                        node = Some(found);
                        let _ = self.call(
                            "DOM.scrollIntoViewIfNeeded",
                            json!({ "backendNodeId": found }),
                        );
                        found
                    }
                    Err(Lost::Missing(_)) if !late => {
                        thread::sleep(FIND_POLL);
                        continue;
                    }
                    Err(lost) => return Err(lost.message()),
                },
            };
            let seen = self.call_on(
                current,
                AIM,
                json!([{ "value": align }, { "value": enabled }, { "value": false }]),
            );
            align = None;
            let problem = match seen {
                Ok(seen) if seen["why"].is_null() => {
                    let point = (
                        seen["x"].as_f64().unwrap_or_default(),
                        seen["y"].as_f64().unwrap_or_default(),
                    );
                    let rests = last.is_some_and(|(x, y): (f64, f64)| {
                        (x - point.0).abs() < AT_REST && (y - point.1).abs() < AT_REST
                    });
                    if rests || late {
                        return Ok(point);
                    }
                    last = Some(point);
                    thread::sleep(ACTIONABLE_POLL);
                    continue;
                }
                Ok(seen) => seen,
                Err(_) if self.dialog_says().is_none() => json!({ "why": "gone" }),
                Err(error) => return Err(error),
            };
            last = None;
            let why = problem["why"].as_str().unwrap_or_default();
            if why == "gone" {
                node = None;
            }
            if matches!(why, "covered" | "outside") {
                align = alignments.next();
            }
            if late {
                return Err(refusal(reference, why, &problem));
            }
            // Some only show, or take clicks, once the pointer is over what holds them.
            if !pointed
                && matches!(why, "hidden" | "covered" | "passed")
                && let (Some(x), Some(y)) = (problem["x"].as_f64(), problem["y"].as_f64())
            {
                pointed = true;
                self.mouse("mouseMoved", (x, y), json!({}))?;
            }
            thread::sleep(ACTIONABLE_POLL);
        }
    }

    /// The node the ref names, and the place to show the user the agent acts at.
    fn reach(&mut self, reference: &str) -> Result<i64, String> {
        let deadline = Instant::now() + ACTIONABLE_TIMEOUT;
        let node = loop {
            match self.resolve(reference) {
                Ok(node) => break node,
                Err(Lost::Missing(_)) if Instant::now() < deadline => thread::sleep(FIND_POLL),
                Err(lost) => return Err(lost.message()),
            }
        };
        let _ = self.call(
            "DOM.scrollIntoViewIfNeeded",
            json!({ "backendNodeId": node }),
        );
        let seen = self.call_on(
            node,
            AIM,
            json!([{ "value": null }, { "value": false }, { "value": true }]),
        );
        self.acted_at = seen
            .ok()
            .and_then(|seen| Some((seen["x"].as_f64()?, seen["y"].as_f64()?)));
        Ok(node)
    }

    /// What the dialog that holds the page says, and how to answer it.
    pub fn dialog_says(&self) -> Option<String> {
        lock(&self.dialog).as_ref().map(super::Dialog::says)
    }

    fn mouse(&self, kind: &str, (x, y): (f64, f64), extra: Value) -> Result<(), String> {
        let mut params = json!({ "type": kind, "x": x, "y": y });
        if let (Some(params), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
            params.extend(extra.clone());
        }
        self.call("Input.dispatchMouseEvent", params).map(|_| ())
    }

    pub fn click(&mut self, reference: &str) -> Result<(), String> {
        let point = self.aim(reference, true)?;
        self.click_point(point)
    }

    /// Clicks `point` for the agent.
    pub fn click_point(&mut self, point: (f64, f64)) -> Result<(), String> {
        self.acted_at = Some(point);
        self.mouse("mouseMoved", point, json!({}))?;
        self.reaches_page();
        self.click_at(point)
    }

    pub fn click_at(&self, point: (f64, f64)) -> Result<(), String> {
        let press = json!({ "button": "left", "clickCount": 1 });
        self.mouse("mouseMoved", point, json!({}))?;
        self.mouse("mousePressed", point, press.clone())?;
        self.mouse("mouseReleased", point, press)
    }

    pub fn hover(&mut self, reference: &str) -> Result<(), String> {
        let point = self.aim(reference, false)?;
        self.acted_at = Some(point);
        self.reaches_page();
        self.mouse("mouseMoved", point, json!({}))
    }

    /// Scrolls by `delta` pixels, at the element when given, else at the page's center.
    pub fn scroll(&mut self, reference: Option<&str>, delta: (f64, f64)) -> Result<(), String> {
        let point = match reference {
            Some(reference) => {
                self.reach(reference)?;
                self.acted_at.ok_or("the element isn't shown")?
            }
            None => (f64::from(self.size.0) / 2.0, f64::from(self.size.1) / 2.0),
        };
        self.reaches_page();
        self.scroll_at(point, delta)
    }

    pub fn scroll_at(&self, point: (f64, f64), delta: (f64, f64)) -> Result<(), String> {
        self.mouse(
            "mouseWheel",
            point,
            json!({ "deltaX": delta.0, "deltaY": delta.1 }),
        )
    }

    /// Types `text` into the element, after emptying it when `clear`.
    pub fn type_text(&mut self, reference: &str, text: &str, clear: bool) -> Result<(), String> {
        let deadline = Instant::now() + ACTIONABLE_TIMEOUT;
        let (node, into) = loop {
            let node = self.reach(reference)?;
            let into = self.call_on(node, FOCUS, json!([{ "value": clear }]))?;
            match into["why"].as_str() {
                None => break (node, into),
                Some(_) if Instant::now() < deadline => thread::sleep(FIND_POLL),
                Some(why) => return Err(refusal(reference, why, &into)),
            }
        };
        if into["picked"] == true {
            self.reaches_page();
            let taken = self.call_on(node, PICK, json!([{ "value": text }]))?;
            return if taken == true {
                Ok(())
            } else {
                Err(format!(
                    "{reference} doesn't take {text:?}: give its value, such as 2026-01-31 for a date, 14:30 for a time or #ff8800 for a color"
                ))
            };
        }
        if into["holds"] == true {
            self.press_key("Backspace")?;
            self.call_on(node, EMPTY, json!([]))?;
        }
        if into["keyed"] == true {
            self.type_keys(text)?;
        } else {
            self.insert_text(text)?;
        }
        self.acted = true;
        Ok(())
    }

    /// Types `text` key by key, for pages that follow the keys, where that is quick; line
    /// breaks stay text, as Enter may send a form.
    fn type_keys(&self, text: &str) -> Result<(), String> {
        let count = text.chars().count();
        if text.contains(['\n', '\r']) || count > MAX_TYPED {
            return self.insert_text(text);
        }
        for (typed, character) in text.chars().enumerate() {
            match keys::typed(character) {
                Some(key) => self
                    .key("keyDown", &key, 0)
                    .and_then(|()| self.key("keyUp", &key, 0)),
                None => self.insert_text(&character.to_string()),
            }
            .map_err(|error| format!("typed {typed} of {count} characters: {error}"))?;
        }
        Ok(())
    }

    /// Types `text` where the focus is.
    pub fn insert_text(&self, text: &str) -> Result<(), String> {
        if text.is_empty() {
            return Ok(());
        }
        self.call("Input.insertText", json!({ "text": text }))
            .map(|_| ())
    }

    /// Picks the options of a `<select>` whose value or text is among `values`.
    pub fn select(&mut self, reference: &str, values: &[String]) -> Result<(), String> {
        let deadline = Instant::now() + ACTIONABLE_TIMEOUT;
        loop {
            let node = self.reach(reference)?;
            self.reaches_page();
            let answer = self.call_on(node, SELECT, json!([{ "value": values }]))?;
            match answer["why"].as_str() {
                None => return Ok(()),
                Some("disabled" | "missing") if Instant::now() < deadline => {
                    thread::sleep(FIND_POLL);
                }
                Some(why) => return Err(refusal(reference, why, &answer)),
            }
        }
    }

    fn key(&self, kind: &str, key: &Key, modifiers: u8) -> Result<(), String> {
        let mut params = json!({
            "type": kind,
            "key": key.key,
            "code": key.code,
            "windowsVirtualKeyCode": key.number,
            "modifiers": modifiers,
        });
        if let Some(text) = key.text.as_ref().filter(|_| kind == "keyDown") {
            params["text"] = json!(text);
        }
        self.call("Input.dispatchKeyEvent", params).map(|_| ())
    }

    /// Presses a key such as `Enter`, `Tab` or a single character, or several together as
    /// `Control+a`. Enter and Escape answer a dialog the page shows.
    pub fn press(&self, text: &str) -> Result<(), String> {
        if matches!(text, "Enter" | "Escape") && self.answer_dialog(text == "Enter")? {
            return Ok(());
        }
        self.press_key(text)
    }

    /// Presses for the agent.
    pub fn press_as_agent(&mut self, text: &str) -> Result<(), String> {
        self.reaches_page();
        self.acted |= matches!(text, "Enter" | "Escape");
        self.press(text)
    }

    fn press_key(&self, text: &str) -> Result<(), String> {
        let Chord { held, key } =
            keys::chord(text).ok_or_else(|| format!("hoshi can't press {text}"))?;
        let mut modifiers = 0;
        for (bit, modifier) in &held {
            modifiers |= bit;
            self.key("keyDown", modifier, modifiers)?;
        }
        let pressed = self
            .key("keyDown", &key, modifiers)
            .and_then(|()| self.key("keyUp", &key, modifiers));
        for (bit, modifier) in held.iter().rev() {
            modifiers &= !bit;
            self.key("keyUp", modifier, modifiers)?;
        }
        pressed
    }

    /// Waits for what an action started to show: until the page stops changing, or loads when
    /// it went elsewhere.
    pub fn settle(&self) {
        let quiet = format!(
            "new Promise((done) => {{ let timer; const end = () => {{ observer.disconnect(); clearTimeout(timer); clearTimeout(most); done(true); }}; const wait = () => {{ clearTimeout(timer); timer = setTimeout(end, {QUIET_MS}); }}; const observer = new MutationObserver((changes) => {{ if (changes.some((change) => change.attributeName !== 'style')) wait(); }}); observer.observe(document, {{ subtree: true, childList: true, characterData: true, attributes: true }}); const most = setTimeout(end, {}); wait(); }})",
            MAX_SETTLE.as_millis()
        );
        let mut params =
            json!({ "expression": quiet, "returnByValue": true, "awaitPromise": true });
        // Apart from the page's own script, which may have put in timers of its own.
        let world = self.call_within(
            "Page.createIsolatedWorld",
            json!({ "frameId": self.target_id, "worldName": "hoshi" }),
            MAX_SETTLE,
        );
        if let Ok(world) = world {
            params["contextId"] = world["executionContextId"].clone();
        }
        let waited = self.call_within("Runtime.evaluate", params, MAX_SETTLE * 2);
        let deadline = Instant::now() + MAX_LOAD;
        let mut settled = waited.is_ok();
        while !settled && self.dialog_says().is_none() && Instant::now() < deadline {
            thread::sleep(ACTIONABLE_POLL);
            let state = self.call_within(
                "Runtime.evaluate",
                json!({ "expression": "document.readyState", "returnByValue": true }),
                MAX_SETTLE,
            );
            settled = state.is_ok_and(|state| state["result"]["value"] == "complete");
        }
    }
}

/// Why nobody could act on the element.
fn refusal(reference: &str, why: &str, details: &Value) -> String {
    let by = details["by"].as_str().unwrap_or_default();
    match why {
        "gone" => format!("{reference} is gone from the page: take a new snapshot"),
        "disabled" => format!("{reference} is disabled"),
        "read-only" => format!("{reference} is read-only"),
        "hidden" => format!("{reference} isn't shown"),
        "unfocusable" => format!("{reference} can't be focused to type into it"),
        "option" => format!("{reference} is an option of a closed <select>: select it there"),
        "outside" => format!("{reference} is outside of what the page can show"),
        "covered" => format!("{reference} is covered by {by}, which would take the click"),
        "passed" => format!("{reference} takes no clicks: {by} under it would"),
        "other" => {
            format!("{reference} isn't a <select>: click it to open it, then click the option")
        }
        "missing" => format!(
            "{reference} has no option {}; it has {}",
            details["missing"], details["has"]
        ),
        other => format!("{reference} can't be used: {other}"),
    }
}

#[cfg(test)]
mod tests;
