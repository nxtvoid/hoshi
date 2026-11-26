use super::agent::AgentState;
use super::{AfterExit, Onboarding, OnboardingEvent, complete};
use crate::look;
use crate::state::{self, Workspace};
use crate::themes::appearance::ThemePreference;
use chrono::Utc;
use gpui_kit::component::theme::ThemeMode;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, IntoElement, Modifiers, ParentElement as _, Point,
    Render, Styled as _, Subscription, TestAppContext, VisualTestContext, Window, WindowBounds,
    WindowOptions, div, px, size,
};
use hoshi_core::{Language, ProviderAccountStatus};
use hoshi_engine::RecentFolder;
use std::path::PathBuf;
use std::sync::Arc;

/// The setup filling the window, doing what the window does when it closes.
struct Host {
    view: Entity<Onboarding>,
    workspace: Entity<Workspace>,
    open: bool,
    exits: Vec<Option<AfterExit>>,
    added: Vec<String>,
    _events: Subscription,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .when(self.open, |this| this.child(self.view.clone()))
    }
}

struct Harness {
    cx: VisualTestContext,
    host: Entity<Host>,
    view: Entity<Onboarding>,
    workspace: Entity<Workspace>,
    settings_path: PathBuf,
    folder: PathBuf,
}

fn harness(cx: &mut TestAppContext) -> Harness {
    cx.executor().allow_parking();
    let dir =
        std::env::temp_dir().join(format!("hoshi-native-onboarding-{}", uuid::Uuid::new_v4()));
    let home = dir.join("home");
    let folder = dir.join("first-project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&folder).unwrap();
    let settings_path = home.join("settings.json");
    let (notify, notifications) = state::channel();
    let engine = Arc::new(hoshi_engine::Engine::open(&home, notify).unwrap());
    cx.update(|cx| {
        gpui_kit::init(cx);
        hoshi_ui::DialogLayer::init(cx);
        hoshi_ui::theme::apply(ThemeMode::Light, cx);
    });
    let path = settings_path.clone();
    let workspace = cx.update(|cx| {
        cx.new(|cx| {
            let mut workspace = Workspace::new(engine, notifications, cx);
            workspace.settings_path = Some(path);
            workspace
        })
    });
    let (window, host) = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1100.), px(900.)),
            })),
            ..Default::default()
        };
        let workspace = workspace.clone();
        gpui_kit::open_window(options, cx, |window, cx| {
            look::init(None, window, cx);
            let view = cx.new(|cx| Onboarding::new(workspace.clone(), cx));
            let focus = view.read(cx).focus.clone();
            window.focus(&focus, cx);
            cx.new(|cx| {
                let events = cx.subscribe(&view, |host: &mut Host, _, event, cx| match event {
                    OnboardingEvent::Exit(then) => {
                        complete(&host.workspace, cx);
                        host.exits.push(*then);
                        host.open = false;
                        cx.notify();
                    }
                    OnboardingEvent::ProjectAdded(id) => host.added.push(id.clone()),
                });
                Host {
                    view,
                    workspace,
                    open: true,
                    exits: Vec::new(),
                    added: Vec::new(),
                    _events: events,
                }
            })
        })
        .unwrap()
    });
    let view = cx.update(|cx| host.read(cx).view.clone());
    let cx = VisualTestContext::from_window(window, cx);
    cx.run_until_parked();
    Harness {
        cx,
        host,
        view,
        workspace,
        settings_path,
        folder,
    }
}

fn click(h: &mut Harness, selector: &'static str) {
    let bounds =
        h.cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is shown"));
    h.cx.simulate_click(bounds.center(), Modifiers::none());
    h.cx.run_until_parked();
}

fn recent(path: &std::path::Path, added: bool) -> RecentFolder {
    RecentFolder {
        path: path.to_string_lossy().into_owned(),
        name: path.file_name().unwrap().to_string_lossy().into_owned(),
        providers: vec!["claudeCode".to_owned()],
        sessions: 2,
        active_at: Utc::now().to_rfc3339(),
        added,
    }
}

