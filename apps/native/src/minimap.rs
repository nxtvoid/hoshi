//! A mark per message the user sent, beside the transcript, as the web shows it. The marks of
//! the turns on screen are brighter; pointing at a mark previews it and its answer, clicking
//! scrolls there, and the arrows past the rail's ends step to the message before or after.

use crate::m;
use crate::transcript::{Row, TranscriptView};
use gpui_kit::*;
use hoshi_core::{ItemContent, ThreadItem};
use hoshi_ui::assets::IconName;
use hoshi_ui::prelude::*;
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

/// Fewer messages than this need no map.
const MIN_ITEMS: usize = 2;
/// Room between two marks, until the rail fills the height it may take.
const SPACING: Pixels = px(8.);
/// Beside a narrower gutter the map only shows while pointed at, over the messages.
const MIN_GUTTER: Pixels = px(48.);
/// The strip that shows the map beside a narrow gutter.
const NARROW_STRIP: Pixels = px(8.);
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
                resumed: None,
                pull_request_update: None,
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

/// The message being read, among the rows `shown`: the first on screen, else the one whose
/// turn is.
pub fn current(marks: &[Mark], shown: (usize, usize)) -> Option<usize> {
    let (start, end) = shown;
    let mut preceding = None;
    for (index, mark) in marks.iter().enumerate() {
        if mark.row >= start {
            return if mark.row <= end {
                Some(index)
            } else {
                preceding
            };
        }
        preceding = Some(index);
    }
    preceding
}

/// Whether the turn of the mark at `index`, from its message up to the next, has a row among
/// those `shown`.
pub fn in_view(marks: &[Mark], index: usize, shown: (usize, usize)) -> bool {
    let (start, end) = shown;
    marks.get(index).is_some_and(|mark| {
        mark.row <= end && marks.get(index + 1).is_none_or(|next| next.row > start)
    })
}

/// The first and last of the rows `painted` that reach between `top` and `bottom`. A list
/// lays its rows out again, top to bottom, when one asks to be scrolled into view: only the
/// last pass counts.
pub fn shown_rows(
    painted: &[(usize, Bounds<Pixels>)],
    top: Pixels,
    bottom: Pixels,
) -> Option<(usize, usize)> {
    let last_pass = painted
        .windows(2)
        .rposition(|pair| pair[1].0 <= pair[0].0 || pair[1].1.top() < pair[0].1.top())
        .map_or(0, |at| at + 1);
    painted[last_pass..]
        .iter()
        .filter(|(_, bounds)| {
            bounds.top() < bottom && bounds.top() + bounds.size.height.max(px(1.)) > top
        })
        .fold(None, |shown, (row, _)| match shown {
            None => Some((*row, *row)),
            Some((start, end)) => Some((start.min(*row), end.max(*row))),
        })
}

/// The marks the arrows step to from the message being read, by index: none before the
/// first, none after the last or once the end is shown, and the first when only rows before
/// it are on screen.
pub fn steps(reading: Option<usize>, count: usize, at_end: bool) -> (Option<usize>, Option<usize>) {
    let previous = reading.and_then(|index| index.checked_sub(1));
    let next = reading.map_or(0, |index| index + 1);
    (previous, (next < count && !at_end).then_some(next))
}

/// Markdown as one line of plain text, for a preview.
pub fn plain_preview(markdown: &str, limit: usize) -> String {
    let text: String = markdown.chars().take(limit).collect();
    crate::markdown::plain_text(&text)
}

/// Each row laid out this frame, with where.
pub type PaintedRows = Rc<RefCell<Vec<(usize, Bounds<Pixels>)>>>;

#[derive(Default)]
pub struct MinimapState {
    pub size: Size<Pixels>,
    pub origin: Point<Pixels>,
    pub pointed: Option<usize>,
    pub over: bool,
    /// Where each row was laid out this frame, as the list lays out only the rows near the
    /// screen.
    pub painted: PaintedRows,
    /// The first and last rows on screen.
    pub shown: Option<(usize, usize)>,
}

impl MinimapState {
    /// Follows the rows on screen once the rows `changed` became `count` others: those after
    /// the change move with it, and rows on screen within it are forgotten until measured.
    pub fn rows_spliced(&mut self, changed: Range<usize>, count: usize) {
        self.shown = self.shown.and_then(|(start, end)| {
            (start >= changed.end)
                .then(|| (start - changed.len() + count, end - changed.len() + count))
        });
    }
}

