use super::ThreadDetails;
use crate::state::{self, Workspace};
use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, IntoElement, Modifiers, ParentElement as _, Point,
    Render, Styled as _, TestAppContext, VisualTestContext, Window, WindowBounds, WindowOptions,
    div, px, size,
};
use hoshi_core::{AccessMode, Command, Editor, GitAction, GitActionRequest, ThreadWorkspace};
use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Chat {
    details: Entity<ThreadDetails>,
}

impl Render for Chat {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let toggle = self
            .details
            .update(cx, |details, cx| details.header_toggle(cx));
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().h_10().flex().child(toggle))
            .child(div().relative().flex_1().child(self.details.clone()))
    }
}

struct Harness {
    cx: VisualTestContext,
    details: Entity<ThreadDetails>,
}

fn git(dir: &Path, args: &[&str]) {
    let status = Process::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?}");
}

fn repository(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hoshi-native-{name}-{}", uuid::Uuid::new_v4()));
    let project = dir.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(dir.join("home")).unwrap();
    git(&project, &["init", "-q", "-b", "main"]);
    git(&project, &["config", "user.name", "hoshi"]);
    git(&project, &["config", "user.email", "hoshi@example.com"]);
    std::fs::write(project.join("notes.txt"), "one\ntwo\n").unwrap();
    git(&project, &["add", "."]);
    git(&project, &["commit", "-q", "-m", "first"]);
    git(&project, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(project.join("notes.txt"), "one\nthree\nfour\n").unwrap();
    dir
}

fn harness(cx: &mut TestAppContext, name: &str, width: f32) -> Harness {
    cx.executor().allow_parking();
    let dir = repository(name);
    let (notify, notifications) = state::channel();
    let engine = Arc::new(hoshi_engine::Engine::open(&dir.join("home"), notify).unwrap());
    engine
        .dispatch(Command::ProjectCreate {
            project_id: "p".into(),
            title: "project".into(),
            workspace_root: dir.join("project").to_string_lossy().into_owned(),
        })
        .unwrap();
    engine
        .dispatch(Command::ThreadCreate {
            thread_id: "t".into(),
            project_id: Some("p".into()),
            title: "thread".into(),
            workspace: ThreadWorkspace::Local,
            provider: hoshi_core::default_provider(),
            model: None,
            effort: None,
            fast_mode: false,
            ultracode: false,
            access_mode: AccessMode::default(),
            plan_mode: false,
        })
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        hoshi_ui::theme::apply(ThemeMode::Dark, cx);
    });
    let workspace = cx.update(|cx| cx.new(|cx| Workspace::new(engine, notifications, cx)));
    workspace.update(cx, |workspace, cx| {
        workspace.settings_path = Some(dir.join("home").join("settings.json"));
        workspace.open_thread("t", cx)
    });
    let (window, details) = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(width), px(600.)),
            })),
            ..Default::default()
        };
        let mut details = None;
        let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
            let shown = cx.new(|cx| ThreadDetails::new(workspace.clone(), window, cx));
            details = Some(shown.clone());
            cx.new(|_| Chat { details: shown })
        })
        .unwrap();
        (window, details.unwrap())
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    redraw(&mut cx);
    Harness { cx, details }
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
    }
}

