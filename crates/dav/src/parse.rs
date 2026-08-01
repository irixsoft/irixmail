use calcard::common::timezone::Tz;
use calcard::icalendar::{ICalendar, ICalendarProperty, ICalendarValue};
use calcard::vcard::{VCard, VCardProperty};
use calcard::{Entry, Parser};
use irixmail_core::{Error, Result};

pub const MAX_EXPANSIONS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IcsInfo {
    pub uid: String,
    pub starts_min: i64,
    pub ends_max: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VcfInfo {
    pub uid: Option<String>,
    pub full_name: String,
    pub emails: Vec<String>,
}

pub fn parse_ics(raw: &str) -> Result<(ICalendar, IcsInfo)> {
    let Entry::ICalendar(ical) = Parser::new(raw).entry() else {
        return Err(Error::invalid_input("not a calendar object"));
    };
    let uid = ical
        .uids()
        .next()
        .map(str::to_string)
        .ok_or_else(|| Error::invalid_input("calendar object has no uid"))?;
    let expand = ical.expand_dates(Tz::UTC, MAX_EXPANSIONS);
    let mut starts_min = i64::MAX;
    let mut ends_max = i64::MIN;
    for event in &expand.events {
        let (start, end) = event.timestamps();
        starts_min = starts_min.min(start);
        ends_max = ends_max.max(end);
    }
    if starts_min == i64::MAX {
        return Err(Error::invalid_input(
            "calendar object has no dated components",
        ));
    }
    if has_unbounded_rrule(&ical) {
        ends_max = i64::MAX;
    }
    Ok((
        ical,
        IcsInfo {
            uid,
            starts_min,
            ends_max,
        },
    ))
}

fn has_unbounded_rrule(ical: &ICalendar) -> bool {
    ical.components.iter().any(|component| {
        component.entries.iter().any(|entry| {
            entry.name == ICalendarProperty::Rrule
                && entry.values.iter().any(|value| {
                    matches!(
                        value,
                        ICalendarValue::RecurrenceRule(rule)
                            if rule.until.is_none() && rule.count.is_none()
                    )
                })
        })
    })
}

pub fn parse_vcf(raw: &str) -> Result<(VCard, VcfInfo)> {
    let Entry::VCard(card) = Parser::new(raw).entry() else {
        return Err(Error::invalid_input("not a vcard"));
    };
    let uid = card.uid().map(str::to_string);
    let full_name = card
        .property(&VCardProperty::Fn)
        .and_then(|entry| entry.values.first())
        .and_then(|value| value.as_text())
        .unwrap_or_default()
        .to_string();
    let emails = card
        .properties(&VCardProperty::Email)
        .filter_map(|entry| entry.values.first())
        .filter_map(|value| value.as_text())
        .map(|text| text.to_ascii_lowercase())
        .filter(|text| !text.is_empty())
        .collect();
    Ok((
        card,
        VcfInfo {
            uid,
            full_name,
            emails,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE_EVENT: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\nBEGIN:VEVENT\r\nUID:one@example.com\r\nDTSTAMP:20260101T000000Z\r\nDTSTART:20260210T100000Z\r\nDTEND:20260210T110000Z\r\nSUMMARY:Meeting\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    const RECURRING_FOREVER: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\nBEGIN:VEVENT\r\nUID:two@example.com\r\nDTSTAMP:20260101T000000Z\r\nDTSTART:20260210T100000Z\r\nDURATION:PT30M\r\nRRULE:FREQ=WEEKLY\r\nSUMMARY:Standup\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    const RECURRING_COUNTED: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\nBEGIN:VEVENT\r\nUID:three@example.com\r\nDTSTAMP:20260101T000000Z\r\nDTSTART:20260210T100000Z\r\nDTEND:20260210T110000Z\r\nRRULE:FREQ=DAILY;COUNT=3\r\nSUMMARY:Sprint\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    const NO_UID: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nDTSTAMP:20260101T000000Z\r\nDTSTART:20260210T100000Z\r\nDTEND:20260210T110000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    const SIMPLE_CARD: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:card-1\r\nFN:Saeed Sakib\r\nEMAIL;TYPE=INTERNET:Saeed@Example.com\r\nEMAIL:second@example.com\r\nTEL:+123\r\nEND:VCARD\r\n";

    #[test]
    fn a_single_event_yields_uid_and_time_envelope() {
        let (_ical, info) = parse_ics(SIMPLE_EVENT).unwrap();
        assert_eq!(info.uid, "one@example.com");
        assert_eq!(info.starts_min, 1770717600);
        assert_eq!(info.ends_max, 1770721200);
    }

    #[test]
    fn an_unbounded_recurrence_extends_the_envelope_forever() {
        let (_ical, info) = parse_ics(RECURRING_FOREVER).unwrap();
        assert_eq!(info.uid, "two@example.com");
        assert_eq!(info.starts_min, 1770717600);
        assert_eq!(info.ends_max, i64::MAX);
    }

    #[test]
    fn a_counted_recurrence_ends_at_its_last_occurrence() {
        let (_ical, info) = parse_ics(RECURRING_COUNTED).unwrap();
        assert_eq!(info.starts_min, 1770717600);
        assert_eq!(info.ends_max, 1770721200 + 2 * 86_400);
    }

    #[test]
    fn an_event_without_a_uid_is_rejected() {
        assert!(parse_ics(NO_UID).is_err());
    }

    #[test]
    fn junk_and_vcards_are_rejected_as_calendar_objects() {
        assert!(parse_ics("hello world").is_err());
        assert!(parse_ics(SIMPLE_CARD).is_err());
    }

    #[test]
    fn a_vcard_yields_uid_name_and_lowercased_emails() {
        let (_card, info) = parse_vcf(SIMPLE_CARD).unwrap();
        assert_eq!(info.uid.as_deref(), Some("card-1"));
        assert_eq!(info.full_name, "Saeed Sakib");
        assert_eq!(info.emails, vec!["saeed@example.com", "second@example.com"]);
    }

    #[test]
    fn a_vcard_without_uid_or_fn_still_parses() {
        let (_card, info) =
            parse_vcf("BEGIN:VCARD\r\nVERSION:3.0\r\nTEL:+1\r\nEND:VCARD\r\n").unwrap();
        assert_eq!(info.uid, None);
        assert_eq!(info.full_name, "");
        assert!(info.emails.is_empty());
    }

    #[test]
    fn junk_and_calendars_are_rejected_as_vcards() {
        assert!(parse_vcf("junk").is_err());
        assert!(parse_vcf(SIMPLE_EVENT).is_err());
    }
}
