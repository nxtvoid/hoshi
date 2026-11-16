//! News on a pull request the thread watches, which woke its agent: one line that opens to
//! what the agent was told.

use crate::m;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_core::PullRequestUpdate;
use hoshi_ui::assets::IconName;
use hoshi_ui::prelude::{Colors, PressSelectsNothing as _};

/// What changed, in a few words: "checks failed, 2 new comments".
pub fn update_summary(update: &PullRequestUpdate) -> String {
    let mut parts = Vec::new();
    if !update.failed_checks.is_empty() {
        parts.push(m::pr_update_checks_failed());
    }
    if update.checks_passed {
        parts.push(m::pr_update_checks_passed());
    }
    if update.comments > 0 {
        parts.push(m::pr_update_comments(update.comments as usize));
    }
    if update.conflicting {
        parts.push(m::pr_update_conflicting());
    }
    if update.closed {
        parts.push(m::pr_update_closed());
    }
    if update.stopped && !update.closed {
        parts.push(m::pr_update_stopped());
    }
    parts.join(", ")
}

pub fn pull_request_update_row(
    id: &str,
    update: &PullRequestUpdate,
    text: &str,
    open: bool,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    colors: &Colors,
) -> AnyElement {
    let alarming = !update.failed_checks.is_empty() || update.conflicting;
    let selector = format!("pr-update-{id}");
    let foreground = colors.foreground;
    let line = div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .flex()
        .min_h_6()
        .items_center()
        .gap_2()
        .rounded_md()
        .text_sm()
        .text_color(colors.muted_foreground)
        .cursor_pointer()
        .hover(move |line| line.text_color(foreground))
        .press_selects_nothing()
        .on_click(on_toggle)
        .child(
            Icon::new(if alarming {
                IconName::CircleAlert
            } else {
                IconName::Eye
            })
            .size_4()
            .flex_none()
            .when(alarming, |icon| icon.text_color(colors.status_error)),
        )
        .child(
            div()
                .flex_none()
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors.foreground)
                .child(format!("#{}", update.number)),
        )
        .child(div().min_w_0().truncate().child(update_summary(update)))
        .child(
            Icon::new(IconName::ChevronRight)
                .size_3p5()
                .flex_none()
                .when(open, |icon| {
                    icon.transform(Transformation::rotate(percentage(0.25)))
                }),
        );
    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .child(line)
        .when(open, |row| {
            row.child(
                div()
                    .ml_2()
                    .pl_4()
                    .border_l_1()
                    .border_color(colors.border)
                    .text_sm()
                    .text_color(colors.muted_foreground)
                    .child(text.to_owned()),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::update_summary;
    use hoshi_core::PullRequestUpdate;

    #[test]
    fn a_summary_names_what_changed() {
        let update = PullRequestUpdate {
            number: 7,
            failed_checks: vec!["build".into()],
            checks_passed: false,
            comments: 2,
            conflicting: true,
            closed: false,
            stopped: true,
        };
        assert_eq!(
            update_summary(&update),
            "checks failed, 2 new comments, conflicts with its base, stopped watching"
        );
        let closed = PullRequestUpdate {
            comments: 1,
            closed: true,
            stopped: true,
            failed_checks: Vec::new(),
            conflicting: false,
            ..update
        };
        assert_eq!(
            update_summary(&closed),
            "1 new comment, closed, stopped watching"
        );
    }
}