fn offer(h: &mut Harness, folders: Vec<RecentFolder>) {
    h.view
        .update(&mut h.cx, |view, cx| view.receive_folders(folders, cx));
    h.cx.run_until_parked();
}

fn set_accounts(h: &mut Harness, accounts: &[(&str, ProviderAccountStatus)]) {
    h.workspace.update(&mut h.cx, |workspace, cx| {
        for (provider, account) in accounts {
            workspace
                .providers
                .accounts
                .insert((*provider).to_owned(), *account);
        }
        cx.notify();
    });
    h.cx.run_until_parked();
}

fn agent_states(h: &mut Harness) -> Vec<(&'static str, AgentState)> {
    h.view.read_with(&h.cx, |view, cx| {
        view.agents(cx)
            .into_iter()
            .map(|(provider, state)| (provider.id, state))
            .collect()
    })
}

fn project_titles(h: &mut Harness) -> Vec<String> {
    h.workspace.read_with(&h.cx, |workspace, _| {
        workspace
            .model
            .projects()
            .map(|project| project.title.clone())
            .collect()
    })
}

#[gpui_kit::test]
fn starting_adds_the_recent_folders_and_saves_the_setup_done(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let folder = h.folder.clone();
    set_accounts(&mut h, &[("claudeCode", ProviderAccountStatus::Ready)]);
    offer(&mut h, vec![recent(&folder, false)]);

    click(&mut h, "onboarding-theme-Dark");
    let preference = h.cx.update(|_, cx| look::appearance(cx).preference);
    assert_eq!(preference, ThemePreference::Dark);

    h.cx.simulate_keystrokes("enter");
    h.cx.run_until_parked();

    assert_eq!(project_titles(&mut h), ["first-project"]);
    let (exits, added) = h
        .host
        .read_with(&h.cx, |host, _| (host.exits.clone(), host.added.clone()));
    assert_eq!(exits, vec![None]);
    assert_eq!(added.len(), 1);
    let shown = h.workspace.read_with(&h.cx, |workspace, _| {
        workspace.settings.onboarding_completed_at.clone()
    });
    assert!(shown.is_some());
    let saved = hoshi_app::settings::load(&h.settings_path);
    assert_eq!(saved.onboarding_completed_at, shown);
}

#[gpui_kit::test]
fn picking_a_language_shows_and_saves_it_without_leaving_the_setup(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    set_accounts(&mut h, &[("claudeCode", ProviderAccountStatus::Ready)]);
    let english = crate::m::onboarding_start();
    let exits = |h: &Harness| h.host.read_with(&h.cx, |host, _| host.exits.len());

    click(&mut h, "select-trigger");
    h.cx.simulate_keystrokes("escape");
    h.cx.run_until_parked();
    assert_eq!(exits(&h), 0, "escape only closes the menu");

    click(&mut h, "select-trigger");
    h.cx.simulate_keystrokes("end enter");
    h.cx.run_until_parked();

    assert_eq!(exits(&h), 0, "enter only picks the language");
    assert_ne!(crate::m::onboarding_start(), english);
    let saved = hoshi_app::settings::load(&h.settings_path);
    assert_eq!(saved.general.language, Language::Spanish);

    h.cx.simulate_keystrokes("escape");
    h.cx.run_until_parked();
    assert_eq!(exits(&h), 1, "the page has its keys back");
}

#[gpui_kit::test]
fn the_review_lists_every_folder_and_unpicks_one(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let other = h.folder.with_file_name("second-project");
    let known = h.folder.with_file_name("known-project");
    for folder in [&other, &known] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let first = h.folder.clone();
    offer(
        &mut h,
        vec![
            recent(&first, false),
            recent(&other, false),
            recent(&known, true),
        ],
    );
    let picked = |h: &mut Harness| h.view.read_with(&h.cx, |view, _| view.to_add().len());
    assert_eq!(picked(&mut h), 2);

    h.view.update(&mut h.cx, |view, cx| {
        view.reviewing = true;
        cx.notify();
    });
    h.cx.run_until_parked();
    assert!(h.cx.debug_bounds("onboarding-folder-2").is_some());
    click(&mut h, "onboarding-folder-1");
    assert_eq!(picked(&mut h), 1);
    click(&mut h, "onboarding-folder-2");
    assert_eq!(picked(&mut h), 1, "a folder already added can't be picked");

    click(&mut h, "onboarding-folders-check");
    assert_eq!(picked(&mut h), 0);
    click(&mut h, "onboarding-folders-check");
    assert_eq!(picked(&mut h), 2);
}

