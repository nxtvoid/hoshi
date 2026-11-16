use super::UsageView;
use crate::state::{self, Workspace};
use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{
    AppContext as _, Bounds, Entity, Point, TestAppContext, VisualTestContext, WindowBounds,
    WindowOptions, px, size,
};
use hoshi_core::{
    BankedResets, DEFAULT_PROVIDER, ProviderAccountStatus, UsageLimits, UsageWindow,
    UsageWindowKind,
};
use std::sync::Arc;

struct Harness {
    cx: VisualTestContext,
    workspace: Entity<Workspace>,
}

fn harness(cx: &mut TestAppContext, name: &str) -> Harness {
    cx.executor().allow_parking();
    let home = std::env::temp_dir().join(format!("hoshi-native-{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    let (notify, notifications) = state::channel();
    let engine = Arc::new(hoshi_engine::Engine::open(&home, notify).unwrap());
    cx.update(|cx| {
        gpui_kit::init(cx);
        hoshi_ui::DialogLayer::init(cx);
        hoshi_ui::theme::apply(ThemeMode::Light, cx);
    });
    let workspace = cx.update(|cx| cx.new(|cx| Workspace::new(engine, notifications, cx)));
    let window = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(900.), px(700.)),
            })),
            ..Default::default()
        };
        let workspace = workspace.clone();
        gpui_kit::open_window(options, cx, move |_, cx| {
            cx.new(|cx| UsageView::new(workspace, cx))
        })
        .unwrap()
        .0
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    redraw(&mut cx);
    Harness { cx, workspace }
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
    }
}

fn window(kind: UsageWindowKind, model: Option<&str>, used_percent: u8) -> UsageWindow {
    UsageWindow {
        kind,
        model: model.map(str::to_owned),
        used_percent,
        window_minutes: 300,
        resets_at: None,
    }
}

const CLAUDE: &str = DEFAULT_PROVIDER;
const CODEX: &str = "codex";

fn limits(windows: Vec<UsageWindow>) -> UsageLimits {
    UsageLimits {
        windows,
        banked_resets: None,
        account: None,
    }
}

fn shows(h: &mut Harness, selector: &'static str) -> bool {
    h.cx.debug_bounds(selector).is_some()
}

#[gpui_kit::test]
fn without_a_provider_each_section_reads_as_unavailable(cx: &mut TestAppContext) {
    let mut h = harness(cx, "usage-unavailable");
    assert!(shows(&mut h, "usage-unavailable-claudeCode"));
    assert!(shows(&mut h, "usage-unavailable-codex"));
    assert!(shows(&mut h, "usage-not-reported-copilot"));
    assert!(!shows(&mut h, "usage-unavailable-copilot"));
    assert!(!shows(&mut h, "usage-row-claudeCode-0"));
}

#[gpui_kit::test]
fn each_provider_has_its_own_rows_and_a_used_up_window_stays(cx: &mut TestAppContext) {
    let mut h = harness(cx, "usage-rows");
    h.workspace.update(&mut h.cx, |workspace, cx| {
        let claude = UsageLimits {
            banked_resets: Some(BankedResets {
                count: 2,
                next_grant_id: Some("grant".into()),
                expires_at: Vec::new(),
            }),
            ..limits(vec![
                window(UsageWindowKind::Session, None, 95),
                window(UsageWindowKind::Weekly, None, 20),
                window(UsageWindowKind::Weekly, Some("Fable"), 0),
            ])
        };
        let codex = limits(vec![
            window(UsageWindowKind::Session, None, 100),
            window(UsageWindowKind::Weekly, None, 40),
        ]);
        let known = &mut workspace.providers.usage_limits;
        known.insert(CLAUDE.into(), claude);
        known.insert(CODEX.into(), codex);
        cx.notify();
    });
    redraw(&mut h.cx);
    for row in [
        "usage-row-claudeCode-0",
        "usage-row-claudeCode-1",
        "usage-row-claudeCode-2",
        "usage-row-codex-0",
        "usage-row-codex-1",
        "usage-left-codex-0",
    ] {
        assert!(shows(&mut h, row), "{row} shows");
    }
    assert!(shows(&mut h, "usage-banked-claudeCode-0"));
    assert!(!shows(&mut h, "usage-banked-claudeCode-1"));
    assert!(!shows(&mut h, "usage-banked-codex-0"));
    assert!(!shows(&mut h, "usage-unavailable-claudeCode"));
    assert!(!shows(&mut h, "usage-unavailable-codex"));
}

#[gpui_kit::test]
fn a_plan_without_windows_says_it_has_no_limits(cx: &mut TestAppContext) {
    let mut h = harness(cx, "usage-none");
    h.workspace.update(&mut h.cx, |workspace, cx| {
        workspace
            .providers
            .usage_limits
            .insert(CODEX.into(), UsageLimits::default());
        cx.notify();
    });
    redraw(&mut h.cx);
    assert!(shows(&mut h, "usage-no-limits-codex"));
    assert!(shows(&mut h, "usage-unavailable-claudeCode"));
}

#[gpui_kit::test]
fn a_provider_that_cannot_run_says_why_in_its_section(cx: &mut TestAppContext) {
    let mut h = harness(cx, "usage-accounts");
    h.workspace.update(&mut h.cx, |workspace, cx| {
        let providers = &mut workspace.providers;
        providers.usage_limits.insert(
            CODEX.into(),
            limits(vec![window(UsageWindowKind::Session, None, 10)]),
        );
        providers
            .accounts
            .insert(CODEX.into(), ProviderAccountStatus::SignedOut);
        providers
            .accounts
            .insert(CLAUDE.into(), ProviderAccountStatus::NotInstalled);
        cx.notify();
    });
    redraw(&mut h.cx);
    assert!(shows(&mut h, "usage-signed-out-codex"));
    assert!(!shows(&mut h, "usage-row-codex-0"));
    assert!(shows(&mut h, "usage-not-installed-claudeCode"));
}

#[gpui_kit::test]
fn with_every_provider_off_the_page_points_to_settings(cx: &mut TestAppContext) {
    let mut h = harness(cx, "usage-off");
    h.workspace.update(&mut h.cx, |workspace, cx| {
        let providers = &mut workspace.settings.providers;
        providers.claude_code.enabled = false;
        providers.copilot.enabled = false;
        providers.codex.added = false;
        cx.notify();
    });
    redraw(&mut h.cx);
    assert!(shows(&mut h, "usage-no-providers"));
    assert!(!shows(&mut h, "usage-not-reported-copilot"));
}
