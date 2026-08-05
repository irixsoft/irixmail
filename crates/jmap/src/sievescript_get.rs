use serde_json::{json, Value};

use crate::context::JmapContext;
use crate::reply::{account_id, requested_ids, STATE};
use crate::request::Invocation;

pub fn sievescript_get(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let scripts = ctx
        .directory
        .sieve()
        .list(ctx.account_id)
        .unwrap_or_default();
    let wanted = requested_ids(args);

    let list: Vec<Value> = scripts
        .iter()
        .filter(|script| {
            wanted
                .as_ref()
                .is_none_or(|ids| ids.iter().any(|wanted| wanted == &script.id))
        })
        .map(|script| {
            json!({
                "id": script.id,
                "name": script.name,
                "rules": script.rules.clone().unwrap_or(Value::Null),
                "source": irixmail_mail::script_source(script),
                "isActive": script.active,
            })
        })
        .collect();

    let not_found: Vec<Value> = match &wanted {
        Some(ids) => ids
            .iter()
            .filter(|id| !scripts.iter().any(|script| &script.id == *id))
            .cloned()
            .map(Value::String)
            .collect(),
        None => Vec::new(),
    };

    Invocation::new(
        "SieveScript/get",
        json!({
            "accountId": account_id(args),
            "state": STATE,
            "list": list,
            "notFound": not_found,
        }),
        call_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_context;

    #[test]
    fn an_account_without_scripts_returns_an_empty_list() {
        let ctx = test_context();
        let response = sievescript_get(&ctx, &json!({"accountId": "1", "ids": null}), "c0");
        assert_eq!(response.name(), "SieveScript/get");
        assert_eq!(response.arguments()["list"], json!([]));
    }

    #[test]
    fn a_script_reports_its_rules_source_and_active_flag() {
        let ctx = test_context();
        let rules = json!([{"id": "r1", "name": "receipts", "field": "subject",
            "operator": "contains", "value": "receipt", "action": "fileinto",
            "target": "Receipts"}]);
        let script = ctx
            .directory
            .sieve()
            .create(ctx.account_id, "filters", "", Some(rules.clone()))
            .unwrap();

        let response = sievescript_get(&ctx, &json!({"accountId": "1", "ids": null}), "c0");
        let listed = &response.arguments()["list"][0];
        assert_eq!(listed["id"], json!(script.id));
        assert_eq!(listed["rules"], rules);
        assert_eq!(listed["isActive"], json!(true));
        assert!(listed["source"]
            .as_str()
            .unwrap()
            .contains("fileinto \"Receipts\";"));
    }

    #[test]
    fn an_externally_edited_script_reports_null_rules() {
        let ctx = test_context();
        ctx.directory
            .sieve()
            .create(ctx.account_id, "custom", "keep;", None)
            .unwrap();

        let response = sievescript_get(&ctx, &json!({"accountId": "1", "ids": null}), "c0");
        let listed = &response.arguments()["list"][0];
        assert_eq!(listed["rules"], Value::Null);
        assert_eq!(listed["source"], json!("keep;"));
    }

    #[test]
    fn unknown_requested_ids_are_reported_as_not_found() {
        let ctx = test_context();
        let response = sievescript_get(&ctx, &json!({"accountId": "1", "ids": ["missing"]}), "c0");
        assert_eq!(response.arguments()["notFound"], json!(["missing"]));
    }
}