fn wait_for(h: &mut Harness, what: &str, done: impl Fn(&ThreadDetails) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        redraw(&mut h.cx);
        if h.details.read_with(&h.cx, |details, _| done(details)) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[gpui_kit::test]
fn the_card_shows_the_checkout_of_a_wide_chat(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-card", 1200.);
    wait_for(&mut h, "the checkout", |details| {
        details.changes.contains_key("t") && details.statuses.contains_key("t")
    });

    let card =
        h.cx.debug_bounds("thread-details-card")
            .expect("the card shows");
    assert_eq!(card.size.width, px(super::CARD_WIDTH));
    assert!(card.origin.x > px(800.), "card: {card:?}");
    h.details.read_with(&h.cx, |details, _| {
        assert!(details.is_inline());
        let changes = &details.changes["t"];
        assert_eq!(
            (changes.files, changes.additions, changes.deletions),
            (1, 2, 1)
        );
        let status = &details.statuses["t"];
        assert_eq!(status.branch.as_deref(), Some("feature"));
        assert!(status.has_changes);
    });
}

#[gpui_kit::test]
fn the_editor_menu_drops_under_its_whole_row(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-editor-menu", 1200.);
    h.details.update(&mut h.cx, |details, cx| {
        details.editors = Some(vec![Editor::Vscode, Editor::FileManager]);
        details.set_open(true, cx);
    });
    redraw(&mut h.cx);

    let row =
        h.cx.debug_bounds("details-editor-row")
            .expect("the editor row shows");
    let arrow =
        h.cx.debug_bounds("details-editor-arrow")
            .expect("the arrow shows");
    h.cx.simulate_click(arrow.center(), Modifiers::default());
    redraw(&mut h.cx);

    let menu = h.cx.debug_bounds("popover-popup").expect("the menu opens");
    assert_eq!(menu.origin.x, row.origin.x, "menu: {menu:?}, row: {row:?}");
    assert_eq!(
        menu.size.width, row.size.width,
        "menu: {menu:?}, row: {row:?}"
    );
    assert!(
        menu.origin.y >= row.bottom(),
        "menu: {menu:?}, row: {row:?}"
    );
}

#[gpui_kit::test]
fn the_git_menu_drops_under_the_whole_branch_row(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-git-menu-width", 1200.);
    h.details
        .update(&mut h.cx, |details, cx| details.set_open(true, cx));
    wait_for(&mut h, "the checkout", |details| {
        details.statuses.contains_key("t")
    });

    let row =
        h.cx.debug_bounds("details-branch-row")
            .expect("the branch row shows");
    let arrow =
        h.cx.debug_bounds("details-git-arrow")
            .expect("the git menu's arrow shows");
    h.cx.simulate_click(arrow.center(), Modifiers::default());
    redraw(&mut h.cx);

    let menu = h.cx.debug_bounds("popover-popup").expect("the menu opens");
    assert_eq!(menu.origin.x, row.origin.x, "menu: {menu:?}, row: {row:?}");
    assert_eq!(
        menu.size.width, row.size.width,
        "menu: {menu:?}, row: {row:?}"
    );
    assert!(
        menu.origin.y >= row.bottom(),
        "menu: {menu:?}, row: {row:?}"
    );
}

#[gpui_kit::test]
fn closing_the_card_holds_for_that_thread_only_and_across_restarts(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-closed", 1200.);
    let workspace = h
        .details
        .read_with(&h.cx, |details, _| details.workspace.clone());
    assert!(h.cx.debug_bounds("thread-details-card").is_some());

    h.details
        .update(&mut h.cx, |details, cx| details.toggle(cx));
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("thread-details-card").is_none());

    workspace.update(&mut h.cx, |workspace, cx| {
        workspace.start_draft(Some("p".into()), cx)
    });
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("thread-details-card").is_some());

    workspace.update(&mut h.cx, |workspace, cx| workspace.open_thread("t", cx));
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("thread-details-card").is_none());

    let home = workspace.read_with(&h.cx, |workspace, _| {
        workspace
            .settings_path
            .clone()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    });
    let closed = super::prefs::load(&home).closed_threads;
    assert_eq!(closed.into_iter().collect::<Vec<_>>(), vec!["t".to_owned()]);
    let reopened =
        h.cx.update(|window, cx| cx.new(|cx| ThreadDetails::new(workspace.clone(), window, cx)));
    assert!(!reopened.read_with(&h.cx, |details, cx| details.is_open(cx)));
}

#[gpui_kit::test]
fn a_narrow_chat_shows_the_details_from_a_popover(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-narrow", 700.);
    h.details
        .update(&mut h.cx, |details, cx| details.set_open(true, cx));
    redraw(&mut h.cx);

    assert!(!h.details.read_with(&h.cx, |details, _| details.is_inline()));
    assert!(h.cx.debug_bounds("thread-details-card").is_none());

    h.details.update(&mut h.cx, |details, cx| {
        details.popover_open = true;
        cx.notify();
    });
    wait_for(&mut h, "the checkout", |details| {
        details.changes.contains_key("t")
    });
    assert!(h.cx.debug_bounds("popover-popup").is_some());
}

