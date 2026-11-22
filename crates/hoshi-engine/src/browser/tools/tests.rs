use super::*;

#[test]
fn an_agent_opens_reads_uses_and_closes_its_page() {
    if find_browser().is_none() {
        eprintln!("no Chromium-based browser here; skipping");
        return;
    }
    let profile = std::env::temp_dir().join(format!("hoshi-preview-{}", std::process::id()));
    let notify: Notify = Arc::new(|_| {});
    let browsing = Browsing::new(profile.clone(), notify);
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
fn the_user_side_reads_values_off_an_open_page_only() {
    if find_browser().is_none() {
        eprintln!("no Chromium-based browser here; skipping");
        return;
    }
    let profile = std::env::temp_dir().join(format!("hoshi-evaluate-{}", std::process::id()));
    let browsing = Browsing::new(profile.clone(), Arc::new(|_| {}));
    let before = browsing.user_evaluate("t", "1 + 1");
    browsing
            .user_open(
                "t",
                "data:text/html,<!doctype%20html><style>body{margin:0}</style><div%20style=height:321px></div>",
            )
            .unwrap();
    let height = browsing
        .user_evaluate(
            "t",
            "document.documentElement.getBoundingClientRect().height",
        )
        .unwrap();
    browsing.close();

    assert_eq!(before.unwrap_err(), NO_PAGE);
    assert_eq!(height.as_f64(), Some(321.));
    let _ = std::fs::remove_dir_all(&profile);
}

#[test]
fn a_page_the_user_closed_says_so_and_opens_again_in_the_same_browser() {
    if find_browser().is_none() {
        eprintln!("no Chromium-based browser here; skipping");
        return;
    }
    let profile = std::env::temp_dir().join(format!("hoshi-closed-{}", std::process::id()));
    let browsing = Browsing::new(profile.clone(), Arc::new(|_| {}));
    let page = "data:text/html,<p>Signed in</p>";
    browsing
        .call("t", "preview_open", &json!({ "url": page }))
        .unwrap();

    browsing.user_close("t");
    let refused = browsing.call("t", "preview_snapshot", &json!({}));
    let status = browsing.call("t", "preview_status", &json!({})).unwrap();
    let kept = browsing.state().browser.is_some();
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
    let browsing = Browsing::new(profile.clone(), notify);
    let page = "data:text/html,<button style=\"width:400px;height:200px\" onclick=\"this.textContent='Clicked'\">Press</button>";

    let opened = browsing
        .call("t", "preview_open", &json!({ "url": page }))
        .unwrap();
    browsing.view("t", true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let frame = std::iter::from_fn(|| {
        received
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .ok()
    })
    .find_map(|notification| match notification {
        Notification::BrowserFrame(frame) => Some(frame),
        _ => None,
    })
    .expect("the panel got no frame of the page within 30 seconds");
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
    assert!(frame.jpeg.starts_with(&[0xff, 0xd8]));
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
    let browsing = Browsing::new(profile.clone(), Arc::new(|_| {}));
    let page = json!({ "url": "about:blank" });
    browsing.call("t", "preview_open", &page).unwrap();
    let browser = browsing.state().browser.clone().unwrap();
    let _ =
        browser
            .connection
            .call_within("Browser.close", json!({}), None, Duration::from_secs(5));
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
    assert!(browsing.state().browser.is_none());
    let _ = std::fs::remove_dir_all(&profile);
}

#[test]
fn a_busy_page_keeps_no_other_page_waiting() {
    let browsing = Arc::new(Browsing::new(std::env::temp_dir(), Arc::new(|_| {})));
    let connection = Arc::new(super::super::cdp::Connection::offline());
    for thread_id in ["slow", "other"] {
        let page = Page {
            connection: connection.clone(),
            target_id: thread_id.into(),
            session_id: thread_id.into(),
            refs: HashMap::new(),
            console: Arc::default(),
            size: DEFAULT_SIZE,
            zoom: 1.0,
            scale: 1.0,
        };
        browsing.state().pages.insert(
            thread_id.into(),
            OpenPage {
                page: Arc::new(Mutex::new(page)),
                target_id: thread_id.into(),
            },
        );
    }
    let slow = browsing.page("slow").unwrap();
    let busy = lock(&slow);

    let (done, finished) = std::sync::mpsc::channel();
    let other = browsing.clone();
    std::thread::spawn(move || {
        let input = other.user_input(
            "other",
            &BrowserInput::Key {
                key: "Enter".into(),
            },
        );
        let _ = other.view("other", true);
        let _ = other.user_fit("other", (800, 600));
        let _ = other.user_scale("other", 2.0);
        let _ = other.call("other", "preview_status", &json!({}));
        other.forget("slow");
        let _ = done.send(input);
    });
    let input = finished.recv_timeout(Duration::from_secs(5));
    drop(busy);

    assert_eq!(input, Ok(Err("the browser closed".into())));
    assert!(browsing.page("slow").is_err());
    assert!(browsing.page("other").is_ok());
}

#[test]
fn a_page_that_waits_keeps_no_other_thread_s_page_waiting() {
    if find_browser().is_none() {
        eprintln!("no Chromium-based browser here; skipping");
        return;
    }
    let profile = std::env::temp_dir().join(format!("hoshi-apart-{}", std::process::id()));
    let browsing = Arc::new(Browsing::new(profile.clone(), Arc::new(|_| {})));
    let page = json!({ "url": "data:text/html,<p>Ready</p>" });
    browsing.call("slow", "preview_open", &page).unwrap();
    let waiting = browsing.clone();
    let slow = std::thread::spawn(move || {
        waiting.call(
            "slow",
            "preview_wait_for",
            &json!({ "text": "never shown", "timeoutMs": 6000 }),
        )
    });
    std::thread::sleep(Duration::from_millis(300));

    let started = Instant::now();
    let opened = browsing.call("fast", "preview_open", &page).unwrap();
    browsing
        .user_input("fast", &BrowserInput::Key { key: "Tab".into() })
        .unwrap();
    let took = started.elapsed();
    let waited = slow.join().unwrap().unwrap();
    browsing.close();
    let _ = std::fs::remove_dir_all(&profile);

    assert_eq!(opened["opened"], true);
    assert!(
        took < Duration::from_secs(3),
        "the other page waited {took:?}"
    );
    assert_eq!(waited["found"], false);
}

#[test]
fn bare_addresses_get_a_scheme() {
    assert_eq!(address("localhost:5173"), "http://localhost:5173");
    assert_eq!(address("127.0.0.1:3000/x"), "http://127.0.0.1:3000/x");
    assert_eq!(address("example.com"), "https://example.com");
    assert_eq!(address("file:///tmp/a.html"), "file:///tmp/a.html");
    assert_eq!(address("about:blank"), "about:blank");
    assert_eq!(address("/tmp/a.html"), "file:///tmp/a.html");
    assert_eq!(address("C:\\work\\a.html"), "file:///C:/work/a.html");
    assert_eq!(
        address("/tmp/My Report.html"),
        "file:///tmp/My%20Report.html"
    );
}

#[test]
fn local_paths_with_spaces_are_not_searched_for() {
    let settings = BrowserSettings::default();
    assert_eq!(
        address_or_search("/tmp/My Report.html", &settings),
        "file:///tmp/My%20Report.html"
    );
    assert_eq!(
        address_or_search("C:\\Program Files\\report.html", &settings),
        "file:///C:/Program%20Files/report.html"
    );
    assert_eq!(
        address_or_search("file:///tmp/My Report.html", &settings),
        "file:///tmp/My%20Report.html"
    );
}

#[test]
fn words_in_the_address_bar_are_searched_for() {
    let mut settings = BrowserSettings::default();
    assert_eq!(
        address_or_search("rust borrow checker", &settings),
        "https://duckduckgo.com/?q=rust+borrow+checker"
    );
    assert_eq!(
        address_or_search("localhost:5173", &settings),
        "http://localhost:5173"
    );
    assert_eq!(
        address_or_search("example.com/a", &settings),
        "https://example.com/a"
    );
    settings.search_engine = SearchEngine::Google;
    assert_eq!(
        address_or_search("c++ & rust", &settings),
        "https://www.google.com/search?q=c%2B%2B+%26+rust"
    );
    settings.search_engine = SearchEngine::Custom;
    settings.custom_search_url = "https://search.example/?q=%s&lang=es".into();
    assert_eq!(
        address_or_search("ñandú", &settings),
        "https://search.example/?q=%C3%B1and%C3%BA&lang=es"
    );
    settings.custom_search_url = "not a url".into();
    assert_eq!(
        address_or_search("hoshi", &settings),
        "https://duckduckgo.com/?q=hoshi"
    );
}

#[test]
fn agents_are_refused_the_browser_once_the_user_turns_it_off() {
    let browsing = Browsing::new(std::env::temp_dir(), Arc::new(|_| {}));
    browsing.configure(BrowserSettings {
        agent_access: false,
        ..BrowserSettings::default()
    });
    let refused = browsing.call("t", "preview_status", &json!({}));
    assert_eq!(refused, Err(NO_AGENT_ACCESS.to_owned()));
    assert!(browsing.state().browser.is_none());
}
