use super::{RightPanel, SurfaceKind, SurfaceView, new_id};
use crate::browser::{Browsers, BrowsersEvent, PAGE_SIZES, PageFit, PageView, ViewportChanged};
use crate::m;
use crate::state::Workspace;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{Selectable as _, Sizable as _};
use gpui_kit::*;
use hoshi_ui::prelude::*;
use std::time::Duration;

/// Quiet time after the panel's last resize before the page follows it.
const FIT_DELAY: Duration = Duration::from_millis(150);

/// The thread's headless browser page, live as it paints: the agent's, which the user can
/// take over by clicking and typing, or one the user opens from the address bar.
pub struct BrowserSurface {
    thread_id: String,
    browsers: Entity<Browsers>,
    address: Entity<InputState>,
    page: Entity<PageView>,
    /// Whether the page filled the panel when last told.
    filled: bool,
    fit: Option<Task<()>>,
    url: String,
    size_menu_open: bool,
    _subscriptions: Vec<Subscription>,
}

impl BrowserSurface {
    pub fn new(
        workspace: Entity<Workspace>,
        thread_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let browsers = workspace.read(cx).browsers.clone();
        let address = cx.new(|cx| crate::locale::input(m::browser_address_placeholder, window, cx));
        let shown_page = browsers.read(cx).page(&thread_id);
        let url = shown_page
            .map(|page| page.page.url.clone())
            .unwrap_or_default();
        let filled = shown_page.and_then(|page| page.fitted).unwrap_or(true);
        if !url.is_empty() {
            address.update(cx, |address, cx| address.set_value(url.clone(), window, cx));
        }
        let page = cx.new(|cx| {
            PageView::new(
                thread_id.clone(),
                browsers.clone(),
                PageFit::Engine,
                "browser-page",
                cx,
            )
        });
        let enter = cx.subscribe(&address, |this, _, event, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.open(cx);
            }
        });
        let changes = cx.observe_in(&browsers, window, |this, _, window, cx| {
            this.follow(window, cx);
        });
        let resized = cx.subscribe(&page, |this, _, _: &ViewportChanged, cx| {
            this.schedule_fit(cx);
            cx.notify();
        });
        Self {
            thread_id,
            browsers,
            address,
            page,
            filled,
            fit: None,
            url,
            size_menu_open: false,
            _subscriptions: vec![enter, changes, resized],
        }
    }

    /// The panel shows this surface or stopped showing it.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.page.update(cx, |page, cx| page.set_viewing(shown, cx));
        if shown {
            self.schedule_fit(cx);
        }
    }

    fn fill(&self, cx: &App) -> bool {
        self.browsers
            .read(cx)
            .page(&self.thread_id)
            .and_then(|page| page.fitted)
            .unwrap_or(true)
    }

    fn follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self
            .browsers
            .read(cx)
            .page(&self.thread_id)
            .map(|page| page.page.url.clone());
        if let Some(url) = url.filter(|url| !url.is_empty() && *url != self.url) {
            self.address
                .update(cx, |address, cx| address.set_value(url.clone(), window, cx));
            self.url = url;
        }
        let fill = self.fill(cx);
        if fill && !self.filled {
            self.schedule_fit(cx);
        }
        self.filled = fill;
        cx.notify();
    }

    fn open(&mut self, cx: &mut Context<Self>) {
        let address = self.address.read(cx).value().to_string();
        let thread_id = self.thread_id.clone();
        self.browsers
            .update(cx, |browsers, cx| browsers.open(&thread_id, &address, cx));
    }

    fn schedule_fit(&mut self, cx: &mut Context<Self>) {
        if !self.fill(cx) {
            return;
        }
        self.fit = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FIT_DELAY).await;
            this.update(cx, |this, cx| this.fit_now(cx)).ok();
        }));
    }

    fn fit_now(&mut self, cx: &mut Context<Self>) {
        let Some(viewport) = self.page.read(cx).viewport() else {
            return;
        };
        let width = f32::from(viewport.size.width).floor();
        let height = f32::from(viewport.size.height).floor();
        if width > 0. && height > 0. {
            self.browsers
                .read(cx)
                .fit(&self.thread_id, width as u32, height as u32);
        }
    }

    fn toolbar(&self, has_page: bool, colors: &Colors, cx: &mut Context<Self>) -> Div {
        let thread = self.thread_id.clone();
        let nav = |id: &'static str, icon: IconName, label: String, delta: i32| {
            let thread = thread.clone();
            Button::new(id)
                .icon(icon)
                .size(ButtonSize::IconSm)
                .variant(ButtonVariant::Ghost)
                .disabled(!has_page)
                .tooltip(Tooltip::new(label).side(Side::Bottom))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.browsers.read(cx).go(&thread, delta);
                }))
        };
        let back = nav("browser-back", IconName::ArrowLeft, m::browser_back(), -1);
        let forward = nav(
            "browser-forward",
            IconName::ArrowRight,
            m::browser_forward(),
            1,
        );
        let reload = nav(
            "browser-reload",
            IconName::RefreshCw,
            m::browser_reload(),
            0,
        );
        let external = self
            .browsers
            .read(cx)
            .page(&self.thread_id)
            .map(|page| page.page.url.clone())
            .filter(|url| url.starts_with("http"));
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_1p5()
            .border_b_1()
            .border_color(colors.border)
            .child(back)
            .child(forward)
            .child(reload)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(text_input(&self.address).small()),
            )
            .child(self.size_menu(has_page, cx))
            .child(
                Button::new("browser-float")
                    .icon(IconName::PictureInPicture2)
                    .size(ButtonSize::IconSm)
                    .variant(ButtonVariant::Ghost)
                    .disabled(!has_page)
                    .tooltip(Tooltip::new(m::browser_float()).side(Side::Bottom))
                    .on_click(cx.listener(|this, _, _, cx| {
                        let thread_id = this.thread_id.clone();
                        this.browsers
                            .update(cx, |browsers, cx| browsers.float(&thread_id, cx));
                    })),
            )
            .child(
                Button::new("browser-open-external")
                    .icon(IconName::ExternalLink)
                    .size(ButtonSize::IconSm)
                    .variant(ButtonVariant::Ghost)
                    .disabled(external.is_none())
                    .tooltip(Tooltip::new(m::browser_open_external()).side(Side::Bottom))
                    .on_click(move |_, _, cx| {
                        if let Some(url) = &external {
                            cx.open_url(url);
                        }
                    }),
            )
    }

    fn size_menu(&self, has_page: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let fill = self.fill(cx);
        let frame = self
            .browsers
            .read(cx)
            .frame(&self.thread_id)
            .map(|frame| (frame.width, frame.height));
        let label = match (fill, frame) {
            (true, _) => m::browser_size_fill(),
            (false, Some((width, height))) => format!("{width} × {height}"),
            (false, None) => m::browser_size(),
        };
        let this = cx.weak_entity();
        Popover::new("browser-size-menu")
            .align(Align::End)
            .w(px(208.))
            .open(self.size_menu_open && has_page)
            .on_open_change(cx.listener(|this, change: &PopoverChange, _, cx| {
                this.size_menu_open = change.open;
                cx.notify();
            }))
            .trigger(move |open| {
                Button::new("browser-size")
                    .label(label)
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Outline)
                    .disabled(!has_page)
                    .selected(open)
            })
            .menu(move |menu, _, _| {
                let fit = this.clone();
                let fill_option =
                    MenuOption::radio("browser-size-fill", m::browser_size_fill(), fill)
                        .close_on_click(true)
                        .on_select(move |_, cx| {
                            fit.update(cx, |this, cx| this.fit_now(cx)).ok();
                        });
                let labels = [
                    m::browser_size_desktop(),
                    m::browser_size_tablet(),
                    m::browser_size_phone(),
                ];
                menu.item(fill_option)
                    .items(PAGE_SIZES.into_iter().zip(labels).enumerate().map(
                        |(index, ((width, height), label))| {
                            let this = this.clone();
                            MenuOption::new(("browser-size", index), label)
                                .shortcut(format!("{width} × {height}"))
                                .on_select(move |_, cx| {
                                    this.update(cx, |this, cx| {
                                        this.browsers.read(cx).resize(
                                            &this.thread_id,
                                            width,
                                            height,
                                        )
                                    })
                                    .ok();
                                })
                        },
                    ))
            })
    }
}