#[gpui_kit::test]
fn the_git_action_commits_and_says_so_in_place_of_the_button(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-commit", 1200.);
    h.details
        .update(&mut h.cx, |details, cx| details.set_open(true, cx));
    wait_for(&mut h, "the checkout", |details| {
        details.statuses.contains_key("t")
    });
    let quick = h.details.read_with(&h.cx, |details, _| {
        super::git::quick_action(&details.statuses["t"], false)
    });
    assert_eq!(quick.action, GitAction::Commit);

    let request = GitActionRequest {
        message: Some("Note three and four".into()),
        ..h.details
            .read_with(&h.cx, |details, _| details.quick_request(quick.action))
    };
    h.details.update_in(&mut h.cx, |details, window, cx| {
        details.run_git("t".into(), request, window, cx)
    });
    wait_for(&mut h, "the commit", |details| {
        details.done.contains_key("t")
    });
    wait_for(&mut h, "a clean checkout", |details| {
        details
            .statuses
            .get("t")
            .is_some_and(|status| !status.has_changes)
    });

    h.details.read_with(&h.cx, |details, _| {
        let done = &details.done["t"];
        assert!(done.commit.is_some());
        assert_eq!(done.subject.as_deref(), Some("Note three and four"));
        assert!(details.running.is_empty());
    });
}

#[gpui_kit::test]
fn the_main_git_action_waits_a_moment_in_which_it_can_be_undone(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-undo", 1200.);
    h.details
        .update(&mut h.cx, |details, cx| details.set_open(true, cx));
    wait_for(&mut h, "the checkout", |details| {
        details.statuses.contains_key("t")
    });
    let press = |h: &mut Harness| {
        h.details.update_in(&mut h.cx, |details, window, cx| {
            let quick = super::git::quick_action(&details.statuses["t"], false);
            let request = GitActionRequest {
                message: Some("Note three and four".into()),
                ..details.quick_request(quick.action)
            };
            details.start_git(request, Some(quick.label), window, cx)
        });
        redraw(&mut h.cx);
    };

    press(&mut h);
    assert!(h.cx.debug_bounds("details-git-pending").is_some());
    let undo =
        h.cx.debug_bounds("details-git-undo")
            .expect("the undo button shows");
    h.cx.simulate_click(undo.center(), Modifiers::default());
    h.cx.executor().advance_clock(super::UNDO_FOR);
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("details-git-pending").is_none());
    h.details.read_with(&h.cx, |details, _| {
        assert!(details.pending_git.is_empty());
        assert!(details.running.is_empty() && details.done.is_empty());
        assert!(details.statuses["t"].has_changes, "nothing was committed");
    });

    press(&mut h);
    h.cx.executor().advance_clock(super::UNDO_FOR);
    wait_for(&mut h, "the commit", |details| {
        details.done.contains_key("t")
    });
    assert!(h.cx.debug_bounds("details-git-pending").is_none());
}

#[gpui_kit::test]
fn a_disabled_git_action_says_why_on_hover(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-git-why", 1200.);
    h.details
        .update(&mut h.cx, |details, cx| details.set_open(true, cx));
    wait_for(&mut h, "the checkout", |details| {
        details.statuses.contains_key("t")
    });
    let arrow =
        h.cx.debug_bounds("details-git-arrow")
            .expect("the git menu's arrow shows");
    h.cx.simulate_click(arrow.center(), Modifiers::default());
    redraw(&mut h.cx);

    let push =
        h.cx.debug_bounds("menu-item-details-git-push")
            .expect("the push item shows");
    h.cx.simulate_mouse_move(push.center(), None, Modifiers::default());
    redraw(&mut h.cx);
    h.cx.executor().advance_clock(Duration::from_millis(700));
    redraw(&mut h.cx);
    let tooltip = h.cx.debug_bounds("tooltip").expect("the reason shows");
    assert!(
        tooltip.right() <= push.left(),
        "tooltip: {tooltip:?}, item: {push:?}"
    );
}

