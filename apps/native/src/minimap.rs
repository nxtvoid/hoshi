//! A mark per message the user sent, beside the transcript, as the web shows it. The message
//! being read is brighter; pointing at a mark previews it and its answer, clicking scrolls
//! there, and the arrows past the rail's ends step to the message before or after.

use crate::m;
use crate::transcript::{Row, TranscriptView};
use gpui_kit::*;
use hoshi_core::{ItemContent, ThreadItem};
use hoshi_ui::assets::IconName;
use hoshi_ui::prelude::*;

/// Fewer messages than this need no map.
const MIN_ITEMS: usize = 2;
/// Room between two marks, until the rail fills the height it may take.
const SPACING: Pixels = px(8.);
/// The map needs this much room beside the chat, so it never covers the messages.
const MIN_GUTTER: Pixels = px(48.);
const MARK_WIDTH: f32 = 24.;
const RAIL_LEFT: Pixels = px(12.);

/// A message the user sent, by its row, with what the agent last answered to it.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub row: usize,
    pub user: String,
    pub answer: Option<String>,
}

pub fn marks(rows: &[Row], items: &[ThreadItem]) -> Vec<Mark> {
    let mut marks: Vec<Mark> = Vec::new();
    for (at, row) in rows.iter().enumerate() {
        let Row::Item(index) = row else {
            continue;
        };
        match items.get(*index).map(|item| &item.content) {
            Some(ItemContent::UserMessage {
                text,
                steer: false,
                resumed: None,
                ..
            }) => marks.push(Mark {
                row: at,
                user: text.clone(),
                answer: None,
            }),
            Some(ItemContent::AssistantMessage { text }) => {
                if let Some(last) = marks.last_mut() {
                    last.answer = Some(text.clone());
                }
            }
            _ => {}
        }
    }
    marks
}

/// The mark nearest `y` on a rail that starts at `top` and is `height` tall.
pub fn index_at(y: Pixels, top: Pixels, height: Pixels, count: usize) -> Option<usize> {
    if count == 0 || height <= px(0.) {
        return None;
    }
    if count == 1 {
        return Some(0);
    }
    let progress = ((y - top) / height).clamp(0., 1.);
    Some((progress * (count - 1) as f32).round() as usize)
}

/// The message being read: the last one that starts at or above the first row shown.
pub fn current(marks: &[Mark], top_row: usize) -> Option<usize> {
    marks
        .iter()
        .rposition(|mark| mark.row <= top_row)
        .or_else(|| (!marks.is_empty()).then_some(0))
}

/// Markdown as one line of plain text, for a preview.
pub fn plain_preview(markdown: &str, limit: usize) -> String {
    let text: String = markdown.chars().take(limit).collect();
    crate::markdown::plain_text(&text)
}

#[derive(Default)]
pub struct MinimapState {
    pub size: Size<Pixels>,
    pub origin: Point<Pixels>,
    pub pointed: Option<usize>,
    pub over: bool,
}

impl TranscriptView {
    fn jump(&mut self, row: usize, cx: &mut Context<Self>) {
        self.scroller()
            .update(cx, |scroller, cx| scroller.scroll_to_item(row, cx));
    }

