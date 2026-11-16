//! Beside the model picker: how full the thread's context window is, with compacting at hand,
//! and whether the next send compacts first because the prompt cache ran out.

use super::Composer;
use crate::format::compact_number;
use crate::m;
use gpui_kit::component::hover_card::HoverCard;
use gpui_kit::*;
use hoshi_core::{Command, ContextUsage, compacts_on_return};
use hoshi_ui::prelude::*;
use std::f32::consts::{FRAC_PI_2, TAU};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const COMPACT_COMMAND: &str = "/compact";
const CROWDED_PERCENT: f32 = 90.;
/// How often a stale cache is looked for while the composer shows.
pub(super) const CHECK_EVERY: Duration = Duration::from_secs(30);

pub fn context_percent(usage: &ContextUsage) -> Option<f32> {
    let max = usage.max_tokens.filter(|max| *max > 0)?;
    Some((usage.used_tokens as f32 / max as f32 * 100.).min(100.))
}

pub fn is_compact_command(text: &str) -> bool {
    let text = text.trim();
    text.get(..COMPACT_COMMAND.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(COMPACT_COMMAND))
        && text[COMPACT_COMMAND.len()..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

/// The open thread's next send while compacting is due: its size, and whether it compacts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Due {
    pub tokens: u64,
    pub compact: bool,
}

impl Composer {
    fn turn_running(&self, thread_id: &str, cx: &App) -> bool {
        self.workspace
            .read(cx)
            .model
            .thread(thread_id)
            .is_some_and(|thread| thread.active_turn_id.is_some())
    }

    pub(super) fn compaction(&self, cx: &App) -> Option<Due> {
        let workspace = self.workspace.read(cx);
        let thread_id = workspace.open_thread_id(cx)?;
        let thread = workspace.model.thread(&thread_id)?;
        let compacts = workspace
            .catalog(&thread.provider)
            .is_some_and(|catalog| catalog.compacts_idle_threads);
        let usage = workspace.context_usages.get(&thread_id);
        let mode = workspace.settings.providers.idle_compaction;
        if !compacts
            || thread.active_turn_id.is_some()
            || !compacts_on_return(mode, usage, now_millis())
        {
            return None;
        }
        Some(Due {
            tokens: usage?.used_tokens,
            compact: !self.kept_history.contains(&thread_id),
        })
    }

    /// Whether `text`, sent now, goes after compacting the thread.
    pub(super) fn compacts_first(&self, text: &str, cx: &App) -> bool {
        self.compaction(cx).is_some_and(|due| due.compact) && !is_compact_command(text)
    }

    fn toggle_kept_history(&mut self, cx: &mut Context<Self>) {
        let Some(thread_id) = self.workspace.read(cx).open_thread_id(cx) else {
            return;
        };
        if !self.kept_history.remove(&thread_id) {
            self.kept_history.insert(thread_id);
        }
        cx.notify();
    }

    fn compact_context(&mut self, cx: &mut Context<Self>) {
        let Some(thread_id) = self.workspace.read(cx).open_thread_id(cx) else {
            return;
        };
        self.dispatch(
            Command::MessageSend {
                thread_id,
                message_id: uuid::Uuid::new_v4().to_string(),
                text: COMPACT_COMMAND.to_owned(),
                attachments: Vec::new(),
            },
            cx,
        );
    }

    pub(super) fn render_compaction_chip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let due = self.compaction(cx)?;
        let colors = cx.hoshi();
        let tokens = compact_number(due.tokens);
        let (label, hint) = if due.compact {
            (
                m::compaction_chip_compact(&tokens),
                m::compaction_chip_compact_hint(&tokens),
            )
        } else {
            (
                m::compaction_chip_keep(&tokens),
                m::compaction_chip_keep_hint(&tokens),
            )
        };
        let color = if due.compact {
            colors.foreground
        } else {
            colors.muted_foreground
        };
        Some(
            Button::new("compaction-chip")
                .icon(IconName::Minimize2)
                .label(label)
                .size(ButtonSize::Xs)
                .variant(ButtonVariant::Ghost)
                .text_color(color)
                .tooltip(Tooltip::new(hint).side(Side::Top).max_width(px(256.)))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_kept_history(cx)))
                .into_any_element(),
        )
    }

    pub(super) fn render_context_meter(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let workspace = self.workspace.read(cx);
        let thread_id = workspace.open_thread_id(cx)?;
        let usage = *workspace.context_usages.get(&thread_id)?;
        let working = self.turn_running(&thread_id, cx);
        let colors = cx.hoshi().clone();
        let percent = context_percent(&usage);
        let used = compact_number(usage.used_tokens);
        let shown = percent.map(|percent| percent.round() as u32);
        let crowded = percent.is_some_and(|percent| percent > CROWDED_PERCENT);
        let label: SharedString = match shown {
            None => m::context_tokens_used(&used),
            Some(shown) => m::context_window_usage(
                compact_number(usage.max_tokens.unwrap_or_default()),
                shown,
                &used,
            ),
        }
        .into();
        let (rest, hover) = if crowded {
            (colors.status_error, colors.status_error)
        } else {
            (colors.muted_foreground, colors.foreground)
        };
        let trigger = div()
            .id("context-meter")
            .flex()
            .h_6()
            .items_center()
            .gap_1p5()
            .rounded_md()
            .px_1p5()
            .text_color(rest)
            .hover(move |style| style.text_color(hover))
            .child(ring(percent.unwrap_or_default()))
            .child(match shown {
                None => used.clone(),
                Some(shown) => format!("{shown}%"),
            });
        let this = cx.weak_entity();
        Some(
            HoverCard::new("context-meter-card")
                .anchor(Anchor::BottomRight)
                .open_delay(Duration::from_millis(150))
                .trigger(trigger)
                .content(move |_, _, cx| {
                    let colors = cx.hoshi();
                    let this = this.clone();
                    div()
                        .w(px(240.))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .text_xs()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_3()
                                .child(
                                    div()
                                        .text_color(colors.muted_foreground)
                                        .child(m::context_window()),
                                )
                                .child(div().font_weight(FontWeight::MEDIUM).child(label.clone())),
                        )
                        .when_some(percent, |card, percent| {
                            card.child(
                                div()
                                    .h(px(6.))
                                    .overflow_hidden()
                                    .rounded_full()
                                    .bg(colors.muted)
                                    .child(
                                        div()
                                            .h_full()
                                            .rounded_full()
                                            .w(relative(percent / 100.))
                                            .bg(if crowded {
                                                colors.status_error
                                            } else {
                                                colors.muted_foreground
                                            }),
                                    ),
                            )
                        })
                        .child(
                            div()
                                .text_color(colors.muted_foreground)
                                .child(match percent {
                                    None => m::context_window_size_pending(),
                                    Some(_) => m::context_compacts_automatically(),
                                }),
                        )
                        .child(
                            div().flex().child(
                                Button::new("compact-context")
                                    .icon(IconName::Minimize2)
                                    .label(m::compact_context())
                                    .size(ButtonSize::Xs)
                                    .variant(ButtonVariant::Outline)
                                    .disabled(working)
                                    .on_click(move |_, _, cx| {
                                        this.update(cx, |this, cx| this.compact_context(cx)).ok();
                                    }),
                            ),
                        )
                })
                .into_any_element(),
        )
    }
}

