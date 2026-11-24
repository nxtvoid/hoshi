//! Dates and times as the locale writes them.

use crate::m;
use chrono::{DateTime, Datelike, TimeZone, Weekday};

fn weekday_abbr(day: Weekday) -> String {
    match day {
        Weekday::Mon => m::weekday_abbr_monday(),
        Weekday::Tue => m::weekday_abbr_tuesday(),
        Weekday::Wed => m::weekday_abbr_wednesday(),
        Weekday::Thu => m::weekday_abbr_thursday(),
        Weekday::Fri => m::weekday_abbr_friday(),
        Weekday::Sat => m::weekday_abbr_saturday(),
        Weekday::Sun => m::weekday_abbr_sunday(),
    }
}

fn month_abbr(month: u32) -> String {
    match month {
        1 => m::month_abbr_january(),
        2 => m::month_abbr_february(),
        3 => m::month_abbr_march(),
        4 => m::month_abbr_april(),
        5 => m::month_abbr_may(),
        6 => m::month_abbr_june(),
        7 => m::month_abbr_july(),
        8 => m::month_abbr_august(),
        9 => m::month_abbr_september(),
        10 => m::month_abbr_october(),
        11 => m::month_abbr_november(),
        _ => m::month_abbr_december(),
    }
}

/// "4:30 PM".
pub fn time<Tz: TimeZone>(at: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    at.format(&m::time_format()).to_string()
}

/// "Mon".
pub fn weekday<Tz: TimeZone>(at: &DateTime<Tz>) -> String {
    weekday_abbr(at.weekday())
}

/// "Nov 18".
pub fn day<Tz: TimeZone>(at: &DateTime<Tz>) -> String {
    m::date_day(at.day(), month_abbr(at.month()))
}

/// "Wed, Nov 18".
pub fn weekday_day<Tz: TimeZone>(at: &DateTime<Tz>) -> String {
    m::date_weekday_day(day(at), weekday(at))
}

/// "Mon 9:00 AM".
pub fn weekday_time<Tz: TimeZone>(at: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    m::date_weekday_time(time(at), weekday(at))
}

/// "Nov 24, 9:00 AM".
pub fn day_time<Tz: TimeZone>(at: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    m::date_day_time(day(at), time(at))
}

#[cfg(test)]
mod tests {
    use super::{day, day_time, time, weekday_day, weekday_time};
    use crate::locale;
    use chrono::{DateTime, TimeZone, Utc};
    use gpui_kit::TestAppContext;
    use hoshi_core::Language;

    fn afternoon() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 18, 16, 5, 0).unwrap()
    }

    fn written() -> [String; 5] {
        let at = afternoon();
        [
            time(&at),
            day(&at),
            weekday_day(&at),
            weekday_time(&at),
            day_time(&at),
        ]
    }

    #[gpui_kit::test]
    fn dates_follow_the_locale(cx: &mut TestAppContext) {
        assert_eq!(
            written(),
            [
                "4:05 PM",
                "Nov 18",
                "Wed, Nov 18",
                "Wed 4:05 PM",
                "Nov 18, 4:05 PM"
            ]
        );
        cx.update(|cx| locale::show(Language::Spanish, cx));
        assert_eq!(
            written(),
            [
                "16:05",
                "18 nov",
                "mié, 18 nov",
                "mié 16:05",
                "18 nov, 16:05"
            ]
        );
    }
}