    /// The map beside the rows, while there are enough messages and room for it.
    pub(crate) fn minimap(&self, marks: Vec<Mark>, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.minimap_state();
        let size = state.size;
        let measure = canvas(
            {
                let view = cx.weak_entity();
                move |bounds, _, cx| {
                    view.update(cx, |view, cx| {
                        let state = view.minimap_state_mut();
                        if state.size != bounds.size || state.origin != bounds.origin {
                            state.size = bounds.size;
                            state.origin = bounds.origin;
                            cx.notify();
                        }
                    })
                    .ok();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let fits = (size.width - px(Lane::WIDTH)) / 2. >= MIN_GUTTER;
        if marks.len() < MIN_ITEMS || !fits {
            return Some(measure.into_any_element());
        }
        let colors = cx.hoshi().clone();
        let count = marks.len();
        let rail_height = (SPACING * (count - 1) as f32).min(size.height - px(128.));
        let rail_top = (size.height - rail_height) / 2.;
        let top_row = self.scroller().read(cx).logical_scroll_top().item_ix;
        let reading = current(&marks, top_row);
        let pointed = state.pointed.filter(|index| *index < count);
        let mark_top = |index: usize| {
            if count <= 1 {
                rail_top
            } else {
                rail_top + rail_height * (index as f32 / (count - 1) as f32)
            }
        };
        let lines = (0..count).map(|index| {
            let distance = pointed.map(|pointed| pointed.abs_diff(index));
            let scale = match distance {
                Some(0) => 1.,
                Some(1) => 0.667,
                Some(2) => 0.417,
                _ => 0.333,
            };
            let color = if distance == Some(0) {
                colors.foreground.opacity(0.7)
            } else if Some(index) == reading {
                colors.foreground.opacity(0.45)
            } else {
                colors.muted_foreground.opacity(0.25)
            };
            div()
                .absolute()
                .left(RAIL_LEFT)
                .top(mark_top(index) - px(1.))
                .h(px(2.))
                .w(px(MARK_WIDTH * scale))
                .rounded_full()
                .bg(color)
        });
        let rows: Vec<usize> = marks.iter().map(|mark| mark.row).collect();
        let pointer_rows = rows.clone();
        let hit = div()
            .id("minimap-rail")
            .absolute()
            .left(RAIL_LEFT)
            .top(rail_top - px(4.))
            .w(px(32.))
            .h(rail_height + px(8.))
            .cursor_pointer()
            .on_mouse_move(cx.listener(move |view, event: &MouseMoveEvent, _, cx| {
                let pointed = index_at(
                    event.position.y - view.minimap_origin_y(),
                    rail_top,
                    rail_height,
                    count,
                );
                if view.minimap_state().pointed != pointed {
                    view.minimap_state_mut().pointed = pointed;
                    cx.notify();
                }
            }))
            .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
                view.minimap_state_mut().over = *hovered;
                if !*hovered {
                    view.minimap_state_mut().pointed = None;
                }
                cx.notify();
            }))
            .on_click(cx.listener(move |view, event: &ClickEvent, _, cx| {
                let y = event.position().y - view.minimap_origin_y();
                if let Some(index) = index_at(y, rail_top, rail_height, count) {
                    view.jump(pointer_rows[index], cx);
                }
            }));
        let preview = pointed.and_then(|index| marks.get(index).map(|mark| (index, mark)));
        let preview =
            preview.map(|(index, mark)| {
                let user = plain_preview(&mark.user, 1500);
                let top = mark_top(index);
                let card_height = px(84.);
                let top = if index == 0 {
                    top
                } else if index + 1 == count {
                    top - card_height
                } else {
                    top - card_height / 2.
                };
                div()
                    .absolute()
                    .left(RAIL_LEFT + px(32.))
                    .top(top.max(px(0.)))
                    .w(px(320.))
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(colors.foreground.opacity(0.1))
                    .bg(colors.popover)
                    .text_color(colors.popover_foreground)
                    .shadow_md()
                    .text_sm()
                    .child(div().truncate().font_weight(FontWeight::MEDIUM).child(
                        if user.is_empty() {
                            m::user_message()
                        } else {
                            user
                        },
                    ))
                    .children(mark.answer.as_ref().map(|answer| {
                        div()
                            .mt_1()
                            .line_clamp(3)
                            .text_color(colors.muted_foreground)
                            .child(plain_preview(answer, 1500))
                    }))
            });
        let previous = reading
            .and_then(|index| index.checked_sub(1))
            .map(|index| rows[index]);
        let next = reading
            .map(|index| index + 1)
            .filter(|index| *index < count)
            .map(|index| rows[index]);
        let step = |id: &'static str,
                    icon: IconName,
                    label: String,
                    target: Option<usize>,
                    top: Pixels,
                    cx: &mut Context<Self>| {
            div().absolute().left(px(4.)).top(top).child(
                Button::new(id)
                    .icon(icon)
                    .size(ButtonSize::IconXs)
                    .variant(ButtonVariant::Ghost)
                    .disabled(target.is_none())
                    .tooltip(Tooltip::new(label))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if let Some(row) = target {
                            view.jump(row, cx);
                        }
                    })),
            )
        };
        let over = state.over;
        let steps = over.then(|| {
            [
                step(
                    "minimap-previous",
                    IconName::ChevronUp,
                    m::previous_message(),
                    previous,
                    rail_top - px(30.),
                    cx,
                ),
                step(
                    "minimap-next",
                    IconName::ChevronDown,
                    m::next_message(),
                    next,
                    rail_top + rail_height + px(6.),
                    cx,
                ),
            ]
        });
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .child(measure)
                .child(
                    div()
                        .id("minimap")
                        .absolute()
                        .top_0()
                        .left_0()
                        .h_full()
                        .w(px(56.))
                        .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
                            view.minimap_state_mut().over = *hovered;
                            cx.notify();
                        }))
                        .children(lines)
                        .child(hit)
                        .children(steps.into_iter().flatten())
                        .children(preview),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Mark, current, index_at, plain_preview};
    use gpui_kit::px;

    #[test]
    fn the_rail_maps_the_pointer_to_the_nearest_mark() {
        assert_eq!(index_at(px(0.), px(10.), px(80.), 5), Some(0));
        assert_eq!(index_at(px(50.), px(10.), px(80.), 5), Some(2));
        assert_eq!(index_at(px(200.), px(10.), px(80.), 5), Some(4));
        let marks: Vec<Mark> = [0, 4, 9]
            .into_iter()
            .map(|row| Mark {
                row,
                user: String::new(),
                answer: None,
            })
            .collect();
        assert_eq!(current(&marks, 5), Some(1));
        assert_eq!(current(&marks, 9), Some(2));
        assert_eq!(
            plain_preview("# Title\n- **bold** `code`", 100),
            "Title bold code"
        );
    }
}
