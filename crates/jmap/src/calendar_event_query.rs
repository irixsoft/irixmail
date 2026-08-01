use calcard::common::timezone::Tz;
use serde_json::{json, Value};

use irixmail_dav::model::CalendarEventRecord;
use irixmail_dav::parse::parse_ics;
use irixmail_store::Collection;

use crate::calendar_object::naive_epoch;
use crate::context::JmapContext;
use crate::reply::{account_id, collection_state, dav_store, now_millis};
use crate::request::Invocation;

const EXPANSION_LIMIT: usize = 512;
const MAX_OCCURRENCES: usize = 2048;

pub fn calendar_event_query(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let store = dav_store(ctx);
    let _ = store.ensure_defaults(now_millis());

    let filter = args.get("filter").filter(|value| !value.is_null());
    let after = filter
        .and_then(|filter| filter.get("after"))
        .and_then(Value::as_str)
        .and_then(naive_epoch)
        .unwrap_or(i64::MIN);
    let before = filter
        .and_then(|filter| filter.get("before"))
        .and_then(Value::as_str)
        .and_then(naive_epoch)
        .unwrap_or(i64::MAX);
    let calendars: Option<Vec<u32>> = filter
        .and_then(|filter| filter.get("inCalendars"))
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .filter_map(|id| id.parse::<u32>().ok())
                .collect()
        });

    let mut matched: Vec<CalendarEventRecord> = store
        .list_events(None)
        .unwrap_or_default()
        .into_iter()
        .filter(|record| match &calendars {
            Some(ids) => ids.contains(&record.calendar_id),
            None => true,
        })
        .filter(|record| record.starts_min < before && record.ends_max > after)
        .collect();
    matched.sort_by_key(|record| (record.starts_min, record.id));

    let ids: Vec<Value> = matched
        .iter()
        .map(|record| Value::String(record.id.to_string()))
        .collect();
    let mut response = json!({
        "accountId": account_id(args),
        "queryState": collection_state(ctx.store.as_ref(), ctx.account_id as u32, Collection::CalendarEvent),
        "canCalculateChanges": false,
        "position": 0,
        "ids": ids,
        "total": matched.len(),
    });

    if args
        .get("expandRecurrences")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let occurrences = expand(&matched, after, before);
        if let Some(map) = response.as_object_mut() {
            map.insert("occurrences".to_string(), Value::Array(occurrences));
        }
    }

    Invocation::new("CalendarEvent/query", response, call_id)
}

fn expand(records: &[CalendarEventRecord], after: i64, before: i64) -> Vec<Value> {
    let mut occurrences: Vec<(i64, i64, String)> = Vec::new();
    for record in records {
        let Ok((ical, _)) = parse_ics(&record.ics) else {
            continue;
        };
        for event in ical.expand_dates(Tz::UTC, EXPANSION_LIMIT).events {
            let (start, end) = event.timestamps();
            if start < before && end > after {
                occurrences.push((start, end, record.id.to_string()));
            }
        }
    }
    occurrences.sort_by(|left, right| left.0.cmp(&right.0).then(left.2.cmp(&right.2)));
    occurrences.truncate(MAX_OCCURRENCES);
    occurrences
        .into_iter()
        .map(|(start, end, id)| json!({"id": id, "start": start, "end": end}))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar_event_set::calendar_event_set;
    use crate::calendar_get::calendar_get;
    use crate::context::test_context;

    fn default_calendar(ctx: &JmapContext) -> String {
        calendar_get(ctx, &json!({"accountId": "1"}), "c0").arguments()["list"][0]["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn create(ctx: &JmapContext, object: Value) -> String {
        calendar_event_set(
            ctx,
            &json!({"accountId": "1", "create": {"a": object}}),
            "c0",
        )
        .arguments()["created"]["a"]["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn a_time_window_keeps_only_the_overlapping_events() {
        let ctx = test_context();
        let calendar = default_calendar(&ctx);
        let inside = create(
            &ctx,
            json!({"calendarId": calendar, "title": "Inside", "start": "2026-02-10T09:00:00", "timeZone": "UTC", "duration": "PT1H"}),
        );
        create(
            &ctx,
            json!({"calendarId": calendar, "title": "Outside", "start": "2026-05-10T09:00:00", "timeZone": "UTC", "duration": "PT1H"}),
        );

        let response = calendar_event_query(
            &ctx,
            &json!({
                "accountId": "1",
                "filter": {"after": "2026-02-01T00:00:00Z", "before": "2026-03-01T00:00:00Z"},
            }),
            "c1",
        );
        assert_eq!(response.arguments()["ids"], json!([inside]));
        assert_eq!(response.arguments()["total"], 1);
        assert_eq!(response.arguments()["canCalculateChanges"], false);
        assert!(response.arguments().get("occurrences").is_none());
    }

    #[test]
    fn an_unbounded_filter_returns_every_event_sorted_by_start() {
        let ctx = test_context();
        let calendar = default_calendar(&ctx);
        let later = create(
            &ctx,
            json!({"calendarId": calendar, "title": "Later", "start": "2026-05-10T09:00:00", "timeZone": "UTC", "duration": "PT1H"}),
        );
        let earlier = create(
            &ctx,
            json!({"calendarId": calendar, "title": "Earlier", "start": "2026-02-10T09:00:00", "timeZone": "UTC", "duration": "PT1H"}),
        );
        let response = calendar_event_query(&ctx, &json!({"accountId": "1"}), "c1");
        assert_eq!(response.arguments()["ids"], json!([earlier, later]));
    }

    #[test]
    fn a_calendar_filter_excludes_other_calendars() {
        let ctx = test_context();
        let calendar = default_calendar(&ctx);
        create(
            &ctx,
            json!({"calendarId": calendar, "title": "Mine", "start": "2026-02-10T09:00:00", "timeZone": "UTC", "duration": "PT1H"}),
        );
        let response = calendar_event_query(
            &ctx,
            &json!({"accountId": "1", "filter": {"inCalendars": ["999"]}}),
            "c1",
        );
        assert_eq!(response.arguments()["ids"], json!([]));
        assert_eq!(response.arguments()["total"], 0);
    }

    #[test]
    fn expanding_a_weekly_rule_lists_every_occurrence_in_the_window() {
        let ctx = test_context();
        let calendar = default_calendar(&ctx);
        let id = create(
            &ctx,
            json!({
                "calendarId": calendar,
                "title": "Standup",
                "start": "2026-02-02T09:00:00",
                "timeZone": "UTC",
                "duration": "PT30M",
                "recurrenceRule": {"frequency": "weekly", "byDay": ["mo"]},
            }),
        );
        let response = calendar_event_query(
            &ctx,
            &json!({
                "accountId": "1",
                "filter": {"after": "2026-02-01T00:00:00Z", "before": "2026-03-01T00:00:00Z"},
                "expandRecurrences": true,
            }),
            "c1",
        );
        let occurrences = response.arguments()["occurrences"].as_array().unwrap();
        assert_eq!(occurrences.len(), 4, "{occurrences:?}");
        assert!(occurrences.iter().all(|item| item["id"] == id));
        let starts: Vec<i64> = occurrences
            .iter()
            .map(|item| item["start"].as_i64().unwrap())
            .collect();
        let mut sorted = starts.clone();
        sorted.sort_unstable();
        assert_eq!(starts, sorted);
        assert_eq!(occurrences[0]["end"].as_i64().unwrap() - starts[0], 1_800);
    }
}
