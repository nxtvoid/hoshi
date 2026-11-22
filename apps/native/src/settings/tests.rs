use super::{Section, SettingsView};
use crate::state::{self, Workspace};
use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{
    AppContext as _, Bounds, Entity, Point, TestAppContext, VisualTestContext, WindowBounds,
    WindowOptions, px, size,
};
use std::path::PathBuf;
use std::sync::Arc;

struct Harness {
    cx: VisualTestContext,
    view: Entity<SettingsView>,
    host: Entity<Host>,
    workspace: Entity<Workspace>,
    settings_path: PathBuf,
}

/// The view in its page dialog, as the window shows it.
struct Host {
    view: Entity<SettingsView>,
    open: bool,
}

impl gpui_kit::Render for Host {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        cx: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::prelude::FluentBuilder as _;
        use gpui_kit::{ParentElement as _, Styled as _};
        gpui_kit::div().size_full().when(self.open, |this| {
            this.child(
                hoshi_ui::Dialog::new("settings")
                    .page()
                    .open(true)
                    .close_button(false)
                    .on_open_change(cx.listener(|this, change: &hoshi_ui::OpenChange, _, cx| {
                        this.open = change.open;
                        cx.notify();
                    }))
                    .child(self.view.clone()),
            )
        })
    }
}

fn harness(cx: &mut TestAppContext, section: Section) -> Harness {
    cx.executor().allow_parking();
    let home = std::env::temp_dir().join(format!("hoshi-native-settings-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    let settings_path = home.join("settings.json");
    let (notify, notifications) = state::channel();
    let engine = Arc::new(hoshi_engine::Engine::open(&home, notify).unwrap());
    cx.update(|cx| {
        gpui_kit::init(cx);
        hoshi_ui::DialogLayer::init(cx);
        hoshi_ui::theme::apply(ThemeMode::Dark, cx);
    });
    let path = settings_path.clone();
    let workspace = cx.update(|cx| {
        cx.new(|cx| {
            let mut workspace = Workspace::new(engine, notifications, cx);
            workspace.settings_path = Some(path);
            workspace
        })
    });
    let (window, view) = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1100.), px(800.)),
            })),
            ..Default::default()
        };
        let workspace = workspace.clone();
        let mut view = None;
        let (window, host) = gpui_kit::open_window(options, cx, |window, cx| {
            let shown = cx.new(|cx| SettingsView::new(workspace, section, window, cx));
            view = Some(shown.clone());
            cx.new(|_| Host {
                view: shown,
                open: true,
            })
        })
        .unwrap();
        (window, (view.unwrap(), host))
    });
    let (view, host) = view;
    let cx = VisualTestContext::from_window(window, cx);
    cx.run_until_parked();
    Harness {
        cx,
        view,
        host,
        workspace,
        settings_path,
    }
}

#[gpui_kit::test]
fn a_field_saves_its_trimmed_value_on_enter(cx: &mut TestAppContext) {
    let mut h = harness(cx, Section::Providers);
    let field = h.view.read_with(&h.cx, |view, _| view.binary_path.clone());
    field.update_in(&mut h.cx, |field, window, cx| field.focus(window, cx));
    h.cx.simulate_input("  /opt/claude  ");
    h.cx.simulate_keystrokes("enter");
    h.cx.run_until_parked();

    let shown = h.workspace.read_with(&h.cx, |workspace, _| {
        workspace.settings.providers.claude_code.binary_path.clone()
    });
    assert_eq!(shown.as_deref(), Some("/opt/claude"));
    let saved = hoshi_app::settings::load(&h.settings_path);
    assert_eq!(
        saved.providers.claude_code.binary_path.as_deref(),
        Some("/opt/claude")
    );
}

#[gpui_kit::test]
fn clicking_a_provider_shows_its_settings(cx: &mut TestAppContext) {
    let mut h = harness(cx, Section::Providers);
    assert_eq!(
        h.view.read_with(&h.cx, |view, _| view.provider),
        hoshi_core::DEFAULT_PROVIDER
    );

    let bounds = h.cx.debug_bounds("settings-item-provider-copilot").unwrap();
    h.cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    h.cx.run_until_parked();

    assert_eq!(h.view.read_with(&h.cx, |view, _| view.provider), "copilot");
}

#[gpui_kit::test]
fn last_used_in_the_new_thread_model_picker_unfixes_the_model(cx: &mut TestAppContext) {
    let mut h = harness(cx, Section::Threads);
    h.workspace.update(&mut h.cx, |workspace, _| {
        workspace.settings.new_threads.model = Some(hoshi_core::NewThreadModel {
            provider: hoshi_core::default_provider(),
            model: "opus".into(),
            effort: None,
            fast_mode: false,
            ultracode: false,
        });
    });
    h.cx.run_until_parked();

    let trigger = h.cx.debug_bounds("new-thread-model-trigger").unwrap();
    h.cx.simulate_click(trigger.center(), gpui_kit::Modifiers::none());
    h.cx.run_until_parked();
    let last_used = h.cx.debug_bounds("new-thread-model-last-used").unwrap();
    h.cx.simulate_click(last_used.center(), gpui_kit::Modifiers::none());
    h.cx.run_until_parked();

    let shown = h.workspace.read_with(&h.cx, |workspace, _| {
        workspace.settings.new_threads.model.clone()
    });
    assert_eq!(shown, None);
    assert_eq!(
        hoshi_app::settings::load(&h.settings_path)
            .new_threads
            .model,
        None
    );
}

