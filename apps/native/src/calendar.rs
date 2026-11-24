//! A month of days to pick one from: the month and year between arrows, Mo–Su, and the days.
//! Days before today can't be picked, and the arrows never go to a month that is all past.

use crate::m;
use chrono::{Datelike, Days, Months, NaiveDate};
use gpui_kit::*;
use hoshi_ui::prelude::*;
use std::rc::Rc;

type DateHandler = Rc<dyn Fn(NaiveDate, &mut Window, &mut App)>;

fn month_name(month: u32) -> String {
    match month {
        1 => m::month_january(),
        2 => m::month_february(),
        3 => m::month_march(),
        4 => m::month_april(),
        5 => m::month_may(),
        6 => m::month_june(),
        7 => m::month_july(),
        8 => m::month_august(),
        9 => m::month_september(),
        10 => m::month_october(),
        11 => m::month_november(),
        _ => m::month_december(),
    }
}

fn weekday_headers() -> [String; 7] {
    [
        m::weekday_short_monday(),
        m::weekday_short_tuesday(),
        m::weekday_short_wednesday(),
        m::weekday_short_thursday(),
        m::weekday_short_friday(),
        m::weekday_short_saturday(),
        m::weekday_short_sunday(),
    ]
}

/// The first day of `date`'s month.
pub fn month_of(date: NaiveDate) -> NaiveDate {
    date.with_day(1).unwrap_or(date)
}

/// The weeks shown for `month`, Monday first, from the week of its first day to the week of
/// its last.
pub fn month_weeks(month: NaiveDate) -> Vec<[NaiveDate; 7]> {
    let first = month_of(month);
    let mut day = first - Days::new(first.weekday().num_days_from_monday().into());
    let next = first + Months::new(1);
    let mut weeks = Vec::new();
    while day < next {
        weeks.push(std::array::from_fn(|offset| day + Days::new(offset as u64)));
        day = day + Days::new(7);
    }
    weeks
}

/// `month` moved by `step` months, unless that is a month entirely before `today`'s.
pub fn step_month(month: NaiveDate, step: i32, today: NaiveDate) -> Option<NaiveDate> {
    let month = month_of(month);
    let moved = if step < 0 {
        month.checked_sub_months(Months::new(step.unsigned_abs()))?
    } else {
        month.checked_add_months(Months::new(step.unsigned_abs()))?
    };
    (moved >= month_of(today)).then_some(moved)
}

#[derive(IntoElement)]
pub struct MonthCalendar {
    id: SharedString,
    month: NaiveDate,
    selected: NaiveDate,
    today: NaiveDate,
    on_pick: Option<DateHandler>,
    on_month: Option<DateHandler>,
}

impl MonthCalendar {
    /// Shows `month` with `selected` picked; `today` and later can be picked.
    pub fn new(
        id: impl Into<SharedString>,
        month: NaiveDate,
        selected: NaiveDate,
        today: NaiveDate,
    ) -> Self {
        Self {
            id: id.into(),
            month: month_of(month),
            selected,
            today,
            on_pick: None,
            on_month: None,
        }
    }

    pub fn on_pick(mut self, handler: impl Fn(NaiveDate, &mut Window, &mut App) + 'static) -> Self {
        self.on_pick = Some(Rc::new(handler));
        self
    }

    /// Gets the first day of the month to show next.
    pub fn on_month(
        mut self,
        handler: impl Fn(NaiveDate, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_month = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for MonthCalendar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let id = self.id.clone();
        let arrow = |name: &str, icon: IconName, step: i32, tooltip: String| {
            let target = step_month(self.month, step, self.today);
            let on_month = self.on_month.clone();
            let selector = format!("{id}-{name}");
            div().debug_selector(move || selector.clone()).child(
                Button::new(SharedString::from(format!("{id}-{name}")))
                    .icon(icon)
                    .size(ButtonSize::IconXs)
                    .variant(ButtonVariant::Ghost)
                    .tooltip(Tooltip::new(tooltip))
                    .disabled(target.is_none())
                    .on_click(move |_, window, cx| {
                        if let (Some(target), Some(on_month)) = (target, &on_month) {
                            on_month(target, window, cx)
                        }
                    }),
            )
        };
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .text_sm()
            .child(arrow(
                "previous",
                IconName::ChevronLeft,
                -1,
                m::calendar_previous_month(),
            ))
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .child(m::calendar_month(
                        month_name(self.month.month()),
                        self.month.year(),
                    )),
            )
            .child(arrow(
                "next",
                IconName::ChevronRight,
                1,
                m::calendar_next_month(),
            ));
        let cell = || {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .h_7()
                .rounded_md()
        };
        let weekdays = div()
            .flex()
            .gap(px(2.))
            .children(weekday_headers().map(|name| {
                cell()
                    .text_size(rems(0.6875))
                    .text_color(colors.muted_foreground)
                    .child(name)
            }));
        let hover = colors.muted;
        let weeks = month_weeks(self.month).into_iter().map(|week| {
            div().flex().gap(px(2.)).children(week.map(|day| {
                let pickable = day >= self.today && month_of(day) == self.month;
                let selected = pickable && day == self.selected;
                let selector = format!("{}-day-{}", self.id, day.format("%Y-%m-%d"));
                let on_pick = self.on_pick.clone();
                cell()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .text_xs()
                    .child(day.day().to_string())
                    .when(!pickable, |this| {
                        this.text_color(colors.muted_foreground).opacity(0.45)
                    })
                    .when(day == self.today && !selected, |this| {
                        this.border_1().border_color(colors.border)
                    })
                    .when(selected, |this| {
                        this.bg(colors.primary)
                            .text_color(colors.primary_foreground)
                    })
                    .when(pickable && !selected, |this| {
                        this.cursor_pointer().hover(move |style| style.bg(hover))
                    })
                    .when(pickable, |this| {
                        this.on_click(move |_, window, cx| {
                            if let Some(on_pick) = &on_pick {
                                on_pick(day, window, cx)
                            }
                        })
                    })
            }))
        });
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(header)
            .child(weekdays)
            .children(weeks)
    }
}

#[cfg(test)]
mod tests {
    use super::{month_weeks, step_month};
    use chrono::NaiveDate;

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn a_month_shows_whole_weeks_from_monday() {
        let weeks = month_weeks(day("2026-11-15"));
        assert_eq!(weeks.len(), 6);
        assert_eq!(weeks[0][0], day("2026-10-26"));
        assert_eq!(weeks[0][6], day("2026-11-01"));
        assert_eq!(weeks[5][0], day("2026-11-30"));
        let february = month_weeks(day("2027-02-01"));
        assert_eq!(february.len(), 4);
        assert_eq!(february[3][6], day("2027-02-28"));
    }

    #[test]
    fn the_arrows_never_reach_a_month_that_is_all_past() {
        let today = day("2026-11-15");
        assert_eq!(step_month(day("2026-11-01"), -1, today), None);
        assert_eq!(
            step_month(day("2026-12-01"), -1, today),
            Some(day("2026-11-01"))
        );
        assert_eq!(
            step_month(day("2026-12-31"), 1, today),
            Some(day("2027-01-01"))
        );
    }
}
