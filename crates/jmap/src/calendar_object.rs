use calcard::common::PartialDateTime;
use calcard::icalendar::{
    ICalendar, ICalendarComponent, ICalendarComponentType, ICalendarDay, ICalendarDuration,
    ICalendarEntry, ICalendarFrequency, ICalendarParameter, ICalendarParameterName,
    ICalendarParameterValue, ICalendarProperty, ICalendarRecurrenceRule, ICalendarStatus,
    ICalendarValue, ICalendarValueType, ICalendarWeekday,
};
use serde_json::{json, Map, Value};

use irixmail_dav::parse::parse_ics;

pub const PRODID: &str = "-//IRIXMAIL//EN";

const FIELDS: [&str; 10] = [
    "title",
    "description",
    "location",
    "start",
    "timeZone",
    "showWithoutTime",
    "duration",
    "status",
    "recurrenceRule",
    "alerts",
];

pub fn vevent_index(ical: &ICalendar) -> Option<usize> {
    ical.components
        .iter()
        .position(|component| component.component_type == ICalendarComponentType::VEvent)
}

pub fn event_fields(ical: &ICalendar) -> Value {
    let Some(index) = vevent_index(ical) else {
        return json!({});
    };
    let component = &ical.components[index];
    let start_entry = component.property(&ICalendarProperty::Dtstart);
    let start_time = start_entry.and_then(|entry| entry.values.first());
    let start_date = match start_time {
        Some(ICalendarValue::PartialDateTime(value)) => Some(value.as_ref()),
        _ => None,
    };
    let show_without_time = start_entry.map(is_date_only).unwrap_or(false);

    json!({
        "title": text_of(component, &ICalendarProperty::Summary),
        "description": text_of(component, &ICalendarProperty::Description),
        "location": text_of(component, &ICalendarProperty::Location),
        "start": start_date.map(naive_string).map(Value::String).unwrap_or(Value::Null),
        "timeZone": zone_of(start_entry, start_date, show_without_time),
        "showWithoutTime": show_without_time,
        "duration": duration_of(component, start_date, show_without_time),
        "status": status_of(component),
        "recurrenceRule": recurrence_of(component),
        "alerts": alerts_of(ical, index),
    })
}

pub fn build_event(fields: &Value, uid: &str, now_seconds: i64) -> Result<String, String> {
    if fields.get("start").and_then(Value::as_str).is_none() {
        return Err("start".to_string());
    }
    let mut ical = ICalendar {
        components: vec![
            ICalendarComponent {
                component_type: ICalendarComponentType::VCalendar,
                entries: vec![
                    text_entry(ICalendarProperty::Version, "2.0"),
                    text_entry(ICalendarProperty::Prodid, PRODID),
                ],
                component_ids: vec![1],
            },
            ICalendarComponent {
                component_type: ICalendarComponentType::VEvent,
                entries: vec![text_entry(ICalendarProperty::Uid, uid)],
                component_ids: Vec::new(),
            },
        ],
    };
    stamp(&mut ical.components[1], now_seconds);
    apply(&mut ical, 1, fields, fields)?;
    Ok(ical.to_string())
}

pub fn patch_event(ics: &str, patch: &Value, now_seconds: i64) -> Result<String, String> {
    let (mut ical, _) = parse_ics(ics).map_err(|_| "id".to_string())?;
    let index = vevent_index(&ical).ok_or_else(|| "id".to_string())?;
    let merged = merge_object(event_fields(&ical), patch);
    apply(&mut ical, index, patch, &merged)?;
    stamp(&mut ical.components[index], now_seconds);
    Ok(ical.to_string())
}

