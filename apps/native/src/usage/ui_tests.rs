use super::UsageView;
use crate::state::{self, Workspace};
use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{
    AppContext as _, Bounds, Entity, Point, TestAppContext, VisualTestContext, WindowBounds,
    WindowOptions, px, size,
};
use hoshi_core::{
    BankedGrant, BankedResets, DEFAULT_PROVIDER, ExtraUsage, ProviderAccountStatus, ResetKind,
    SpendUnit, UsageLimits, UsageWindow, UsageWindowKind,
};
use std::sync::Arc;

struct Harness {
    cx: VisualTestContext,
    workspace: Entity<Workspace>,
    view: Entity<UsageView>,
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
    let opened = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = opened.clone();
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
            let view = cx.new(|cx| UsageView::new(workspace, cx));
            *slot.borrow_mut() = Some(view.clone());
            view
        })
        .unwrap()
        .0
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    redraw(&mut cx);
    let view = opened.borrow_mut().take().unwrap();
    Harness {
        cx,
        workspace,
        view,
    }
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
        extra_usage: None,
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
                grants: Vec::new(),
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
    assert!(!shows(&mut h, "usage-in-details"));
}

#[gpui_kit::test]
fn the_switch_for_the_threads_card_shows_under_the_providers_with_limits(cx: &mut TestAppContext) {
    let mut h = harness(cx, "usage-in-details");
    let file = std::env::temp_dir()
        .join(format!(
            "hoshi-native-usage-switch-{}",
            uuid::Uuid::new_v4()
        ))
        .join("settings.json");
    h.workspace.update(&mut h.cx, |workspace, _| {
        workspace.settings_path = Some(file.clone())
    });
    assert!(shows(&mut h, "usage-in-details"));
    let switch =
        h.cx.debug_bounds("usage-in-details-switch")
            .unwrap()
            .center();
    h.cx.simulate_click(switch, gpui_kit::Modifiers::none());
    redraw(&mut h.cx);
    assert!(hoshi_app::settings::load(&file).usage_limits_in_details);
    assert!(h.workspace.read_with(&h.cx, |workspace, _| {
        workspace.settings.usage_limits_in_details
    }));
    h.cx.simulate_click(switch, gpui_kit::Modifiers::none());
    redraw(&mut h.cx);
    assert!(!h.workspace.read_with(&h.cx, |workspace, _| {
        workspace.settings.usage_limits_in_details
    }));
    h.workspace.update(&mut h.cx, |workspace, cx| {
        let providers = &mut workspace.settings.providers;
        providers.claude_code.enabled = false;
        providers.codex.added = false;
        cx.notify();
    });
    redraw(&mut h.cx);
    assert!(shows(&mut h, "usage-not-reported-copilot"));
    assert!(!shows(&mut h, "usage-in-details"));
}

fn grant(id: &str, kind: ResetKind) -> BankedGrant {
    BankedGrant {
        id: id.into(),
        kind,
        resets: 1,
        expires_at: None,
    }
}

#[gpui_kit::test]
fn a_session_card_lists_each_banked_grant_to_pick_from(cx: &mut TestAppContext) {
    let mut h = harness(cx, "usage-grants");
    h.workspace.update(&mut h.cx, |workspace, cx| {
        let claude = UsageLimits {
            banked_resets: Some(BankedResets {
                count: 2,
                next_grant_id: Some("session".into()),
                expires_at: Vec::new(),
                grants: vec![
                    grant("session", ResetKind::Session),
                    grant("full", ResetKind::Full),
                ],
            }),
            ..limits(vec![window(UsageWindowKind::Session, None, 100)])
        };
        workspace
            .providers
            .usage_limits
            .insert(CLAUDE.into(), claude);
        cx.notify();
    });
    h.view.update(&mut h.cx, |view, cx| {
        view.card = Some((CLAUDE, 0));
        cx.notify();
    });
    redraw(&mut h.cx);
    assert!(shows(&mut h, "usage-grant-claudeCode-0"));
    assert!(shows(&mut h, "usage-grant-claudeCode-1"));
}

#[gpui_kit::test]
fn extra_usage_is_one_more_row_under_the_windows(cx: &mut TestAppContext) {
    let mut h = harness(cx, "usage-extra");
    h.workspace.update(&mut h.cx, |workspace, cx| {
        let claude = UsageLimits {
            extra_usage: Some(ExtraUsage {
                unit: SpendUnit::Money {
                    currency: Some("USD".into()),
                    exponent: 2,
                },
                used: Some(3_800),
                limit: Some(10_000),
                balance: None,
                unlimited: false,
                resets_at: None,
            }),
            ..limits(vec![window(UsageWindowKind::Session, None, 20)])
        };
        let codex = limits(vec![window(UsageWindowKind::Session, None, 20)]);
        let known = &mut workspace.providers.usage_limits;
        known.insert(CLAUDE.into(), claude);
        known.insert(CODEX.into(), codex);
        cx.notify();
    });
    redraw(&mut h.cx);
    assert!(shows(&mut h, "usage-extra-claudeCode"));
    assert!(!shows(&mut h, "usage-extra-codex"));
}
