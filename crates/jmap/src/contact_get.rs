use std::collections::BTreeMap;

use serde_json::{json, Value};

use irixmail_dav::model::{AddressBookCollection, ContactCardRecord};
use irixmail_dav::parse::parse_vcf;
use irixmail_dav::storage::{DavStore, DEFAULT_ADDRESS_BOOK_NAME};
use irixmail_store::Collection;

use crate::contact_object::card_fields;
use crate::context::JmapContext;
use crate::reply::{
    account_id, collection_changes, collection_state, dav_store, now_millis, requested_ids,
};
use crate::request::Invocation;
use crate::utc_date;

pub fn address_book_json(book: &AddressBookCollection) -> Value {
    json!({
        "id": book.id.to_string(),
        "name": book.display_name,
        "description": book.description,
        "isDefault": book.name == DEFAULT_ADDRESS_BOOK_NAME,
    })
}

pub fn card_json(record: &ContactCardRecord, member_ids: &BTreeMap<String, String>) -> Value {
    let mut item = match parse_vcf(&record.vcf) {
        Ok((card, _)) => card_fields(&card, member_ids),
        Err(_) => json!({}),
    };
    let Some(map) = item.as_object_mut() else {
        return item;
    };
    map.insert("id".to_string(), json!(record.id.to_string()));
    map.insert(
        "addressBookId".to_string(),
        json!(record.book_id.to_string()),
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

pub fn member_index(store: &DavStore<'_>) -> BTreeMap<String, String> {
    store
        .list_cards(None)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|record| record.uid.map(|uid| (uid, record.id.to_string())))
        .collect()
}

pub fn addressbook_get(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let store = dav_store(ctx);
    let _ = store.ensure_defaults(now_millis());
    let wanted = requested_ids(args);
    let books = store.list_address_books().unwrap_or_default();

    let mut list = Vec::new();
    let mut found = Vec::new();
    for book in &books {
        let id = book.id.to_string();
        if let Some(ids) = &wanted {
            if !ids.contains(&id) {
                continue;
            }
        }
        found.push(id);
        list.push(address_book_json(book));
    }

    Invocation::new(
        "AddressBook/get",
        json!({
            "accountId": account_id(args),
            "state": collection_state(ctx.store.as_ref(), ctx.account_id as u32, Collection::AddressBook),
            "list": list,
            "notFound": missing(&wanted, &found),
        }),
        call_id,
    )
}

pub fn addressbook_changes(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    collection_changes(
        ctx.store.as_ref(),
        ctx.account_id as u32,
        args,
        "AddressBook/changes",
        call_id,
        Collection::AddressBook,
    )
}

pub fn contact_get(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let store = dav_store(ctx);
    let _ = store.ensure_defaults(now_millis());
    let wanted = requested_ids(args);
    let members = member_index(&store);
    let records = store.list_cards(None).unwrap_or_default();

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
        list.push(card_json(record, &members));
    }

    Invocation::new(
        "ContactCard/get",
        json!({
            "accountId": account_id(args),
            "state": collection_state(ctx.store.as_ref(), ctx.account_id as u32, Collection::ContactCard),
            "list": list,
            "notFound": missing(&wanted, &found),
        }),
        call_id,
    )
}

pub fn contact_changes(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    collection_changes(
        ctx.store.as_ref(),
        ctx.account_id as u32,
        args,
        "ContactCard/changes",
        call_id,
        Collection::ContactCard,
    )
}

fn missing(wanted: &Option<Vec<String>>, found: &[String]) -> Vec<Value> {
    match wanted {
        Some(ids) => ids
            .iter()
            .filter(|id| !found.contains(id))
            .cloned()
            .map(Value::String)
            .collect(),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_context;

    #[test]
    fn a_default_address_book_appears_on_the_first_get() {
        let ctx = test_context();
        let response = addressbook_get(&ctx, &json!({"accountId": "1"}), "c0");
        let list = response.arguments()["list"].as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["name"], "Contacts");
        assert_eq!(list[0]["isDefault"], true);
        assert_eq!(list[0]["description"], Value::Null);
    }

    #[test]
    fn an_unknown_address_book_id_is_reported_not_found() {
        let ctx = test_context();
        let response = addressbook_get(&ctx, &json!({"accountId": "1", "ids": ["99"]}), "c0");
        assert_eq!(response.arguments()["notFound"], json!(["99"]));
    }

    #[test]
    fn a_fresh_account_has_no_contact_cards() {
        let ctx = test_context();
        let response = contact_get(&ctx, &json!({"accountId": "1"}), "c0");
        assert_eq!(response.arguments()["list"], json!([]));
    }
}