#[gpui_kit::test]
fn many_previous_agents_stay_in_the_show_all_menu(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-many-agents", 1200.);
    let workspace = h
        .details
        .read_with(&h.cx, |details, _| details.workspace.clone());
    let open = workspace
        .read_with(&h.cx, |workspace, _| workspace.open.clone())
        .expect("the thread is open");
    let deadline = Instant::now() + Duration::from_secs(30);
    while open.read_with(&h.cx, |thread, _| thread.loading) {
        assert!(Instant::now() < deadline, "timed out loading the thread");
        redraw(&mut h.cx);
        std::thread::sleep(Duration::from_millis(20));
    }
    let count = 50;
    open.update(&mut h.cx, |thread, cx| {
        for index in 0..count {
            thread.items.push(hoshi_core::ThreadItem {
                id: format!("call-{index}"),
                thread_id: "t".into(),
                created_at: "2026-11-13T10:00:00Z".into(),
                content: hoshi_core::ItemContent::ToolCall {
                    name: "Agent".into(),
                    input: r#"{"description":"Look around","subagent_type":"Explore"}"#.into(),
                    output: Some("done".into()),
                    is_error: false,
                    detail: None,
                    output_omitted: false,
                    interrupted: false,
                },
            });
        }
        cx.notify();
    });
    h.details
        .update(&mut h.cx, |details, cx| details.set_open(true, cx));
    redraw(&mut h.cx);

    let rows = |h: &mut Harness| {
        (0..count)
            .filter(|index| {
                h.cx.debug_bounds(Box::leak(
                    format!("details-agent-call-{index}").into_boxed_str(),
                ))
                .is_some()
            })
            .count()
    };
    assert_eq!(rows(&mut h), 0, "done agents get no row of their own");
    assert!(h.cx.debug_bounds("details-agents-summary").is_some());

    let show_all =
        h.cx.debug_bounds("details-agents-show-all")
            .expect("the show all button shows");
    h.cx.simulate_click(show_all.center(), Modifiers::default());
    redraw(&mut h.cx);
    assert!(
        h.cx.debug_bounds("menu-item-details-agent-option-call-0")
            .is_some(),
        "the menu lists the previous agents"
    );
}

#[gpui_kit::test]
fn the_first_servers_get_a_row_and_the_rest_a_menu(cx: &mut TestAppContext) {
    use hoshi_core::{LocalServer, ServerOrigin};
    let mut h = harness(cx, "details-servers", 1200.);
    let workspace = h
        .details
        .read_with(&h.cx, |details, _| details.workspace.clone());
    let servers: Vec<LocalServer> = [5173, 8080, 6006, 3000, 9229]
        .into_iter()
        .enumerate()
        .map(|(index, port)| LocalServer {
            port,
            pid: 4000 + index as u32,
            process_name: Some("node".into()),
            origin: ServerOrigin::Terminal,
            started_at: None,
            web: true,
        })
        .collect();
    workspace.update(&mut h.cx, |workspace, cx| {
        workspace.servers.insert("t".into(), servers);
        cx.notify();
    });
    h.details
        .update(&mut h.cx, |details, cx| details.set_open(true, cx));
    redraw(&mut h.cx);

    for shown in ["4000:5173", "4001:8080", "4002:6006"] {
        let selector = format!("details-server-{shown}");
        assert!(
            h.cx.debug_bounds(Box::leak(selector.into_boxed_str()))
                .is_some(),
            "{shown} has a row"
        );
    }
    assert!(h.cx.debug_bounds("details-server-4003:3000").is_none());
    let panel =
        h.cx.debug_bounds("details-servers")
            .expect("the servers box shows");
    let first =
        h.cx.debug_bounds("details-server-4000:5173")
            .expect("the first server shows");
    h.cx.simulate_mouse_move(first.center(), None, Modifiers::default());
    redraw(&mut h.cx);
    h.cx.executor().advance_clock(Duration::from_millis(700));
    redraw(&mut h.cx);
    let tooltip = h.cx.debug_bounds("tooltip").expect("the detail shows");
    assert!(
        tooltip.right() < panel.left(),
        "tooltip: {tooltip:?}, box: {panel:?}"
    );

    let more =
        h.cx.debug_bounds("details-servers-more")
            .expect("the rest fold behind +2 more");
    h.cx.simulate_click(more.center(), Modifiers::default());
    redraw(&mut h.cx);
    assert!(
        h.cx.debug_bounds("details-server-4003:3000").is_some(),
        "the popover shows the servers past the first ones as rows"
    );
    assert!(
        h.cx.debug_bounds("details-server-stop-4003:3000").is_some(),
        "they can be stopped from there too"
    );
    let menu =
        h.cx.debug_bounds("details-servers-more-list")
            .expect("the popover shows");
    assert!(
        menu.right() < panel.left(),
        "menu: {menu:?}, box: {panel:?}"
    );
}