#[gpui_kit::test]
fn without_a_ready_agent_enter_waits_for_one(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    set_accounts(
        &mut h,
        &[
            ("claudeCode", ProviderAccountStatus::NotInstalled),
            ("copilot", ProviderAccountStatus::SignedOut),
        ],
    );
    assert!(h.cx.debug_bounds("onboarding-needs-agent").is_some());
    h.cx.simulate_keystrokes("enter");
    h.cx.run_until_parked();
    let exits = h.host.read_with(&h.cx, |host, _| host.exits.clone());
    assert!(exits.is_empty(), "Enter waits for an agent");

    h.view.update(&mut h.cx, |view, cx| view.start(None, cx));
    h.cx.run_until_parked();
    let exits = h.host.read_with(&h.cx, |host, _| host.exits.clone());
    assert_eq!(exits, vec![None]);
}

#[gpui_kit::test]
fn ready_agents_come_first(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.workspace.update(&mut h.cx, |workspace, cx| {
        let copilot = &mut workspace.settings.providers.copilot;
        (copilot.added, copilot.enabled) = (true, true);
        cx.notify();
    });
    set_accounts(
        &mut h,
        &[
            ("claudeCode", ProviderAccountStatus::NotInstalled),
            ("copilot", ProviderAccountStatus::Ready),
            ("codex", ProviderAccountStatus::SignedOut),
        ],
    );
    assert_eq!(
        agent_states(&mut h),
        [
            ("copilot", AgentState::Ready),
            ("codex", AgentState::SignedOut),
            ("claudeCode", AgentState::NotInstalled),
        ]
    );
    assert!(h.cx.debug_bounds("onboarding-needs-agent").is_none());
}

#[gpui_kit::test]
fn a_setup_terminal_opens_under_its_agent_and_closes(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    set_accounts(
        &mut h,
        &[("claudeCode", ProviderAccountStatus::NotInstalled)],
    );
    h.cx.update(|window, cx| {
        h.view.update(cx, |view, cx| {
            view.start_setup("claudeCode", "exit 0", window, cx)
        })
    });
    h.cx.run_until_parked();
    assert!(
        h.cx.debug_bounds("onboarding-terminal-claudeCode")
            .is_some()
    );

    let terminal_focused = h.cx.update(|window, cx| {
        let view = h.view.read(cx);
        view.setup.is_some() && !view.focus.is_focused(window)
    });
    assert!(terminal_focused, "the terminal takes the keys");

    h.view.update(&mut h.cx, |view, cx| view.close_setup(cx));
    h.cx.run_until_parked();
    assert!(
        h.cx.debug_bounds("onboarding-terminal-claudeCode")
            .is_none()
    );
    h.cx.simulate_keystrokes("escape");
    h.cx.run_until_parked();
    let exits = h.host.read_with(&h.cx, |host, _| host.exits.clone());
    assert_eq!(exits, vec![None], "the page has the keys again");
}

