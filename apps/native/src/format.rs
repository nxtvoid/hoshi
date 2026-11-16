const DURATION_UNITS: [(&str, i64); 3] = [("h", 3_600), ("m", 60), ("s", 1)];

/// A short duration such as "1m 12s": its two largest units, narrow.
pub fn format_duration(milliseconds: i64) -> String {
    let mut rest = (milliseconds.saturating_add(500)).div_euclid(1000).max(0);
    let mut parts = Vec::new();
    for (unit, size) in DURATION_UNITS {
        let value = rest / size;
        rest -= value * size;
        if value == 0 && !(unit == "s" && parts.is_empty()) {
            continue;
        }
        parts.push(format!("{value}{unit}"));
    }
    parts.truncate(2);
    parts.join(" ")
}

/// A count as `Intl.NumberFormat` writes it compactly: 950, 1.2K, 3.4M.
pub fn compact_number(count: u64) -> String {
    let scaled = |value: f64, unit: &str| {
        let rounded = (value * 10.).round() / 10.;
        if rounded.fract() == 0. {
            format!("{rounded:.0}{unit}")
        } else {
            format!("{rounded:.1}{unit}")
        }
    };
    match count {
        0..1_000 => count.to_string(),
        1_000..999_950 => scaled(count as f64 / 1_000., "K"),
        999_950..999_950_000 => scaled(count as f64 / 1_000_000., "M"),
        _ => scaled(count as f64 / 1_000_000_000., "B"),
    }
}

#[cfg(test)]
mod tests {
    use super::{compact_number, format_duration};

    #[test]
    fn a_count_reads_compactly_and_rounds_up_into_the_next_unit() {
        assert_eq!(compact_number(950), "950");
        assert_eq!(compact_number(1_000), "1K");
        assert_eq!(compact_number(1_234), "1.2K");
        assert_eq!(compact_number(200_000), "200K");
        assert_eq!(compact_number(999_960), "1M");
        assert_eq!(compact_number(3_400_000), "3.4M");
    }

    #[test]
    fn a_duration_shows_its_two_largest_units() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(45_000), "45s");
        assert_eq!(format_duration(125_000), "2m 5s");
        assert_eq!(format_duration(120_000), "2m");
        assert_eq!(format_duration(3_725_000), "1h 2m");
        assert_eq!(format_duration(3_605_000), "1h 5s");
    }

    #[test]
    fn a_duration_rounds_to_the_second_and_never_goes_below_zero() {
        assert_eq!(format_duration(1_499), "1s");
        assert_eq!(format_duration(1_500), "2s");
        assert_eq!(format_duration(-4_000), "0s");
    }
}