#[gpui_kit::test]
fn a_subagent_row_asks_to_show_its_call_in_the_transcript(cx: &mut TestAppContext) {
    use super::DetailsEvent;
    use hoshi_core::{ItemContent, ThreadItem};
    use std::cell::RefCell;
    use std::rc::Rc;

    let mut h = harness(cx, "details-subagent", 1200.);
    let workspace = h
        .details
        .read_with(&h.cx, |details, _| details.workspace.clone());
    let open = workspace
        .read_with(&h.cx, |workspace, _| workspace.open.clone())
        .expect("the thread is open");
    let deadline = Instant::now() + Duration::from_secs(30);
    while open.read_with(&h.cx, |thread, _| thread.loading) {
        assert!(Instant::now() < deadline, "timed out loading the thread");
        redraw(&mut h.cx);
        std::thread::sleep(Duration::from_millis(20));
    }
    open.update(&mut h.cx, |thread, cx| {
        thread.items.push(ThreadItem {
            id: "call-1".into(),
            thread_id: "t".into(),
            created_at: "2026-11-13T10:00:00Z".into(),
            content: ItemContent::ToolCall {
                name: "Agent".into(),
                input: r#"{"description":"Look around","subagent_type":"Explore"}"#.into(),
                output: None,
                is_error: false,
                detail: None,
                output_omitted: false,
                interrupted: false,
            },
        });
        cx.notify();
    });
    let shown = Rc::new(RefCell::new(Vec::new()));
    let _events = h.cx.update({
        let shown = shown.clone();
        let details = h.details.clone();
        move |_, cx| {
            cx.subscribe(&details, move |_, event: &DetailsEvent, _| {
                shown.borrow_mut().push(event.clone())
            })
        }
    });
    h.details
        .update(&mut h.cx, |details, cx| details.set_open(true, cx));
    redraw(&mut h.cx);

    let row =
        h.cx.debug_bounds("details-agent-call-1")
            .expect("the subagent's row shows");
    let provider = hoshi_core::default_provider();
    let mark = format!("details-agent-mark-call-1-{provider}");
    assert!(
        h.cx.debug_bounds(Box::leak(mark.into_boxed_str()))
            .is_some(),
        "the row shows the thread's provider mark"
    );
    assert!(
        h.cx.debug_bounds("details-agent-dot-call-1").is_some(),
        "the row keeps its status dot"
    );
    h.cx.simulate_click(row.center(), Modifiers::default());
    redraw(&mut h.cx);

    assert_eq!(
        *shown.borrow(),
        [DetailsEvent::ShowItem {
            thread_id: "t".into(),
            item_id: "call-1".into(),
        }]
    );
}

#[gpui_kit::test]
fn the_card_shows_the_plans_extra_usage_even_without_windows(cx: &mut TestAppContext) {
    use hoshi_core::{ExtraUsage, SpendUnit, UsageLimits};
    let mut h = harness(cx, "details-extra", 1200.);
    h.details
        .update(&mut h.cx, |details, cx| details.set_open(true, cx));
    let workspace = h
        .details
        .read_with(&h.cx, |details, _| details.workspace.clone());
    let set_extra = |extra: Option<ExtraUsage>| {
        move |workspace: &mut Workspace, cx: &mut Context<Workspace>| {
            workspace.providers.usage_limits.insert(
                hoshi_core::DEFAULT_PROVIDER.to_owned(),
                UsageLimits {
                    windows: Vec::new(),
                    banked_resets: None,
                    account: None,
                    extra_usage: extra,
                },
            );
            workspace.set_usage_limits_in_details(true, cx);
        }
    };
    workspace.update(&mut h.cx, set_extra(None));
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("details-usage").is_none());

    workspace.update(
        &mut h.cx,
        set_extra(Some(ExtraUsage {
            unit: SpendUnit::Credits { exponent: 2 },
            used: None,
            limit: None,
            balance: Some(51_250),
            unlimited: false,
            resets_at: None,
        })),
    );
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("details-usage").is_some());
    assert!(h.cx.debug_bounds("details-extra").is_some());
}

