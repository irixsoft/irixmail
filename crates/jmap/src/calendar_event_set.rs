use rand::Rng;
use serde_json::{json, Map, Value};

use irixmail_dav::parse::parse_ics;
use irixmail_dav::storage::DavStore;
use irixmail_store::Collection;

use crate::calendar_object::{build_event, patch_event};
use crate::context::JmapContext;
use crate::reply::{
    account_id, collection_state, dav_store, invalid_property, now_millis, set_error,
};
use crate::request::{method_error, Invocation};

pub fn calendar_event_set(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let account = ctx.account_id as u32;
    let old_state = collection_state(ctx.store.as_ref(), account, Collection::CalendarEvent);
    if let Some(expected) = args.get("ifInState").and_then(Value::as_str) {
        if expected != old_state {
            return method_error("stateMismatch", call_id);
        }
    }
    let store = dav_store(ctx);
    let now = now_millis();
    let _ = store.ensure_defaults(now);

    let mut created = Map::new();
    let mut not_created = Map::new();
    let mut updated = Map::new();
    let mut not_updated = Map::new();
    let mut destroyed = Vec::new();
    let mut not_destroyed = Map::new();

    if let Some(objects) = args.get("create").and_then(Value::as_object) {
        for (creation_id, object) in objects {
            match create_one(&store, object, now) {
                Ok(value) => {
                    created.insert(creation_id.clone(), value);
                }
                Err(error) => {
                    not_created.insert(creation_id.clone(), error);
                }
            }
        }
    }

    if let Some(objects) = args.get("update").and_then(Value::as_object) {
        for (id, patch) in objects {
            match update_one(&store, id, patch, now) {
                Ok(moved) => {
                    updated.insert(id.clone(), moved);
                }
                Err(error) => {
                    not_updated.insert(id.clone(), error);
                }
            }
        }
    }

    if let Some(ids) = args.get("destroy").and_then(Value::as_array) {
        for id in ids.iter().filter_map(Value::as_str) {
            match destroy_one(&store, id) {
                Ok(()) => destroyed.push(Value::String(id.to_string())),
                Err(error) => {
                    not_destroyed.insert(id.to_string(), error);
                }
            }
        }
    }

    Invocation::new(
        "CalendarEvent/set",
        json!({
            "accountId": account_id(args),
            "oldState": old_state,
            "newState": collection_state(ctx.store.as_ref(), account, Collection::CalendarEvent),
            "created": created,
            "updated": updated,
            "destroyed": destroyed,
            "notCreated": not_created,
            "notUpdated": not_updated,
            "notDestroyed": not_destroyed,
        }),
        call_id,
    )
}

fn create_one(store: &DavStore<'_>, object: &Value, now: u64) -> Result<Value, Value> {
    let calendar_id = calendar_of(store, object.get("calendarId"))?;
    let uid = new_uid();
    let ics = build_event(object, &uid, (now / 1000) as i64)
        .map_err(|property| invalid_property(&property, "the event could not be built"))?;
    let (_, info) = parse_ics(&ics)
        .map_err(|_| invalid_property("start", "the event is not a valid calendar object"))?;
    let name = format!("{uid}.ics");
    let (record, _) = store
        .upsert_event(calendar_id, &name, &ics, &info, now)
        .map_err(|_| set_error("serverFail", "the event could not be stored"))?;
    Ok(json!({
        "id": record.id.to_string(),
        "uid": record.uid,
        "etag": record.etag,
    }))
}

fn update_one(store: &DavStore<'_>, id: &str, patch: &Value, now: u64) -> Result<Value, Value> {
    let id = id
        .parse::<u32>()
        .map_err(|_| set_error("notFound", "no such event"))?;
    let record = store
        .list_events(None)
        .map_err(|_| set_error("serverFail", "the events could not be read"))?
        .into_iter()
        .find(|record| record.id == id)
        .ok_or_else(|| set_error("notFound", "no such event"))?;

    let target = match patch.get("calendarId") {
        Some(value) => calendar_of(store, Some(value))?,
        None => record.calendar_id,
    };
    let ics = patch_event(&record.ics, patch, (now / 1000) as i64)
        .map_err(|property| invalid_property(&property, "the event could not be updated"))?;
    let (_, info) = parse_ics(&ics)
        .map_err(|_| invalid_property("start", "the event is not a valid calendar object"))?;

    if target != record.calendar_id {
        store
            .delete_event(record.calendar_id, &record.name)
            .map_err(|_| set_error("serverFail", "the event could not be moved"))?;
    }
    let (stored, _) = store
        .upsert_event(target, &record.name, &ics, &info, now)
        .map_err(|_| set_error("serverFail", "the event could not be stored"))?;
    if stored.id == record.id {
        Ok(Value::Null)
    } else {
        Ok(json!({ "id": stored.id.to_string() }))
    }
}