fn apply(
    ical: &mut ICalendar,
    index: usize,
    touched: &Value,
    merged: &Value,
) -> Result<(), String> {
    let touched = match touched.as_object() {
        Some(map) => map,
        None => return Err("update".to_string()),
    };
    for field in FIELDS {
        if !touched.contains_key(field) {
            continue;
        }
        match field {
            "title" => set_text(ical, index, ICalendarProperty::Summary, merged.get("title")),
            "description" => set_text(
                ical,
                index,
                ICalendarProperty::Description,
                merged.get("description"),
            ),
            "location" => set_text(
                ical,
                index,
                ICalendarProperty::Location,
                merged.get("location"),
            ),
            "start" | "timeZone" | "showWithoutTime" => set_start(ical, index, merged)?,
            "duration" => set_duration(ical, index, merged)?,
            "status" => set_status(ical, index, merged.get("status"))?,
            "recurrenceRule" => set_recurrence(ical, index, merged.get("recurrenceRule"))?,
            "alerts" => set_alerts(ical, index, merged),
            _ => {}
        }
    }
    Ok(())
}

fn set_start(ical: &mut ICalendar, index: usize, merged: &Value) -> Result<(), String> {
    let start = merged
        .get("start")
        .and_then(Value::as_str)
        .ok_or_else(|| "start".to_string())?;
    let all_day = merged
        .get("showWithoutTime")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let zone = merged.get("timeZone").and_then(Value::as_str);
    let utc = zone == Some("UTC");
    let value = partial_date_time(start, all_day, utc).ok_or_else(|| "start".to_string())?;

    let mut params = Vec::new();
    if all_day {
        params.push(ICalendarParameter {
            name: ICalendarParameterName::Value,
            value: ICalendarParameterValue::Value(ICalendarValueType::Date),
        });
    } else if let Some(zone) = zone.filter(|zone| *zone != "UTC") {
        params.push(ICalendarParameter {
            name: ICalendarParameterName::Tzid,
            value: ICalendarParameterValue::Text(zone.to_string()),
        });
    }
    set_entry(
        &mut ical.components[index],
        ICalendarEntry {
            name: ICalendarProperty::Dtstart,
            params,
            values: vec![ICalendarValue::PartialDateTime(Box::new(value))],
        },
    );
    Ok(())
}

fn set_duration(ical: &mut ICalendar, index: usize, merged: &Value) -> Result<(), String> {
    let component = &mut ical.components[index];
    remove_entry(component, &ICalendarProperty::Dtend);
    let Some(text) = merged.get("duration").and_then(Value::as_str) else {
        remove_entry(component, &ICalendarProperty::Duration);
        return Ok(());
    };
    let seconds = parse_duration(text).ok_or_else(|| "duration".to_string())?;
    set_entry(
        component,
        ICalendarEntry {
            name: ICalendarProperty::Duration,
            params: Vec::new(),
            values: vec![ICalendarValue::Duration(ICalendarDuration::from_seconds(
                seconds,
            ))],
        },
    );
    Ok(())
}

fn set_status(ical: &mut ICalendar, index: usize, value: Option<&Value>) -> Result<(), String> {
    let component = &mut ical.components[index];
    let Some(text) = value.and_then(Value::as_str) else {
        remove_entry(component, &ICalendarProperty::Status);
        return Ok(());
    };
    let status = match text {
        "confirmed" => ICalendarStatus::Confirmed,
        "tentative" => ICalendarStatus::Tentative,
        "cancelled" => ICalendarStatus::Cancelled,
        _ => return Err("status".to_string()),
    };
    set_entry(
        component,
        ICalendarEntry {
            name: ICalendarProperty::Status,
            params: Vec::new(),
            values: vec![ICalendarValue::Status(status)],
        },
    );
    Ok(())
}

