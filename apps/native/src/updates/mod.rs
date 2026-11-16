mod model;
pub mod provider;
#[cfg(test)]
mod ui_tests;

use crate::m;
use crate::state::Workspace;
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_core::{AppRelease, ProviderUpdate, ProviderVersion};
use hoshi_ui::assets::brand;
use hoshi_ui::prelude::*;
use model::{
    AppUpdate, UpdateBadge, UpdateButton, after_check, check_due, provider_badge, release_changes,
    release_compare_url, release_notes_key, release_notes_target, update_button, update_ended,
};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

const CHECK_EVERY: Duration = Duration::from_secs(5 * 60);
const UP_TO_DATE_FOR: Duration = Duration::from_secs(2);
const INSTALLED: &str = env!("CARGO_PKG_VERSION");

/// Dev builds never update.
const APP_UPDATES_ENABLED: bool = !cfg!(debug_assertions);

/// Checks hoshi's release feed, or opens what an update found or is doing.
pub struct UpdatesButton {
    workspace: Entity<Workspace>,
    app: AppUpdate,
    /// The user looked at how the last provider update ended.
    seen: bool,
    provider_update: Option<ProviderUpdate>,
    /// Releases between the running build and an update, by `release_notes_key`.
    release_notes: HashMap<String, Vec<AppRelease>>,
    loading_notes: HashSet<String>,
    last_checked: Option<Instant>,
    checking: Option<Task<()>>,
    up_to_date: Option<Task<()>>,
    open: bool,
    _observe: [Subscription; 2],
    _watch: Task<()>,
}

