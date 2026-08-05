use serde_json::{json, Map, Value};

use irixmail_core::Error;

use crate::context::JmapContext;
use crate::reply::{account_id, STATE};
use crate::request::Invocation;

pub fn sievescript_set(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let registry = ctx.directory.sieve();
    let account = ctx.account_id;
    let mut created = Map::new();
    let mut not_created = Map::new();
    let mut updated = Map::new();
    let mut not_updated = Map::new();
    let mut destroyed = Vec::new();
    let mut not_destroyed = Map::new();

    if let Some(create) = args.get("create").and_then(Value::as_object) {
        for (creation_id, body) in create {
            let name = body
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("filters");
            let rules = body.get("rules").cloned().unwrap_or_else(|| json!([]));
            let source = rules_source(&rules);
            match registry.create(account, name, &source, Some(rules)) {
                Ok(script) => {
                    created.insert(creation_id.clone(), json!({ "id": script.id }));
                }
                Err(Error::InvalidInput(_)) => {
                    not_created.insert(creation_id.clone(), set_error("invalidProperties"));
                }
                Err(_) => {
                    not_created.insert(creation_id.clone(), set_error("serverFail"));
                }
            }
        }
    }

    if let Some(update) = args.get("update").and_then(Value::as_object) {
        for (id, body) in update {
            let name = body.get("name").and_then(Value::as_str);
            let rules = body.get("rules").cloned();
            let source = rules.as_ref().map(rules_source);
            match registry.update(account, id, name, source.as_deref(), rules.map(Some)) {
                Ok(true) => {
                    updated.insert(id.clone(), Value::Null);
                }
                Ok(false) => {
                    not_updated.insert(id.clone(), set_error("notFound"));
                }
                Err(Error::InvalidInput(_)) => {
                    not_updated.insert(id.clone(), set_error("invalidProperties"));
                }
                Err(_) => {
                    not_updated.insert(id.clone(), set_error("serverFail"));
                }
            }
        }
    }

    if let Some(destroy) = args.get("destroy").and_then(Value::as_array) {
        for id in destroy.iter().filter_map(Value::as_str) {
            match registry.destroy(account, id) {
                Ok(true) => destroyed.push(Value::String(id.to_string())),
                Ok(false) => {
                    not_destroyed.insert(id.to_string(), set_error("notFound"));
                }
                Err(_) => {
                    not_destroyed.insert(id.to_string(), set_error("serverFail"));
                }
            }
        }
    }

    Invocation::new(
        "SieveScript/set",
        json!({
            "accountId": account_id(args),
            "oldState": STATE,
            "newState": STATE,
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

fn rules_source(rules: &Value) -> String {
    irixmail_mail::emit_script(&irixmail_mail::stored_rule_set(rules))
}

fn set_error(kind: &str) -> Value {
    json!({ "type": kind })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_context;

    #[test]
    fn a_created_script_is_listed_back() {
        let ctx = test_context();
        let response = sievescript_set(
            &ctx,
            &json!({"accountId": "1", "create": {"filters": {"name": "filters", "rules": []}}}),
            "c0",
        );
        let created = response.arguments()["created"]["filters"]["id"].as_str();
        assert!(created.is_some());
        let stored = ctx.directory.sieve().list(ctx.account_id).unwrap();
        assert_eq!(stored.len(), 1);
        assert!(stored[0].active);
        assert!(stored[0].source.starts_with("require ["));
    }

    #[test]
    fn destroying_an_unknown_script_reports_not_found() {
        let ctx = test_context();
        let response = sievescript_set(
            &ctx,
            &json!({"accountId": "1", "destroy": ["missing"]}),
            "c0",
        );
        assert_eq!(
            response.arguments()["notDestroyed"]["missing"]["type"],
            "notFound"
        );
    }

    #[test]
    fn updating_rules_reemits_the_stored_source() {
        let ctx = test_context();
        let script = ctx
            .directory
            .sieve()
            .create(ctx.account_id, "filters", "", Some(json!([])))
            .unwrap();
        let rules = json!([{"id": "r1", "name": "drop", "field": "from",
            "operator": "is", "value": "spam@example.com", "action": "discard",
            "target": ""}]);

        let response = sievescript_set(
            &ctx,
            &json!({"accountId": "1", "update": {(script.id.clone()): {"rules": rules}}}),
            "c0",
        );
        assert_eq!(response.arguments()["updated"][&script.id], Value::Null);
        let stored = ctx.directory.sieve().list(ctx.account_id).unwrap();
        assert!(stored[0].source.contains("discard;"));
        assert!(stored[0].rules.is_some());
    }

    #[test]
    fn creating_a_duplicate_name_reports_invalid_properties() {
        let ctx = test_context();
        ctx.directory
            .sieve()
            .create(ctx.account_id, "filters", "", None)
            .unwrap();
        let response = sievescript_set(
            &ctx,
            &json!({"accountId": "1", "create": {"x": {"name": "filters"}}}),
            "c0",
        );
        assert_eq!(
            response.arguments()["notCreated"]["x"]["type"],
            "invalidProperties"
        );
    }
}