fn set_recurrence(ical: &mut ICalendar, index: usize, value: Option<&Value>) -> Result<(), String> {
    let component = &mut ical.components[index];
    let Some(rule) = value.and_then(Value::as_object) else {
        remove_entry(component, &ICalendarProperty::Rrule);
        return Ok(());
    };
    let freq = match rule.get("frequency").and_then(Value::as_str) {
        Some("daily") => ICalendarFrequency::Daily,
        Some("weekly") => ICalendarFrequency::Weekly,
        Some("monthly") => ICalendarFrequency::Monthly,
        Some("yearly") => ICalendarFrequency::Yearly,
        _ => return Err("recurrenceRule".to_string()),
    };
    let interval = rule
        .get("interval")
        .and_then(Value::as_u64)
        .filter(|interval| *interval > 1)
        .map(|interval| interval as u16);
    let count = rule
        .get("count")
        .and_then(Value::as_u64)
        .map(|count| count as u32);
    let until = match rule.get("until").and_then(Value::as_str) {
        Some(text) => {
            Some(partial_date_time(text, false, true).ok_or_else(|| "recurrenceRule".to_string())?)
        }
        None => None,
    };
    let mut byday = Vec::new();
    for day in rule
        .get("byDay")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let weekday = day
            .as_str()
            .and_then(weekday_from_name)
            .ok_or_else(|| "recurrenceRule".to_string())?;
        byday.push(ICalendarDay {
            ordwk: None,
            weekday,
        });
    }
    set_entry(
        component,
        ICalendarEntry {
            name: ICalendarProperty::Rrule,
            params: Vec::new(),
            values: vec![ICalendarValue::RecurrenceRule(Box::new(
                ICalendarRecurrenceRule {
                    freq,
                    until,
                    count,
                    interval,
                    byday,
                    ..Default::default()
                },
            ))],
        },
    );
    Ok(())
}

fn set_alerts(ical: &mut ICalendar, index: usize, merged: &Value) {
    let kept: Vec<u32> = ical.components[index]
        .component_ids
        .iter()
        .copied()
        .filter(|id| {
            ical.components
                .get(*id as usize)
                .map(|component| component.component_type != ICalendarComponentType::VAlarm)
                .unwrap_or(false)
        })
        .collect();
    ical.components[index].component_ids = kept;

    let title = merged
        .get("title")
        .and_then(Value::as_str)
        .filter(|title| !title.is_empty())
        .unwrap_or("Reminder")
        .to_string();
    let alerts = merged
        .get("alerts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for alert in alerts {
        let Some(minutes) = alert.get("minutesBefore").and_then(Value::as_i64) else {
            continue;
        };
        let alarm = ICalendarComponent {
            component_type: ICalendarComponentType::VAlarm,
            entries: vec![
                ICalendarEntry {
                    name: ICalendarProperty::Action,
                    params: Vec::new(),
                    values: vec![ICalendarValue::Action(
                        calcard::icalendar::ICalendarAction::Display,
                    )],
                },
                ICalendarEntry {
                    name: ICalendarProperty::Trigger,
                    params: Vec::new(),
                    values: vec![ICalendarValue::Duration(ICalendarDuration::from_seconds(
                        -minutes * 60,
                    ))],
                },
                text_entry(ICalendarProperty::Description, &title),
            ],
            component_ids: Vec::new(),
        };
        let id = ical.components.len() as u32;
        ical.components.push(alarm);
        ical.components[index].component_ids.push(id);
    }
}

fn stamp(component: &mut ICalendarComponent, now_seconds: i64) {
    set_entry(
        component,
        ICalendarEntry {
            name: ICalendarProperty::Dtstamp,
            params: Vec::new(),
            values: vec![ICalendarValue::PartialDateTime(Box::new(
                PartialDateTime::from_utc_timestamp(now_seconds),
            ))],
        },
    );
}

fn set_text(
    ical: &mut ICalendar,
    index: usize,
    property: ICalendarProperty,
    value: Option<&Value>,
) {
    let component = &mut ical.components[index];
    match value.and_then(Value::as_str) {
        Some(text) => set_entry(component, text_entry(property, text)),
        None => remove_entry(component, &property),
    }
}

fn text_entry(name: ICalendarProperty, text: &str) -> ICalendarEntry {
    ICalendarEntry {
        name,
        params: Vec::new(),
        values: vec![ICalendarValue::Text(text.to_string())],
    }
}

fn set_entry(component: &mut ICalendarComponent, entry: ICalendarEntry) {
    remove_entry(component, &entry.name);
    component.entries.push(entry);
}

fn remove_entry(component: &mut ICalendarComponent, property: &ICalendarProperty) {
    component.entries.retain(|entry| &entry.name != property);
}