impl UpdatesButton {
    pub fn new(workspace: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        provider::load_version(&workspace, cx);
        let provider_update = workspace
            .read(cx)
            .provider_version()
            .map(|version| version.update.clone());
        let observe = [
            cx.observe(&workspace, |this, workspace, cx| {
                this.provider_changed(&workspace, cx)
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() && check_due(this.last_checked.map(|at| at.elapsed()))
                {
                    this.check(false, cx);
                }
            }),
        ];
        let watch = cx.spawn(async move |this, cx| {
            loop {
                if this.update(cx, |this, cx| this.check(false, cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(CHECK_EVERY).await;
            }
        });
        Self {
            workspace,
            app: AppUpdate::Idle,
            seen: true,
            provider_update,
            release_notes: HashMap::new(),
            loading_notes: HashSet::new(),
            last_checked: None,
            checking: None,
            up_to_date: None,
            open: false,
            _observe: observe,
            _watch: watch,
        }
    }

    fn provider_changed(&mut self, workspace: &Entity<Workspace>, cx: &mut Context<Self>) {
        let next = workspace
            .read(cx)
            .provider_version()
            .map(|version| version.update.clone());
        if next == self.provider_update {
            return;
        }
        if let Some(update) = &next
            && update_ended(self.provider_update.as_ref(), update)
        {
            self.seen = false;
            if *update == ProviderUpdate::Succeeded {
                reload_models(workspace, cx);
            }
        }
        self.provider_update = next;
        cx.notify();
    }

    /// Reads the release feed; `manual` shows the check and how it ended.
    fn check(&mut self, manual: bool, cx: &mut Context<Self>) {
        if !APP_UPDATES_ENABLED {
            return;
        }
        if manual {
            self.app = AppUpdate::Checking;
            cx.notify();
        }
        if self.checking.is_some() {
            return;
        }
        self.up_to_date = None;
        self.checking = Some(cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async { hoshi_app::app_release::latest_release(INSTALLED) })
                .await;
            this.update(cx, |this, cx| this.checked(found, cx)).ok();
        }));
    }

    fn checked(&mut self, found: Result<Option<AppRelease>, String>, cx: &mut Context<Self>) {
        if let Some(task) = self.checking.take() {
            task.detach();
        }
        self.last_checked = Some(Instant::now());
        if let Err(error) = &found {
            eprintln!("hoshi-native: failed to check for an app update: {error}");
        }
        self.app = after_check(&self.app, found, INSTALLED);
        match self.app {
            AppUpdate::UpToDate { .. } => {
                self.up_to_date = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(UP_TO_DATE_FOR).await;
                    this.update(cx, |this, cx| {
                        if matches!(this.app, AppUpdate::UpToDate { .. }) {
                            this.app = AppUpdate::Idle;
                            cx.notify();
                        }
                    })
                    .ok();
                }));
            }
            AppUpdate::Available { .. } => self.load_release_notes(cx),
            _ => {}
        }
        cx.notify();
    }

    /// Fetches the notes of the update found last, once per session.
    fn load_release_notes(&mut self, cx: &mut Context<Self>) {
        let Some(available) = release_notes_target(&self.app).map(str::to_owned) else {
            return;
        };
        let key = release_notes_key(INSTALLED, &available);
        if self.release_notes.contains_key(&key) || !self.loading_notes.insert(key.clone()) {
            return;
        }
        cx.spawn(async move |this, cx| {
            let releases = cx
                .background_spawn(async move {
                    hoshi_app::app_release::release_notes(INSTALLED, &available)
                })
                .await;
            this.update(cx, |this, cx| {
                this.loading_notes.remove(&key);
                match releases {
                    Ok(releases) => {
                        this.release_notes.insert(key, releases);
                        cx.notify();
                    }
                    Err(error) => {
                        eprintln!("hoshi-native: failed to read the release notes: {error}")
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.open = open;
        if open {
            self.load_release_notes(cx);
        } else {
            self.seen = true;
        }
        cx.notify();
    }

    fn check_button(&self, button: UpdateButton, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.hoshi().clone();
        let off = button == UpdateButton::Off;
        let checking = button == UpdateButton::Check { checking: true };
        let label = match &button {
            UpdateButton::Off => m::app_update_off_in_dev(),
            UpdateButton::UpToDate { version } => m::app_update_up_to_date(version),
            UpdateButton::CheckFailed { message } => m::app_update_check_failed(message),
            _ if checking => m::app_update_checking(),
            _ => m::check_for_updates(),
        };
        let up_to_date = matches!(button, UpdateButton::UpToDate { .. });
        let icon = if up_to_date {
            IconName::CircleCheck
        } else {
            IconName::RefreshCw
        };
        let tone = if off {
            colors.muted_foreground
        } else if up_to_date {
            colors.status_done
        } else {
            colors.foreground
        };
        let dot = matches!(button, UpdateButton::CheckFailed { .. }).then_some(colors.status_error);
        let frame = trigger_frame(
            "updates-check",
            icon_view(icon, tone, checking),
            dot,
            false,
            &colors,
        )
        .when(!off, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(colors.muted))
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                    gpui_kit::base::GlobalState::suppress_text_selection(cx)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    if !checking {
                        this.check(true, cx);
                    }
                }))
        });
        TooltipTrigger::new(
            "updates-check-tooltip",
            frame,
            Tooltip::new(label).side(Side::Top).max_width(px(288.)),
        )
        .into_any_element()
    }

    fn details_button(&self, badge: UpdateBadge, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.hoshi().clone();
        let label = match &self.app {
            AppUpdate::Available { release } if badge == UpdateBadge::Available => {
                m::app_update_to(&release.version)
            }
            _ => m::updates(),
        };
        let (icon, tone, spin) = match badge {
            UpdateBadge::Running => (IconName::LoaderCircle, colors.foreground, true),
            UpdateBadge::Done => (IconName::CircleCheck, colors.status_done, false),
            UpdateBadge::Failed | UpdateBadge::Available => {
                (IconName::Download, colors.foreground, false)
            }
        };
        let dot = match badge {
            UpdateBadge::Available => Some(colors.foreground),
            UpdateBadge::Failed => Some(colors.status_error),
            _ => None,
        };
        let this = cx.weak_entity();
        Popover::new("updates-popover")
            .side(Side::Top)
            .w(px(320.))
            .gap_3()
            .open(self.open)
            .on_open_change(
                cx.listener(|this, change: &PopoverChange, _, cx| this.set_open(change.open, cx)),
            )
            .trigger(move |open| {
                let frame = trigger_frame(
                    "updates-details",
                    icon_view(icon, tone, spin),
                    dot,
                    open,
                    &colors,
                )
                .cursor_pointer()
                .hover(|style| style.bg(colors.muted));
                TooltipTrigger::new(
                    "updates-details-tooltip",
                    frame,
                    Tooltip::new(label).side(Side::Top).disabled(open),
                )
            })
            .content(move |_, _, cx| {
                this.update(cx, |this, cx| this.details(cx))
                    .unwrap_or_else(|_| div().into_any_element())
            })
            .into_any_element()
    }

    fn details(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let version = self.workspace.read(cx).provider_version().cloned();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .when(matches!(self.app, AppUpdate::Available { .. }), |this| {
                this.child(self.app_details(cx))
            })
            .when_some(version, |this, version| {
                this.child(self.provider_details(&version, cx))
            })
            .into_any_element()
    }

    fn app_details(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.hoshi().clone();
        let AppUpdate::Available { release } = &self.app else {
            return div().into_any_element();
        };
        let url = release.url.clone();
        let tile = div()
            .flex()
            .flex_none()
            .size_8()
            .items_center()
            .justify_center()
            .rounded_lg()
            .bg(colors.muted)
            .child(
                svg()
                    .path(brand::MARK)
                    .size_4()
                    .text_color(colors.foreground),
            );
        div()
            .debug_selector(|| "updates-app".into())
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(update_header(
                tile,
                m::app_name(),
                version_change(INSTALLED, &release.version, &colors).into_any_element(),
                &colors,
            ))
            .child(update_action(
                m::app_update_from_release_page(),
                Some(
                    Button::new("app-update-download")
                        .label(m::app_update_download())
                        .variant(ButtonVariant::Outline)
                        .size(ButtonSize::Sm)
                        .on_click(move |_, _, cx| cx.open_url(&url))
                        .into_any_element(),
                ),
                &colors,
            ))
            .children(self.release_notes_view(&release.version, &colors))
            .into_any_element()
    }

    fn release_notes_view(&self, available: &str, colors: &Colors) -> Option<AnyElement> {
        let releases = self
            .release_notes
            .get(&release_notes_key(INSTALLED, available))?;
        let shown: Vec<(&AppRelease, Vec<String>)> = releases
            .iter()
            .map(|release| (release, release_changes(&release.body)))
            .collect();
        let count: usize = shown.iter().map(|(_, changes)| changes.len()).sum();
        if count == 0 {
            return None;
        }
        let now = chrono::Utc::now();
        let compare = release_compare_url(INSTALLED, available);
        let muted = colors.muted_foreground;
        let foreground = colors.foreground;
        let sections = shown.into_iter().map(|(release, changes)| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .justify_between()
                        .gap_2()
                        .text_xs()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(foreground)
                                .child(release.version.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_color(muted)
                                .child(crate::sections::relative_time(&release.published_at, now)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .children(changes.into_iter().map(|change| {
                            div()
                                .flex()
                                .gap_1p5()
                                .text_xs()
                                .text_color(muted)
                                .child(
                                    div()
                                        .flex_none()
                                        .mt(px(6.))
                                        .size_1()
                                        .rounded_full()
                                        .bg(muted.opacity(0.5)),
                                )
                                .child(div().min_w_0().child(change))
                        })),
                )
        });
        Some(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .pt(px(10.))
                .border_t_1()
                .border_color(colors.border)
                .child(
                    div()
                        .id("app-release-notes")
                        .debug_selector(|| "updates-release-notes".into())
                        .flex()
                        .flex_col()
                        .gap_3()
                        .max_h(px(248.))
                        .overflow_y_scroll()
                        .children(sections),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .text_xs()
                        .text_color(muted)
                        .child(m::app_update_changes(count))
                        .child(
                            div()
                                .id("app-release-compare")
                                .flex()
                                .items_center()
                                .gap_1()
                                .rounded_sm()
                                .cursor_pointer()
                                .hover(move |style| style.text_color(foreground))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                    gpui_kit::base::GlobalState::suppress_text_selection(cx)
                                })
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    cx.open_url(&compare)
                                })
                                .child(m::view_on_github())
                                .child(Icon::new(IconName::ExternalLink).size_3().flex_none()),
                        ),
                )
                .into_any_element(),
        )
    }

    fn provider_details(&self, version: &ProviderVersion, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.hoshi().clone();
        let mono = cx.theme().mono_font_family.clone();
        let turns_running = provider::turns_running(self.workspace.read(cx));
        let running = matches!(version.update, ProviderUpdate::Running { .. });
        let summary = match (&version.installed, &version.latest) {
            (Some(installed), Some(latest)) if running || version.update_available => {
                version_change(installed, latest, &colors).into_any_element()
            }
            _ => div()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(colors.muted_foreground)
                .child(provider::version_summary(version))
                .into_any_element(),
        };
        let tile = div()
            .flex()
            .flex_none()
            .size_8()
            .items_center()
            .justify_center()
            .rounded_lg()
            .bg(colors.provider_claude.opacity(0.15))
            .child(
                svg()
                    .path(brand::CLAUDE)
                    .size_4()
                    .text_color(colors.provider_claude),
            );
        let action = version
            .update_command
            .clone()
            .filter(|_| version.update_available && !running)
            .map(|command| {
                let retry = matches!(
                    version.update,
                    ProviderUpdate::Failed { .. } | ProviderUpdate::Unchanged
                );
                let button = Button::new("provider-update")
                    .label(if retry {
                        m::provider_update_retry()
                    } else {
                        m::provider_update()
                    })
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::Sm)
                    .disabled(turns_running)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.seen = false;
                        provider::start_update(&this.workspace, cx);
                    }))
                    .into_any_element();
                let text = if turns_running {
                    div().child(m::provider_update_busy()).into_any_element()
                } else {
                    TooltipTrigger::new(
                        "provider-update-command",
                        div()
                            .id("provider-update-command-text")
                            .truncate()
                            .font_family(mono.clone())
                            .child(format!("$ {command}")),
                        Tooltip::new(m::provider_update_runs(&command)).side(Side::Top),
                    )
                    .into_any_element()
                };
                update_action(text, Some(button), &colors)
            });
        div()
            .debug_selector(|| "updates-provider".into())
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(update_header(tile, m::claude_code(), summary, &colors))
            .children(provider::update_status(version, &colors, mono))
            .children(action)
            .into_any_element()
    }
}

