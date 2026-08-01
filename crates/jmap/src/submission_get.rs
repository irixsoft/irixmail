use std::collections::HashSet;

use serde_json::{json, Value};

use irixmail_store::{Collection, Flow, KeyPrefix, Subspace};

use crate::context::JmapContext;
use crate::reply::{account_id, requested_ids, STATE};
use crate::request::{method_error, Invocation};

pub fn submission_get(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let account = ctx.account_id as u32;
    let wanted = requested_ids(args);

    let prefix = KeyPrefix::collection(Subspace::Property, account, Collection::EmailSubmission);
    let mut records: Vec<Value> = Vec::new();
    let listed = ctx.store.iterate(&prefix, &mut |_key, value| {
        if let Ok(record) = serde_json::from_slice::<Value>(value) {
            records.push(record);
        }
        Ok(Flow::Continue)
    });
    if listed.is_err() {
        return method_error("serverFail", call_id);
    }

    let mut list = Vec::new();
    let mut found = HashSet::new();
    for record in records {
        let id = record
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if let Some(ids) = &wanted {
            if !ids.contains(&id) {
                continue;
            }
        }
        found.insert(id);
        list.push(record);
    }

    let not_found: Vec<Value> = match &wanted {
        Some(ids) => ids
            .iter()
            .filter(|id| !found.contains(*id))
            .cloned()
            .map(Value::String)
            .collect(),
        None => Vec::new(),
    };

    Invocation::new(
        "EmailSubmission/get",
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
    use crate::submission_set::submission_key;

    fn seed(ctx: &JmapContext, account: u32, id: u32) {
        let record =
            json!({"id": id.to_string(), "emailId": id.to_string(), "undoStatus": "final"});
        ctx.store
            .put(
                &submission_key(account, id),
                &serde_json::to_vec(&record).unwrap(),
            )
            .unwrap();
    }

    #[test]
    fn submission_get_lists_persisted_submissions() {
        let ctx = test_context();
        seed(&ctx, 1, 10);
        seed(&ctx, 1, 11);
        let response = submission_get(&ctx, &json!({"accountId": "1", "ids": null}), "c0");
        let list = response.arguments()["list"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert!(list
            .iter()
            .any(|s| s["id"] == "10" && s["undoStatus"] == "final"));
    }

    #[test]
    fn a_listing_store_error_is_a_server_fail_not_a_truncated_list() {
        use std::sync::atomic::Ordering;
        use std::sync::Arc;

        let base = test_context();
        seed(&base, 1, 10);
        let flaky = crate::context::test_flaky::FlakyStore::wrap(Arc::clone(&base.store));
        flaky.fail_iterates.store(true, Ordering::SeqCst);
        let store: Arc<dyn irixmail_store::Store> = flaky;
        let ctx = JmapContext::from_parts(
            store,
            Arc::clone(&base.blobs),
            Arc::clone(&base.notifier),
            base.directory.clone(),
            base.account_id,
            None,
        );

        let response = submission_get(&ctx, &json!({"accountId": "1", "ids": null}), "c0");
        assert_eq!(response.name(), "error", "got: {:?}", response.arguments());
        assert_eq!(response.arguments()["type"], "serverFail");
    }

    #[test]
    fn submission_get_filters_by_id_and_reports_not_found() {
        let ctx = test_context();
        seed(&ctx, 1, 10);
        let response = submission_get(&ctx, &json!({"accountId": "1", "ids": ["10", "99"]}), "c0");
        assert_eq!(response.arguments()["list"].as_array().unwrap().len(), 1);
        assert_eq!(response.arguments()["notFound"], json!(["99"]));
    }
}