#[gpui_kit::test]
fn the_card_shows_the_threads_usage_limits_only_once_the_user_turns_them_on(
    cx: &mut TestAppContext,
) {
    use hoshi_core::{UsageLimits, UsageWindow, UsageWindowKind};
    let mut h = harness(cx, "details-usage", 1200.);
    h.details
        .update(&mut h.cx, |details, cx| details.set_open(true, cx));
    wait_for(&mut h, "the checkout", |details| {
        details.statuses.contains_key("t")
    });
    let window = |kind, used_percent| UsageWindow {
        kind,
        model: None,
        used_percent,
        window_minutes: 300,
        resets_at: None,
    };
    let workspace = h
        .details
        .read_with(&h.cx, |details, _| details.workspace.clone());
    workspace.update(&mut h.cx, |workspace, cx| {
        workspace.providers.usage_limits.insert(
            hoshi_core::DEFAULT_PROVIDER.to_owned(),
            UsageLimits {
                windows: vec![
                    window(UsageWindowKind::Session, 18),
                    window(UsageWindowKind::Weekly, 83),
                ],
                banked_resets: None,
                account: None,
                extra_usage: None,
            },
        );
        cx.notify();
    });
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("details-usage-0").is_none());

    workspace.update(&mut h.cx, |workspace, cx| {
        workspace.set_usage_limits_in_details(true, cx)
    });
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("details-usage-0").is_some());
    assert!(h.cx.debug_bounds("details-usage-1").is_some());

    workspace.update(&mut h.cx, |workspace, cx| {
        workspace.providers.accounts.insert(
            hoshi_core::DEFAULT_PROVIDER.to_owned(),
            hoshi_core::ProviderAccountStatus::SignedOut,
        );
        cx.notify();
    });
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("details-usage-0").is_none());

    workspace.update(&mut h.cx, |workspace, cx| {
        workspace.providers.accounts.clear();
        workspace.settings.providers.claude_code.enabled = false;
        cx.notify();
    });
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("details-usage-0").is_none());
    workspace.update(&mut h.cx, |workspace, cx| {
        workspace.settings.providers.claude_code.enabled = true;
        cx.notify();
    });
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("details-usage-0").is_some());

    workspace.update(&mut h.cx, |workspace, cx| {
        workspace.providers.accounts.insert(
            hoshi_core::DEFAULT_PROVIDER.to_owned(),
            hoshi_core::ProviderAccountStatus::NotInstalled,
        );
        cx.notify();
    });
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("details-usage-0").is_none());

    workspace.update(&mut h.cx, |workspace, cx| {
        workspace.providers.accounts.clear();
        workspace.settings.providers.codex.added = true;
        workspace.settings.providers.codex.enabled = true;
        workspace.providers.usage_limits.insert(
            "codex".to_owned(),
            UsageLimits {
                windows: vec![window(UsageWindowKind::Weekly, 10)],
                banked_resets: None,
                account: None,
                extra_usage: None,
            },
        );
        let pending = |choice| hoshi_core::Event::ThreadModelPending {
            thread_id: "t".into(),
            choice,
        };
        workspace
            .model
            .apply(&pending(Some(hoshi_core::ModelChoice {
                provider: "codex".into(),
                model: None,
                effort: None,
                fast_mode: false,
                ultracode: false,
            })));
        cx.notify();
    });
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("details-usage-0").is_some());
    assert!(
        h.cx.debug_bounds("details-usage-1").is_none(),
        "the pending provider's one window"
    );
    workspace.update(&mut h.cx, |workspace, cx| {
        workspace
            .model
            .apply(&hoshi_core::Event::ThreadModelPending {
                thread_id: "t".into(),
                choice: None,
            });
        workspace
            .providers
            .usage_limits
            .get_mut(hoshi_core::DEFAULT_PROVIDER)
            .unwrap()
            .windows
            .clear();
        cx.notify();
    });
    redraw(&mut h.cx);
    assert!(
        h.cx.debug_bounds("details-usage-0").is_none(),
        "no windows, no section"
    );

    workspace.update(&mut h.cx, |workspace, cx| {
        workspace.providers.usage_limits.clear();
        workspace.providers.usage_limits.insert(
            "codex".to_owned(),
            UsageLimits {
                windows: vec![window(UsageWindowKind::Weekly, 10)],
                banked_resets: None,
                account: None,
                extra_usage: None,
            },
        );
        cx.notify();
    });
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("details-usage-0").is_none());
}

