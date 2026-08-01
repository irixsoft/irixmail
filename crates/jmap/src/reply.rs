use std::collections::BTreeMap;

use serde_json::{json, Value};

use irixmail_dav::storage::DavStore;
use irixmail_store::{ChangeKind, ChangeLog, Collection, Store};

use crate::context::JmapContext;
use crate::request::Invocation;

pub const STATE: &str = "0";

pub fn dav_store(ctx: &JmapContext) -> DavStore<'_> {
    DavStore::new(
        ctx.store.as_ref(),
        ctx.notifier.as_ref(),
        ctx.account_id as u32,
    )
}

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

pub fn set_error(kind: &str, description: &str) -> serde_json::Value {
    json!({ "type": kind, "description": description })
}

pub fn invalid_property(property: &str, description: &str) -> serde_json::Value {
    json!({
        "type": "invalidProperties",
        "properties": [property],
        "description": description,
    })
}

pub fn collection_state(store: &dyn Store, account: u32, collection: Collection) -> String {
    ChangeLog::new(store)
        .latest_change_id(account, collection)
        .unwrap_or(0)
        .to_string()
}

pub fn account_id(args: &Value) -> String {
    args.get("accountId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

pub fn get_too_large(args: &Value) -> bool {
    matches!(args.get("ids"), Some(Value::Array(items)) if items.len() > crate::session::MAX_OBJECTS_IN_GET)
}

pub fn set_too_large(args: &Value) -> bool {
    let mapped = |name: &str| {
        args.get(name)
            .and_then(Value::as_object)
            .map(|map| map.len())
            .unwrap_or(0)
    };
    let destroyed = args
        .get("destroy")
        .and_then(Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0);
    mapped("create") + mapped("update") + destroyed > crate::session::MAX_OBJECTS_IN_SET
}

pub fn requested_ids(args: &Value) -> Option<Vec<String>> {
    match args.get("ids") {
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect(),
        ),
        _ => None,
    }
}

pub fn get_response(method: &str, args: &Value, call_id: &str, list: Value) -> Invocation {
    let not_found = match requested_ids(args) {
        Some(ids) => Value::Array(ids.into_iter().map(Value::String).collect()),
        None => json!([]),
    };
    Invocation::new(
        method,
        json!({
            "accountId": account_id(args),
            "state": STATE,
            "list": list,
            "notFound": not_found,
        }),
        call_id,
    )
}

pub fn set_response(method: &str, args: &Value, call_id: &str) -> Invocation {
    Invocation::new(
        method,
        json!({
            "accountId": account_id(args),
            "oldState": STATE,
            "newState": STATE,
            "created": {},
            "updated": {},
            "destroyed": [],
            "notCreated": {},
            "notUpdated": {},
            "notDestroyed": {},
        }),
        call_id,
    )
}

pub fn query_response(method: &str, args: &Value, call_id: &str) -> Invocation {
    Invocation::new(
        method,
        json!({
            "accountId": account_id(args),
            "queryState": STATE,
            "canCalculateChanges": false,
            "position": 0,
            "ids": [],
            "total": 0,
            "limit": 256,
        }),
        call_id,
    )
}

pub fn changes_response(method: &str, args: &Value, call_id: &str) -> Invocation {
    let old_state = args
        .get("sinceState")
        .and_then(Value::as_str)
        .unwrap_or(STATE);
    Invocation::new(
        method,
        json!({
            "accountId": account_id(args),
            "oldState": old_state,
            "newState": STATE,
            "hasMoreChanges": false,
            "created": [],
            "updated": [],
            "destroyed": [],
        }),
        call_id,
    )
}

pub fn collection_changes(
    store: &dyn Store,
    account: u32,
    args: &Value,
    method: &str,
    call_id: &str,
    collection: Collection,
) -> Invocation {
    let since = args
        .get("sinceState")
        .and_then(Value::as_str)
        .and_then(|state| state.parse::<u64>().ok())
        .unwrap_or(0);
    let max_changes = args
        .get("maxChanges")
        .and_then(Value::as_u64)
        .filter(|max| *max > 0)
        .map(|max| max as usize)
        .unwrap_or(usize::MAX);
    let log = ChangeLog::new(store);
    if !log
        .can_calculate(account, collection, since)
        .unwrap_or(true)
    {
        return crate::request::method_error("cannotCalculateChanges", call_id);
    }
    let (entries, has_more) = log
        .changes_page(account, collection, since, max_changes)
        .unwrap_or_default();

    let mut effect: BTreeMap<u32, (bool, bool, bool)> = BTreeMap::new();
    let mut new_state = since;
    for entry in &entries {
        let slot = effect.entry(entry.document_id).or_default();
        match entry.kind {
            ChangeKind::Insert => slot.0 = true,
            ChangeKind::Update => slot.1 = true,
            ChangeKind::Delete => slot.2 = true,
        }
        new_state = new_state.max(entry.change_id);
    }

    let mut created = Vec::new();
    let mut updated = Vec::new();
    let mut destroyed = Vec::new();
    for (document_id, (inserted, mutated, deleted)) in effect {
        let id = Value::String(document_id.to_string());
        if inserted && deleted {
            continue;
        } else if inserted {
            created.push(id);
        } else if deleted {
            destroyed.push(id);
        } else if mutated {
            updated.push(id);
        }
    }

    Invocation::new(
        method,
        json!({
            "accountId": account_id(args),
            "oldState": since.to_string(),
            "newState": new_state.to_string(),
            "hasMoreChanges": has_more,
            "created": created,
            "updated": updated,
            "destroyed": destroyed,
        }),
        call_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_id_is_extracted() {
        assert_eq!(account_id(&json!({"accountId": "a1"})), "a1");
        assert_eq!(account_id(&json!({})), "");
    }

    #[test]
    fn requested_ids_distinguishes_null_from_a_list() {
        assert_eq!(requested_ids(&json!({"ids": null})), None);
        assert_eq!(
            requested_ids(&json!({"ids": ["x", "y"]})),
            Some(vec!["x".to_string(), "y".to_string()])
        );
    }

    #[test]
    fn a_get_with_specific_ids_reports_them_not_found() {
        let response = get_response(
            "Mailbox/get",
            &json!({"accountId": "a1", "ids": ["m1"]}),
            "c0",
            json!([]),
        );
        assert_eq!(response.arguments()["notFound"], json!(["m1"]));
        assert_eq!(response.arguments()["list"], json!([]));
        assert_eq!(response.arguments()["accountId"], "a1");
    }

    #[test]
    fn a_get_for_all_ids_finds_nothing_missing() {
        let response = get_response(
            "Mailbox/get",
            &json!({"accountId": "a1", "ids": null}),
            "c0",
            json!([]),
        );
        assert_eq!(response.arguments()["notFound"], json!([]));
    }

    #[test]
    fn a_set_response_has_the_standard_shape() {
        let response = set_response("Mailbox/set", &json!({"accountId": "a1"}), "c0");
        let args = response.arguments();
        assert_eq!(args["oldState"], STATE);
        assert!(args["created"].is_object());
        assert!(args["destroyed"].is_array());
    }

    #[test]
    fn a_query_response_is_empty() {
        let response = query_response("Email/query", &json!({"accountId": "a1"}), "c0");
        assert_eq!(response.arguments()["ids"], json!([]));
        assert_eq!(response.arguments()["total"], 0);
    }

    #[test]
    fn a_changes_response_echoes_the_since_state() {
        let response = changes_response("Email/changes", &json!({"sinceState": "7"}), "c0");
        assert_eq!(response.arguments()["oldState"], "7");
        assert_eq!(response.arguments()["newState"], STATE);
    }
}