fn destroy_one(store: &DavStore<'_>, id: &str) -> Result<(), Value> {
    let id = id
        .parse::<u32>()
        .map_err(|_| set_error("notFound", "no such event"))?;
    let record = store
        .list_events(None)
        .map_err(|_| set_error("serverFail", "the events could not be read"))?
        .into_iter()
        .find(|record| record.id == id)
        .ok_or_else(|| set_error("notFound", "no such event"))?;
    match store.delete_event(record.calendar_id, &record.name) {
        Ok(true) => Ok(()),
        Ok(false) => Err(set_error("notFound", "no such event")),
        Err(_) => Err(set_error("serverFail", "the event could not be removed")),
    }
}

fn calendar_of(store: &DavStore<'_>, value: Option<&Value>) -> Result<u32, Value> {
    let id = value
        .and_then(Value::as_str)
        .and_then(|id| id.parse::<u32>().ok())
        .ok_or_else(|| invalid_property("calendarId", "a calendar id is required"))?;
    match store.calendar_by_id(id) {
        Ok(Some(_)) => Ok(id),
        _ => Err(invalid_property("calendarId", "no such calendar")),
    }
}

fn new_uid() -> String {
    let value: u128 = rand::rng().random();
    format!("{value:032x}@irixmail")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar_event_get::{calendar_event_changes, calendar_event_get};
    use crate::calendar_get::calendar_get;
    use crate::context::test_context;

    fn default_calendar(ctx: &JmapContext) -> String {
        calendar_get(ctx, &json!({"accountId": "1"}), "c0").arguments()["list"][0]["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn create(ctx: &JmapContext, object: Value) -> Value {
        calendar_event_set(
            ctx,
            &json!({"accountId": "1", "create": {"a": object}}),
            "c0",
        )
        .arguments()
        .clone()
    }

    #[test]
    fn a_created_event_comes_back_from_get_with_its_fields() {
        let ctx = test_context();
        let calendar = default_calendar(&ctx);
        let response = create(
            &ctx,
            json!({
                "calendarId": calendar,
                "title": "Standup",
                "start": "2026-02-10T09:00:00",
                "timeZone": "UTC",
                "duration": "PT30M",
                "alerts": [{"minutesBefore": 10}],
            }),
        );
        let id = response["created"]["a"]["id"].as_str().unwrap().to_string();
        let list = calendar_event_get(&ctx, &json!({"accountId": "1", "ids": [id]}), "c1");
        let item = &list.arguments()["list"][0];
        assert_eq!(item["title"], "Standup");
        assert_eq!(item["calendarId"], calendar);
        assert_eq!(item["start"], "2026-02-10T09:00:00");
        assert_eq!(item["duration"], "PT30M");
        assert_eq!(item["alerts"], json!([{"minutesBefore": 10}]));
        assert!(item["uid"].as_str().unwrap().ends_with("@irixmail"));
        assert!(item["etag"].is_string());
        assert!(item["created"].as_str().unwrap().ends_with('Z'));
    }

    #[test]
    fn an_event_without_a_calendar_is_not_created() {
        let ctx = test_context();
        let response = create(&ctx, json!({"start": "2026-02-10T09:00:00"}));
        assert_eq!(response["notCreated"]["a"]["type"], "invalidProperties");
    }

    #[test]
    fn an_event_without_a_start_is_not_created() {
        let ctx = test_context();
        let calendar = default_calendar(&ctx);
        let response = create(&ctx, json!({"calendarId": calendar, "title": "x"}));
        assert_eq!(response["notCreated"]["a"]["properties"], json!(["start"]));
    }

    #[test]
    fn an_update_patches_only_the_named_fields() {
        let ctx = test_context();
        let calendar = default_calendar(&ctx);
        let id = create(
            &ctx,
            json!({
                "calendarId": calendar,
                "title": "Standup",
                "location": "Room 1",
                "start": "2026-02-10T09:00:00",
                "timeZone": "UTC",
                "duration": "PT30M",
            }),
        )["created"]["a"]["id"]
            .as_str()
            .unwrap()
            .to_string();

        let mut update = Map::new();
        update.insert(id.clone(), json!({"title": "Sync"}));
        let response = calendar_event_set(&ctx, &json!({"accountId": "1", "update": update}), "c1");
        assert!(response.arguments()["updated"].get(&id).is_some());

        let item = calendar_event_get(&ctx, &json!({"accountId": "1", "ids": [id]}), "c2")
            .arguments()["list"][0]
            .clone();
        assert_eq!(item["title"], "Sync");
        assert_eq!(item["location"], "Room 1");
        assert_eq!(item["duration"], "PT30M");
    }

    #[test]
    fn an_update_can_move_an_event_between_calendars() {
        let ctx = test_context();
        let calendar = default_calendar(&ctx);
        let other = crate::calendar_set::calendar_set(
            &ctx,
            &json!({"accountId": "1", "create": {"b": {"name": "Trips"}}}),
            "c0",
        )
        .arguments()["created"]["b"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let id = create(
            &ctx,
            json!({"calendarId": calendar, "title": "Flight", "start": "2026-02-10T09:00:00", "timeZone": "UTC", "duration": "PT2H"}),
        )["created"]["a"]["id"]
            .as_str()
            .unwrap()
            .to_string();

        let mut update = Map::new();
        update.insert(id.clone(), json!({"calendarId": other.clone()}));
        let response = calendar_event_set(&ctx, &json!({"accountId": "1", "update": update}), "c1");
        let moved = response.arguments()["updated"][&id]["id"]
            .as_str()
            .unwrap()
            .to_string();

        let item = calendar_event_get(&ctx, &json!({"accountId": "1", "ids": [moved]}), "c2")
            .arguments()["list"][0]
            .clone();
        assert_eq!(item["calendarId"], other);
        assert_eq!(item["title"], "Flight");
    }

    #[test]
    fn an_update_of_a_missing_event_is_not_found() {
        let ctx = test_context();
        let response = calendar_event_set(
            &ctx,
            &json!({"accountId": "1", "update": {"404": {"title": "x"}}}),
            "c0",
        );
        assert_eq!(
            response.arguments()["notUpdated"]["404"]["type"],
            "notFound"
        );
    }

    #[test]
    fn a_destroyed_event_disappears_and_shows_in_changes() {
        let ctx = test_context();
        let calendar = default_calendar(&ctx);
        let id = create(
            &ctx,
            json!({"calendarId": calendar, "title": "Gone", "start": "2026-02-10T09:00:00", "timeZone": "UTC", "duration": "PT1H"}),
        )["created"]["a"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let before = collection_state(ctx.store.as_ref(), 1, Collection::CalendarEvent);

        let response = calendar_event_set(
            &ctx,
            &json!({"accountId": "1", "destroy": [id.clone()]}),
            "c1",
        );
        assert_eq!(response.arguments()["destroyed"], json!([id]));

        let list = calendar_event_get(&ctx, &json!({"accountId": "1"}), "c2");
        assert_eq!(list.arguments()["list"], json!([]));

        let changes =
            calendar_event_changes(&ctx, &json!({"accountId": "1", "sinceState": before}), "c3");
        assert_eq!(changes.arguments()["destroyed"], json!([id]));
    }

    #[test]
    fn a_destroy_of_a_missing_event_is_not_found() {
        let ctx = test_context();
        let response =
            calendar_event_set(&ctx, &json!({"accountId": "1", "destroy": ["404"]}), "c0");
        assert_eq!(
            response.arguments()["notDestroyed"]["404"]["type"],
            "notFound"
        );
    }
}
