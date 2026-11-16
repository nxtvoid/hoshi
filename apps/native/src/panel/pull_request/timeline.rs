use super::super::panel_message;
use super::{Editing, PullRequestSurface, card, markdown, review_look, section_title};
use crate::m;
use crate::sections::relative_time;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::{PullRequestDetail, ReviewState, TimelineItem};
use hoshi_ui::prelude::*;

impl PullRequestSurface {
    pub(super) fn timeline(
        &self,
        detail: &PullRequestDetail,
        commenting: bool,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = chrono::Utc::now();
        let mono = cx.theme().mono_font_family.clone();
        let items = if detail.timeline.is_empty() {
            panel_message(m::pr_no_activity(), colors).into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap_3()
                .children(
                    detail
                        .timeline
                        .iter()
                        .enumerate()
                        .map(|(index, item)| match item {
                            TimelineItem::Commit {
                                sha,
                                message,
                                created_at,
                                url,
                                ..
                            } => {
                                let url = url.clone();
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .text_xs()
                                    .text_color(colors.muted_foreground)
                                    .child(
                                        Icon::new(IconName::GitCommitHorizontal)
                                            .size_3p5()
                                            .flex_none(),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_color(colors.foreground)
                                            .child(
                                                message
                                                    .lines()
                                                    .next()
                                                    .unwrap_or_default()
                                                    .to_owned(),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .id(("timeline-commit", index))
                                            .flex_none()
                                            .font_family(mono.clone())
                                            .cursor_pointer()
                                            .hover(|sha| sha.text_color(colors.foreground))
                                            .child(sha.chars().take(7).collect::<String>())
                                            .on_click(move |_, _, cx| cx.open_url(&url)),
                                    )
                                    .child(div().flex_none().child(relative_time(created_at, now)))
                                    .into_any_element()
                            }
                            TimelineItem::Comment {
                                author,
                                body,
                                created_at,
                                ..
                            } => entry(
                                index,
                                author.as_deref(),
                                None,
                                body,
                                &relative_time(created_at, now),
                                colors,
                            ),
                            TimelineItem::Review {
                                author,
                                state,
                                body,
                                created_at,
                                ..
                            } => entry(
                                index,
                                author.as_deref(),
                                Some(*state),
                                body,
                                &relative_time(created_at, now),
                                colors,
                            ),
                        }),
                )
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .child(items)
            .when(commenting, |timeline| {
                timeline.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1p5()
                        .pt_3()
                        .border_t_1()
                        .border_color(colors.border)
                        .child(section_title(m::add_comment(), colors))
                        .child(self.editor(Editing::Comment, m::comment(), false, colors, cx)),
                )
            })
            .into_any_element()
    }
}

fn entry(
    index: usize,
    author: Option<&str>,
    review: Option<ReviewState>,
    body: &str,
    time: &str,
    colors: &Colors,
) -> AnyElement {
    let (icon, color, label) = match review {
        Some(state) => {
            let (icon, color, label) = review_look(state, colors);
            (icon, color, Some(label))
        }
        None => (IconName::MessageSquare, colors.muted_foreground, None),
    };
    card(colors)
        .child(
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .px_2p5()
                .py_1p5()
                .when(!body.trim().is_empty(), |header| {
                    header.border_b_1().border_color(colors.border.opacity(0.6))
                })
                .text_color(colors.muted_foreground)
                .child(Icon::new(icon).size_3p5().text_color(color))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(colors.foreground)
                        .child(author.unwrap_or_default().to_owned()),
                )
                .children(label)
                .child(div().flex_1())
                .child(time.to_owned()),
        )
        .when(!body.trim().is_empty(), |card| {
            card.child(div().px_2p5().py_2().child(markdown(
                &format!("pr-timeline-{index}"),
                body,
                colors,
            )))
        })
        .into_any_element()
}
