use serde_json::{json, Value};

use irixmail_dav::model::CalendarCollection;
use irixmail_dav::storage::DEFAULT_CALENDAR_NAME;
use irixmail_store::Collection;

use crate::context::JmapContext;
use crate::reply::{
    account_id, collection_changes, collection_state, dav_store, now_millis, requested_ids,
};
use crate::request::Invocation;

pub fn calendar_json(calendar: &CalendarCollection) -> Value {
    json!({
        "id": calendar.id.to_string(),
        "name": calendar.display_name,
        "color": calendar.color,
        "sortOrder": calendar.order,
        "timeZone": calendar.time_zone,
        "isDefault": calendar.name == DEFAULT_CALENDAR_NAME,
    })
}

pub fn calendar_get(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let store = dav_store(ctx);
    let _ = store.ensure_defaults(now_millis());
    let wanted = requested_ids(args);
    let calendars = store.list_calendars().unwrap_or_default();

    let mut list = Vec::new();
    let mut found = Vec::new();
    for calendar in &calendars {
        let id = calendar.id.to_string();
        if let Some(ids) = &wanted {
            if !ids.contains(&id) {
                continue;
            }
        }
        found.push(id);
        list.push(calendar_json(calendar));
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
        "Calendar/get",
        json!({
            "accountId": account_id(args),
            "state": collection_state(ctx.store.as_ref(), ctx.account_id as u32, Collection::Calendar),
            "list": list,
            "notFound": not_found,
        }),
        call_id,
    )
}

pub fn calendar_changes(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    collection_changes(
        ctx.store.as_ref(),
        ctx.account_id as u32,
        args,
        "Calendar/changes",
        call_id,
        Collection::Calendar,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_context;

    #[test]
    fn a_default_calendar_appears_on_the_first_get() {
        let ctx = test_context();
        let response = calendar_get(&ctx, &json!({"accountId": "1"}), "c0");
        let list = response.arguments()["list"].as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["name"], "Calendar");
        assert_eq!(list[0]["isDefault"], true);
        assert_eq!(list[0]["color"], Value::Null);
        assert!(list[0]["id"].is_string());
    }

    #[test]
    fn an_unknown_calendar_id_is_reported_not_found() {
        let ctx = test_context();
        let response = calendar_get(&ctx, &json!({"accountId": "1", "ids": ["99"]}), "c0");
        assert_eq!(response.arguments()["list"], json!([]));
        assert_eq!(response.arguments()["notFound"], json!(["99"]));
    }

    #[test]
    fn changes_report_the_calendars_created_since_a_state() {
        let ctx = test_context();
        let before = collection_state(ctx.store.as_ref(), 1, Collection::Calendar);
        calendar_get(&ctx, &json!({"accountId": "1"}), "c0");
        let response =
            calendar_changes(&ctx, &json!({"accountId": "1", "sinceState": before}), "c1");
        assert_eq!(response.arguments()["created"].as_array().unwrap().len(), 1);
    }
}