fn draft_workspace(h: &mut Harness) -> Entity<Workspace> {
    use hoshi_core::{UsageLimits, UsageWindow, UsageWindowKind};
    let workspace = h
        .details
        .read_with(&h.cx, |details, _| details.workspace.clone());
    h.details.update(&mut h.cx, |details, cx| {
        details.editors = Some(vec![Editor::Vscode]);
        details.set_open(true, cx);
    });
    workspace.update(&mut h.cx, |workspace, cx| {
        workspace.settings.usage_limits_in_details = true;
        workspace.settings.providers.codex.added = true;
        workspace.settings.providers.codex.enabled = true;
        workspace.providers.usage_limits.insert(
            crate::providers::CODEX.to_owned(),
            UsageLimits {
                windows: vec![UsageWindow {
                    kind: UsageWindowKind::Session,
                    model: None,
                    used_percent: 40,
                    window_minutes: 300,
                    resets_at: None,
                }],
                banked_resets: None,
                account: None,
                extra_usage: None,
            },
        );
        workspace.start_draft(Some("p".into()), cx);
        workspace.choices.model.provider = crate::providers::CODEX.to_owned();
        cx.notify();
    });
    workspace
}

fn wait_for_workspace(h: &mut Harness, what: &str, done: impl Fn(&Workspace) -> bool) {
    let workspace = h
        .details
        .read_with(&h.cx, |details, _| details.workspace.clone());
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        redraw(&mut h.cx);
        if workspace.read_with(&h.cx, |workspace, _| done(workspace)) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[gpui_kit::test]
fn a_new_thread_in_a_project_shows_the_card_without_a_thread(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-draft", 1200.);
    let workspace = draft_workspace(&mut h);
    wait_for_workspace(&mut h, "the project's branch", |workspace| {
        workspace
            .project_branches
            .get("p")
            .cloned()
            .flatten()
            .as_deref()
            == Some("feature")
    });

    assert!(h.cx.debug_bounds("thread-details-card").is_some());
    assert!(h.cx.debug_bounds("details-editor-row").is_some());
    assert!(h.cx.debug_bounds("details-draft-branch").is_some());
    assert!(h.cx.debug_bounds("details-git-arrow").is_none());
    assert!(h.cx.debug_bounds("details-usage-0").is_some());

    workspace.update(&mut h.cx, |workspace, cx| {
        workspace.choices.model.provider = hoshi_core::DEFAULT_PROVIDER.to_owned();
        cx.notify();
    });
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("details-usage-0").is_none());

    workspace.update(&mut h.cx, |workspace, cx| workspace.start_draft(None, cx));
    wait_for(&mut h, "the new chat's folder", |details| {
        details.chat_folders.len() == 1
    });
    assert!(h.cx.debug_bounds("thread-details-card").is_some());
    let draft_id = workspace.read_with(&h.cx, |workspace, _| {
        workspace.draft.as_ref().unwrap().thread_id.clone()
    });
    let folder = h.details.read_with(&h.cx, |details, _| {
        details.chat_folders.get(&draft_id).cloned()
    });
    assert!(folder.is_some_and(|folder| folder.ends_with(&draft_id)));
    assert!(h.cx.debug_bounds("details-editor-row").is_some());
    assert!(h.cx.debug_bounds("details-draft-branch").is_none());
}

#[gpui_kit::test]
fn a_new_threads_card_goes_with_its_project(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-draft-removed", 1200.);
    let workspace = draft_workspace(&mut h);
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("thread-details-card").is_some());

    let engine = workspace.read_with(&h.cx, |workspace, _| workspace.engine.clone());
    engine
        .dispatch(Command::ProjectRemove {
            project_id: "p".into(),
        })
        .unwrap();
    wait_for_workspace(&mut h, "the removal", |workspace| {
        workspace.model.project("p").is_none()
    });
    assert!(h.cx.debug_bounds("thread-details-card").is_none());
    h.details.update(&mut h.cx, |details, cx| {
        assert!(!details.open_in_target(cx));
    });
}