fn text_of(component: &ICalendarComponent, property: &ICalendarProperty) -> Value {
    component
        .property(property)
        .and_then(|entry| entry.values.first())
        .and_then(ICalendarValue::as_text)
        .map(|text| Value::String(text.to_string()))
        .unwrap_or(Value::Null)
}

fn is_date_only(entry: &ICalendarEntry) -> bool {
    let tagged = entry.params.iter().any(|param| {
        matches!(
            (&param.name, &param.value),
            (
                ICalendarParameterName::Value,
                ICalendarParameterValue::Value(ICalendarValueType::Date)
            )
        )
    });
    tagged
        || matches!(entry.values.first(), Some(ICalendarValue::PartialDateTime(value)) if value.hour.is_none())
}

fn zone_of(
    entry: Option<&ICalendarEntry>,
    start: Option<&PartialDateTime>,
    show_without_time: bool,
) -> Value {
    if show_without_time {
        return Value::Null;
    }
    let tzid = entry.and_then(|entry| {
        entry
            .params
            .iter()
            .find(|param| param.name == ICalendarParameterName::Tzid)
            .and_then(|param| match &param.value {
                ICalendarParameterValue::Text(text) => Some(text.clone()),
                _ => None,
            })
    });
    match tzid {
        Some(zone) => Value::String(zone),
        None if start.map(|start| start.tz_hour.is_some()).unwrap_or(false) => json!("UTC"),
        None => Value::Null,
    }
}

fn duration_of(
    component: &ICalendarComponent,
    start: Option<&PartialDateTime>,
    show_without_time: bool,
) -> Value {
    let explicit = component
        .property(&ICalendarProperty::Duration)
        .and_then(|entry| entry.values.first())
        .and_then(|value| match value {
            ICalendarValue::Duration(duration) => Some(duration.to_string()),
            _ => None,
        });
    if let Some(duration) = explicit {
        return Value::String(duration);
    }
    let end = component
        .property(&ICalendarProperty::Dtend)
        .and_then(|entry| entry.values.first())
        .and_then(|value| match value {
            ICalendarValue::PartialDateTime(value) => Some(value.as_ref()),
            _ => None,
        });
    match (start, end) {
        (Some(start), Some(end)) => {
            let seconds = naive_seconds(end).unwrap_or(0) - naive_seconds(start).unwrap_or(0);
            Value::String(ICalendarDuration::from_seconds(seconds).to_string())
        }
        _ if show_without_time => json!("P1D"),
        _ => json!("PT0S"),
    }
}

fn status_of(component: &ICalendarComponent) -> Value {
    match component.status() {
        Some(ICalendarStatus::Confirmed) => json!("confirmed"),
        Some(ICalendarStatus::Tentative) => json!("tentative"),
        Some(ICalendarStatus::Cancelled) => json!("cancelled"),
        _ => Value::Null,
    }
}

fn recurrence_of(component: &ICalendarComponent) -> Value {
    let rule = component
        .property(&ICalendarProperty::Rrule)
        .and_then(|entry| entry.values.first())
        .and_then(|value| match value {
            ICalendarValue::RecurrenceRule(rule) => Some(rule.as_ref()),
            _ => None,
        });
    let Some(rule) = rule else {
        return Value::Null;
    };
    let frequency = match rule.freq {
        ICalendarFrequency::Daily => "daily",
        ICalendarFrequency::Weekly => "weekly",
        ICalendarFrequency::Monthly => "monthly",
        ICalendarFrequency::Yearly => "yearly",
        _ => return Value::Null,
    };
    let by_day: Vec<Value> = rule
        .byday
        .iter()
        .map(|day| json!(weekday_name(day.weekday)))
        .collect();
    json!({
        "frequency": frequency,
        "interval": rule.interval.unwrap_or(1),
        "count": rule.count,
        "until": rule.until.as_ref().map(naive_string),
        "byDay": by_day,
    })
}

