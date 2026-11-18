//! The settings modal: a nav of sections with search on the left, the section's page on the
//! right. Changes save as they are made, like the web's.

mod about;
pub mod appearance;
mod browser;
mod forges;
mod general;
mod providers;
mod sections;
mod snooze_options;
mod theme_editor;
mod theme_import;
mod threads;

use crate::m;
use crate::state::Workspace;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_core::{AppSettings, ForgeAccount};
use hoshi_ui::prelude::*;
use std::collections::HashSet;

pub use sections::Section;

pub struct SettingsView {
    workspace: Entity<Workspace>,
    section: Section,
    search: Entity<InputState>,
    projects_folder: Entity<InputState>,
    binary_path: Entity<InputState>,
    copilot_binary_path: Entity<InputState>,
    codex: providers::CodexFields,
    model_picker: threads::ModelPicker,
    custom_search_url: Entity<InputState>,
    title_instructions: Entity<TextareaState>,
    snooze_options: Entity<snooze_options::SnoozeOptions>,
    bitbucket: BitbucketForm,
    /// `None` until read from the engine.
    accounts: Option<Vec<ForgeAccount>>,
    checking_providers: bool,
    /// The merged worktrees are being removed at the user's ask.
    cleaning_merged: bool,
    /// The provider whose settings show beside the list.
    provider: &'static str,
    /// The folded "Advanced" groups that are open.
    advanced: HashSet<&'static str>,
    save_failed: bool,
    looks: appearance::AppearanceState,
    _subscriptions: Vec<Subscription>,
}

struct BitbucketForm {
    open: bool,
    email: Entity<InputState>,
    token: Entity<InputState>,
    pending: bool,
    error: Option<String>,
}

