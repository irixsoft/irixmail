use std::collections::{BTreeMap, HashSet};

use serde_json::{json, Value};

use irixmail_mail::MessageStoreCache;

use crate::context::JmapContext;
use crate::reply::{account_id, requested_ids, STATE};
use crate::request::Invocation;

pub fn thread_get(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let account = ctx.account_id as u32;
    let wanted = requested_ids(args);

    let mut threads: BTreeMap<u32, Vec<(u64, u32)>> = BTreeMap::new();
    if let Ok(cache) = MessageStoreCache::build(ctx.store.as_ref(), account) {
        for entry in cache.entries() {
            threads
                .entry(entry.thread_id)
                .or_default()
                .push((entry.received_at, entry.document_id));
        }
    }

    let mut list = Vec::new();
    let mut found = HashSet::new();
    for (thread_id, mut emails) in threads {
        let id = thread_id.to_string();
        if let Some(ids) = &wanted {
            if !ids.contains(&id) {
                continue;
            }
        }
        emails.sort_unstable();
        let email_ids: Vec<Value> = emails
            .into_iter()
            .map(|(_, document_id)| Value::String(document_id.to_string()))
            .collect();
        found.insert(id.clone());
        list.push(json!({ "id": id, "emailIds": email_ids }));
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
        "Thread/get",
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
    use irixmail_mail::MessageData;
    use irixmail_store::{serialize, Collection, Key, Subspace};

    fn seed(ctx: &JmapContext, account: u32, document_id: u32, thread_id: u32) {
        let data = MessageData::new(thread_id, 100);
        let key = Key::new(Subspace::Property, account, Collection::Email, document_id).encode();
        ctx.store
            .put(&key, &serialize::archive(&data).unwrap())
            .unwrap();
    }

    #[test]
    fn thread_get_groups_emails_by_thread_id() {
        let ctx = test_context();
        let account = ctx.account_id as u32;
        seed(&ctx, account, 10, 100);
        seed(&ctx, account, 11, 100);
        seed(&ctx, account, 12, 200);

        let response = thread_get(&ctx, &json!({"accountId": "1", "ids": ["100"]}), "c0");
        let list = response.arguments()["list"].as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["id"], "100");
        let emails = list[0]["emailIds"].as_array().unwrap();
        assert!(emails.contains(&json!("10")) && emails.contains(&json!("11")));
        assert!(!emails.contains(&json!("12")));
    }

    #[test]
    fn thread_get_reports_an_unknown_thread_as_not_found() {
        let ctx = test_context();
        let response = thread_get(&ctx, &json!({"accountId": "1", "ids": ["999"]}), "c0");
        assert_eq!(response.arguments()["notFound"], json!(["999"]));
        assert!(response.arguments()["list"].as_array().unwrap().is_empty());
    }
}
