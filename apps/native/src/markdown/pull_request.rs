//! The card a pull request link shows on hover: the pull request it points to, read from the
//! forge, or its address until then and when the forge doesn't know it.

use super::Links;
use crate::sidebar::cards::pull_request_summary;
use gpui_kit::*;
use hoshi_core::PullRequest;
use hoshi_ui::prelude::ActiveTheme as _;

/// Whether a link points to a pull request on a forge hoshi reads.
pub fn is_pull_request_url(href: &str) -> bool {
    let Some(host) = super::sites::web_host(href) else {
        return false;
    };
    let path: Vec<&str> = href
        .split_once("://")
        .and_then(|(_, rest)| rest.split_once('/'))
        .map(|(_, path)| path.split(['?', '#']).next().unwrap_or_default())
        .unwrap_or_default()
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let marker = match host.as_str() {
        "github.com" => "pull",
        "bitbucket.org" => "pull-requests",
        _ => return false,
    };
    matches!(path.as_slice(), [_, _, kind, number, ..]
        if *kind == marker && number.bytes().all(|byte| byte.is_ascii_digit()))
}

pub struct PullRequestTip {
    url: SharedString,
    pull_request: Option<PullRequest>,
}

impl PullRequestTip {
    pub fn new(url: SharedString, cx: &mut Context<Self>) -> Self {
        if let Some(engine) = cx.try_global::<Links>().map(|links| links.engine.clone()) {
            let address = url.to_string();
            let read = cx.background_spawn(async move { engine.linked_pull_request(&address) });
            cx.spawn(async move |this, cx| {
                if let Ok(Some(pull_request)) = read.await {
                    this.update(cx, |this, cx| {
                        this.pull_request = Some(pull_request);
                        cx.notify();
                    })
                    .ok();
                }
            })
            .detach();
        }
        Self {
            url,
            pull_request: None,
        }
    }
}

impl Render for PullRequestTip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let card = div()
            .font_family(gpui_kit::component::Theme::global(cx).font_family.clone())
            .rounded_lg()
            .border_1()
            .border_color(colors.foreground.opacity(0.1))
            .bg(colors.popover)
            .text_color(colors.popover_foreground)
            .shadow_md();
        match &self.pull_request {
            Some(pull_request) => {
                let logins = cx
                    .try_global::<Links>()
                    .and_then(|links| links.workspace.as_ref()?.upgrade())
                    .map(|workspace| workspace.read(cx).logins.clone())
                    .unwrap_or_default();
                card.w(px(320.)).p_3().text_sm().child(pull_request_summary(
                    pull_request,
                    &logins,
                    &colors,
                    cx,
                ))
            }
            None => card
                .max_w(rems(24.))
                .px(rems(0.625))
                .py(rems(0.375))
                .text_xs()
                .text_color(colors.muted_foreground)
                .child(self.url.clone()),
        }
    }
}

/// Lets the window know, each frame the pointer rests on a pull request link, that the link's
/// card should show. It draws nothing itself: the card stays while the pointer moves onto it.
pub struct LinkHovered(pub SharedString);

impl Render for LinkHovered {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if hover_card::hovered(&self.0, window.mouse_position(), cx) {
            window.refresh();
        }
        div()
    }
}

/// The card of the pull request link the pointer rests on, drawn by the window above
/// everything, so the pointer can move onto it without it closing.
pub mod hover_card {
    use super::PullRequestTip;
    use gpui_kit::*;
    use std::cell::RefCell;
    use std::time::{Duration, Instant};

    /// How long the card stays once the pointer left both the link and the card.
    const CLOSE_DELAY: Duration = Duration::from_millis(150);
    const CHECK_EVERY: Duration = Duration::from_millis(100);

    struct Shown {
        url: SharedString,
        at: Point<Pixels>,
        link_seen: Instant,
        card: Entity<PullRequestTip>,
        bounds: Option<Bounds<Pixels>>,
        left: Option<Instant>,
    }

    thread_local! {
        static SHOWN: RefCell<Option<Shown>> = const { RefCell::new(None) };
    }

    /// Notes the pointer on a link; `true` when its card just opened.
    pub fn hovered(url: &SharedString, at: Point<Pixels>, cx: &mut App) -> bool {
        let same = SHOWN.with(|shown| {
            let mut shown = shown.borrow_mut();
            match shown.as_mut() {
                Some(shown) if shown.url == *url => {
                    shown.link_seen = Instant::now();
                    shown.left = None;
                    true
                }
                _ => false,
            }
        });
        if same {
            return false;
        }
        let card = cx.new(|cx| PullRequestTip::new(url.clone(), cx));
        SHOWN.with(|shown| {
            *shown.borrow_mut() = Some(Shown {
                url: url.clone(),
                at,
                link_seen: Instant::now(),
                card,
                bounds: None,
                left: None,
            })
        });
        true
    }

    fn set_bounds(bounds: Bounds<Pixels>) {
        SHOWN.with(|shown| {
            if let Some(shown) = shown.borrow_mut().as_mut() {
                shown.bounds = Some(bounds);
            }
        });
    }

    /// The card while the pointer is on its link or on it, and a little after it left both.
    pub fn render(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        let mouse = window.mouse_position();
        let (card, at) = SHOWN.with(|shown| {
            let mut slot = shown.borrow_mut();
            let shown = slot.as_mut()?;
            let on_card = shown
                .bounds
                .is_some_and(|bounds| bounds.dilate(px(6.)).contains(&mouse));
            let on_link = shown.link_seen.elapsed() < CHECK_EVERY * 2;
            if on_card || on_link {
                shown.left = None;
            } else if shown.left.get_or_insert_with(Instant::now).elapsed() >= CLOSE_DELAY {
                *slot = None;
                return None;
            }
            Some((shown.card.clone(), shown.at))
        })?;
        cx.spawn(async move |cx| {
            cx.background_executor().timer(CHECK_EVERY).await;
            cx.update(|cx| cx.refresh_windows());
        })
        .detach();
        Some(
            deferred(
                anchored()
                    .position(at + point(px(4.), px(18.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div().relative().occlude().child(card).child(
                            canvas(|bounds, _, _| set_bounds(bounds), |_, _, _, _| {})
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full(),
                        ),
                    ),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::is_pull_request_url;

    #[test]
    fn pull_request_links_of_both_forges() {
        assert!(is_pull_request_url(
            "https://bitbucket.org/noshi-test-dev/test/pull-requests/2"
        ));
        assert!(is_pull_request_url(
            "https://github.com/nxtvoid/hoshi/pull/11#discussion"
        ));
        assert!(!is_pull_request_url(
            "https://github.com/nxtvoid/hoshi/issues/11"
        ));
        assert!(!is_pull_request_url(
            "https://bitbucket.org/noshi-test-dev/test"
        ));
    }
}