impl SettingsView {
    pub fn new(
        workspace: Entity<Workspace>,
        section: Section,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = workspace.read(cx).settings.clone();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(m::search_settings()));
        let projects_folder = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(home_dir())
                .default_value(settings.general.projects_folder.clone().unwrap_or_default())
        });
        let binary_path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(m::claude_code_binary_placeholder())
                .default_value(
                    settings
                        .providers
                        .claude_code
                        .binary_path
                        .clone()
                        .unwrap_or_default(),
                )
        });
        let copilot_binary_path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(m::copilot_binary_placeholder())
                .default_value(
                    settings
                        .providers
                        .copilot
                        .binary_path
                        .clone()
                        .unwrap_or_default(),
                )
        });
        let codex_binary_path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(m::codex_binary_placeholder())
                .default_value(
                    settings
                        .providers
                        .codex
                        .binary_path
                        .clone()
                        .unwrap_or_default(),
                )
        });
        let codex_home = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(m::codex_home_placeholder())
                .default_value(settings.providers.codex.home.clone().unwrap_or_default())
        });
        let model_search = cx.new(|cx| InputState::new(window, cx).placeholder(m::search_models()));
        let custom_search_url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://example.com/search?q=%s")
                .default_value(settings.browser.custom_search_url.clone())
        });
        let title_instructions = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 8)
                .placeholder(m::thread_titles_instructions_placeholder())
                .default_value(settings.thread_titles.instructions.clone())
        });
        let snooze_options =
            cx.new(|cx| snooze_options::SnoozeOptions::new(&workspace, window, cx));
        let email = cx.new(|cx| InputState::new(window, cx).placeholder("you@example.com"));
        let token = cx.new(|cx| InputState::new(window, cx).masked(true));

        let subscriptions = vec![
            cx.subscribe_in(
                &search,
                window,
                |this, search, event, window, cx| match event {
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter { .. } => {
                        let query = search.read(cx).value();
                        if let Some((_, section)) = sections::search(&query).first() {
                            this.open_section(*section, window, cx);
                        }
                    }
                    _ => {}
                },
            ),
            cx.subscribe_in(&projects_folder, window, |this, field, event, _, cx| {
                if commits(event) {
                    let value = optional(&field.read(cx).value());
                    this.change(cx, |settings| settings.general.projects_folder = value);
                }
            }),
            cx.subscribe_in(&binary_path, window, |this, field, event, _, cx| {
                if commits(event) {
                    let value = optional(&field.read(cx).value());
                    this.change(cx, |settings| {
                        settings.providers.claude_code.binary_path = value
                    });
                }
            }),
            cx.subscribe_in(&copilot_binary_path, window, |this, field, event, _, cx| {
                if commits(event) {
                    let value = optional(&field.read(cx).value());
                    this.change(cx, |settings| {
                        settings.providers.copilot.binary_path = value
                    });
                }
            }),
            cx.subscribe_in(&codex_binary_path, window, |this, field, event, _, cx| {
                if commits(event) {
                    let value = optional(&field.read(cx).value());
                    this.change(cx, |settings| settings.providers.codex.binary_path = value);
                }
            }),
            cx.subscribe_in(&codex_home, window, |this, field, event, _, cx| {
                if commits(event) {
                    let value = optional(&field.read(cx).value());
                    this.change(cx, |settings| settings.providers.codex.home = value);
                }
            }),
            cx.subscribe_in(&model_search, window, |_, _, event, _, cx| {
                if let InputEvent::Change = event {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&custom_search_url, window, |this, field, event, _, cx| {
                if commits(event) {
                    let value = field.read(cx).value().trim().to_string();
                    this.change(cx, |settings| settings.browser.custom_search_url = value);
                }
            }),
            cx.subscribe_in(&title_instructions, window, |this, field, event, _, cx| {
                if let InputEvent::Blur = event {
                    let value = field.read(cx).value().trim().to_string();
                    this.change(cx, |settings| settings.thread_titles.instructions = value);
                }
            }),
            cx.subscribe(
                &snooze_options,
                |this, _, event: &snooze_options::SnoozeOptionsChanged, cx| {
                    let presets = event.0.clone();
                    this.change(cx, |settings| settings.general.snooze_presets = presets);
                },
            ),
            cx.subscribe_in(&email, window, |_, _, event, _, cx| {
                if let InputEvent::Change = event {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&token, window, |this, _, event, _, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => this.connect_bitbucket(cx),
                _ => {}
            }),
            cx.observe(&workspace, |_, _, cx| cx.notify()),
        ];

        let mut view = Self {
            workspace,
            section,
            search,
            projects_folder,
            binary_path,
            copilot_binary_path,
            codex: providers::CodexFields {
                binary_path: codex_binary_path,
                home: codex_home,
                signing_out: false,
                sign_out_failed: false,
            },
            model_picker: threads::ModelPicker {
                search: model_search,
                tab: None,
                show_legacy: false,
                open: None,
            },
            custom_search_url,
            title_instructions,
            snooze_options,
            bitbucket: BitbucketForm {
                open: false,
                email,
                token,
                pending: false,
                error: None,
            },
            accounts: None,
            checking_providers: false,
            cleaning_merged: false,
            provider: hoshi_core::DEFAULT_PROVIDER,
            advanced: HashSet::new(),
            save_failed: false,
            looks: appearance::AppearanceState::new(window, cx),
            _subscriptions: subscriptions,
        };
        view.load_accounts(cx);
        view.load_providers(cx);
        view
    }

    pub fn open_section(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        self.section = section;
        self.search
            .update(cx, |search, cx| search.set_value("", window, cx));
        cx.notify();
    }

    fn settings<'a>(&self, cx: &'a App) -> &'a AppSettings {
        &self.workspace.read(cx).settings
    }

    /// Changes the settings, saves them and hands the engine what changed.
    fn change(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut AppSettings)) {
        let (engine, path, previous, next) = self.workspace.update(cx, |workspace, cx| {
            let previous = workspace.settings.clone();
            change(&mut workspace.settings);
            cx.notify();
            (
                workspace.engine.clone(),
                workspace.settings_path.clone(),
                previous,
                hoshi_app::settings::Snapshot::of(&workspace.settings),
            )
        });
        if previous == next.settings {
            return;
        }
        let providers_changed = previous.providers != next.settings.providers;
        let workspace = self.workspace.downgrade();
        cx.spawn(async move |this, cx| {
            if let Some(path) = path {
                let applying = engine.clone();
                let saved = cx
                    .background_spawn(async move {
                        hoshi_app::apply_settings(&applying, &path, &previous, &next)
                    })
                    .await;
                if let Err(error) = &saved {
                    eprintln!("hoshi: could not save settings: {error}");
                }
                if !matches!(saved, Ok(false)) {
                    this.update(cx, |this, cx| {
                        this.save_failed = saved.is_err();
                        cx.notify();
                    })
                    .ok();
                }
            }
            if providers_changed {
                let models = cx
                    .background_spawn(async move { engine.provider_models() })
                    .await;
                workspace
                    .update(cx, |workspace, cx| {
                        workspace.models = models;
                        cx.notify();
                    })
                    .ok();
                this.update(cx, |this, cx| this.check_providers(cx)).ok();
            }
        })
        .detach();
    }

    /// A menu option's handler that changes the settings.
    fn setter(
        &self,
        cx: &Context<Self>,
        change: impl Fn(&mut AppSettings) + 'static,
    ) -> impl Fn(&mut Window, &mut App) + 'static {
        let this = cx.weak_entity();
        move |_, cx| {
            this.update(cx, |this, cx| this.change(cx, &change)).ok();
        }
    }

    /// A switch's handler that changes the settings.
    fn toggler(
        &self,
        cx: &Context<Self>,
        change: impl Fn(&mut AppSettings, bool) + 'static,
    ) -> impl Fn(bool, &mut Window, &mut App) + 'static {
        let this = cx.weak_entity();
        move |on, _, cx| {
            this.update(cx, |this, cx| {
                this.change(cx, |settings| change(settings, on))
            })
            .ok();
        }
    }

    fn advanced_group(
        &self,
        key: &'static str,
        rows: impl IntoIterator<Item = AnyElement>,
        cx: &Context<Self>,
    ) -> SettingsAdvanced {
        let this = cx.weak_entity();
        SettingsAdvanced::new(SharedString::from(format!("advanced-{key}")), m::advanced())
            .open(self.advanced.contains(key))
            .on_toggle(move |_, cx| {
                this.update(cx, |this, cx| {
                    if !this.advanced.remove(key) {
                        this.advanced.insert(key);
                    }
                    cx.notify();
                })
                .ok();
            })
            .children(rows)
    }

    fn save_error(&self, cx: &App) -> Option<AnyElement> {
        self.save_failed.then(|| {
            div()
                .px_4()
                .text_xs()
                .text_color(cx.hoshi().destructive)
                .child(m::settings_save_failed())
                .into_any_element()
        })
    }

    fn nav(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let query = self.search.read(cx).value();
        let searching = !query.trim().is_empty();
        let body =
            if searching {
                let results = sections::search(&query);
                if results.is_empty() {
                    div()
                        .px_2()
                        .py_6()
                        .text_center()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(m::no_matching_settings())
                        .into_any_element()
                } else {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .children(results.into_iter().enumerate().map(
                            |(index, (title, section))| {
                                nav_item(("settings-result", index), false, &colors)
                                    .flex_col()
                                    .items_start()
                                    .gap_0()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_section(section, window, cx)
                                    }))
                                    .child(div().text_color(colors.foreground).child(title))
                                    .child(div().text_xs().child(section.label()))
                            },
                        ))
                        .into_any_element()
                }
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .children(sections::nav_groups().into_iter().map(|(heading, group)| {
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .when_some(heading, |this, heading| {
                                this.child(
                                    div()
                                        .px_2()
                                        .pb_1()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(colors.muted_foreground)
                                        .child(heading),
                                )
                            })
                            .children(group.iter().map(|&section| {
                                nav_item(
                                    SharedString::from(format!("settings-nav-{section:?}")),
                                    section == self.section,
                                    &colors,
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_section(section, window, cx)
                                }))
                                .child(Icon::new(section.icon()).size_4())
                                .child(section.label())
                            }))
                    }))
                    .into_any_element()
            };
        let search_hover = colors.muted.opacity(0.6);
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(224.))
            .gap_3()
            .p_2()
            .bg(colors.sidebar)
            .rounded_l(px(15.))
            .border_r_1()
            .border_color(colors.border)
            .child(
                div()
                    .id("settings-search")
                    .on_action(cx.listener(|this, _: &DismissDialog, window, cx| {
                        if this.search.read(cx).value().is_empty() {
                            cx.propagate();
                            return;
                        }
                        this.search
                            .update(cx, |search, cx| search.set_value("", window, cx));
                        cx.notify();
                    }))
                    .flex()
                    .items_center()
                    .gap_2()
                    .h_8()
                    .px_2()
                    .rounded_lg()
                    .text_sm()
                    .text_color(colors.muted_foreground)
                    .hover(move |style| style.bg(search_hover))
                    .child(Icon::new(IconName::Search).size_4().flex_none())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.search).appearance(false)),
                    ),
            )
            .child(body)
    }

    fn header(&self, colors: &Colors) -> impl IntoElement {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .h_12()
            .pl_5()
            .pr_2()
            .border_b_1()
            .border_color(colors.border)
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap_1p5()
                    .text_sm()
                    .child(
                        div()
                            .text_color(colors.muted_foreground)
                            .child(m::settings()),
                    )
                    .child(
                        Icon::new(IconName::ChevronRight)
                            .size_3p5()
                            .text_color(colors.muted_foreground),
                    )
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.section.label()),
                    ),
            )
            .child(
                div().ml_auto().child(
                    DialogButton::new("close-settings")
                        .icon(IconName::X)
                        .variant(DialogButtonVariant::Ghost)
                        .size(DialogButtonSize::IconSm)
                        .accessible_label(m::close_settings())
                        .close(),
                ),
            )
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let page = match self.section {
            Section::General => self.general(cx),
            Section::Appearance => self.appearance(window, cx),
            Section::Providers => self.providers(cx),
            Section::Threads => self.threads(cx),
            Section::Browser => self.browser(cx),
            Section::Forges => self.forges(cx),
            Section::About => self.about(cx),
        };
        div()
            .flex()
            .flex_1()
            .min_h_0()
            .size_full()
            .child(self.nav(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .child(self.header(&colors))
                    .child(
                        div()
                            .id("settings-page")
                            .debug_selector(|| "settings-page".into())
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(
                                div()
                                    .mx_auto()
                                    .w_full()
                                    .max_w(px(896.))
                                    .px_6()
                                    .pt_6()
                                    .pb_12()
                                    .flex()
                                    .flex_col()
                                    .gap_8()
                                    .children(page),
                            ),
                    ),
            )
            .children(self.theme_editor())
    }
}