impl Render for BrowserSurface {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let browsers = self.browsers.read(cx);
        let page = browsers.page(&self.thread_id);
        let has_page = page.is_some();
        let errors = page.map_or(0, |page| page.page.errors);
        let failed = browsers.failed(&self.thread_id);
        let agent_active = browsers.agent_active(&self.thread_id);
        let empty = page.is_none() || browsers.frame(&self.thread_id).is_none();
        let fill = self.fill(cx);
        let toolbar = self.toolbar(has_page, &colors, cx);

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(toolbar)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .map(|viewport| {
                        if fill {
                            viewport.bg(colors.background)
                        } else {
                            viewport.bg(colors.muted.opacity(0.3))
                        }
                    })
                    .child(self.page.clone())
                    .when(empty, |viewport| {
                        viewport.child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full()
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap_1()
                                .px_6()
                                .text_center()
                                .text_xs()
                                .text_color(colors.muted_foreground)
                                .child(div().text_sm().text_color(colors.foreground).child(
                                    if failed {
                                        m::browser_failed()
                                    } else {
                                        m::browser_empty()
                                    },
                                ))
                                .child(m::browser_empty_hint()),
                        )
                    })
                    .when(agent_active, |viewport| {
                        viewport.child(
                            div()
                                .absolute()
                                .right_3()
                                .bottom_3()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .px_2p5()
                                .py_1()
                                .rounded_full()
                                .border_1()
                                .border_color(colors.border)
                                .bg(colors.popover)
                                .shadow_sm()
                                .text_xs()
                                .text_color(colors.muted_foreground)
                                .child(div().size_1p5().rounded_full().bg(colors.primary))
                                .child(m::browser_agent_using()),
                        )
                    }),
            )
            .when(errors > 0, |surface| {
                surface.child(
                    div()
                        .flex_none()
                        .px_3()
                        .py_1p5()
                        .border_t_1()
                        .border_color(colors.border)
                        .text_xs()
                        .text_color(colors.status_error)
                        .child(m::browser_console_errors(errors as usize)),
                )
            })
    }
}