impl Render for UpdatesButton {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let version = self.workspace.read(cx).provider_version();
        let badge = provider_badge(version, self.seen);
        match update_button(&self.app, badge, APP_UPDATES_ENABLED) {
            UpdateButton::Details { badge } => self.details_button(badge, cx),
            button => self.check_button(button, cx),
        }
    }
}

fn reload_models(workspace: &Entity<Workspace>, cx: &mut Context<UpdatesButton>) {
    let engine = workspace.read(cx).engine.clone();
    let workspace = workspace.downgrade();
    cx.spawn(async move |_, cx| {
        let models = cx
            .background_spawn(async move { engine.provider_models() })
            .await;
        workspace
            .update(cx, |workspace, cx| {
                workspace.models = models;
                cx.notify();
            })
            .ok();
    })
    .detach();
}

fn icon_view(icon: IconName, tone: Hsla, spin: bool) -> AnyElement {
    let icon = Icon::new(icon).size_4().text_color(tone);
    if spin {
        icon.with_animation(
            "spin",
            Animation::new(Duration::from_secs(1)).repeat(),
            |icon, delta| icon.transform(Transformation::rotate(percentage(delta))),
        )
        .into_any_element()
    } else {
        icon.into_any_element()
    }
}

fn trigger_frame(
    id: &'static str,
    icon: AnyElement,
    dot: Option<Hsla>,
    selected: bool,
    colors: &Colors,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(28.))
        .rounded(px(10.))
        .when(selected, |this| this.bg(colors.muted))
        .child(icon)
        .when_some(dot, |this, dot| {
            this.child(
                div()
                    .absolute()
                    .top(px(2.))
                    .right(px(2.))
                    .size(px(10.))
                    .rounded_full()
                    .border_2()
                    .border_color(colors.sidebar)
                    .bg(dot),
            )
        })
}

fn update_header(
    icon: impl IntoElement,
    title: String,
    summary: AnyElement,
    colors: &Colors,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(icon)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors.foreground)
                .child(title),
        )
        .child(summary)
}

fn version_change(installed: &str, latest: &str, colors: &Colors) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .text_xs()
        .text_color(colors.muted_foreground)
        .child(installed.to_owned())
        .child("→")
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors.foreground)
                .child(latest.to_owned()),
        )
}

/// What updating does, beside the button that does it.
fn update_action(
    text: impl IntoElement,
    action: Option<AnyElement>,
    colors: &Colors,
) -> AnyElement {
    div()
        .flex()
        .min_h(px(40.))
        .items_center()
        .gap_2()
        .rounded_lg()
        .py(px(6.))
        .pr(px(6.))
        .pl(px(10.))
        .bg(colors.muted.opacity(0.5))
        .text_xs()
        .text_color(colors.muted_foreground)
        .child(div().flex_1().min_w_0().child(text))
        .children(action)
        .into_any_element()
}
