use serde_json::{json, Map, Value};

use irixmail_mail::{
    allocate_document_id, append_message, load_mailboxes, load_raw, provision_mailboxes,
    AppendRequest, Keyword, Mailbox, FIRST_USER_MAILBOX_ID,
};

use irixmail_store::Collection;

use crate::context::JmapContext;
use crate::reply::{account_id, collection_state};
use crate::request::Invocation;

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

pub fn email_copy(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let account = ctx.account_id as u32;
    let from_account = args
        .get("fromAccountId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let old_state = collection_state(ctx.store.as_ref(), account, Collection::Email);

    let mut created = Map::new();
    let mut not_created = Map::new();
    if let Some(create) = args.get("create").and_then(Value::as_object) {
        for (creation_id, object) in create {
            match copy_one(ctx, account, object) {
                Ok(id) => {
                    created.insert(creation_id.clone(), json!({ "id": id.to_string() }));
                }
                Err(err) => {
                    not_created.insert(creation_id.clone(), err);
                }
            }
        }
    }

    Invocation::new(
        "Email/copy",
        json!({
            "fromAccountId": from_account,
            "accountId": account_id(args),
            "oldState": old_state,
            "newState": collection_state(ctx.store.as_ref(), account, Collection::Email),
            "created": created,
            "notCreated": not_created,
        }),
        call_id,
    )
}

fn copy_one(ctx: &JmapContext, account: u32, object: &Value) -> Result<u32, Value> {
    let source = object
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| id.parse::<u32>().ok())
        .ok_or_else(|| set_error("invalidProperties"))?;
    let raw = load_raw(ctx.store.as_ref(), ctx.blobs.as_ref(), account, source)
        .map_err(|_| set_error("serverFail"))?
        .ok_or_else(|| set_error("notFound"))?;
    let mailbox_id = object
        .get("mailboxIds")
        .and_then(Value::as_object)
        .and_then(|ids| {
            ids.iter()
                .find(|(_, on)| on.as_bool() == Some(true))
                .map(|(id, _)| id)
        })
        .and_then(|id| id.parse::<u32>().ok())
        .ok_or_else(|| set_error("invalidProperties"))?;
    let record = ctx
        .directory
        .accounts()
        .get(ctx.account_id)
        .map_err(|_| set_error("serverFail"))?;
    let mailbox = resolve_mailbox(ctx, account, record.created_at, mailbox_id)
        .ok_or_else(|| set_error("notFound"))?;
    let flags = keywords_from(object.get("keywords"));
    let document_id =
        allocate_document_id(ctx.store.as_ref(), account).map_err(|_| set_error("serverFail"))?;
    let request = AppendRequest {
        account: &record,
        mailbox: &mailbox,
        flags,
        received_at: now_seconds(),
        document_id,
        raw: &raw,
    };
    let outcome = append_message(
        ctx.store.as_ref(),
        ctx.blobs.as_ref(),
        ctx.notifier.as_ref(),
        &request,
    )
    .map_err(|_| set_error("serverFail"))?;
    if outcome.over_quota {
        return Err(set_error("overQuota"));
    }
    Ok(document_id)
}

fn resolve_mailbox(
    ctx: &JmapContext,
    account: u32,
    created_at: u64,
    mailbox_id: u32,
) -> Option<Mailbox> {
    let mut mailboxes = provision_mailboxes(created_at);
    if let Ok(persisted) = load_mailboxes(ctx.store.as_ref(), account) {
        mailboxes.extend(
            persisted
                .into_iter()
                .filter(|m| m.id >= FIRST_USER_MAILBOX_ID),
        );
    }
    mailboxes.into_iter().find(|m| m.id == mailbox_id)
}

fn keywords_from(value: Option<&Value>) -> Vec<Keyword> {
    value
        .and_then(Value::as_object)
        .map(|keywords| {
            keywords
                .iter()
                .filter(|(_, on)| on.as_bool() == Some(true))
                .map(|(name, _)| Keyword::from_jmap(name))
                .collect()
        })
        .unwrap_or_default()
}

fn set_error(kind: &str) -> Value {
    json!({ "type": kind })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_context_with_account;
    use irixmail_mail::{MessageStoreCache, INBOX_ID};

    #[test]
    fn email_copy_duplicates_a_message_into_a_target_mailbox() {
        let ctx = test_context_with_account();
        let account = ctx.account_id as u32;
        let record = ctx.directory.accounts().get(ctx.account_id).unwrap();
        let mailboxes = provision_mailboxes(record.created_at);
        let inbox = mailboxes.iter().find(|m| m.id == INBOX_ID).unwrap();
        let raw: &[u8] = b"Subject: Copy me\r\nFrom: a@example.net\r\n\r\nbody\r\n";
        let source = allocate_document_id(ctx.store.as_ref(), account).unwrap();
        append_message(
            ctx.store.as_ref(),
            ctx.blobs.as_ref(),
            ctx.notifier.as_ref(),
            &AppendRequest {
                account: &record,
                mailbox: inbox,
                flags: vec![],
                received_at: 0,
                document_id: source,
                raw,
            },
        )
        .unwrap();

        let args = json!({
            "fromAccountId": account.to_string(),
            "accountId": account.to_string(),
            "create": { "c1": { "id": source.to_string(), "mailboxIds": { "3": true } } }
        });
        let response = email_copy(&ctx, &args, "c0");
        let new_id: u32 = response.arguments()["created"]["c1"]["id"]
            .as_str()
            .expect("copy created")
            .parse()
            .unwrap();
        assert_ne!(new_id, source);
        let cache = MessageStoreCache::build(ctx.store.as_ref(), account).unwrap();
        assert!(cache.get(new_id).unwrap().in_mailbox(3));
    }

    #[test]
    fn copying_an_unknown_email_is_not_created() {
        let ctx = test_context_with_account();
        let account = ctx.account_id as u32;
        let args = json!({
            "accountId": account.to_string(),
            "create": { "c1": { "id": "999", "mailboxIds": { "3": true } } }
        });
        let response = email_copy(&ctx, &args, "c0");
        assert_eq!(response.arguments()["notCreated"]["c1"]["type"], "notFound");
    }
}