fn alerts_of(ical: &ICalendar, index: usize) -> Value {
    let alerts: Vec<Value> = ical
        .alarms_for_id(index as u32)
        .filter_map(|alarm| {
            alarm
                .property(&ICalendarProperty::Trigger)
                .and_then(|entry| entry.values.first())
                .and_then(|value| match value {
                    ICalendarValue::Duration(duration) => Some(duration.as_seconds()),
                    _ => None,
                })
        })
        .map(|seconds| json!({ "minutesBefore": -seconds / 60 }))
        .collect();
    Value::Array(alerts)
}

fn naive_string(value: &PartialDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        value.year.unwrap_or(0),
        value.month.unwrap_or(1),
        value.day.unwrap_or(1),
        value.hour.unwrap_or(0),
        value.minute.unwrap_or(0),
        value.second.unwrap_or(0)
    )
}

fn naive_seconds(value: &PartialDateTime) -> Option<i64> {
    crate::utc_date::parse(&naive_string(value)).map(|seconds| seconds as i64)
}

pub fn naive_epoch(text: &str) -> Option<i64> {
    let text = text.trim_end_matches('Z');
    let padded = if text.len() == 10 {
        format!("{text}T00:00:00")
    } else {
        text.to_string()
    };
    crate::utc_date::parse(&padded).map(|seconds| seconds as i64)
}

fn partial_date_time(text: &str, all_day: bool, utc: bool) -> Option<PartialDateTime> {
    let bytes = text.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    let year: u16 = text.get(0..4)?.parse().ok()?;
    let month: u8 = text.get(5..7)?.parse().ok()?;
    let day: u8 = text.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut value = PartialDateTime {
        year: Some(year),
        month: Some(month),
        day: Some(day),
        ..Default::default()
    };
    if all_day {
        return Some(value);
    }
    value.hour = Some(
        text.get(11..13)
            .and_then(|part| part.parse().ok())
            .unwrap_or(0),
    );
    value.minute = Some(
        text.get(14..16)
            .and_then(|part| part.parse().ok())
            .unwrap_or(0),
    );
    value.second = Some(
        text.get(17..19)
            .and_then(|part| part.parse().ok())
            .unwrap_or(0),
    );
    if utc {
        value.tz_hour = Some(0);
        value.tz_minute = Some(0);
    }
    Some(value)
}

fn parse_duration(text: &str) -> Option<i64> {
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let rest = rest.strip_prefix('P')?;
    let mut seconds = 0i64;
    let mut digits = String::new();
    let mut in_time = false;
    for character in rest.chars() {
        match character {
            'T' => in_time = true,
            '0'..='9' => digits.push(character),
            unit => {
                let count: i64 = digits.parse().ok()?;
                digits.clear();
                seconds += match (unit, in_time) {
                    ('W', _) => count * 604_800,
                    ('D', _) => count * 86_400,
                    ('H', true) => count * 3_600,
                    ('M', true) => count * 60,
                    ('S', true) => count,
                    _ => return None,
                };
            }
        }
    }
    if !digits.is_empty() {
        return None;
    }
    Some(if negative { -seconds } else { seconds })
}

fn weekday_name(weekday: ICalendarWeekday) -> &'static str {
    match weekday {
        ICalendarWeekday::Monday => "mo",
        ICalendarWeekday::Tuesday => "tu",
        ICalendarWeekday::Wednesday => "we",
        ICalendarWeekday::Thursday => "th",
        ICalendarWeekday::Friday => "fr",
        ICalendarWeekday::Saturday => "sa",
        ICalendarWeekday::Sunday => "su",
    }
}

fn weekday_from_name(name: &str) -> Option<ICalendarWeekday> {
    match name.to_ascii_lowercase().as_str() {
        "mo" => Some(ICalendarWeekday::Monday),
        "tu" => Some(ICalendarWeekday::Tuesday),
        "we" => Some(ICalendarWeekday::Wednesday),
        "th" => Some(ICalendarWeekday::Thursday),
        "fr" => Some(ICalendarWeekday::Friday),
        "sa" => Some(ICalendarWeekday::Saturday),
        "su" => Some(ICalendarWeekday::Sunday),
        _ => None,
    }
}

