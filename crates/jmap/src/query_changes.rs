use std::collections::HashSet;

use serde_json::{json, Value};

use irixmail_store::{ChangeKind, ChangeLog, Collection};

use crate::context::JmapContext;
use crate::reply::account_id;
use crate::request::{method_error, Invocation};

pub fn email_querychanges(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    run(
        ctx,
        args,
        call_id,
        "Email/queryChanges",
        Collection::Email,
        crate::email_query::query_ids,
    )
}

pub fn mailbox_querychanges(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    run(
        ctx,
        args,
        call_id,
        "Mailbox/queryChanges",
        Collection::Mailbox,
        crate::mailbox_query::query_ids,
    )
}

fn run(
    ctx: &JmapContext,
    args: &Value,
    call_id: &str,
    method: &str,
    collection: Collection,
    query_ids: fn(&JmapContext, &Value) -> Result<Vec<u32>, &'static str>,
) -> Invocation {
    let account = ctx.account_id as u32;
    let Some(since) = args
        .get("sinceQueryState")
        .and_then(Value::as_str)
        .and_then(|state| state.parse::<u64>().ok())
    else {
        return method_error("invalidArguments", call_id);
    };
    let log = ChangeLog::new(ctx.store.as_ref());
    if !log
        .can_calculate(account, collection, since)
        .unwrap_or(true)
    {
        return method_error("cannotCalculateChanges", call_id);
    }
    let entries = log
        .changes_since(account, collection, since)
        .unwrap_or_default();
    let mut changed: HashSet<u32> = HashSet::new();
    let mut removed_ids: HashSet<u32> = HashSet::new();
    for entry in &entries {
        match entry.kind {
            ChangeKind::Insert => {
                changed.insert(entry.document_id);
            }
            ChangeKind::Update => {
                changed.insert(entry.document_id);
                removed_ids.insert(entry.document_id);
            }
            ChangeKind::Delete => {
                changed.remove(&entry.document_id);
                removed_ids.insert(entry.document_id);
            }
        }
    }

    let ids = match query_ids(ctx, args) {
        Ok(ids) => ids,
        Err(kind) => return method_error(kind, call_id),
    };
    let up_to = args
        .get("upToId")
        .and_then(Value::as_str)
        .and_then(|id| id.parse::<u32>().ok());

    let mut added = Vec::new();
    for (index, id) in ids.iter().enumerate() {
        if changed.contains(id) {
            added.push(json!({ "id": id.to_string(), "index": index }));
        }
        if Some(*id) == up_to {
            break;
        }
    }
    let removed: Vec<Value> = removed_ids
        .into_iter()
        .map(|id| Value::String(id.to_string()))
        .collect();

    Invocation::new(
        method,
        json!({
            "accountId": account_id(args),
            "oldQueryState": since.to_string(),
            "newQueryState": crate::reply::collection_state(ctx.store.as_ref(), account, collection),
            "total": ids.len(),
            "removed": removed,
            "added": added,
        }),
        call_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_context_with_account;
    use irixmail_mail::{
        allocate_document_id, append_message, delete_message, provision_mailboxes, AppendRequest,
    };

    fn seed(ctx: &JmapContext, subject: &str) -> u32 {
        let account = ctx.account_id as u32;
        let record = ctx.directory.accounts().get(ctx.account_id).unwrap();
        let mailboxes = provision_mailboxes(record.created_at);
        let document_id = allocate_document_id(ctx.store.as_ref(), account).unwrap();
        let raw = format!("From: a@example.com\r\nSubject: {subject}\r\n\r\nbody\r\n");
        append_message(
            ctx.store.as_ref(),
            ctx.blobs.as_ref(),
            &ctx.notifier,
            &AppendRequest {
                account: &record,
                mailbox: &mailboxes[0],
                flags: Vec::new(),
                received_at: 1_700_000_000,
                document_id,
                raw: raw.as_bytes(),
            },
        )
        .unwrap();
        document_id
    }

    fn email_state(ctx: &JmapContext) -> String {
        crate::reply::collection_state(ctx.store.as_ref(), ctx.account_id as u32, Collection::Email)
    }

    #[test]
    fn added_and_removed_messages_are_reported_against_the_old_state() {
        let ctx = test_context_with_account();
        let first = seed(&ctx, "one");
        let state = email_state(&ctx);
        let second = seed(&ctx, "two");
        delete_message(
            ctx.store.as_ref(),
            ctx.blobs.as_ref(),
            &ctx.notifier,
            ctx.account_id as u32,
            first,
        )
        .unwrap();

        let response = email_querychanges(
            &ctx,
            &json!({"accountId": "1", "sinceQueryState": state}),
            "c0",
        );
        let args = response.arguments();
        assert_eq!(response.name(), "Email/queryChanges");
        let added = args["added"].as_array().unwrap();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0]["id"], second.to_string());
        assert!(args["removed"]
            .as_array()
            .unwrap()
            .contains(&Value::String(first.to_string())));
    }

    #[test]
    fn a_pruned_change_log_reports_cannot_calculate_changes() {
        let ctx = test_context_with_account();
        for index in 0..5 {
            seed(&ctx, &format!("m{index}"));
        }
        irixmail_store::prune_change_logs(ctx.store.as_ref(), 1).unwrap();

        let response = email_querychanges(
            &ctx,
            &json!({"accountId": "1", "sinceQueryState": "0"}),
            "c0",
        );
        assert_eq!(response.name(), "error");
        assert_eq!(response.arguments()["type"], "cannotCalculateChanges");
    }

    #[test]
    fn mailbox_querychanges_reports_a_created_mailbox() {
        let ctx = test_context_with_account();
        let state = crate::reply::collection_state(
            ctx.store.as_ref(),
            ctx.account_id as u32,
            Collection::Mailbox,
        );
        irixmail_mail::create_mailbox(
            ctx.store.as_ref(),
            &ctx.notifier,
            ctx.account_id as u32,
            "Projects",
            1_700,
        )
        .unwrap();

        let response = mailbox_querychanges(
            &ctx,
            &json!({"accountId": "1", "sinceQueryState": state}),
            "c0",
        );
        let added = response.arguments()["added"].as_array().unwrap().clone();
        assert_eq!(added.len(), 1, "{added:?}");
    }

    #[test]
    fn a_missing_since_state_is_invalid() {
        let ctx = test_context_with_account();
        let response = email_querychanges(&ctx, &json!({"accountId": "1"}), "c0");
        assert_eq!(response.name(), "error");
    }

    use crate::context::JmapContext;
}
