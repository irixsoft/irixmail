use serde_json::{json, Map, Value};

use irixmail_dav::storage::DavStore;
use irixmail_store::Collection;

use crate::context::JmapContext;
use crate::reply::{
    account_id, collection_state, dav_store, invalid_property, now_millis, set_error,
};
use crate::request::{method_error, Invocation};

pub fn calendar_set(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let account = ctx.account_id as u32;
    let old_state = collection_state(ctx.store.as_ref(), account, Collection::Calendar);
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
                Ok(id) => {
                    created.insert(creation_id.clone(), json!({ "id": id }));
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
                Ok(()) => {
                    updated.insert(id.clone(), Value::Null);
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
        "Calendar/set",
        json!({
            "accountId": account_id(args),
            "oldState": old_state,
            "newState": collection_state(ctx.store.as_ref(), account, Collection::Calendar),
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

fn create_one(store: &DavStore<'_>, object: &Value, now: u64) -> Result<String, Value> {
    let display_name = object
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| invalid_property("name", "a calendar needs a name"))?;
    let taken: Vec<String> = store
        .list_calendars()
        .map_err(|_| set_error("serverFail", "the calendars could not be read"))?
        .into_iter()
        .map(|calendar| calendar.name)
        .collect();
    let name = unique_name(&slugify(display_name), &taken);
    let mut calendar = store
        .create_calendar(&name, display_name, now)
        .map_err(|_| set_error("serverFail", "the calendar could not be created"))?;
    if let Some(color) = object.get("color").and_then(Value::as_str) {
        calendar.color = Some(color.to_string());
        store
            .save_calendar(&calendar, now)
            .map_err(|_| set_error("serverFail", "the calendar could not be saved"))?;
    }
    Ok(calendar.id.to_string())
}

fn update_one(store: &DavStore<'_>, id: &str, patch: &Value, now: u64) -> Result<(), Value> {
    let id = id
        .parse::<u32>()
        .map_err(|_| set_error("notFound", "no such calendar"))?;
    let mut calendar = store
        .calendar_by_id(id)
        .ok()
        .flatten()
        .ok_or_else(|| set_error("notFound", "no such calendar"))?;

    if let Some(value) = patch.get("name") {
        let name = value
            .as_str()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| invalid_property("name", "a calendar needs a name"))?;
        calendar.display_name = name.to_string();
    }
    if let Some(value) = patch.get("color") {
        calendar.color = match value {
            Value::Null => None,
            Value::String(color) => Some(color.clone()),
            _ => return Err(invalid_property("color", "color must be a string or null")),
        };
    }
    if let Some(value) = patch.get("sortOrder") {
        calendar.order = value
            .as_u64()
            .ok_or_else(|| invalid_property("sortOrder", "sortOrder must be a number"))?
            as u32;
    }
    if let Some(value) = patch.get("timeZone") {
        calendar.time_zone = match value {
            Value::Null => None,
            Value::String(zone) => Some(zone.clone()),
            _ => {
                return Err(invalid_property(
                    "timeZone",
                    "timeZone must be a string or null",
                ))
            }
        };
    }

    store
        .save_calendar(&calendar, now)
        .map_err(|_| set_error("serverFail", "the calendar could not be saved"))
}

fn destroy_one(store: &DavStore<'_>, id: &str) -> Result<(), Value> {
    let id = id
        .parse::<u32>()
        .map_err(|_| set_error("notFound", "no such calendar"))?;
    let calendars = store
        .list_calendars()
        .map_err(|_| set_error("serverFail", "the calendars could not be read"))?;
    if !calendars.iter().any(|calendar| calendar.id == id) {
        return Err(set_error("notFound", "no such calendar"));
    }
    if calendars.len() <= 1 {
        return Err(set_error(
            "forbidden",
            "the last calendar cannot be destroyed",
        ));
    }
    match store.delete_calendar(id) {
        Ok(true) => Ok(()),
        Ok(false) => Err(set_error("notFound", "no such calendar")),
        Err(_) => Err(set_error("serverFail", "the calendar could not be removed")),
    }
}

pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "calendar".to_string()
    } else {
        slug
    }
}

pub fn unique_name(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|name| name == base) {
        return base.to_string();
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("{base}-{suffix}");
        if !taken.contains(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar_get::calendar_get;
    use crate::context::test_context;

    fn create(ctx: &JmapContext, object: Value) -> Value {
        let response = calendar_set(
            ctx,
            &json!({"accountId": "1", "create": {"a": object}}),
            "c0",
        );
        response.arguments().clone()
    }

    #[test]
    fn a_created_calendar_gets_a_slugged_storage_name_and_shows_up_in_get() {
        let ctx = test_context();
        let response = create(&ctx, json!({"name": "Work Trips!", "color": "#ff0000"}));
        let id = response["created"]["a"]["id"].as_str().unwrap().to_string();
        let store = dav_store(&ctx);
        let calendar = store.calendar_by_id(id.parse().unwrap()).unwrap().unwrap();
        assert_eq!(calendar.name, "work-trips");
        assert_eq!(calendar.display_name, "Work Trips!");

        let list = calendar_get(&ctx, &json!({"accountId": "1", "ids": [id]}), "c1");
        assert_eq!(list.arguments()["list"][0]["name"], "Work Trips!");
        assert_eq!(list.arguments()["list"][0]["color"], "#ff0000");
        assert_eq!(list.arguments()["list"][0]["isDefault"], false);
    }

    #[test]
    fn a_repeated_display_name_gets_a_numbered_storage_name() {
        let ctx = test_context();
        create(&ctx, json!({"name": "Trips"}));
        create(&ctx, json!({"name": "Trips"}));
        let store = dav_store(&ctx);
        let names: Vec<String> = store
            .list_calendars()
            .unwrap()
            .into_iter()
            .map(|calendar| calendar.name)
            .collect();
        assert!(names.contains(&"trips".to_string()), "{names:?}");
        assert!(names.contains(&"trips-2".to_string()), "{names:?}");
    }

    #[test]
    fn a_calendar_without_a_name_is_not_created() {
        let ctx = test_context();
        let response = create(&ctx, json!({"color": "#fff"}));
        assert_eq!(response["notCreated"]["a"]["type"], "invalidProperties");
        assert_eq!(response["created"], json!({}));
    }

    #[test]
    fn an_update_changes_the_display_name_colour_and_zone() {
        let ctx = test_context();
        let id = create(&ctx, json!({"name": "Trips"}))["created"]["a"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let response = calendar_set(
            &ctx,
            &json!({
                "accountId": "1",
                "update": {id.clone(): {"name": "Journeys", "color": null, "sortOrder": 4, "timeZone": "Europe/Stockholm"}},
            }),
            "c1",
        );
        assert!(response.arguments()["updated"].get(&id).is_some());
        let list = calendar_get(&ctx, &json!({"accountId": "1", "ids": [id]}), "c2");
        let item = &list.arguments()["list"][0];
        assert_eq!(item["name"], "Journeys");
        assert_eq!(item["color"], Value::Null);
        assert_eq!(item["sortOrder"], 4);
        assert_eq!(item["timeZone"], "Europe/Stockholm");
    }

    #[test]
    fn an_update_of_a_missing_calendar_is_not_found() {
        let ctx = test_context();
        let response = calendar_set(
            &ctx,
            &json!({"accountId": "1", "update": {"404": {"name": "Nope"}}}),
            "c0",
        );
        assert_eq!(
            response.arguments()["notUpdated"]["404"]["type"],
            "notFound"
        );
    }

    #[test]
    fn the_last_calendar_cannot_be_destroyed() {
        let ctx = test_context();
        calendar_get(&ctx, &json!({"accountId": "1"}), "c0");
        let store = dav_store(&ctx);
        let id = store.list_calendars().unwrap()[0].id.to_string();
        let response = calendar_set(
            &ctx,
            &json!({"accountId": "1", "destroy": [id.clone()]}),
            "c1",
        );
        assert_eq!(
            response.arguments()["notDestroyed"][&id]["type"],
            "forbidden"
        );
    }

    #[test]
    fn a_second_calendar_can_be_destroyed() {
        let ctx = test_context();
        let id = create(&ctx, json!({"name": "Trips"}))["created"]["a"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let response = calendar_set(
            &ctx,
            &json!({"accountId": "1", "destroy": [id.clone()]}),
            "c1",
        );
        assert_eq!(response.arguments()["destroyed"], json!([id]));
        assert_ne!(
            response.arguments()["oldState"],
            response.arguments()["newState"]
        );
    }

    #[test]
    fn names_slug_down_to_lowercase_dashes() {
        assert_eq!(slugify("Work Trips!"), "work-trips");
        assert_eq!(slugify("  "), "calendar");
        assert_eq!(
            unique_name("trips", &["trips".to_string(), "trips-2".to_string()]),
            "trips-3"
        );
    }
}