/// Records where the row at `index` is laid out, for the map.
pub fn row_probe(painted: &PaintedRows, index: usize) -> impl IntoElement + use<> {
    let painted = painted.clone();
    canvas(
        move |bounds, _, _| painted.borrow_mut().push((index, bounds)),
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
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
                let painted = state.painted.clone();
                move |bounds, _, cx| {
                    let painted = std::mem::take(&mut *painted.borrow_mut());
                    let shown = shown_rows(&painted, bounds.top(), bounds.bottom());
                    let view = view.clone();
                    cx.defer(move |cx| {
                        view.update(cx, |view, cx| {
                            let state = view.minimap_state_mut();
                            if state.size != bounds.size
                                || state.origin != bounds.origin
                                || state.shown != shown
                            {
                                state.size = bounds.size;
                                state.origin = bounds.origin;
                                state.shown = shown;
                                cx.notify();
                            }
                        })
                        .ok();
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let gutter = (size.width - px(Lane::WIDTH)) / 2.;
        let roomy = gutter >= MIN_GUTTER;
        if marks.len() < MIN_ITEMS || gutter <= px(0.) {
            return Some(measure.into_any_element());
        }
        let colors = cx.hoshi().clone();
        let count = marks.len();
        let rail_height = (SPACING * (count - 1) as f32).min(size.height - px(128.));
        let rail_top = (size.height - rail_height) / 2.;
        let scroller = self.scroller().read(cx);
        let at_end = !scroller.is_scrolled_up();
        let shown = state.shown.unwrap_or_else(|| {
            let top_row = scroller.logical_scroll_top().item_ix;
            (top_row, top_row)
        });
        let reading = current(&marks, shown);
        let pointed = state.pointed.filter(|index| *index < count);
        let over = state.over;
        let rail_left = if roomy { RAIL_LEFT } else { px(0.) };
        let strip_width = if roomy {
            px(32.)
        } else {
            NARROW_STRIP.min(gutter)
        };
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
            } else if in_view(&marks, index, shown) {
                colors.foreground.opacity(0.45)
            } else {
                colors.muted_foreground.opacity(0.25)
            };
            div()
                .absolute()
                .left(rail_left)
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
            .left(rail_left)
            .top(rail_top - px(4.))
            .w(strip_width)
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
                let state = view.minimap_state_mut();
                if *hovered {
                    state.over = true;
                } else {
                    state.pointed = None;
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
                    .left(rail_left + px(32.))
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
        let (previous, next) = steps(reading, count, at_end);
        let previous = previous.map(|index| rows[index]);
        let next = next.map(|index| rows[index]);
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
        let steps = (over && roomy).then(|| {
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
                        .w(if roomy { px(56.) } else { strip_width })
                        .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
                            view.minimap_state_mut().over = *hovered;
                            cx.notify();
                        }))
                        .when(roomy || over, |map| map.children(lines))
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
    use super::{
        Mark, MinimapState, current, in_view, index_at, marks, plain_preview, shown_rows, steps,
    };
    use crate::transcript::Row;
    use gpui_kit::{Bounds, point, px, size};
    use hoshi_core::{ItemContent, ThreadItem};

    fn item(id: &str, content: ItemContent) -> ThreadItem {
        ThreadItem {
            id: id.into(),
            thread_id: "t".into(),
            created_at: "2026-11-08T10:00:00Z".into(),
            content,
        }
    }

    fn message(id: &str, steer: bool) -> ThreadItem {
        item(
            id,
            ItemContent::UserMessage {
                text: id.into(),
                attachments: Vec::new(),
                steer,
                task_result: None,
                pull_request_update: None,
                resumed: None,
                rewind: None,
                dropped: false,
                model_switch: None,
            },
        )
    }

    fn answer(id: &str) -> ThreadItem {
        item(id, ItemContent::AssistantMessage { text: id.into() })
    }

    #[test]
    fn a_message_sent_while_the_agent_works_gets_its_own_mark() {
        let items = [
            message("u1", false),
            answer("partial"),
            message("s", true),
            answer("a1"),
        ];
        let rows: Vec<Row> = (0..items.len()).map(Row::Item).collect();
        assert_eq!(
            marks(&rows, &items),
            [
                Mark {
                    row: 0,
                    user: "u1".into(),
                    answer: Some("partial".into()),
                },
                Mark {
                    row: 2,
                    user: "s".into(),
                    answer: Some("a1".into()),
                },
            ]
        );
    }

    #[test]
    fn the_rail_maps_the_pointer_to_the_nearest_mark() {
        assert_eq!(index_at(px(0.), px(10.), px(80.), 5), Some(0));
        assert_eq!(index_at(px(50.), px(10.), px(80.), 5), Some(2));
        assert_eq!(index_at(px(200.), px(10.), px(80.), 5), Some(4));
        assert_eq!(
            plain_preview("# Title\n- **bold** `code`", 100),
            "Title bold code"
        );
    }

    fn at_rows(rows: &[usize]) -> Vec<Mark> {
        rows.iter()
            .map(|row| Mark {
                row: *row,
                user: String::new(),
                answer: None,
            })
            .collect()
    }

    #[test]
    fn the_message_read_is_the_first_on_screen_else_the_one_whose_turn_is() {
        let marks = at_rows(&[0, 4, 9]);
        assert_eq!(current(&marks, (2, 6)), Some(1));
        assert_eq!(current(&marks, (4, 12)), Some(1));
        assert_eq!(current(&marks, (5, 8)), Some(1));
        assert_eq!(current(&marks, (10, 12)), Some(2));
        assert_eq!(current(&marks, (0, 2)), Some(0));
    }

    #[test]
    fn every_turn_with_a_row_on_screen_is_in_view() {
        let marks = at_rows(&[0, 4, 9]);
        let lit = |shown| {
            (0..marks.len())
                .filter(|index| in_view(&marks, *index, shown))
                .collect::<Vec<_>>()
        };
        assert_eq!(lit((2, 3)), [0]);
        assert_eq!(lit((3, 9)), [0, 1, 2]);
        assert_eq!(lit((5, 8)), [1]);
        assert_eq!(lit((12, 14)), [2]);
    }

    #[test]
    fn the_rows_shown_are_those_that_reach_into_the_view() {
        let row = |index, top: f32, height: f32| {
            (
                index,
                Bounds::new(point(px(0.), px(top)), size(px(100.), px(height))),
            )
        };
        let painted = [
            row(3, -40., 30.),
            row(4, -10., 50.),
            row(5, 40., 100.),
            row(6, 140., 80.),
            row(7, 220., 40.),
        ];
        assert_eq!(shown_rows(&painted, px(0.), px(200.)), Some((4, 6)));
        assert_eq!(shown_rows(&painted, px(300.), px(400.)), None);
    }

    #[test]
    fn only_the_last_layout_pass_of_a_frame_counts() {
        let row = |index, top: f32| {
            (
                index,
                Bounds::new(point(px(0.), px(top)), size(px(100.), px(100.))),
            )
        };
        let index_back = [
            row(10, 0.),
            row(11, 100.),
            row(12, 200.),
            row(12, -50.),
            row(13, 50.),
        ];
        assert_eq!(shown_rows(&index_back, px(0.), px(150.)), Some((12, 13)));
        let top_back = [
            row(10, 0.),
            row(11, 100.),
            row(12, 200.),
            row(13, -50.),
            row(14, 50.),
        ];
        assert_eq!(shown_rows(&top_back, px(0.), px(150.)), Some((13, 14)));
    }

    #[test]
    fn the_arrows_step_from_the_message_read_or_to_the_first() {
        assert_eq!(steps(Some(1), 3, false), (Some(0), Some(2)));
        assert_eq!(steps(Some(0), 3, false), (None, Some(1)));
        assert_eq!(steps(Some(2), 3, false), (Some(1), None));
        assert_eq!(steps(Some(1), 3, true), (Some(0), None));
        assert_eq!(steps(None, 3, false), (None, Some(0)));
    }

    #[test]
    fn the_rows_shown_follow_a_change_before_them_and_are_forgotten_within_one() {
        let spliced = |changed, count| {
            let mut state = MinimapState {
                shown: Some((4, 9)),
                ..Default::default()
            };
            state.rows_spliced(changed, count);
            state.shown
        };
        assert_eq!(spliced(0..0, 20), Some((24, 29)));
        assert_eq!(spliced(0..1, 21), Some((24, 29)));
        assert_eq!(spliced(0..10, 30), None);
        assert_eq!(spliced(2..5, 1), None);
    }
}
