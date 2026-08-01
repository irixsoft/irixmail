use std::collections::HashSet;

use serde_json::{json, Value};

use irixmail_mail::{load_mailboxes, provision_mailboxes, Keyword, MessageStoreCache, SpecialUse};

use irixmail_store::Collection;

use crate::context::JmapContext;
use crate::reply::{account_id, collection_state, requested_ids};
use crate::request::Invocation;

pub fn mailbox_get(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let cache = MessageStoreCache::build(ctx.store.as_ref(), ctx.account_id as u32).ok();
    let wanted = requested_ids(args);
    let mut list = Vec::new();
    let mut found = HashSet::new();

    let mailboxes = match load_mailboxes(ctx.store.as_ref(), ctx.account_id as u32) {
        Ok(rows) if !rows.is_empty() => rows,
        _ => provision_mailboxes(0),
    };

    for (order, mailbox) in mailboxes.into_iter().enumerate() {
        let id = mailbox.id.to_string();
        if let Some(ids) = &wanted {
            if !ids.contains(&id) {
                continue;
            }
        }
        found.insert(id.clone());
        let (total, unread) = match &cache {
            Some(cache) => {
                let mut total = 0usize;
                let mut unread = 0usize;
                for entry in cache.in_mailbox(mailbox.id) {
                    total += 1;
                    if !entry.has_keyword(&Keyword::Seen) {
                        unread += 1;
                    }
                }
                (total, unread)
            }
            None => (0, 0),
        };
        list.push(json!({
            "id": id,
            "name": mailbox.name,
            "role": role_name(mailbox.role),
            "parentId": Value::Null,
            "sortOrder": order,
            "totalEmails": total,
            "unreadEmails": unread,
        }));
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
        "Mailbox/get",
        json!({
            "accountId": account_id(args),
            "state": collection_state(ctx.store.as_ref(), ctx.account_id as u32, Collection::Mailbox),
            "list": list,
            "notFound": not_found,
        }),
        call_id,
    )
}

fn role_name(role: SpecialUse) -> Value {
    match role {
        SpecialUse::Inbox => json!("inbox"),
        SpecialUse::Sent => json!("sent"),
        SpecialUse::Drafts => json!("drafts"),
        SpecialUse::Trash => json!("trash"),
        SpecialUse::Junk => json!("junk"),
        SpecialUse::Archive => json!("archive"),
        SpecialUse::None => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_context;

    #[test]
    fn persisted_mailbox_rows_take_precedence_over_the_defaults() {
        use irixmail_mail::{mailbox_ops, Mailbox};

        let ctx = test_context();
        let mut mailboxes = provision_mailboxes(1_700_000_000_000);
        mailboxes.push(Mailbox::new(
            6,
            "Archive",
            SpecialUse::Archive,
            1_700_000_000,
        ));
        ctx.store
            .batch(&mailbox_ops(ctx.account_id as u32, &mailboxes))
            .unwrap();

        let response = mailbox_get(&ctx, &json!({"accountId": "1", "ids": null}), "c0");
        let list = response.arguments()["list"].as_array().unwrap();
        assert_eq!(list.len(), 6);
        assert!(list
            .iter()
            .any(|m| m["name"] == "Archive" && m["role"] == "archive"));
    }

    #[test]
    fn the_standard_mailboxes_are_listed() {
        let ctx = test_context();
        let response = mailbox_get(&ctx, &json!({"accountId": "1", "ids": null}), "c0");
        let list = response.arguments()["list"].as_array().unwrap();
        assert_eq!(list.len(), 5);
        assert_eq!(list[0]["name"], "Inbox");
        assert_eq!(list[0]["role"], "inbox");
        assert_eq!(list[0]["totalEmails"], 0);
        assert_eq!(list[0]["unreadEmails"], 0);
    }

    #[test]
    fn a_requested_id_filters_the_list() {
        let ctx = test_context();
        let response = mailbox_get(&ctx, &json!({"accountId": "1", "ids": ["2"]}), "c0");
        let list = response.arguments()["list"].as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["id"], "2");
        assert_eq!(list[0]["role"], "sent");
    }

    #[test]
    fn an_unknown_id_is_reported_not_found() {
        let ctx = test_context();
        let response = mailbox_get(&ctx, &json!({"accountId": "1", "ids": ["99"]}), "c0");
        assert_eq!(response.arguments()["notFound"], json!(["99"]));
        assert!(response.arguments()["list"].as_array().unwrap().is_empty());
    }
}