#[gpui_kit::test]
fn an_error_stays_with_the_new_thread_it_happened_in(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-draft-error", 1200.);
    let workspace = draft_workspace(&mut h);
    h.details.update(&mut h.cx, |details, cx| {
        details.error = Some("failed".into());
        cx.notify();
    });
    workspace.update(&mut h.cx, |workspace, cx| workspace.start_draft(None, cx));
    redraw(&mut h.cx);
    workspace.update(&mut h.cx, |workspace, cx| {
        workspace.start_draft(Some("p".into()), cx)
    });
    redraw(&mut h.cx);
    assert!(
        h.details
            .read_with(&h.cx, |details, _| details.error.is_none())
    );
}

#[gpui_kit::test]
fn a_narrow_new_thread_shows_the_details_from_a_popover(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-draft-narrow", 700.);
    draft_workspace(&mut h);
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("thread-details-card").is_none());

    h.details.update(&mut h.cx, |details, cx| {
        details.popover_open = true;
        cx.notify();
    });
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("popover-popup").is_some());
    assert!(h.cx.debug_bounds("details-editor-row").is_some());
}

#[gpui_kit::test]
fn a_late_failure_stays_off_the_next_new_thread(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-draft-late", 1200.);
    let workspace = draft_workspace(&mut h);
    redraw(&mut h.cx);
    h.details.update(&mut h.cx, |details, cx| {
        details.run(
            || {
                Err(hoshi_core::CommandError::Internal {
                    detail: "failed".into(),
                })
            },
            cx,
        )
    });
    workspace.update(&mut h.cx, |workspace, cx| workspace.start_draft(None, cx));
    redraw(&mut h.cx);
    assert!(
        h.details
            .read_with(&h.cx, |details, _| details.error.is_none())
    );

    workspace.update(&mut h.cx, |workspace, cx| {
        workspace.start_draft(Some("p".into()), cx)
    });
    redraw(&mut h.cx);
    h.details.update(&mut h.cx, |details, cx| {
        details.run(
            || {
                Err(hoshi_core::CommandError::Internal {
                    detail: "failed".into(),
                })
            },
            cx,
        )
    });
    redraw(&mut h.cx);
    assert!(
        h.details
            .read_with(&h.cx, |details, _| details.error.is_some())
    );
}

#[gpui_kit::test]
fn a_failure_from_the_shortcut_opens_the_card_to_tell_it(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-shortcut-failure", 1200.);
    h.details
        .update(&mut h.cx, |details, cx| details.set_open(false, cx));
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("thread-details-card").is_none());
    h.details.update(&mut h.cx, |details, cx| {
        details.run(
            || {
                Err(hoshi_core::CommandError::Internal {
                    detail: "failed".into(),
                })
            },
            cx,
        )
    });
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("thread-details-card").is_some());
    assert!(
        h.details
            .read_with(&h.cx, |details, _| details.error.is_some())
    );
}

#[gpui_kit::test]
fn only_the_latest_launch_tells_its_failure(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-superseded", 1200.);
    h.details.update(&mut h.cx, |details, cx| {
        details.set_open(false, cx);
        details.run(
            || {
                Err(hoshi_core::CommandError::Internal {
                    detail: "failed".into(),
                })
            },
            cx,
        );
        details.run(|| Ok(()), cx);
    });
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("thread-details-card").is_none());
    assert!(
        h.details
            .read_with(&h.cx, |details, _| details.error.is_none())
    );
}

#[gpui_kit::test]
fn the_compact_button_shows_whether_or_not_compacting_is_worth_it_yet(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-compact", 1200.);
    let workspace = h
        .details
        .read_with(&h.cx, |details, _| details.workspace.clone());
    for used_tokens in [3_000, 150_000] {
        workspace.update(&mut h.cx, |workspace, cx| {
            workspace.context_usages.insert(
                "t".into(),
                hoshi_core::ContextUsage {
                    used_tokens,
                    max_tokens: Some(200_000),
                    cache_ttl_secs: None,
                    cache_expires_at: None,
                },
            );
            cx.notify();
        });
        redraw(&mut h.cx);
        assert!(
            h.cx.debug_bounds("details-compact").is_some(),
            "the button shows at {used_tokens} tokens"
        );
    }

    workspace.update(&mut h.cx, |workspace, cx| {
        workspace.set_context_meter(hoshi_core::ContextMeterPlace::Composer, cx)
    });
    redraw(&mut h.cx);
    assert!(
        h.cx.debug_bounds("details-context").is_none(),
        "the context shows under the composer instead"
    );
}