pub fn merge_object(base: Value, patch: &Value) -> Value {
    let mut map = match base {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    if let Some(source) = patch.as_object() {
        for (key, value) in source {
            map.insert(key.clone(), value.clone());
        }
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(fields: &Value) -> Value {
        let ics = build_event(fields, "u-1@irixmail", 1_770_000_000).unwrap();
        let (ical, _) = parse_ics(&ics).unwrap();
        event_fields(&ical)
    }

    #[test]
    fn a_timed_event_survives_a_json_round_trip() {
        let fields = json!({
            "title": "Standup",
            "description": "daily sync",
            "location": "Room 1",
            "start": "2026-02-10T10:00:00",
            "timeZone": "Europe/Stockholm",
            "showWithoutTime": false,
            "duration": "PT1H",
            "status": "confirmed",
            "recurrenceRule": null,
            "alerts": [{"minutesBefore": 15}],
        });
        let back = round_trip(&fields);
        assert_eq!(back["title"], "Standup");
        assert_eq!(back["description"], "daily sync");
        assert_eq!(back["location"], "Room 1");
        assert_eq!(back["start"], "2026-02-10T10:00:00");
        assert_eq!(back["timeZone"], "Europe/Stockholm");
        assert_eq!(back["showWithoutTime"], false);
        assert_eq!(back["duration"], "PT1H");
        assert_eq!(back["status"], "confirmed");
        assert_eq!(back["recurrenceRule"], Value::Null);
        assert_eq!(back["alerts"], json!([{"minutesBefore": 15}]));
    }

    #[test]
    fn an_all_day_event_has_no_zone_and_a_day_long_default() {
        let fields = json!({
            "title": "Holiday",
            "start": "2026-06-01T00:00:00",
            "showWithoutTime": true,
            "duration": "P1D",
        });
        let ics = build_event(&fields, "u-2@irixmail", 1_770_000_000).unwrap();
        assert!(ics.contains("DTSTART;VALUE=DATE:20260601"), "{ics}");
        let (ical, _) = parse_ics(&ics).unwrap();
        let back = event_fields(&ical);
        assert_eq!(back["showWithoutTime"], true);
        assert_eq!(back["timeZone"], Value::Null);
        assert_eq!(back["start"], "2026-06-01T00:00:00");
        assert_eq!(back["duration"], "P1D");
    }

    #[test]
    fn a_utc_event_writes_a_zulu_stamp_and_reads_back_as_utc() {
        let fields = json!({
            "start": "2026-02-10T10:00:00",
            "timeZone": "UTC",
            "duration": "PT30M",
        });
        let ics = build_event(&fields, "u-3@irixmail", 1_770_000_000).unwrap();
        assert!(ics.contains("DTSTART:20260210T100000Z"), "{ics}");
        let back = round_trip(&fields);
        assert_eq!(back["timeZone"], "UTC");
    }

    #[test]
    fn a_weekly_recurrence_maps_by_day_in_both_directions() {
        let fields = json!({
            "start": "2026-02-10T10:00:00",
            "timeZone": "UTC",
            "duration": "PT1H",
            "recurrenceRule": {
                "frequency": "weekly",
                "interval": 2,
                "count": 6,
                "until": null,
                "byDay": ["mo", "we"],
            },
        });
        let ics = build_event(&fields, "u-4@irixmail", 1_770_000_000).unwrap();
        assert!(ics.contains("BYDAY=MO,WE"), "{ics}");
        let back = round_trip(&fields);
        assert_eq!(back["recurrenceRule"]["frequency"], "weekly");
        assert_eq!(back["recurrenceRule"]["interval"], 2);
        assert_eq!(back["recurrenceRule"]["count"], 6);
        assert_eq!(back["recurrenceRule"]["until"], Value::Null);
        assert_eq!(back["recurrenceRule"]["byDay"], json!(["mo", "we"]));
    }

    #[test]
    fn a_bounded_recurrence_reports_its_until_date() {
        let fields = json!({
            "start": "2026-02-10T10:00:00",
            "timeZone": "UTC",
            "duration": "PT1H",
            "recurrenceRule": {
                "frequency": "daily",
                "until": "2026-03-10T10:00:00",
            },
        });
        let back = round_trip(&fields);
        assert_eq!(back["recurrenceRule"]["until"], "2026-03-10T10:00:00");
        assert_eq!(back["recurrenceRule"]["interval"], 1);
        assert_eq!(back["recurrenceRule"]["byDay"], json!([]));
    }

    #[test]
    fn a_patch_keeps_vendor_properties_and_untouched_fields() {
        let ics = concat!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Other//EN\r\n",
            "BEGIN:VEVENT\r\nUID:keep@example.com\r\nDTSTAMP:20260101T000000Z\r\n",
            "DTSTART:20260210T100000Z\r\nDURATION:PT1H\r\nSUMMARY:Old\r\n",
            "COLOR:turquoise\r\nX-MOZ-GENERATION:4\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        );
        let patched = patch_event(ics, &json!({"title": "New"}), 1_770_000_100).unwrap();
        assert!(patched.contains("X-MOZ-GENERATION:4"), "{patched}");
        assert!(patched.contains("COLOR:turquoise"), "{patched}");
        assert!(patched.contains("SUMMARY:New"), "{patched}");
        assert!(!patched.contains("SUMMARY:Old"), "{patched}");
        let (ical, _) = parse_ics(&patched).unwrap();
        let back = event_fields(&ical);
        assert_eq!(back["start"], "2026-02-10T10:00:00");
        assert_eq!(back["duration"], "PT1H");
    }

    #[test]
    fn a_null_patch_value_removes_the_property() {
        let fields = json!({
            "title": "Standup",
            "location": "Room 1",
            "start": "2026-02-10T10:00:00",
            "timeZone": "UTC",
            "duration": "PT1H",
        });
        let ics = build_event(&fields, "u-5@irixmail", 1_770_000_000).unwrap();
        let patched = patch_event(&ics, &json!({"location": null}), 1_770_000_100).unwrap();
        assert!(!patched.contains("LOCATION"), "{patched}");
        let (ical, _) = parse_ics(&patched).unwrap();
        assert_eq!(event_fields(&ical)["title"], "Standup");
    }

    #[test]
    fn a_patched_alert_replaces_the_previous_alarm() {
        let fields = json!({
            "title": "Standup",
            "start": "2026-02-10T10:00:00",
            "timeZone": "UTC",
            "duration": "PT1H",
            "alerts": [{"minutesBefore": 15}],
        });
        let ics = build_event(&fields, "u-6@irixmail", 1_770_000_000).unwrap();
        let patched = patch_event(
            &ics,
            &json!({"alerts": [{"minutesBefore": 5}]}),
            1_770_000_100,
        )
        .unwrap();
        let (ical, _) = parse_ics(&patched).unwrap();
        assert_eq!(event_fields(&ical)["alerts"], json!([{"minutesBefore": 5}]));
        assert_eq!(patched.matches("BEGIN:VALARM").count(), 1, "{patched}");
    }

    #[test]
    fn an_event_without_a_start_is_rejected() {
        assert_eq!(
            build_event(&json!({"title": "x"}), "u-7@irixmail", 0),
            Err("start".to_string())
        );
    }

    #[test]
    fn an_unknown_status_is_rejected() {
        let fields = json!({"start": "2026-02-10T10:00:00", "status": "snoozed"});
        assert_eq!(
            build_event(&fields, "u-8@irixmail", 0),
            Err("status".to_string())
        );
    }

    #[test]
    fn iso_durations_parse_into_seconds() {
        assert_eq!(parse_duration("PT1H"), Some(3_600));
        assert_eq!(parse_duration("P1D"), Some(86_400));
        assert_eq!(parse_duration("PT1H30M"), Some(5_400));
        assert_eq!(parse_duration("-PT15M"), Some(-900));
        assert_eq!(parse_duration("1H"), None);
    }

    #[test]
    fn a_naive_epoch_accepts_dates_and_zulu_stamps() {
        assert_eq!(naive_epoch("1970-01-01"), Some(0));
        assert_eq!(naive_epoch("1970-01-02T00:00:00Z"), Some(86_400));
        assert_eq!(naive_epoch("nonsense"), None);
    }
}