#[gpui_kit::test]
fn snooze_options_save_as_they_change_and_refuse_what_menus_cannot_use(cx: &mut TestAppContext) {
    use super::snooze_options::RowError;
    use hoshi_core::{SnoozePreset, default_snooze_presets};
    let mut h = harness(cx, Section::Threads);
    let saved = |h: &Harness| {
        hoshi_app::settings::load(&h.settings_path)
            .general
            .snooze_presets
    };
    let click = |h: &mut Harness, selector: &'static str| {
        let page = h.cx.debug_bounds("settings-page").unwrap();
        let target = h.cx.debug_bounds(selector).unwrap();
        h.cx.simulate_event(gpui_kit::ScrollEvent {
            position: page.center(),
            delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                px(0.),
                page.center().y - target.center().y,
            )),
            ..Default::default()
        });
        h.cx.run_until_parked();
        let at = h.cx.debug_bounds(selector).unwrap().center();
        h.cx.simulate_mouse_move(at, None, gpui_kit::Modifiers::none());
        h.cx.simulate_click(at, gpui_kit::Modifiers::none());
        h.cx.run_until_parked();
    };
    let options = h
        .view
        .read_with(&h.cx, |view, _| view.snooze_options.clone());

    click(&mut h, "snooze-options-add");
    let mut expected = default_snooze_presets();
    expected.push(SnoozePreset::After { minutes: 30 });
    assert_eq!(saved(&h), expected);

    let field = options.read_with(&h.cx, |options, _| options.amount_field(0));
    field.update_in(&mut h.cx, |field, window, cx| field.focus(window, cx));
    h.cx.simulate_keystrokes("ctrl-a");
    h.cx.simulate_input("800");
    h.cx.run_until_parked();
    let errors = options.read_with(&h.cx, |options, _| options.errors());
    assert_eq!(errors[0], Some(RowError::TooLong));
    assert_eq!(saved(&h), expected, "a duration over 30 days is not kept");

    h.cx.simulate_keystrokes("ctrl-a");
    h.cx.simulate_input("2");
    h.cx.run_until_parked();
    assert_eq!(
        saved(&h),
        expected,
        "typing keeps nothing until the field commits"
    );
    h.cx.simulate_keystrokes("enter");
    h.cx.run_until_parked();
    expected[0] = SnoozePreset::After { minutes: 120 };
    assert_eq!(saved(&h), expected);
    assert_eq!(
        options.read_with(&h.cx, |options, _| options.errors())[0],
        None
    );

    click(&mut h, "snooze-options-reset");
    assert_eq!(saved(&h), default_snooze_presets());
    h.workspace.update(&mut h.cx, |workspace, cx| {
        workspace.settings.general.snooze_presets = vec![SnoozePreset::At {
            day: hoshi_core::SnoozeDay::Today,
            time: "00:00".into(),
        }];
        cx.notify();
    });
    h.cx.run_until_parked();
    assert!(h.cx.debug_bounds("snooze-options-only-custom").is_some());
    assert_eq!(
        options.read_with(&h.cx, |options, _| options.errors().len()),
        1,
        "the rows follow the settings"
    );
}

#[gpui_kit::test]
fn enter_in_the_search_opens_the_first_match(cx: &mut TestAppContext) {
    let mut h = harness(cx, Section::General);
    let search = h.view.read_with(&h.cx, |view, _| view.search.clone());
    search.update_in(&mut h.cx, |search, window, cx| search.focus(window, cx));
    h.cx.simulate_input("zoom");
    h.cx.simulate_keystrokes("enter");
    h.cx.run_until_parked();

    let (section, query) = h.view.read_with(&h.cx, |view, cx| {
        (view.section, view.search.read(cx).value().to_string())
    });
    assert_eq!(section, Section::Browser);
    assert_eq!(query, "");
}

#[gpui_kit::test]
fn escape_clears_the_search_before_it_closes(cx: &mut TestAppContext) {
    let mut h = harness(cx, Section::General);
    let search = h.view.read_with(&h.cx, |view, _| view.search.clone());
    search.update_in(&mut h.cx, |search, window, cx| search.focus(window, cx));
    h.cx.simulate_input("zoom");
    h.cx.simulate_keystrokes("escape");
    h.cx.run_until_parked();
    let query = search.read_with(&h.cx, |search, _| search.value().to_string());
    assert_eq!(query, "");
    assert!(h.host.read_with(&h.cx, |host, _| host.open));

    h.cx.simulate_keystrokes("escape");
    h.cx.run_until_parked();
    assert!(!h.host.read_with(&h.cx, |host, _| host.open));
}
