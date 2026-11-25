use super::super::{Browser, Page, find_browser};
use super::*;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

static RIGS: AtomicU32 = AtomicU32::new(0);
/// The browser the tests running now share, as each of their own would be too many at once.
static SHARED: Mutex<Weak<Shared>> = Mutex::new(Weak::new());
/// One is held by each test that uses a page: many at once would slow down the tests beside them.
static TURNS: [Mutex<()>; 3] = [const { Mutex::new(()) }; 3];

struct Shared {
    browser: Option<Browser>,
    profile: PathBuf,
}

impl Shared {
    fn get() -> Option<Arc<Self>> {
        let Some(executable) = find_browser() else {
            eprintln!("no Chromium-based browser here; skipping");
            return None;
        };
        let mut shared = lock(&SHARED);
        if let Some(running) = shared.upgrade() {
            return Some(running);
        }
        let name = format!(
            "hoshi-act-{}-{}",
            std::process::id(),
            RIGS.fetch_add(1, Ordering::Relaxed)
        );
        let profile = std::env::temp_dir().join(name);
        let started = Arc::new(Self {
            browser: Some(Browser::start(&executable, &profile).unwrap()),
            profile,
        });
        *shared = Arc::downgrade(&started);
        Some(started)
    }

    fn open_page(&self) -> Page {
        self.browser
            .as_ref()
            .unwrap()
            .open_page((800, 600), Arc::new(|_| {}))
            .unwrap()
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        drop(self.browser.take());
        for _ in 0..20 {
            if std::fs::remove_dir_all(&self.profile).is_ok() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

/// A page with `body`, as its latest snapshot reads.
struct Rig {
    page: Page,
    snapshot: String,
    shared: Arc<Shared>,
    _turn: MutexGuard<'static, ()>,
}

impl Rig {
    fn open(body: &str) -> Option<Self> {
        let shared = Shared::get()?;
        let turn = RIGS.fetch_add(1, Ordering::Relaxed) as usize % TURNS.len();
        let turn = lock(&TURNS[turn]);
        let mut page = shared.open_page();
        let file = shared.profile.join(format!(
            "page-{}.html",
            RIGS.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(
            &file,
            format!("<!doctype html><meta charset=utf-8><script>window.log=[]</script>{body}"),
        )
        .unwrap();
        let url = format!("file:///{}", file.display().to_string().replace('\\', "/"));
        page.navigate(&url, Duration::from_secs(10)).unwrap();
        let mut rig = Self {
            page,
            snapshot: String::new(),
            shared,
            _turn: turn,
        };
        rig.read();
        Some(rig)
    }

    fn read(&mut self) {
        self.snapshot = self.page.snapshot(50_000).unwrap();
    }

    /// The ref of the first element that reads as `label`, whether the outline shows one or not.
    fn reference(&mut self, label: &str) -> String {
        let shown = self
            .snapshot
            .lines()
            .filter(|line| line.contains(label))
            .find_map(|line| line.rsplit_once(" [e"))
            .map(|(_, rest)| format!("e{}", rest.split(']').next().unwrap()));
        if let Some(reference) = shown {
            return reference;
        }
        let nodes = self.page.nodes().unwrap();
        let tree = Tree::new(&nodes);
        nodes
            .iter()
            .filter(|node| node["ignored"] != true)
            .filter(|node| {
                let name = node["name"]["value"].as_str().unwrap_or_default();
                let role = node["role"]["value"].as_str().unwrap_or_default();
                format!("{role} {name:?}").contains(label)
            })
            .find_map(|node| self.page.refs.give(&tree, node))
            .unwrap_or_else(|| panic!("no {label} in\n{}", self.snapshot))
    }

    fn click(&mut self, label: &str) -> Result<(), String> {
        let reference = self.reference(label);
        self.page.click(&reference)
    }

    fn type_into(&mut self, label: &str, text: &str, clear: bool) -> Result<(), String> {
        let reference = self.reference(label);
        self.page.type_text(&reference, text, clear)
    }

    fn js(&self, expression: &str) -> Value {
        self.page.evaluate(expression).unwrap()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.page.close();
    }
}

/// A list drawn as a framework without keys draws it: nodes are reused by place.
const REUSED: &str = r#"<button onclick="items.shift(); draw()">Drop first</button><ul id=list></ul><script>
let items = ['Alpha', 'Beta', 'Gamma', 'Delta'];
const list = document.querySelector('#list');
function draw() {
  while (list.children.length > items.length) list.lastChild.remove();
  items.forEach((item, i) => {
    let li = list.children[i];
    if (!li) { li = document.createElement('li'); li.append(document.createElement('span'), document.createElement('button')); list.append(li); }
    li.firstChild.textContent = item;
    const button = li.lastChild;
    button.textContent = NAME(item);
    button.onclick = () => { log.push(item); items = items.filter((one) => one !== item); draw(); };
  });
}
draw();
</script>"#;

#[test]
fn a_ref_follows_its_element_when_the_page_reuses_the_node() {
    let body = REUSED.replace("NAME(item)", "'Delete ' + item");
    let Some(mut rig) = Rig::open(&body) else {
        return;
    };
    rig.click("Drop first").unwrap();

    rig.click("Delete Beta").unwrap();

    assert_eq!(rig.js("log"), json!(["Beta"]));
}

#[test]
fn a_ref_follows_its_row_when_every_row_has_the_same_button() {
    let body = REUSED.replace("NAME(item)", "'Delete'");
    let Some(mut rig) = Rig::open(&body) else {
        return;
    };
    let beta = rig.snapshot.lines().position(|line| line.contains("Beta"));
    let rest: Vec<&str> = rig.snapshot.lines().skip(beta.unwrap()).collect();
    let reference = rest
        .iter()
        .find(|line| line.contains("button \"Delete\""))
        .and_then(|line| line.rsplit_once(" ["))
        .map(|(_, rest)| rest.trim_end_matches(']').to_owned())
        .unwrap();
    rig.click("Drop first").unwrap();

    rig.page.click(&reference).unwrap();
    assert_eq!(rig.js("log"), json!(["Beta"]));

    let again = rig.page.click(&reference).unwrap_err();
    assert!(
        again.contains("its place is now button \"Delete\" (in \"Delta\")"),
        "{again}"
    );
    assert_eq!(rig.js("log"), json!(["Beta"]));
}

#[test]
fn a_ref_follows_its_element_when_the_page_replaces_the_node() {
    let Some(mut rig) = Rig::open(
        "<div id=root></div><script>let n = 0; function draw() { n += 1; const at = n; root.replaceChildren(Object.assign(document.createElement('button'), { textContent: 'Save', onclick: () => log.push(at) })); } draw();</script><button onclick=draw()>Again</button>",
    ) else {
        return;
    };
    rig.click("Again").unwrap();

    rig.click("button \"Save\"").unwrap();

    assert_eq!(rig.js("log"), json!([2]));
}

#[test]
fn a_ref_that_could_be_several_elements_acts_on_none() {
    let Some(mut rig) = Rig::open(
        "<div id=root><button>Save</button></div><button onclick=\"root.innerHTML = '<button onclick=log.push(1)>Save</button><button onclick=log.push(2)>Save</button>'\">Again</button>",
    ) else {
        return;
    };
    rig.click("Again").unwrap();

    let refused = rig.click("button \"Save\"").unwrap_err();

    assert!(refused.contains("several like it"), "{refused}");
    assert_eq!(rig.js("log"), json!([]));
}

#[test]
fn an_element_whose_name_changes_keeps_its_ref() {
    let Some(mut rig) = Rig::open(
        "<button onclick=\"log.push(this.textContent); this.textContent = 'Unmute'\">Mute</button><button>Resend in 30s</button>",
    ) else {
        return;
    };
    let resend = rig.reference("Resend");
    rig.click("Mute").unwrap();

    rig.click("Mute").unwrap();
    rig.js("document.querySelectorAll('button')[1].textContent = 'Resend in 9s'");
    rig.page.click(&resend).unwrap();

    assert_eq!(rig.js("log"), json!(["Mute", "Unmute"]));
    rig.read();
    assert_eq!(rig.reference("Resend"), resend);
}

#[test]
fn a_ref_whose_element_is_gone_never_acts_on_the_one_left() {
    for (one, two) in [("Alpha", "Beta"), ("Untitled", "Untitled")] {
        let card = |title: &str| {
            format!(
                "<div class=card><h3>{title}</h3><button onclick=\"log.push(1); this.closest('.card').remove()\">Delete</button></div>"
            )
        };
        let Some(mut rig) = Rig::open(&format!("{}{}", card(one), card(two))) else {
            return;
        };
        rig.click("Delete").unwrap();

        assert!(rig.click("Delete").is_err(), "{one}");
        assert_eq!(rig.js("log"), json!([1]), "{one}");
    }
    let Some(mut rig) = Rig::open(
        "<div id=box><p>Archive project?</p><button>Confirm</button></div><button onclick=\"box.innerHTML = '<p>Delete account?</p><button onclick=log.push(1)>Confirm</button>'\">Next</button>",
    ) else {
        return;
    };
    rig.click("Next").unwrap();
    assert!(rig.click("Confirm").is_err());
    assert_eq!(rig.js("log"), json!([]));
}

#[test]
fn a_row_that_changes_what_it_says_keeps_its_refs() {
    let Some(mut rig) = Rig::open(
        "<ul><li><b>Ana</b> <i id=when>1s ago</i> <span>hello there friend</span><button onclick=log.push('replied')>Reply</button></li></ul>",
    ) else {
        return;
    };
    rig.js("document.querySelector('#when').textContent = '2m ago'");

    rig.click("Reply").unwrap();

    assert_eq!(rig.js("log"), json!(["replied"]));
}

#[test]
fn refs_stay_between_snapshots_and_are_never_reused() {
    let Some(mut rig) =
        Rig::open("<div id=root><button>First</button><button>Second</button></div>")
    else {
        return;
    };
    let first = rig.reference("First");
    let second = rig.reference("Second");
    rig.js("document.querySelector('button').remove()");
    rig.js(
        "root.append(Object.assign(document.createElement('button'), { textContent: 'Third' }))",
    );

    rig.read();

    assert_eq!(rig.reference("Second"), second);
    assert_ne!(rig.reference("Third"), second);
    assert_ne!(rig.reference("Third"), first);
    assert!(rig.page.click(&first).is_err());
    let unknown = rig.page.click("e1234").unwrap_err();
    assert!(unknown.contains("no element e1234"), "{unknown}");
}

#[test]
fn refs_of_a_page_that_was_left_name_nothing() {
    let Some(mut rig) = Rig::open("<button onclick=location.reload()>Save</button>") else {
        return;
    };
    let reference = rig.reference("Save");
    rig.page.click(&reference).unwrap();
    rig.page.settle();

    let refused = rig.page.click(&reference).unwrap_err();

    assert!(refused.contains("is gone"), "{refused}");
}

#[test]
fn a_same_named_element_at_another_address_isn_t_the_one_read() {
    let Some(mut rig) = Rig::open(
        "<div id=root><button onclick=log.push('one')>Delete</button></div><button onclick=\"history.pushState(0, '', '?second'); root.innerHTML = '<button onclick=log.push(2)>Delete</button>'\">Next</button>",
    ) else {
        return;
    };
    // A page of a file can't change its address this way.
    rig.js("history.replaceState(0, '', location.href)");
    rig.js("document.querySelectorAll('button')[1].onclick = () => { location.hash = 'second'; root.innerHTML = '<button onclick=log.push(2)>Delete</button>' }");
    rig.click("Next").unwrap();

    let refused = rig.click("Delete").unwrap_err();

    assert!(refused.contains("isn't on the page any more"), "{refused}");
    assert_eq!(rig.js("log"), json!([]));
}

#[test]
fn a_covered_element_isn_t_clicked() {
    let Some(mut rig) = Rig::open(
        "<button onclick=log.push('under')>Under</button><div id=veil class=\"veil dark\" style=\"position:fixed;inset:0\" onclick=log.push('veil')>Wait here</div>",
    ) else {
        return;
    };

    let refused = rig.click("Under").unwrap_err();

    assert!(
        refused.contains("covered by <div#veil.veil.dark> \"Wait here\""),
        "{refused}"
    );
    assert_eq!(rig.js("log"), json!([]));
    rig.js("setTimeout(() => veil.remove(), 500)");
    rig.click("Under").unwrap();
    assert_eq!(rig.js("log"), json!(["under"]));
}

#[test]
fn a_disabled_element_is_waited_for_then_refused() {
    let Some(mut rig) = Rig::open(
        "<button disabled onclick=log.push('native')>Native</button><button aria-disabled=true>Aria</button>",
    ) else {
        return;
    };

    assert!(rig.click("Native").unwrap_err().ends_with("is disabled"));
    assert!(rig.click("Aria").unwrap_err().ends_with("is disabled"));
    let aria = rig.reference("Aria");
    rig.page.hover(&aria).unwrap();
    rig.js("setTimeout(() => { document.querySelector('button').disabled = false }, 400)");
    rig.click("Native").unwrap();

    assert_eq!(rig.js("log"), json!(["native"]));
}

#[test]
fn what_a_person_clicks_through_is_clicked() {
    let Some(mut rig) = Rig::open(
        r#"<style>.sr{position:absolute;width:1px;height:1px;overflow:hidden;clip:rect(0,0,0,0)}.card{position:relative;padding:20px}.card a{position:absolute;inset:0}</style>
        <input type=checkbox id=tos class=sr onchange="log.push('tos')"><label for=tos>Accept</label>
        <label><input type=checkbox class=sr aria-label=Wrapped onchange="log.push('wrapped')"><span>Wrapped text</span></label>
        <div class=card><h2>Stretched</h2><a href="javascript:log.push('card')" aria-label="Open card"></a></div>
        <button onclick="log.push('icon')"><img alt="Gear" width=20 height=20 style="pointer-events:none"></button>
        <div onclick="log.push('area')"><p style="pointer-events:none">Passing text</p></div>
        <div id=host></div><div id=sealed></div>
        <div role=grid><div role=row><div role=gridcell style="display:contents" aria-label="Day 1"><span onclick="log.push('contents')">1st</span></div></div></div>
        <div style="cursor:pointer" onclick="log.push('forwards')"><input type=checkbox aria-label=Forwarded style="pointer-events:none"></div>
        <div style="position:relative;width:200px"><input type=checkbox aria-label=Beneath><span style="position:absolute;inset:0" onclick="log.push('beside')"></span></div>
        <div style="position:relative;width:120px;height:30px" onclick="log.push('tiny')"><input aria-label=Tiny style="position:absolute;width:1px;height:1px;border:0;padding:0;opacity:0;pointer-events:none"></div>
        <div style="position:relative;padding:20px"><h4>Spread</h4><p><a id=spread href="javascript:log.push('spread')">Go</a></p><style>#spread::after{content:'';position:absolute;inset:0}</style></div>
        <div style="position:relative;width:200px;height:30px"><button style="position:absolute;inset:0" onclick="log.push('edge')">Edge strip</button><div style="position:absolute;top:0;bottom:0;left:0;right:20px"></div></div>
        <div aria-disabled=true><button onclick="log.push('enabled')">Enabled inside</button></div>
        <label style="position:relative;padding:20px;display:inline-block"><input type=radio name=size aria-label=Medium style="opacity:0;width:0;height:0" onchange="log.push('medium')"><span>Medium size</span></label>
        <style>.row button{opacity:0;pointer-events:none}.row:hover button{opacity:1;pointer-events:auto}</style><div class=row>Row <button onclick="log.push('hovered')">On hover</button></div>
        <div style="position:relative;width:300px"><button style="width:100%;height:40px" onclick="log.push('partly')">Partly covered</button><div style="position:absolute;left:30%;right:30%;top:0;bottom:0"></div></div>
        <p style="width:120px">aaaaaaaaaaaa <a href="javascript:log.push('wrap')">bb bbbbbbbbbbbb</a> cccc</p>
        <div style="height:2000px"></div>
        <button onclick="log.push('far')">Far away</button>
        <script>host.attachShadow({ mode: 'open' }).innerHTML = '<button onclick="log.push(\'shadow\')"><slot></slot></button>'; host.append('Shadowed'); sealed.attachShadow({ mode: 'closed' }).innerHTML = '<button onclick="log.push(\'closed\')">In closed</button>'</script>"#,
    ) else {
        return;
    };

    for (label, said) in [
        ("checkbox \"Accept\"", "tos"),
        ("checkbox \"Wrapped\"", "wrapped"),
        ("heading \"Stretched\"", "card"),
        ("link \"Open card\"", "card"),
        ("image \"Gear\"", "icon"),
        ("button \"Shadowed\"", "shadow"),
        ("radio \"Medium\"", "medium"),
        ("textbox \"Tiny\"", "tiny"),
        ("heading \"Spread\"", "spread"),
        ("button \"Edge strip\"", "edge"),
        ("button \"In closed\"", "closed"),
        ("gridcell \"Day 1\"", "contents"),
        ("checkbox \"Forwarded\"", "forwards"),
        ("checkbox \"Beneath\"", "beside"),
        ("button \"Enabled inside\"", "enabled"),
        ("button \"On hover\"", "hovered"),
        ("button \"Partly covered\"", "partly"),
        ("link \"bb bbbbbbbbbbbb\"", "wrap"),
        ("button \"Far away\"", "far"),
    ] {
        rig.js("log.length = 0");
        rig.click(label)
            .unwrap_or_else(|error| panic!("{label}: {error}"));
        rig.page.settle();
        assert_eq!(rig.js("log"), json!([said]), "{label}");
    }
}

#[test]
fn an_element_on_the_move_is_clicked_where_it_rests() {
    let Some(mut rig) = Rig::open(
        "<style>#slide{position:absolute;left:0;transition:left .6s linear}#spin{position:absolute;top:200px;animation:a 1s linear infinite alternate}@keyframes a{to{left:300px}}</style><button id=slide onclick=log.push('slide')>Slide</button><button id=spin onclick=log.push('spin')>Spin</button>",
    ) else {
        return;
    };
    rig.js("slide.style.left = '500px'");

    rig.click("Slide").unwrap();
    rig.click("Spin").unwrap();

    assert_eq!(rig.js("log[0]"), json!("slide"));
}

/// An input whose value its framework follows, as React does: what a script sets goes unseen.
const CONTROLLED: &str = r#"<input aria-label=Name value=old><output></output><script>
const input = document.querySelector('input');
const own = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value');
let tracked = input.value, state = 'old';
Object.defineProperty(input, 'value', { configurable: true, get() { return own.get.call(this) }, set(value) { tracked = value; own.set.call(this, value) } });
input.addEventListener('keydown', () => log.push('key'));
input.addEventListener('input', () => { const now = own.get.call(input); if (now !== tracked) { tracked = now; state = now } });
</script>"#;

#[test]
fn a_controlled_input_is_emptied_and_typed_into_by_key() {
    let Some(mut rig) = Rig::open(CONTROLLED) else {
        return;
    };

    rig.type_into("textbox", "", true).unwrap();
    assert_eq!(rig.js("[input.value, state]"), json!(["", ""]));
    rig.js("log.length = 0");
    rig.type_into("textbox", "Año 1.5%!", false).unwrap();
    assert_eq!(
        rig.js("[input.value, state, log.length]"),
        json!(["Año 1.5%!", "Año 1.5%!", 8])
    );
    rig.type_into("textbox", &"long ".repeat(40), true).unwrap();
    assert_eq!(rig.js("state.length"), json!(200));
}

#[test]
fn an_element_waited_for_is_still_the_one_read() {
    let body = REUSED.replace("NAME(item)", "'Delete'")
        + "<div id=veil style=\"position:fixed;inset:0\"></div>";
    let Some(mut rig) = Rig::open(&body) else {
        return;
    };
    rig.js("setTimeout(() => { items.shift(); draw(); veil.remove(); }, 500)");

    let refused = rig.click("button \"Delete\"").unwrap_err();

    assert!(refused.contains("its place is now"), "{refused}");
    assert_eq!(rig.js("log"), json!([]));
}

#[test]
fn another_control_over_a_field_isn_t_clicked_for_it() {
    let Some(mut rig) = Rig::open(
        "<div style=\"position:relative;width:100px;height:30px\"><input type=checkbox aria-label=Agree><button style=\"position:absolute;inset:0\" onclick=log.push('other')>Other</button></div>",
    ) else {
        return;
    };

    assert!(
        rig.click("checkbox")
            .unwrap_err()
            .contains("covered by <button>")
    );
    assert_eq!(rig.js("log"), json!([]));
}

#[test]
fn text_for_a_field_that_passes_the_focus_on_isn_t_typed_elsewhere() {
    let Some(mut rig) =
        Rig::open("<input aria-label=One onfocus='two.focus()'><input aria-label=Two id=two>")
    else {
        return;
    };

    let refused = rig.type_into("\"One\"", "secret", false).unwrap_err();

    assert!(refused.contains("passed the focus on"), "{refused}");
    assert_eq!(rig.js("two.value"), json!(""));
}

#[test]
fn the_parts_the_browser_draws_a_field_with_are_used_too() {
    let Some(mut rig) = Rig::open(
        "<input type=date aria-label=Day><input type=time aria-label=Hour><input type=number aria-label=Count><input type=search aria-label=Find value=x><video controls></video><details><summary>More</summary>Text</details>",
    ) else {
        return;
    };
    let refs: Vec<String> = rig
        .snapshot
        .lines()
        .filter_map(|line| line.rsplit_once(" [e"))
        .map(|(_, rest)| format!("e{}", rest.split(']').next().unwrap()))
        .collect();

    for reference in &refs {
        let _ = rig.page.click(reference);
        let _ = rig.page.hover(reference);
        assert_eq!(rig.js("1"), json!(1), "{reference} in\n{}", rig.snapshot);
    }
    let day = rig.reference("spinbutton");
    rig.page.click(&day).unwrap();
    for key in "02022026".chars() {
        rig.page.press(&key.to_string()).unwrap();
    }

    let typed = rig.js("document.querySelector('input').value");
    assert!(
        typed
            .as_str()
            .is_some_and(|value| value.starts_with("2026")),
        "{typed}"
    );
}

#[test]
fn a_field_that_moves_its_caret_when_focused_is_emptied() {
    let Some(mut rig) = Rig::open(
        "<input aria-label=Phone value='(555) 111-2222' onfocus='requestAnimationFrame(() => this.setSelectionRange(14, 14))'><input aria-label=Late value=old onfocus='setTimeout(() => this.setSelectionRange(3, 3))'>",
    ) else {
        return;
    };

    rig.type_into("\"Phone\"", "999", true).unwrap();
    rig.type_into("\"Late\"", "new", true).unwrap();

    assert_eq!(
        rig.js("[...document.querySelectorAll('input')].map((field) => field.value)"),
        json!(["999", "new"])
    );
}

#[test]
fn text_for_what_is_no_field_is_put_in_without_its_keys() {
    let Some(mut rig) = Rig::open(
        "<button onclick=log.push('clicked')>Go</button><input type=checkbox onchange=log.push('ticked')><div contenteditable role=textbox onkeydown=log.push('key')>start</div>",
    ) else {
        return;
    };

    rig.type_into("button", "a b c", false).unwrap();
    rig.type_into("checkbox", "a b c", false).unwrap();
    for _ in 0..2 {
        rig.type_into("textbox", "x", true).unwrap();
    }

    assert_eq!(rig.js("log"), json!(["key", "key"]));
    assert_eq!(
        rig.js("document.querySelector('[contenteditable]').textContent"),
        json!("x")
    );
}

#[test]
fn every_kind_of_field_takes_text() {
    let Some(mut rig) = Rig::open(
        "<input type=email aria-label=Mail value=a@b.c><input type=number aria-label=Count value=3><textarea aria-label=Notes>old</textarea><div contenteditable role=textbox aria-label=Rich>some <b>bold</b></div><label>Wrapped<input id=wrapped></label><input type=date aria-label=Day onchange=log.push(this.value)><input type=datetime-local aria-label=When><input type=range aria-label=Level><input readonly aria-label=Fixed value=x><input disabled aria-label=Off><p>Plain</p>",
    ) else {
        return;
    };

    rig.type_into("\"Mail\"", "new@test.dev", true).unwrap();
    rig.type_into("\"Count\"", "42", true).unwrap();
    rig.type_into("\"Notes\"", "one\ntwo", true).unwrap();
    rig.type_into("\"Rich\"", "fresh", true).unwrap();
    rig.type_into("LabelText", "by label", false).unwrap();
    rig.type_into("\"Day\"", "2026-01-31", true).unwrap();
    rig.type_into("\"When\"", "2026-01-31 14:30:00", false)
        .unwrap();
    rig.type_into("\"Level\"", "70", false).unwrap();

    assert_eq!(
        rig.js("[...document.querySelectorAll('input,textarea')].slice(0, 7).map((field) => field.value)"),
        json!(["new@test.dev", "42", "one\ntwo", "by label", "2026-01-31",
            "2026-01-31T14:30",
            "70"])
    );
    assert_eq!(
        rig.js("document.querySelector('[contenteditable]').textContent"),
        json!("fresh")
    );
    assert_eq!(rig.js("log"), json!(["2026-01-31"]));
    assert!(rig.type_into("\"Level\"", " ", false).is_err());
    let refused = rig.type_into("\"Day\"", "31/01/2026", true).unwrap_err();
    assert!(refused.contains("doesn't take \"31/01/2026\""), "{refused}");
    assert_eq!(
        rig.js("document.querySelector('[type=date]').value"),
        json!("2026-01-31")
    );
    let fixed = rig.type_into("\"Fixed\"", "y", true).unwrap_err();
    assert!(fixed.ends_with("is read-only"), "{fixed}");
    let off = rig.type_into("\"Off\"", "y", true).unwrap_err();
    assert!(off.ends_with("is disabled"), "{off}");
    let plain = rig.type_into("paragraph", "y", false).unwrap_err();
    assert!(plain.contains("can't be focused"), "{plain}");
}

#[test]
fn the_parts_of_a_date_take_their_text_or_say_why_not() {
    let Some(mut rig) = Rig::open(
        "<input type=date aria-label=Day><input type=date aria-label=Off disabled><input type=date aria-label=Fixed readonly><input type=month aria-label=Month><input type=date aria-label=Once value=2026-03-03 onchange=\"log.push(this.value);this.remove()\"><input type=time aria-label=Hour>",
    ) else {
        return;
    };
    let parts: Vec<String> = rig
        .snapshot
        .lines()
        .filter(|line| line.contains("spinbutton"))
        .filter_map(|line| line.rsplit_once(" [e"))
        .map(|(_, rest)| format!("e{}", rest.split(']').next().unwrap()))
        .collect();
    let value = |rig: &Rig| rig.js("document.querySelector('input').value");

    // Day and month come in the order of the browser's language.
    for (part, text) in parts.iter().zip(["04", "4", "2026"]) {
        rig.page.type_text(part, text, false).unwrap();
    }
    assert_eq!(value(&rig), json!("2026-04-04"));
    rig.page.type_text(&parts[0], "05052027", true).unwrap();
    assert_eq!(value(&rig), json!("2027-05-05"));
    rig.page.type_text(&parts[1], "2028-06-30", false).unwrap();
    assert_eq!(value(&rig), json!("2028-06-30"));

    for text in ["99", "x"] {
        let refused = rig.page.type_text(&parts[0], text, false).unwrap_err();
        assert!(refused.contains("type the whole value"), "{refused}");
        assert_eq!(value(&rig), json!("2028-06-30"));
    }
    let refused = rig
        .page
        .type_text(&parts[0], "30-06-2028", false)
        .unwrap_err();
    assert!(refused.contains("doesn't take"), "{refused}");
    rig.page.type_text(&parts[2], "", true).unwrap();
    assert_eq!(value(&rig), json!(""));
    let off = rig.page.type_text(&parts[3], "4", false).unwrap_err();
    assert!(off.ends_with("is disabled"), "{off}");
    let fixed = rig.page.type_text(&parts[6], "4", false).unwrap_err();
    assert!(fixed.ends_with("is read-only"), "{fixed}");

    rig.page.type_text(&parts[9], "12", false).unwrap();
    rig.page.type_text(&parts[10], "2026", false).unwrap();
    assert_eq!(
        rig.js("document.querySelector('[type=month]').value"),
        json!("2026-12")
    );
    rig.page.type_text(&parts[14], "14:30", false).unwrap();
    assert_eq!(
        rig.js("document.querySelector('[type=time]').value"),
        json!("14:30")
    );
    rig.page.type_text(&parts[11], "5", false).unwrap();
    assert!(rig.js("log").to_string().contains("05"));
}

#[test]
fn a_character_the_user_types_is_a_key_and_pasted_text_is_not() {
    let Some(mut rig) = Rig::open(
        "<input aria-label=Name onkeydown=log.push(event.key)><input type=date aria-label=Day>",
    ) else {
        return;
    };
    rig.type_into("\"Name\"", "", false).unwrap();

    for text in ["a", "ñ", "pasted"] {
        rig.page.type_as_user(text).unwrap();
    }
    assert_eq!(
        rig.js("document.querySelector('input').value"),
        json!("añpasted")
    );
    assert_eq!(rig.js("log"), json!(["a"]));

    for (key, at) in [("Tab", "Day"), ("Shift+Tab", "Name"), ("Tab", "Day")] {
        rig.page.press(key).unwrap();
        assert_eq!(rig.js("document.activeElement.ariaLabel"), json!(at));
    }
    for digit in "04042026".chars() {
        rig.page.type_as_user(&digit.to_string()).unwrap();
    }
    assert_eq!(
        rig.js("document.querySelector('[type=date]').value"),
        json!("2026-04-04")
    );
}

#[test]
fn keys_are_pressed_together() {
    let Some(mut rig) = Rig::open(
        "<input aria-label=Name value=abc onkeydown=\"log.push([event.key, event.code, event.ctrlKey, event.shiftKey])\">",
    ) else {
        return;
    };
    rig.type_into("textbox", "", false).unwrap();

    rig.page.press("Control+a").unwrap();
    rig.page.press("Shift+x").unwrap();
    rig.page.press("ñ").unwrap();

    assert_eq!(rig.js("document.querySelector('input').value"), json!("Xñ"));
    assert_eq!(rig.js("log[1]"), json!(["a", "KeyA", true, false]));
    assert!(rig.page.press("Hyper+a").is_err());
}

#[test]
fn a_select_takes_only_options_it_has() {
    let Some(mut rig) = Rig::open(
        "<select aria-label=Size onchange=log.push(this.value)><option value=s>Small<option value=l>Large<option value=x disabled>Huge</select><button>Not one</button>",
    ) else {
        return;
    };
    let size = rig.reference("\"Size\"");
    let pick = |rig: &mut Rig, value: &str| rig.page.select(&size, &[value.to_owned()]);

    pick(&mut rig, "Large").unwrap();
    let missing = pick(&mut rig, "Tiny").unwrap_err();
    let disabled = pick(&mut rig, "x").unwrap_err();
    let button = rig.reference("Not one");
    let other = rig.page.select(&button, &["a".to_owned()]).unwrap_err();

    assert!(missing.contains("no option [\"Tiny\"]"), "{missing}");
    assert!(
        missing.contains("[\"Small\",\"Large\",\"Huge\"]"),
        "{missing}"
    );
    assert!(disabled.ends_with("is disabled"), "{disabled}");
    assert!(other.contains("isn't a <select>"), "{other}");
    assert_eq!(rig.js("log"), json!(["l"]));
}

#[test]
fn a_dialog_holds_the_page_until_it_is_answered() {
    let Some(mut rig) = Rig::open(
        "<button onclick=\"log.push(confirm('Sure?'))\">Ask</button><button onclick=\"setTimeout(() => alert('Late'), 30)\">Later</button>",
    ) else {
        return;
    };
    let started = Instant::now();

    for (key, answer) in [("Enter", true), ("Escape", false)] {
        let held = rig.click("Ask").unwrap_err();
        assert!(held.contains("(confirm) that says \"Sure?\""), "{held}");
        assert_eq!(rig.page.evaluate("1").unwrap_err(), held);
        assert!(rig.page.snapshot(100).is_err());
        rig.page.press(key).unwrap();
        rig.page.settle();
        assert_eq!(rig.js("log.at(-1)"), json!(answer));
    }
    rig.click("Later").unwrap();
    rig.page.settle();
    assert!(rig.page.dialog_says().unwrap().contains("\"Late\""));
    rig.page.press("Enter").unwrap();
    assert_eq!(rig.js("1"), json!(1));

    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn dialogs_in_a_row_are_answered_one_by_one() {
    let Some(mut rig) = Rig::open(
        "<button onclick=\"for (let i = 0; i < 12; i++) alert(i); log.push(prompt('Name', 'Ana'))\">Many</button>",
    ) else {
        return;
    };
    assert!(rig.click("Many").unwrap_err().contains("\"0\""));

    let deadline = Instant::now() + Duration::from_secs(20);
    while rig.page.evaluate("log.length").is_err() {
        assert!(Instant::now() < deadline);
        rig.page.begin_act();
        let pressed = rig.page.press_as_agent("Enter");
        assert!(pressed.is_ok() && rig.page.has_acted());
        thread::sleep(Duration::from_millis(50));
    }

    assert_eq!(rig.js("log"), json!(["Ana"]));
}

#[test]
fn a_dialog_that_cuts_an_action_short_doesn_t_pass_for_done() {
    let Some(mut rig) = Rig::open(
        "<input aria-label=Name onkeydown=\"if (event.key === 'c') alert('Stop')\"><button disabled>Off</button>",
    ) else {
        return;
    };

    rig.page.begin_act();
    let cut = rig.type_into("textbox", "abcdef", false).unwrap_err();
    assert!(cut.starts_with("typed 2 of 6 characters"), "{cut}");
    assert!(!rig.page.has_acted());
    rig.page.press("Enter").unwrap();

    rig.js("setTimeout(() => alert('Timer'), 300)");
    rig.page.begin_act();
    assert!(rig.click("Off").unwrap_err().contains("Timer"));
    assert!(!rig.page.has_acted());
}

#[test]
fn a_page_held_by_a_dialog_can_be_left() {
    let Some(mut rig) = Rig::open("<button onclick=\"alert(1); alert(2)\">Ask</button>") else {
        return;
    };
    assert!(rig.click("Ask").is_err());

    rig.page
        .navigate("data:text/html,<p>Elsewhere</p>", Duration::from_secs(5))
        .unwrap();

    assert_eq!(rig.js("document.body.innerText"), json!("Elsewhere"));
}

#[test]
fn a_page_that_asks_before_it_is_left_is_left() {
    let Some(mut rig) = Rig::open(
        "<input aria-label=Name><script>onbeforeunload = (event) => event.preventDefault()</script>",
    ) else {
        return;
    };
    rig.type_into("textbox", "dirty", false).unwrap();

    rig.page
        .navigate("data:text/html,<p>Elsewhere</p>", Duration::from_secs(5))
        .unwrap();

    assert_eq!(rig.js("document.body.innerText"), json!("Elsewhere"));
}

#[test]
fn an_action_waits_for_what_it_started() {
    let Some(mut rig) = Rig::open(
        "<button onclick=\"setTimeout(() => { out.textContent = 'one'; setTimeout(() => { out.textContent = 'two' }, 80) }, 60)\">Load</button><output id=out></output><button onclick=\"setInterval(() => { out.textContent = Math.random() }, 20)\">Forever</button><button onclick=\"location.href = 'data:text/html,<title>x</title>'; location.reload()\">Leave</button>",
    ) else {
        return;
    };

    rig.click("Load").unwrap();
    rig.page.settle();
    assert_eq!(rig.js("out.textContent"), json!("two"));
    rig.click("Forever").unwrap();
    let started = Instant::now();
    rig.page.settle();
    let waited = started.elapsed();
    assert!(waited < Duration::from_millis(1_900), "{waited:?}");
    rig.click("Leave").unwrap();
    rig.page.settle();
    assert_eq!(rig.js("document.readyState"), json!("complete"));
}

#[test]
fn a_page_behind_another_isn_t_slowed_down_as_hidden() {
    let Some(mut rig) =
        Rig::open("<input aria-label=Name><button onclick=log.push('clicked')>Go</button>")
    else {
        return;
    };
    let mut front = rig.shared.open_page();
    front
        .navigate("data:text/html,<p>Front</p>", Duration::from_secs(5))
        .unwrap();

    rig.type_into("textbox", "typed", false).unwrap();
    rig.click("Go").unwrap();

    assert_eq!(
        rig.js("[document.querySelector('input').value, log[0]]"),
        json!(["typed", "clicked"])
    );
    assert_eq!(
        rig.js("[document.visibilityState, document.hasFocus()]"),
        json!(["visible", true])
    );
}
