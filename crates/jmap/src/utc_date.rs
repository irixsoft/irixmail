pub(crate) fn parse(value: &str) -> Option<u64> {
    let bytes = value.as_bytes();
    if bytes.len() < 16 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[13] != b':' {
        return None;
    }
    if !bytes[10].eq_ignore_ascii_case(&b'T') {
        return None;
    }
    let year: i64 = value.get(0..4)?.parse().ok()?;
    let month: u64 = value.get(5..7)?.parse().ok()?;
    let day: u64 = value.get(8..10)?.parse().ok()?;
    let hour: u64 = value.get(11..13)?.parse().ok()?;
    let minute: u64 = value.get(14..16)?.parse().ok()?;

    let mut second = 0u64;
    let mut rest = value.get(16..)?;
    if let Some(tail) = rest.strip_prefix(':') {
        second = tail.get(0..2)?.parse().ok()?;
        rest = tail.get(2..)?;
    }
    if let Some(tail) = rest.strip_prefix('.') {
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        rest = &tail[digits..];
    }
    if !(rest.is_empty() || rest.eq_ignore_ascii_case("z") || rest == "+00:00") {
        return None;
    }
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }

    let y = year - i64::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe as i64 - 719_468;
    if days < 0 {
        return None;
    }
    Some(days as u64 * 86_400 + hour * 3_600 + minute * 60 + second.min(59))
}

pub(crate) fn format(seconds: u64) -> String {
    let tod = seconds % 86_400;
    let z = (seconds / 86_400) as i64 + 719_468;
    let era = z / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe as i64 + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        tod / 3_600,
        (tod % 3_600) / 60,
        tod % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_survives_a_parse_format_round_trip() {
        assert_eq!(parse("2026-07-10T00:00:00Z"), Some(1_783_641_600));
        assert_eq!(parse("2026-07-10T00:00:00.000Z"), Some(1_783_641_600));
        assert_eq!(parse("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(format(1_783_641_600), "2026-07-10T00:00:00Z");
        assert_eq!(format(0), "1970-01-01T00:00:00Z");
        let noon = parse("2026-02-28T12:34:56Z").unwrap();
        assert_eq!(format(noon), "2026-02-28T12:34:56Z");
    }

    #[test]
    fn malformed_dates_do_not_parse() {
        assert_eq!(parse("next tuesday"), None);
        assert_eq!(parse("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse("2026-07-10 00:00:00"), None);
        assert_eq!(parse("2026-07-10T00:00:00+02:00"), None);
    }
}