fn nav_item(id: impl Into<ElementId>, current: bool, colors: &Colors) -> Stateful<Div> {
    let (hover_bg, hover_fg) = (colors.muted, colors.foreground);
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded_lg()
        .text_sm()
        .cursor_pointer()
        .map(|this| {
            if current {
                this.bg(colors.muted).text_color(colors.foreground)
            } else {
                this.text_color(colors.muted_foreground)
            }
        })
        .hover(move |style| style.bg(hover_bg).text_color(hover_fg))
}

/// A field saves when it loses focus or Enter is pressed in it.
fn commits(event: &InputEvent) -> bool {
    matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. })
}

fn optional(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn home_dir() -> String {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default()
}

/// A text field as wide as the web's.
fn field(state: &Entity<InputState>, width: Pixels) -> impl IntoElement {
    div().w(width).child(Input::new(state))
}

fn text_area(state: &Entity<TextareaState>, disabled: bool) -> impl IntoElement {
    div().mt_3().child(Textarea::new(state).disabled(disabled))
}

#[cfg(test)]
mod tests {
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
                        .on_open_change(cx.listener(
                            |this, change: &hoshi_ui::OpenChange, _, cx| {
                                this.open = change.open;
                                cx.notify();
                            },
                        ))
                        .child(self.view.clone()),
                )
            })
        }
    }

    fn harness(cx: &mut TestAppContext, section: Section) -> Harness {
        cx.executor().allow_parking();
        let home =
            std::env::temp_dir().join(format!("hoshi-native-settings-{}", uuid::Uuid::new_v4()));
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
    fn snooze_options_save_as_they_change_and_refuse_what_menus_cannot_use(
        cx: &mut TestAppContext,
    ) {
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
}
