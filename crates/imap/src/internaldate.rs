const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub fn format_internaldate(received_at: u64) -> String {
    let secs = received_at as i64;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = tod / 3_600;
    let minute = (tod % 3_600) / 60;
    let second = tod % 60;
    format!(
        "{day:02}-{}-{year:04} {hour:02}:{minute:02}:{second:02} +0000",
        MONTHS[(month - 1) as usize]
    )
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn parse_imap_date(value: &str) -> Option<i64> {
    let mut parts = value.trim().split('-');
    let day: u32 = parts.next()?.parse().ok()?;
    let month = month_number(parts.next()?)?;
    let year: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || day == 0 || day > 31 {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400)
}

pub fn parse_internaldate(value: &str) -> Option<u64> {
    let mut fields = value.trim().split(' ').filter(|field| !field.is_empty());
    let date = fields.next()?;
    let time = fields.next()?;
    let zone = fields.next()?;
    if fields.next().is_some() {
        return None;
    }
    let date_secs = parse_imap_date(date)?;

    let mut clock = time.split(':');
    let hour: u32 = clock.next()?.parse().ok()?;
    let minute: u32 = clock.next()?.parse().ok()?;
    let second: u32 = clock.next()?.parse().ok()?;
    if clock.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let sign = match zone.as_bytes().first()? {
        b'+' => 1i64,
        b'-' => -1i64,
        _ => return None,
    };
    let digits = &zone[1..];
    if digits.len() != 4 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let zone_hour: i64 = digits[..2].parse().ok()?;
    let zone_minute: i64 = digits[2..].parse().ok()?;
    if zone_minute > 59 {
        return None;
    }
    let offset = sign * (zone_hour * 3_600 + zone_minute * 60);

    let epoch =
        date_secs + i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second) - offset;
    u64::try_from(epoch).ok()
}

fn month_number(name: &str) -> Option<u32> {
    MONTHS
        .iter()
        .position(|month| month.eq_ignore_ascii_case(name))
        .map(|index| index as u32 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_epoch_renders_as_new_years_day_1970() {
        assert_eq!(format_internaldate(0), "01-Jan-1970 00:00:00 +0000");
    }

    #[test]
    fn a_known_instant_matches_the_reference_rendering() {
        assert_eq!(
            format_internaldate(482_374_938),
            "15-Apr-1985 01:02:18 +0000"
        );
    }

    #[test]
    fn the_day_and_time_fields_are_zero_padded() {
        assert_eq!(
            format_internaldate(1_704_067_205),
            "01-Jan-2024 00:00:05 +0000"
        );
    }

    #[test]
    fn a_leap_day_is_rendered_correctly() {
        assert_eq!(
            format_internaldate(1_582_934_400),
            "29-Feb-2020 00:00:00 +0000"
        );
    }

    #[test]
    fn parse_imap_date_returns_midnight_utc() {
        assert_eq!(parse_imap_date("01-Jan-1970"), Some(0));
        assert_eq!(parse_imap_date("1-Feb-2020"), Some(1_580_515_200));
        assert_eq!(parse_imap_date("29-feb-2020"), Some(1_582_934_400));
        assert_eq!(parse_imap_date("15-Apr-1985"), Some(482_371_200));
    }

    #[test]
    fn parse_imap_date_is_the_inverse_of_the_formatter() {
        for midnight in [0, 1_582_934_400, 1_580_515_200, 482_371_200] {
            let rendered = format_internaldate(midnight);
            let date = rendered.split(' ').next().unwrap();
            assert_eq!(parse_imap_date(date), Some(midnight as i64));
        }
    }

    #[test]
    fn parse_internaldate_converts_the_zone_offset_to_utc() {
        assert_eq!(
            parse_internaldate("15-Apr-1985 01:02:18 +0000"),
            Some(482_374_938)
        );
        assert_eq!(
            parse_internaldate("15-Apr-1985 03:02:18 +0200"),
            Some(482_374_938)
        );
        assert_eq!(
            parse_internaldate("14-Apr-1985 22:02:18 -0300"),
            Some(482_374_938)
        );
        assert_eq!(parse_internaldate(" 1-Jan-1970 00:00:00 +0000"), Some(0));
    }

    #[test]
    fn parse_internaldate_round_trips_through_the_formatter() {
        for instant in [0u64, 482_374_938, 1_582_934_400, 1_704_067_205] {
            assert_eq!(
                parse_internaldate(&format_internaldate(instant)),
                Some(instant)
            );
        }
    }

    #[test]
    fn parse_internaldate_rejects_malformed_input() {
        assert_eq!(parse_internaldate("not a date"), None);
        assert_eq!(parse_internaldate("15-Apr-1985 01:02:18"), None);
        assert_eq!(parse_internaldate("15-Apr-1985 24:00:00 +0000"), None);
        assert_eq!(parse_internaldate("15-Apr-1985 01:02:18 0000"), None);
        assert_eq!(parse_internaldate("15-Apr-1985 01:02:18 +00"), None);
        assert_eq!(parse_internaldate("31-Dec-1969 23:59:59 +0000"), None);
    }

    #[test]
    fn parse_imap_date_rejects_malformed_input() {
        assert_eq!(parse_imap_date("not-a-date"), None);
        assert_eq!(parse_imap_date("1-Foo-2020"), None);
        assert_eq!(parse_imap_date("1-Feb"), None);
        assert_eq!(parse_imap_date("0-Feb-2020"), None);
    }
}
