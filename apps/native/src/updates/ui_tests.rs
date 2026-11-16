use super::model::{AppUpdate, release_notes_key};
use super::{INSTALLED, UpdatesButton};
use crate::state::{self, Workspace};
use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, IntoElement, Modifiers, ParentElement as _, Point,
    Render, Styled as _, TestAppContext, VisualTestContext, Window, WindowBounds, WindowOptions,
    div, px, size,
};
use hoshi_core::{AppInstaller, AppRelease, ProviderUpdate, ProviderVersion};
use std::sync::Arc;

struct Host(Entity<UpdatesButton>);

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_end()
            .justify_center()
            .child(self.0.clone())
    }
}

struct Harness {
    cx: VisualTestContext,
    workspace: Entity<Workspace>,
    button: Entity<UpdatesButton>,
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
                size: size(px(600.), px(600.)),
            })),
            ..Default::default()
        };
        let workspace = workspace.clone();
        gpui_kit::open_window(options, cx, move |window, cx| {
            let button = cx.new(|cx| UpdatesButton::new(workspace, window, cx));
            cx.new(|_| Host(button))
        })
        .unwrap()
    });
    let (window, host) = window;
    let button = cx.read(|cx| host.read(cx).0.clone());
    let mut cx = VisualTestContext::from_window(window, cx);
    redraw(&mut cx);
    Harness {
        cx,
        workspace,
        button,
    }
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
    }
}

fn set_version(h: &mut Harness, update: ProviderUpdate, update_available: bool) {
    h.workspace.update(&mut h.cx, |workspace, cx| {
        workspace.set_provider_version(
            hoshi_core::default_provider(),
            Some(ProviderVersion {
                installed: Some("2.0.0".into()),
                latest: Some("2.1.0".into()),
                update_available,
                update_command: Some("claude update".into()),
                update,
            }),
        );
        cx.notify();
    });
    redraw(&mut h.cx);
}

fn click(h: &mut Harness, selector: &'static str) {
    let bounds = h.cx.debug_bounds(selector).expect(selector);
    h.cx.simulate_click(bounds.center(), Modifiers::none());
    redraw(&mut h.cx);
}

#[gpui_kit::test]
fn dev_builds_with_nothing_to_update_show_the_check_button(cx: &mut TestAppContext) {
    let mut h = harness(cx, "updates-off");
    assert!(h.cx.debug_bounds("updates-check").is_some());
    assert!(h.cx.debug_bounds("updates-details").is_none());
}

#[gpui_kit::test]
fn an_available_cli_update_opens_its_details(cx: &mut TestAppContext) {
    let mut h = harness(cx, "updates-provider");
    set_version(&mut h, ProviderUpdate::Idle, true);
    assert!(h.cx.debug_bounds("updates-check").is_none());
    assert!(h.cx.debug_bounds("updates-provider").is_none());
    click(&mut h, "updates-details");
    assert!(h.cx.debug_bounds("updates-provider").is_some());
    assert!(h.cx.debug_bounds("updates-app").is_none());
}

#[gpui_kit::test]
fn a_finished_cli_update_stays_until_its_details_close(cx: &mut TestAppContext) {
    let mut h = harness(cx, "updates-done");
    set_version(
        &mut h,
        ProviderUpdate::Running {
            line: None,
            step: None,
        },
        true,
    );
    assert!(h.cx.debug_bounds("updates-details").is_some());
    set_version(&mut h, ProviderUpdate::Succeeded, false);
    assert!(h.cx.debug_bounds("updates-details").is_some());
    click(&mut h, "updates-details");
    assert!(h.cx.debug_bounds("updates-provider").is_some());
    h.cx.simulate_keystrokes("escape");
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("updates-provider").is_none());
    assert!(h.cx.debug_bounds("updates-details").is_none());
    assert!(h.cx.debug_bounds("updates-check").is_some());
}

#[gpui_kit::test]
fn an_app_release_opens_its_page_and_notes(cx: &mut TestAppContext) {
    let mut h = harness(cx, "updates-app");
    let release = AppRelease {
        version: "9.0.0".into(),
        published_at: "2026-11-01T00:00:00Z".into(),
        body: "- Native app\n- Faster sidebar".into(),
        url: "https://github.com/nxtvoid/hoshi/releases/tag/v9.0.0".into(),
        installer: None,
    };
    h.button.update(&mut h.cx, |button, cx| {
        let key = release_notes_key(INSTALLED, &release.version);
        button
            .release_notes
            .insert(key.clone(), vec![release.clone()]);
        button.loading_notes.insert(key);
        button.app = AppUpdate::Available { release };
        cx.notify();
    });
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("updates-check").is_none());
    click(&mut h, "updates-details");
    assert!(h.cx.debug_bounds("updates-app").is_some());
    assert!(h.cx.debug_bounds("updates-release-notes").is_some());
    assert!(h.cx.debug_bounds("updates-provider").is_none());
}

fn installable(version: &str) -> AppRelease {
    let name = format!("hoshi_{version}_x64-setup.exe");
    AppRelease {
        version: version.into(),
        published_at: "2026-11-01T00:00:00Z".into(),
        body: String::new(),
        url: format!("https://github.com/nxtvoid/hoshi/releases/tag/v{version}"),
        installer: Some(AppInstaller {
            url: format!("https://github.com/nxtvoid/hoshi/releases/download/v{version}/{name}"),
            signature_url: format!(
                "https://github.com/nxtvoid/hoshi/releases/download/v{version}/{name}.sig"
            ),
            name,
            size: 1,
        }),
    }
}

fn show(h: &mut Harness, app: AppUpdate) {
    h.button.update(&mut h.cx, |button, cx| {
        button.app = app;
        cx.notify();
    });
    redraw(&mut h.cx);
}

#[gpui_kit::test]
fn an_installable_release_downloads_then_restarts(cx: &mut TestAppContext) {
    let mut h = harness(cx, "updates-install");
    let release = installable("9.0.0");
    show(
        &mut h,
        AppUpdate::Available {
            release: release.clone(),
        },
    );
    click(&mut h, "updates-details");
    assert!(h.cx.debug_bounds("updates-app-install").is_some());
    assert!(h.cx.debug_bounds("updates-app-release-page").is_none());

    show(
        &mut h,
        AppUpdate::Downloading {
            release: release.clone(),
            percent: Some(40),
        },
    );
    assert!(h.cx.debug_bounds("updates-app-downloading").is_some());

    show(
        &mut h,
        AppUpdate::Ready {
            release: release.clone(),
            installer: "hoshi_9.0.0_x64-setup.exe".into(),
        },
    );
    assert!(h.cx.debug_bounds("updates-app-ready").is_some());

    show(&mut h, AppUpdate::Failed { release });
    assert!(h.cx.debug_bounds("updates-app-failed").is_some());
}

#[gpui_kit::test]
fn a_release_without_an_installer_points_at_its_page(cx: &mut TestAppContext) {
    let mut h = harness(cx, "updates-page");
    let mut release = installable("9.0.0");
    release.installer = None;
    show(&mut h, AppUpdate::Available { release });
    click(&mut h, "updates-details");
    assert!(h.cx.debug_bounds("updates-app-release-page").is_some());
}
