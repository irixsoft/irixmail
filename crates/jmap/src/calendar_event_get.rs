use serde_json::{json, Value};

use irixmail_dav::model::CalendarEventRecord;
use irixmail_dav::parse::parse_ics;
use irixmail_store::Collection;

use crate::calendar_object::event_fields;
use crate::context::JmapContext;
use crate::reply::{
    account_id, collection_changes, collection_state, dav_store, now_millis, requested_ids,
};
use crate::request::Invocation;
use crate::utc_date;

pub fn event_json(record: &CalendarEventRecord) -> Value {
    let mut item = match parse_ics(&record.ics) {
        Ok((ical, _)) => event_fields(&ical),
        Err(_) => json!({}),
    };
    let Some(map) = item.as_object_mut() else {
        return item;
    };
    map.insert("id".to_string(), json!(record.id.to_string()));
    map.insert(
        "calendarId".to_string(),
        json!(record.calendar_id.to_string()),
    );
    map.insert("uid".to_string(), json!(record.uid));
    map.insert("etag".to_string(), json!(record.etag));
    map.insert(
        "created".to_string(),
        json!(utc_date::format(record.created / 1000)),
    );
    map.insert(
        "updated".to_string(),
        json!(utc_date::format(record.modified / 1000)),
    );
    item
}

pub fn calendar_event_get(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let store = dav_store(ctx);
    let _ = store.ensure_defaults(now_millis());
    let wanted = requested_ids(args);
    let records = store.list_events(None).unwrap_or_default();

    let mut list = Vec::new();
    let mut found = Vec::new();
    for record in &records {
        let id = record.id.to_string();
        if let Some(ids) = &wanted {
            if !ids.contains(&id) {
                continue;
            }
        }
        found.push(id);
        list.push(event_json(record));
    }
    let not_found: Vec<Value> = match &wanted {
        Some(ids) => ids
            .iter()
            .filter(|id| !found.contains(id))
            .cloned()
            .map(Value::String)
            .collect(),
        None => Vec::new(),
    };

    Invocation::new(
        "CalendarEvent/get",
        json!({
            "accountId": account_id(args),
            "state": collection_state(ctx.store.as_ref(), ctx.account_id as u32, Collection::CalendarEvent),
            "list": list,
            "notFound": not_found,
        }),
        call_id,
    )
}

pub fn calendar_event_changes(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    collection_changes(
        ctx.store.as_ref(),
        ctx.account_id as u32,
        args,
        "CalendarEvent/changes",
        call_id,
        Collection::CalendarEvent,
    )
}
