use rand::Rng;
use serde_json::{json, Map, Value};

use irixmail_dav::parse::parse_vcf;
use irixmail_dav::storage::DavStore;
use irixmail_store::Collection;

use crate::calendar_set::{slugify, unique_name};
use crate::contact_object::{build_card, patch_card};
use crate::context::JmapContext;
use crate::reply::{
    account_id, collection_state, dav_store, invalid_property, now_millis, set_error,
};
use crate::request::{method_error, Invocation};

pub fn addressbook_set(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let account = ctx.account_id as u32;
    let old_state = collection_state(ctx.store.as_ref(), account, Collection::AddressBook);
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
            match create_book(&store, object, now) {
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
            match update_book(&store, id, patch, now) {
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
            match destroy_book(&store, id) {
                Ok(()) => destroyed.push(Value::String(id.to_string())),
                Err(error) => {
                    not_destroyed.insert(id.to_string(), error);
                }
            }
        }
    }

    Invocation::new(
        "AddressBook/set",
        json!({
            "accountId": account_id(args),
            "oldState": old_state,
            "newState": collection_state(ctx.store.as_ref(), account, Collection::AddressBook),
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

pub fn contact_set(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let account = ctx.account_id as u32;
    let old_state = collection_state(ctx.store.as_ref(), account, Collection::ContactCard);
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
            match create_card(&store, object, now) {
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
            match update_card(&store, id, patch, now) {
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
            match destroy_card(&store, id) {
                Ok(()) => destroyed.push(Value::String(id.to_string())),
                Err(error) => {
                    not_destroyed.insert(id.to_string(), error);
                }
            }
        }
    }

    Invocation::new(
        "ContactCard/set",
        json!({
            "accountId": account_id(args),
            "oldState": old_state,
            "newState": collection_state(ctx.store.as_ref(), account, Collection::ContactCard),
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

fn create_book(store: &DavStore<'_>, object: &Value, now: u64) -> Result<String, Value> {
    let display_name = object
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| invalid_property("name", "an address book needs a name"))?;
    let taken: Vec<String> = store
        .list_address_books()
        .map_err(|_| set_error("serverFail", "the address books could not be read"))?
        .into_iter()
        .map(|book| book.name)
        .collect();
    let name = unique_name(&slugify(display_name), &taken);
    let mut book = store
        .create_address_book(&name, display_name, now)
        .map_err(|_| set_error("serverFail", "the address book could not be created"))?;
    if let Some(description) = object.get("description").and_then(Value::as_str) {
        book.description = Some(description.to_string());
        store
            .save_address_book(&book, now)
            .map_err(|_| set_error("serverFail", "the address book could not be saved"))?;
    }
    Ok(book.id.to_string())
}

fn update_book(store: &DavStore<'_>, id: &str, patch: &Value, now: u64) -> Result<(), Value> {
    let id = id
        .parse::<u32>()
        .map_err(|_| set_error("notFound", "no such address book"))?;
    let mut book = store
        .address_book_by_id(id)
        .ok()
        .flatten()
        .ok_or_else(|| set_error("notFound", "no such address book"))?;

    if let Some(value) = patch.get("name") {
        let name = value
            .as_str()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| invalid_property("name", "an address book needs a name"))?;
        book.display_name = name.to_string();
    }
    if let Some(value) = patch.get("description") {
        book.description = match value {
            Value::Null => None,
            Value::String(text) => Some(text.clone()),
            _ => {
                return Err(invalid_property(
                    "description",
                    "description must be a string or null",
                ))
            }
        };
    }

    store
        .save_address_book(&book, now)
        .map_err(|_| set_error("serverFail", "the address book could not be saved"))
}

fn destroy_book(store: &DavStore<'_>, id: &str) -> Result<(), Value> {
    let id = id
        .parse::<u32>()
        .map_err(|_| set_error("notFound", "no such address book"))?;
    let books = store
        .list_address_books()
        .map_err(|_| set_error("serverFail", "the address books could not be read"))?;
    if !books.iter().any(|book| book.id == id) {
        return Err(set_error("notFound", "no such address book"));
    }
    if books.len() <= 1 {
        return Err(set_error(
            "forbidden",
            "the last address book cannot be destroyed",
        ));
    }
    match store.delete_address_book(id) {
        Ok(true) => Ok(()),
        Ok(false) => Err(set_error("notFound", "no such address book")),
        Err(_) => Err(set_error(
            "serverFail",
            "the address book could not be removed",
        )),
    }
}

fn create_card(store: &DavStore<'_>, object: &Value, now: u64) -> Result<Value, Value> {
    let book_id = book_of(store, object.get("addressBookId"))?;
    let uid = new_uid();
    let vcf = build_card(object, &uid)
        .map_err(|property| invalid_property(&property, "the card could not be built"))?;
    let (_, info) = parse_vcf(&vcf)
        .map_err(|_| invalid_property("fullName", "the card is not a valid vcard"))?;
    let name = format!("{uid}.vcf");
    let (record, _) = store
        .upsert_card(book_id, &name, &vcf, &info, now)
        .map_err(|_| set_error("serverFail", "the card could not be stored"))?;
    Ok(json!({
        "id": record.id.to_string(),
        "uid": record.uid,
        "etag": record.etag,
    }))
}

fn update_card(store: &DavStore<'_>, id: &str, patch: &Value, now: u64) -> Result<Value, Value> {
    let id = id
        .parse::<u32>()
        .map_err(|_| set_error("notFound", "no such card"))?;
    let record = store
        .list_cards(None)
        .map_err(|_| set_error("serverFail", "the cards could not be read"))?
        .into_iter()
        .find(|record| record.id == id)
        .ok_or_else(|| set_error("notFound", "no such card"))?;

    let target = match patch.get("addressBookId") {
        Some(value) => book_of(store, Some(value))?,
        None => record.book_id,
    };
    let vcf = patch_card(&record.vcf, patch)
        .map_err(|property| invalid_property(&property, "the card could not be updated"))?;
    let (_, info) = parse_vcf(&vcf)
        .map_err(|_| invalid_property("fullName", "the card is not a valid vcard"))?;

    if target != record.book_id {
        store
            .delete_card(record.book_id, &record.name)
            .map_err(|_| set_error("serverFail", "the card could not be moved"))?;
    }
    let (stored, _) = store
        .upsert_card(target, &record.name, &vcf, &info, now)
        .map_err(|_| set_error("serverFail", "the card could not be stored"))?;
    if stored.id == record.id {
        Ok(Value::Null)
    } else {
        Ok(json!({ "id": stored.id.to_string() }))
    }
}

fn destroy_card(store: &DavStore<'_>, id: &str) -> Result<(), Value> {
    let id = id
        .parse::<u32>()
        .map_err(|_| set_error("notFound", "no such card"))?;
    let record = store
        .list_cards(None)
        .map_err(|_| set_error("serverFail", "the cards could not be read"))?
        .into_iter()
        .find(|record| record.id == id)
        .ok_or_else(|| set_error("notFound", "no such card"))?;
    match store.delete_card(record.book_id, &record.name) {
        Ok(true) => Ok(()),
        Ok(false) => Err(set_error("notFound", "no such card")),
        Err(_) => Err(set_error("serverFail", "the card could not be removed")),
    }
}

fn book_of(store: &DavStore<'_>, value: Option<&Value>) -> Result<u32, Value> {
    let id = value
        .and_then(Value::as_str)
        .and_then(|id| id.parse::<u32>().ok())
        .ok_or_else(|| invalid_property("addressBookId", "an address book id is required"))?;
    match store.address_book_by_id(id) {
        Ok(Some(_)) => Ok(id),
        _ => Err(invalid_property("addressBookId", "no such address book")),
    }
}

fn new_uid() -> String {
    let value: u128 = rand::rng().random();
    format!("{value:032x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contact_get::{addressbook_get, contact_changes, contact_get};
    use crate::context::test_context;

    fn default_book(ctx: &JmapContext) -> String {
        addressbook_get(ctx, &json!({"accountId": "1"}), "c0").arguments()["list"][0]["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn create(ctx: &JmapContext, object: Value) -> Value {
        contact_set(
            ctx,
            &json!({"accountId": "1", "create": {"a": object}}),
            "c0",
        )
        .arguments()
        .clone()
    }

    #[test]
    fn a_created_address_book_shows_up_in_get() {
        let ctx = test_context();
        let response = addressbook_set(
            &ctx,
            &json!({"accountId": "1", "create": {"b": {"name": "Work Mates", "description": "team"}}}),
            "c0",
        );
        let id = response.arguments()["created"]["b"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let item = addressbook_get(&ctx, &json!({"accountId": "1", "ids": [id]}), "c1").arguments()
            ["list"][0]
            .clone();
        assert_eq!(item["name"], "Work Mates");
        assert_eq!(item["description"], "team");
        assert_eq!(item["isDefault"], false);
    }

    #[test]
    fn the_last_address_book_cannot_be_destroyed() {
        let ctx = test_context();
        let id = default_book(&ctx);
        let response = addressbook_set(
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
    fn a_created_card_comes_back_from_get_with_its_fields() {
        let ctx = test_context();
        let book = default_book(&ctx);
        let response = create(
            &ctx,
            json!({
                "addressBookId": book,
                "fullName": "Ada Lovelace",
                "emails": [{"value": "ada@example.com", "label": "work"}],
            }),
        );
        let id = response["created"]["a"]["id"].as_str().unwrap().to_string();
        let item = contact_get(&ctx, &json!({"accountId": "1", "ids": [id]}), "c1").arguments()
            ["list"][0]
            .clone();
        assert_eq!(item["fullName"], "Ada Lovelace");
        assert_eq!(item["addressBookId"], book);
        assert_eq!(
            item["emails"],
            json!([{"value": "ada@example.com", "label": "work"}])
        );
        assert_eq!(item["kind"], "individual");
        assert!(item["etag"].is_string());
    }

    #[test]
    fn a_card_without_an_address_book_is_not_created() {
        let ctx = test_context();
        let response = create(&ctx, json!({"fullName": "Ada"}));
        assert_eq!(response["notCreated"]["a"]["type"], "invalidProperties");
    }

    #[test]
    fn an_update_patches_only_the_named_fields() {
        let ctx = test_context();
        let book = default_book(&ctx);
        let id = create(
            &ctx,
            json!({"addressBookId": book, "fullName": "Ada", "jobTitle": "Programmer"}),
        )["created"]["a"]["id"]
            .as_str()
            .unwrap()
            .to_string();

        let mut update = Map::new();
        update.insert(id.clone(), json!({"nickname": "Ada L"}));
        contact_set(&ctx, &json!({"accountId": "1", "update": update}), "c1");

        let item = contact_get(&ctx, &json!({"accountId": "1", "ids": [id]}), "c2").arguments()
            ["list"][0]
            .clone();
        assert_eq!(item["nickname"], "Ada L");
        assert_eq!(item["jobTitle"], "Programmer");
        assert_eq!(item["fullName"], "Ada");
    }

    #[test]
    fn an_update_of_a_missing_card_is_not_found() {
        let ctx = test_context();
        let response = contact_set(
            &ctx,
            &json!({"accountId": "1", "update": {"404": {"nickname": "x"}}}),
            "c0",
        );
        assert_eq!(
            response.arguments()["notUpdated"]["404"]["type"],
            "notFound"
        );
    }

    #[test]
    fn a_destroyed_card_disappears_and_shows_in_changes() {
        let ctx = test_context();
        let book = default_book(&ctx);
        let id = create(&ctx, json!({"addressBookId": book, "fullName": "Gone"}))["created"]["a"]
            ["id"]
            .as_str()
            .unwrap()
            .to_string();
        let before = collection_state(ctx.store.as_ref(), 1, Collection::ContactCard);

        let response = contact_set(
            &ctx,
            &json!({"accountId": "1", "destroy": [id.clone()]}),
            "c1",
        );
        assert_eq!(response.arguments()["destroyed"], json!([id]));
        assert_eq!(
            contact_get(&ctx, &json!({"accountId": "1"}), "c2").arguments()["list"],
            json!([])
        );
        let changes = contact_changes(&ctx, &json!({"accountId": "1", "sinceState": before}), "c3");
        assert_eq!(changes.arguments()["destroyed"], json!([id]));
    }
}