#[gpui_kit::test]
fn an_install_terminal_closes_once_the_agent_only_needs_a_sign_in(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    set_accounts(
        &mut h,
        &[("claudeCode", ProviderAccountStatus::NotInstalled)],
    );
    h.cx.update(|window, cx| {
        h.view.update(cx, |view, cx| {
            view.start_setup("claudeCode", "exit 0", window, cx)
        })
    });
    h.cx.run_until_parked();
    let settle = |h: &mut Harness| {
        h.view.update(&mut h.cx, |view, cx| {
            if let Some(setup) = &mut view.setup {
                setup.ended = true;
            }
            view.settle_setup(cx);
        });
        h.cx.run_until_parked();
    };

    set_accounts(
        &mut h,
        &[("claudeCode", ProviderAccountStatus::NotInstalled)],
    );
    settle(&mut h);
    assert!(
        h.cx.debug_bounds("onboarding-terminal-claudeCode")
            .is_some(),
        "still not installed, the terminal stays"
    );

    set_accounts(&mut h, &[("claudeCode", ProviderAccountStatus::Unknown)]);
    settle(&mut h);
    assert!(
        h.cx.debug_bounds("onboarding-terminal-claudeCode")
            .is_some(),
        "a check that can't tell keeps the terminal"
    );

    set_accounts(&mut h, &[("claudeCode", ProviderAccountStatus::SignedOut)]);
    settle(&mut h);
    assert!(
        h.cx.debug_bounds("onboarding-terminal-claudeCode")
            .is_none()
    );
    assert!(h.view.read_with(&h.cx, |view, _| view.setup.is_none()));
}

#[gpui_kit::test]
fn turning_an_agent_on_saves_it_and_checks_it_again(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.workspace.update(&mut h.cx, |workspace, _| {
        workspace.settings_path = None;
    });
    h.cx.run_until_parked();
    let copilot = |states: Vec<(&'static str, AgentState)>| {
        states
            .into_iter()
            .find(|(id, _)| *id == "copilot")
            .map(|(_, state)| state)
    };
    assert_eq!(copilot(agent_states(&mut h)), Some(AgentState::Off));

    h.view
        .update(&mut h.cx, |view, cx| view.turn_on("copilot", cx));
    h.cx.run_until_parked();

    let enabled = h.workspace.read_with(&h.cx, |workspace, _| {
        let copilot = &workspace.settings.providers.copilot;
        copilot.added && copilot.enabled
    });
    assert!(enabled);
    assert_eq!(copilot(agent_states(&mut h)), Some(AgentState::Unknown));
}

#[gpui_kit::test]
fn escape_skips_the_setup(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let folder = h.folder.clone();
    offer(&mut h, vec![recent(&folder, false)]);
    h.cx.simulate_keystrokes("escape");
    h.cx.run_until_parked();
    let (open, exits) = h
        .host
        .read_with(&h.cx, |host, _| (host.open, host.exits.clone()));
    assert!(!open);
    assert_eq!(exits, vec![None]);
    assert!(project_titles(&mut h).is_empty(), "skipping adds nothing");
    let saved = hoshi_app::settings::load(&h.settings_path);
    assert!(saved.onboarding_completed_at.is_some());
}

#[gpui_kit::test]
fn add_or_clone_adds_the_picked_folders_then_leaves_for_the_palette(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let folder = h.folder.to_string_lossy().into_owned();
    h.view
        .update(&mut h.cx, |view, cx| view.add_opened(folder, cx));
    h.cx.run_until_parked();
    click(&mut h, "onboarding-add-or-clone");
    let exits = h.host.read_with(&h.cx, |host, _| host.exits.clone());
    assert_eq!(exits, vec![Some(AfterExit::AddProject)]);
    assert_eq!(project_titles(&mut h), ["first-project"]);
}

#[gpui_kit::test]
fn a_folder_that_is_gone_says_why_and_keeps_the_setup_open(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let missing = h.folder.join("missing").to_string_lossy().into_owned();
    h.view
        .update(&mut h.cx, |view, cx| view.add_opened(missing, cx));
    h.view.update(&mut h.cx, |view, cx| view.start(None, cx));
    h.cx.run_until_parked();
    let error = h.view.read_with(&h.cx, |view, _| view.error.clone());
    assert!(error.is_some());
    let (exits, added) = h
        .host
        .read_with(&h.cx, |host, _| (host.exits.clone(), host.added.clone()));
    assert!(exits.is_empty());
    assert!(added.is_empty());
}