/// A circle filled to `percent` clockwise from the top, in the text color around it.
fn ring(percent: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let color = window.text_style().color;
            let size = bounds.size.width.min(bounds.size.height);
            let stroke = size * (3.5 / 24.);
            let radius = size * (9. / 24.);
            let center = bounds.center();
            if let Some(track) = arc(center, radius, stroke, 1.) {
                window.paint_path(track, color.opacity(0.25));
            }
            if percent > 0.
                && let Some(filled) = arc(center, radius, stroke, percent / 100.)
            {
                window.paint_path(filled, color);
            }
        },
    )
    .flex_none()
    .size_3p5()
}

fn arc(
    center: Point<Pixels>,
    radius: Pixels,
    stroke: Pixels,
    fraction: f32,
) -> Option<Path<Pixels>> {
    let mut builder = PathBuilder::stroke(stroke);
    let steps = (48. * fraction).ceil().max(1.) as usize;
    for step in 0..=steps {
        let angle = -FRAC_PI_2 + TAU * fraction * step as f32 / steps as f32;
        let at = point(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        );
        if step == 0 {
            builder.move_to(at);
        } else {
            builder.line_to(at);
        }
    }
    builder.build().ok()
}

#[cfg(test)]
mod tests {
    use super::{context_percent, is_compact_command};
    use hoshi_core::ContextUsage;

    fn usage(
        used_tokens: u64,
        max_tokens: Option<u64>,
        cache_expires_at: Option<u64>,
    ) -> ContextUsage {
        ContextUsage {
            used_tokens,
            max_tokens,
            cache_ttl_secs: None,
            cache_expires_at,
        }
    }

    #[test]
    fn the_percent_is_known_once_the_window_is_and_never_passes_full() {
        assert_eq!(context_percent(&usage(50, None, None)), None);
        assert_eq!(context_percent(&usage(50, Some(0), None)), None);
        assert_eq!(context_percent(&usage(50, Some(200), None)), Some(25.));
        assert_eq!(context_percent(&usage(300, Some(200), None)), Some(100.));
    }

    #[test]
    fn only_the_compact_command_itself_counts() {
        assert!(is_compact_command("/compact"));
        assert!(is_compact_command("  /COMPACT keep the plan "));
        assert!(!is_compact_command("/compacted"));
        assert!(!is_compact_command("please /compact"));
        assert!(!is_compact_command("/comp"));
        assert!(!is_compact_command("/compaçt"));
    }
}