impl RightPanel {
    pub(super) fn watch_browsers(
        workspace: &Entity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        let browsers = workspace.read(cx).browsers.clone();
        vec![
            cx.subscribe(&browsers, |this, browsers, event, cx| match event {
                BrowsersEvent::AgentOpened(thread_id) => {
                    this.agent_opened_page(thread_id, &browsers, cx)
                }
                BrowsersEvent::Floated(thread_id) => this.close(thread_id, cx),
                BrowsersEvent::Docked(thread_id) => {
                    this.open_surface(thread_id, SurfaceKind::Browser, cx)
                }
            }),
            cx.observe_self(Self::sync_browser_views),
        ]
    }

    /// The panel is open on the thread's browser.
    pub fn shows_browser(&self, thread_id: &str) -> bool {
        self.is_open(thread_id)
            && self
                .threads
                .get(thread_id)
                .and_then(|panel| panel.active())
                .is_some_and(|surface| surface.kind() == SurfaceKind::Browser)
    }

    /// Brings the page the agent opened into view: floating over the chat, or as a surface
    /// waiting in the panel.
    fn agent_opened_page(
        &mut self,
        thread_id: &str,
        browsers: &Entity<Browsers>,
        cx: &mut Context<Self>,
    ) {
        if self.shows_browser(thread_id) {
            return;
        }
        if self.workspace.read(cx).settings.browser.float_agent_pages {
            browsers.update(cx, |browsers, cx| browsers.open_mini(thread_id, cx));
        } else {
            self.threads
                .entry(thread_id.to_owned())
                .or_default()
                .open(SurfaceKind::Browser, new_id);
            cx.notify();
        }
    }

    /// Streams each browser surface's page while the panel shows it.
    fn sync_browser_views(&mut self, cx: &mut Context<Self>) {
        let active = self
            .views_for
            .as_deref()
            .filter(|id| self.is_open(id))
            .and_then(|id| self.threads.get(id))
            .and_then(|panel| panel.active_id.clone());
        for (id, view) in &self.views {
            if let SurfaceView::Browser(view) = view {
                let shown = active.as_deref() == Some(id.as_str());
                view.update(cx, |view, cx| view.set_shown(shown, cx));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{RightPanel, SurfaceKind};
    use crate::state::{self, Workspace};
    use gpui_kit::component::theme::ThemeMode;
    use gpui_kit::{
        AppContext as _, Bounds, Entity, Modifiers, Point, TestAppContext, VisualTestContext,
        WindowBounds, WindowOptions, px, size,
    };
    use hoshi_core::{AccessMode, BrowserPage, Command, ThreadWorkspace};
    use std::sync::Arc;

    fn redraw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        for _ in 0..3 {
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.run_until_parked();
        }
    }

    fn agent_page(opened: bool) -> BrowserPage {
        BrowserPage {
            thread_id: "t".into(),
            open: true,
            url: "http://localhost:5173/".into(),
            title: "app".into(),
            errors: 0,
            by_agent: true,
            agent_point: None,
            opened,
            fitted: Some(true),
        }
    }

    fn harness(
        float_agent_pages: bool,
        cx: &mut TestAppContext,
    ) -> (Entity<Workspace>, Entity<RightPanel>, VisualTestContext) {
        cx.executor().allow_parking();
        let home =
            std::env::temp_dir().join(format!("hoshi-native-browser-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let (notify, _engine_notifications) = state::channel();
        let (_, notifications) = state::channel();
        let engine = Arc::new(hoshi_engine::Engine::open(&home, notify).unwrap());
        engine
            .dispatch(Command::ThreadCreate {
                thread_id: "t".into(),
                project_id: None,
                title: "chat".into(),
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
            workspace.settings.browser.float_agent_pages = float_agent_pages;
            workspace.open_thread("t", cx)
        });
        let mut panel: Option<Entity<RightPanel>> = None;
        let (window, _) = cx
            .update(|cx| {
                let options = WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(900.), px(600.)),
                    })),
                    ..Default::default()
                };
                gpui_kit::open_window(options, cx, |window, cx| {
                    let shown = cx.new(|cx| RightPanel::new(workspace.clone(), window, cx));
                    panel = Some(shown.clone());
                    shown
                })
            })
            .unwrap();
        let mut cx = VisualTestContext::from_window(window, cx);
        redraw(&mut cx);
        (workspace, panel.unwrap(), cx)
    }

    #[gpui_kit::test]
    fn a_page_the_agent_opens_floats_and_the_panel_streams_it_while_shown(cx: &mut TestAppContext) {
        let (workspace, panel, mut cx) = harness(true, cx);
        let browsers = workspace.read_with(&cx, |workspace, _| workspace.browsers.clone());
        let viewers = |cx: &mut VisualTestContext| browsers.read_with(cx, |b, _| b.viewers("t"));
        let floating = |cx: &mut VisualTestContext| browsers.read_with(cx, |b, _| b.mini("t"));

        browsers.update(&mut cx, |b, cx| b.receive_page(agent_page(false), cx));
        redraw(&mut cx);
        assert!(!panel.read_with(&cx, |panel, _| panel.is_open("t")));
        assert!(floating(&mut cx).is_none());

        browsers.update(&mut cx, |b, cx| b.receive_page(agent_page(true), cx));
        redraw(&mut cx);
        assert!(floating(&mut cx).is_some());
        assert!(!panel.read_with(&cx, |panel, _| panel.is_open("t")));

        browsers.update(&mut cx, |b, cx| b.dock("t", cx));
        redraw(&mut cx);
        assert!(floating(&mut cx).is_none());
        assert!(panel.read_with(&cx, |panel, _| panel.shows_browser("t")));
        assert!(cx.debug_bounds("browser-page").is_some());
        assert_eq!(viewers(&mut cx), 1);

        browsers.update(&mut cx, |b, cx| b.receive_page(agent_page(true), cx));
        redraw(&mut cx);
        assert!(floating(&mut cx).is_none(), "the panel already shows it");

        panel.update(&mut cx, |panel, cx| panel.close("t", cx));
        redraw(&mut cx);
        assert_eq!(viewers(&mut cx), 0);

        panel.update(&mut cx, |panel, cx| {
            panel.open_surface("t", SurfaceKind::Browser, cx)
        });
        redraw(&mut cx);
        assert_eq!(viewers(&mut cx), 1);

        panel.update(&mut cx, |panel, cx| {
            panel.open_surface("t", SurfaceKind::Diff, cx)
        });
        redraw(&mut cx);
        assert_eq!(viewers(&mut cx), 0);

        let browser = panel.read_with(&cx, |panel, _| {
            let surfaces = panel.surfaces("t").unwrap();
            surfaces
                .surfaces
                .iter()
                .find(|surface| surface.kind() == SurfaceKind::Browser)
                .unwrap()
                .id
                .clone()
        });
        panel.update(&mut cx, |panel, cx| {
            panel.activate_surface("t", &browser, cx)
        });
        redraw(&mut cx);
        assert_eq!(viewers(&mut cx), 1);

        browsers.update(&mut cx, |b, cx| b.float("t", cx));
        redraw(&mut cx);
        assert!(!panel.read_with(&cx, |panel, _| panel.is_open("t")));
        assert!(floating(&mut cx).is_some());
        assert_eq!(viewers(&mut cx), 0);

        panel.update(&mut cx, |panel, cx| {
            panel.open_surface("t", SurfaceKind::Browser, cx)
        });
        redraw(&mut cx);
        panel.update(&mut cx, |panel, cx| panel.close_surface("t", &browser, cx));
        redraw(&mut cx);
        assert_eq!(viewers(&mut cx), 0);
    }

    #[gpui_kit::test]
    fn without_floating_the_page_waits_in_the_panel(cx: &mut TestAppContext) {
        let (workspace, panel, mut cx) = harness(false, cx);
        let browsers = workspace.read_with(&cx, |workspace, _| workspace.browsers.clone());
        browsers.update(&mut cx, |b, cx| b.receive_page(agent_page(true), cx));
        redraw(&mut cx);
        assert!(browsers.read_with(&cx, |b, _| b.mini("t")).is_none());
        assert!(!panel.read_with(&cx, |panel, _| panel.is_open("t")));
        let placed = panel.read_with(&cx, |panel, _| {
            panel.surfaces("t").is_some_and(|surfaces| {
                surfaces
                    .surfaces
                    .iter()
                    .any(|surface| surface.kind() == SurfaceKind::Browser)
            })
        });
        assert!(placed);
    }

    #[gpui_kit::test]
    fn closing_the_browser_tab_asks_the_browser_thread_not_the_engine(cx: &mut TestAppContext) {
        let (workspace, panel, mut cx) = harness(false, cx);
        let (browsers, sent) = crate::browser::Browsers::detached();
        let browsers = cx.update(|_, cx| cx.new(|_| browsers));
        workspace.update(&mut cx, |workspace, _| {
            workspace.browsers = browsers.clone()
        });
        panel.update(&mut cx, |panel, cx| {
            panel.open_surface("t", SurfaceKind::Browser, cx)
        });
        redraw(&mut cx);
        let surface = panel.read_with(&cx, |panel, _| {
            panel.surfaces("t").unwrap().surfaces[0].id.clone()
        });
        sent.take();

        panel.update(&mut cx, |panel, cx| panel.close_surface("t", &surface, cx));
        redraw(&mut cx);

        assert!(sent.take().contains(&"t: Close".to_owned()));
    }

    #[gpui_kit::test]
    fn keys_typed_on_an_empty_panel_go_nowhere(cx: &mut TestAppContext) {
        let (workspace, panel, mut cx) = harness(false, cx);
        let (browsers, sent) = crate::browser::Browsers::detached();
        let browsers = cx.update(|_, cx| cx.new(|_| browsers));
        workspace.update(&mut cx, |workspace, _| {
            workspace.browsers = browsers.clone()
        });
        panel.update(&mut cx, |panel, cx| {
            panel.open_surface("t", SurfaceKind::Browser, cx)
        });
        redraw(&mut cx);
        let page = cx.debug_bounds("browser-page").unwrap();
        cx.simulate_click(page.center(), Modifiers::none());
        cx.simulate_keystrokes("a enter");
        redraw(&mut cx);
        assert!(
            sent.take().iter().all(|request| !request.contains("Input")),
            "no page to type in"
        );

        browsers.update(&mut cx, |b, cx| b.receive_page(agent_page(false), cx));
        cx.simulate_click(page.center(), Modifiers::none());
        cx.simulate_keystrokes("a");
        redraw(&mut cx);
        assert!(sent.take().iter().any(|request| request.contains("Input")));
    }

    #[gpui_kit::test]
    fn tab_moves_within_the_page_and_the_page_keeps_the_focus(cx: &mut TestAppContext) {
        let (workspace, panel, mut cx) = harness(false, cx);
        let (browsers, sent) = crate::browser::Browsers::detached();
        let browsers = cx.update(|_, cx| cx.new(|_| browsers));
        workspace.update(&mut cx, |workspace, _| {
            workspace.browsers = browsers.clone()
        });
        browsers.update(&mut cx, |b, cx| b.receive_page(agent_page(false), cx));
        panel.update(&mut cx, |panel, cx| {
            panel.open_surface("t", SurfaceKind::Browser, cx)
        });
        redraw(&mut cx);
        let page = cx.debug_bounds("browser-page").unwrap();
        cx.simulate_click(page.center(), Modifiers::none());
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("b".into()));
        sent.take();

        for (keys, sends) in [
            ("tab", "\"Tab\""),
            ("shift-tab", "\"Shift+Tab\""),
            ("a", "Key { key: \"a\" }"),
            ("ctrl-v", "Text { text: \"b\" }"),
        ] {
            cx.simulate_keystrokes(keys);
            redraw(&mut cx);
            let inputs: Vec<String> = sent
                .take()
                .into_iter()
                .filter(|request| request.contains("Input"))
                .collect();
            assert!(
                inputs.len() == 1 && inputs[0].contains(sends),
                "{keys}: {inputs:?}"
            );
        }
    }
}
