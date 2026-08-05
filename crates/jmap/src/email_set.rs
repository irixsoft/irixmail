use serde_json::{json, Map, Value};

use irixmail_core::Error;
use irixmail_core::Result;
use irixmail_mail::{
    allocate_document_id, build_message, delete_message, load_mailboxes, provision_mailboxes,
    update_message, Attachment, Compose, ComposeMailbox, DeliveryRequest, DeliveryTarget, Keyword,
    Mailbox, MessageData, SpecialUse,
};
use irixmail_store::{Collection, Store};

use crate::context::JmapContext;
use crate::reply::{account_id, collection_state};
use crate::request::{method_error, Invocation};

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

pub fn email_set(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let account = ctx.account_id as u32;
    let old_state = collection_state(ctx.store.as_ref(), account, Collection::Email);
    if let Some(expected) = args.get("ifInState").and_then(Value::as_str) {
        if expected != old_state {
            return method_error("stateMismatch", call_id);
        }
    }
    let mut created = Map::new();
    let mut updated = Map::new();
    let mut not_updated = Map::new();
    let mut destroyed: Vec<Value> = Vec::new();
    let mut not_destroyed = Map::new();
    let mut not_created = Map::new();

    if let Some(create) = args.get("create").and_then(Value::as_object) {
        for (creation_id, object) in create {
            match create_email(ctx, account, object) {
                Ok(document_id) => {
                    created.insert(
                        creation_id.clone(),
                        json!({ "id": document_id.to_string() }),
                    );
                }
                Err(err) => {
                    let kind = match &err {
                        Error::InvalidInput(detail) if detail == ATTACHMENTS_TOO_LARGE => {
                            "tooLarge"
                        }
                        _ => "serverFail",
                    };
                    not_created.insert(creation_id.clone(), set_error(kind));
                }
            }
        }
    }

    if let Some(update) = args.get("update").and_then(Value::as_object) {
        for (id, patch) in update {
            match id.parse::<u32>() {
                Ok(document_id) => match apply_patch(ctx, account, document_id, patch) {
                    Ok(true) => {
                        updated.insert(id.clone(), Value::Null);
                    }
                    Ok(false) => {
                        not_updated.insert(id.clone(), set_error("notFound"));
                    }
                    Err(_) => {
                        not_updated.insert(id.clone(), set_error("serverFail"));
                    }
                },
                Err(_) => {
                    not_updated.insert(id.clone(), set_error("notFound"));
                }
            }
        }
    }

    if let Some(destroy) = args.get("destroy").and_then(Value::as_array) {
        for id in destroy.iter().filter_map(Value::as_str) {
            let Ok(document_id) = id.parse::<u32>() else {
                not_destroyed.insert(id.to_string(), set_error("notFound"));
                continue;
            };
            match delete_message(
                ctx.store.as_ref(),
                ctx.blobs.as_ref(),
                &ctx.notifier,
                account,
                document_id,
            ) {
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
        "Email/set",
        json!({
            "accountId": account_id(args),
            "oldState": old_state,
            "newState": collection_state(ctx.store.as_ref(), account, Collection::Email),
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

pub(crate) fn apply_patch(
    ctx: &JmapContext,
    account: u32,
    document_id: u32,
    patch: &Value,
) -> Result<bool> {
    let store = ctx.store.clone();
    let inner = ctx.store.clone();
    let patch = patch.clone();
    update_message(
        store.as_ref(),
        &ctx.notifier,
        account,
        document_id,
        move |data| {
            let store = inner;
            let Some(fields) = patch.as_object() else {
                return Ok(());
            };
            for (key, value) in fields {
                if key == "keywords" {
                    data.keywords.clear();
                    if let Some(keywords) = value.as_object() {
                        for (name, on) in keywords {
                            if on.as_bool() == Some(true) {
                                data.add_keyword(Keyword::from_jmap(name));
                            }
                        }
                    }
                } else if let Some(name) = key.strip_prefix("keywords/") {
                    let keyword = Keyword::from_jmap(name);
                    if value.as_bool() == Some(true) {
                        data.add_keyword(keyword);
                    } else {
                        data.remove_keyword(&keyword);
                    }
                } else if key == "mailboxIds" {
                    if let Some(ids) = value.as_object() {
                        let wanted: Vec<u32> = ids
                            .iter()
                            .filter(|(_, on)| on.as_bool() == Some(true))
                            .filter_map(|(id, _)| id.parse::<u32>().ok())
                            .collect();
                        set_mailboxes(data, &wanted, store.as_ref(), account)?;
                    }
                } else if let Some(id) = key.strip_prefix("mailboxIds/") {
                    if let Ok(mailbox_id) = id.parse::<u32>() {
                        if value.as_bool() == Some(true) {
                            if !data.in_mailbox(mailbox_id) {
                                let uid = next_uid(mailbox_id, store.as_ref(), account)?;
                                data.add_mailbox(mailbox_id, uid);
                            }
                        } else {
                            data.remove_mailbox(mailbox_id);
                        }
                    }
                }
            }
            Ok(())
        },
    )
}

fn set_mailboxes(
    data: &mut MessageData,
    wanted: &[u32],
    store: &dyn Store,
    account: u32,
) -> Result<()> {
    let current: Vec<u32> = data.mailboxes.iter().map(|m| m.mailbox_id).collect();
    for mailbox_id in &current {
        if !wanted.contains(mailbox_id) {
            data.remove_mailbox(*mailbox_id);
        }
    }
    for mailbox_id in wanted {
        if !data.in_mailbox(*mailbox_id) {
            let uid = next_uid(*mailbox_id, store, account)?;
            data.add_mailbox(*mailbox_id, uid);
        }
    }
    Ok(())
}

fn next_uid(mailbox_id: u32, store: &dyn Store, account: u32) -> Result<u32> {
    Mailbox::new(mailbox_id, "", SpecialUse::None, 0).next_uid(store, account)
}

fn account_mailboxes(ctx: &JmapContext, account: u32) -> Vec<Mailbox> {
    match load_mailboxes(ctx.store.as_ref(), account) {
        Ok(rows) if !rows.is_empty() => rows,
        _ => provision_mailboxes(0),
    }
}

fn delivery_target(object: &Value, mailboxes: &[Mailbox]) -> Result<DeliveryTarget> {
    let Some(ids) = object.get("mailboxIds").and_then(Value::as_object) else {
        return Ok(DeliveryTarget::Role(SpecialUse::Drafts));
    };
    ids.iter()
        .filter(|(_, on)| on.as_bool() == Some(true))
        .filter_map(|(id, _)| id.parse::<u32>().ok())
        .find(|id| mailboxes.iter().any(|mailbox| mailbox.id == *id))
        .map(DeliveryTarget::Mailbox)
        .ok_or_else(|| Error::invalid_input("mailboxIds names no existing mailbox"))
}

fn create_email(ctx: &JmapContext, account: u32, object: &Value) -> Result<u32> {
    let record = ctx.directory.accounts().get(ctx.account_id)?;
    let domain = ctx
        .directory
        .domains()
        .get(record.domain_id)
        .map(|domain| domain.name)
        .unwrap_or_default();
    let self_email = format!("{}@{}", record.local_part, domain);

    let mut from = mailboxes_from(object.get("from"));
    if from.is_empty() {
        from.push(ComposeMailbox {
            name: record.display_name.clone(),
            email: self_email.clone(),
        });
    }
    let (text_body, html_body) = bodies(object);
    let compose = Compose {
        from: from.into_iter().next(),
        to: mailboxes_from(object.get("to")),
        cc: mailboxes_from(object.get("cc")),
        bcc: mailboxes_from(object.get("bcc")),
        subject: object
            .get("subject")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        text_body,
        html_body,
        attachments: attachments_from(ctx, object)?,
    };
    let raw = build_message(&compose)?;

    let document_id = allocate_document_id(ctx.store.as_ref(), account)?;
    let mailboxes = account_mailboxes(ctx, account);
    let target = delivery_target(object, &mailboxes)?;
    let request = DeliveryRequest {
        account: &record,
        mailboxes: &mailboxes,
        sieve: None,
        mail_from: &self_email,
        recipient: &self_email,
        document_id,
        raw: &raw,
        target_override: Some(target),
        received_at: now_seconds(),
    };
    ctx.mail.deliver(&request)?;

    let object = object.clone();
    update_message(
        ctx.store.as_ref(),
        &ctx.notifier,
        account,
        document_id,
        move |data| {
            if let Some(keywords) = object.get("keywords").and_then(Value::as_object) {
                for (name, on) in keywords {
                    if on.as_bool() == Some(true) {
                        data.add_keyword(Keyword::from_jmap(name));
                    }
                }
            }
            Ok(())
        },
    )?;

    Ok(document_id)
}

fn mailboxes_from(value: Option<&Value>) -> Vec<ComposeMailbox> {
    value
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|entry| {
                    let email = entry.get("email").and_then(Value::as_str)?;
                    Some(ComposeMailbox {
                        name: entry
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        email: email.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn body_part(object: &Value, key: &str) -> Option<String> {
    let values = object.get("bodyValues").and_then(Value::as_object)?;
    let part_id = object
        .get(key)
        .and_then(Value::as_array)
        .and_then(|parts| parts.first())
        .and_then(|part| part.get("partId"))
        .and_then(Value::as_str)?;
    values
        .get(part_id)
        .and_then(|body| body.get("value"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn bodies(object: &Value) -> (String, Option<String>) {
    let html = body_part(object, "htmlBody");
    if let Some(text) = body_part(object, "textBody") {
        return (text, html);
    }
    if let Some(html) = html {
        let text = irixmail_mail::text_from_html(&html);
        return (text, Some(html));
    }
    let text = object
        .get("bodyValues")
        .and_then(Value::as_object)
        .and_then(|values| values.values().next())
        .and_then(|body| body.get("value"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    (text, None)
}

const ATTACHMENTS_TOO_LARGE: &str = "attachments exceed maxSizeAttachmentsPerEmail";

fn attachments_from(ctx: &JmapContext, object: &Value) -> Result<Vec<Attachment>> {
    let mut attachments = Vec::new();
    let mut total = 0usize;
    if let Some(list) = object.get("attachments").and_then(Value::as_array) {
        for entry in list {
            let Some(blob_id) = entry.get("blobId").and_then(Value::as_str) else {
                continue;
            };
            if let Some(data) = crate::fetch_blob(ctx.blobs.as_ref(), blob_id)? {
                total = total.saturating_add(data.len());
                if total > crate::session::MAX_SIZE_ATTACHMENTS {
                    return Err(Error::invalid_input(ATTACHMENTS_TOO_LARGE));
                }
                attachments.push(Attachment {
                    content_type: entry
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("application/octet-stream")
                        .to_string(),
                    name: entry
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("attachment")
                        .to_string(),
                    data,
                });
            }
        }
    }
    Ok(attachments)
}

fn set_error(kind: &str) -> Value {
    json!({ "type": kind })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{test_context, test_context_with_account};
    use irixmail_mail::{
        allocate_document_id, append_message, load_data, provision_mailboxes, AppendRequest,
        INBOX_ID,
    };
    use irixmail_store::{Collection, FtsIndex, Query};

    fn seed_email(ctx: &JmapContext, raw: &[u8]) -> u32 {
        let account = ctx.account_id as u32;
        let record = ctx.directory.accounts().get(ctx.account_id).unwrap();
        let mailboxes = provision_mailboxes(record.created_at);
        let inbox = mailboxes.iter().find(|m| m.id == INBOX_ID).unwrap();
        let document_id = allocate_document_id(ctx.store.as_ref(), account).unwrap();
        append_message(
            ctx.store.as_ref(),
            ctx.blobs.as_ref(),
            ctx.notifier.as_ref(),
            &AppendRequest {
                account: &record,
                mailbox: inbox,
                flags: vec![],
                received_at: 0,
                document_id,
                raw,
            },
        )
        .unwrap();
        document_id
    }

    #[test]
    fn a_set_over_the_object_limit_is_request_too_large() {
        use crate::request::{Request, Router};

        let ctx = test_context();
        let mut router = Router::new();
        router.register_stateful("Email/set", email_set);
        let destroy: Vec<String> = (0..=500).map(|id| id.to_string()).collect();
        let request = Request {
            using: Vec::new(),
            method_calls: vec![crate::request::Invocation::new(
                "Email/set",
                json!({"accountId": "1", "destroy": destroy}),
                "c0",
            )],
        };
        let response = router.process(&ctx, &request, "s");
        assert_eq!(response.method_responses[0].name(), "error");
        assert_eq!(
            response.method_responses[0].arguments()["type"],
            "requestTooLarge"
        );
    }

    #[test]
    fn attachments_over_the_advertised_size_limit_are_rejected() {
        let ctx = test_context_with_account();
        let blob_id = crate::store_upload(
            ctx.store.as_ref(),
            ctx.blobs.as_ref(),
            1,
            &vec![0u8; 50_000_001],
            0,
        )
        .unwrap();
        let response = email_set(
            &ctx,
            &json!({
                "accountId": ctx.account_id.to_string(),
                "create": {"big": {
                    "subject": "Too heavy",
                    "attachments": [{"blobId": blob_id, "type": "application/octet-stream", "name": "big.bin"}],
                }},
            }),
            "c0",
        );
        let args = response.arguments();
        assert!(args["created"].as_object().unwrap().is_empty());
        assert_eq!(args["notCreated"]["big"]["type"], "tooLarge");
    }

    #[test]
    fn create_files_into_the_requested_user_mailbox() {
        use irixmail_mail::{create_mailbox, mailbox_ops, DRAFTS_ID};

        let ctx = test_context_with_account();
        let account = ctx.account_id as u32;
        let record = ctx.directory.accounts().get(ctx.account_id).unwrap();
        ctx.store
            .batch(&mailbox_ops(
                account,
                &provision_mailboxes(record.created_at),
            ))
            .unwrap();
        let project = create_mailbox(
            ctx.store.as_ref(),
            ctx.notifier.as_ref(),
            account,
            "Project X",
            1,
        )
        .unwrap();

        let mut mailbox_ids = Map::new();
        mailbox_ids.insert(project.id.to_string(), json!(true));
        let response = email_set(
            &ctx,
            &json!({
                "accountId": ctx.account_id.to_string(),
                "create": {"draft1": {
                    "mailboxIds": Value::Object(mailbox_ids),
                    "subject": "Filed right",
                    "bodyValues": {"text": {"value": "hello"}},
                    "textBody": [{"partId": "text"}],
                }},
            }),
            "c0",
        );

        let created = response.arguments()["created"]["draft1"]["id"]
            .as_str()
            .expect("created id")
            .parse::<u32>()
            .unwrap();
        let data = load_data(ctx.store.as_ref(), account, created)
            .unwrap()
            .unwrap();
        assert!(data.in_mailbox(project.id));
        assert!(!data.in_mailbox(DRAFTS_ID));
    }

    fn created_raw(ctx: &JmapContext, create: Value) -> String {
        let response = email_set(
            ctx,
            &json!({
                "accountId": ctx.account_id.to_string(),
                "create": {"draft1": create},
            }),
            "c0",
        );
        let created = response.arguments()["created"]["draft1"]["id"]
            .as_str()
            .expect("created id")
            .parse::<u32>()
            .unwrap();
        let raw = irixmail_mail::load_raw(
            ctx.store.as_ref(),
            ctx.blobs.as_ref(),
            ctx.account_id as u32,
            created,
        )
        .unwrap()
        .expect("stored raw message");
        String::from_utf8_lossy(&raw).into_owned()
    }

    #[test]
    fn an_html_body_is_carried_into_the_built_message() {
        let ctx = test_context_with_account();
        let raw = created_raw(
            &ctx,
            json!({
                "subject": "Rich",
                "bodyValues": {"text": {"value": "plain words"}, "html": {"value": "<p>rich words</p>"}},
                "textBody": [{"partId": "text"}],
                "htmlBody": [{"partId": "html"}],
            }),
        );
        assert!(raw.contains("text/html"));
        assert!(raw.contains("rich words"));
        assert!(raw.contains("plain words"));
    }

    #[test]
    fn a_text_only_create_still_builds_a_plain_message() {
        let ctx = test_context_with_account();
        let raw = created_raw(
            &ctx,
            json!({
                "subject": "Plain",
                "bodyValues": {"text": {"value": "just text"}},
                "textBody": [{"partId": "text"}],
            }),
        );
        assert!(!raw.contains("text/html"));
        assert!(raw.contains("just text"));
    }

    #[test]
    fn an_html_only_create_derives_a_plain_text_alternative() {
        let ctx = test_context_with_account();
        let raw = created_raw(
            &ctx,
            json!({
                "subject": "Rich only",
                "bodyValues": {"html": {"value": "<p>only rich</p>"}},
                "htmlBody": [{"partId": "html"}],
            }),
        );
        assert!(raw.contains("text/html"));
        assert!(raw.contains("text/plain"));
        assert!(raw.contains("only rich"));
    }

    #[test]
    fn a_body_value_without_part_lists_still_becomes_the_text_body() {
        let ctx = test_context_with_account();
        let raw = created_raw(
            &ctx,
            json!({
                "subject": "Loose",
                "bodyValues": {"whatever": {"value": "loose body"}},
            }),
        );
        assert!(raw.contains("loose body"));
        assert!(!raw.contains("text/html"));
    }

    #[test]
    fn moves_into_a_user_mailbox_allocate_distinct_uids() {
        use irixmail_mail::create_mailbox;

        let ctx = test_context_with_account();
        let account = ctx.account_id as u32;
        let first = seed_email(
            &ctx,
            b"Subject: One\r\nFrom: a@example.net\r\n\r\nfirst\r\n",
        );
        let second = seed_email(
            &ctx,
            b"Subject: Two\r\nFrom: a@example.net\r\n\r\nsecond\r\n",
        );
        let project = create_mailbox(
            ctx.store.as_ref(),
            ctx.notifier.as_ref(),
            account,
            "Project X",
            1,
        )
        .unwrap();

        for doc in [first, second] {
            let response = email_set(
                &ctx,
                &json!({
                    "accountId": ctx.account_id.to_string(),
                    "update": {(doc.to_string()): {(format!("mailboxIds/{}", project.id)): true}},
                }),
                "c0",
            );
            assert!(response.arguments()["notUpdated"]
                .as_object()
                .unwrap()
                .is_empty());
        }

        let uid_first = load_data(ctx.store.as_ref(), account, first)
            .unwrap()
            .unwrap()
            .uid_in(project.id)
            .unwrap();
        let uid_second = load_data(ctx.store.as_ref(), account, second)
            .unwrap()
            .unwrap()
            .uid_in(project.id)
            .unwrap();
        assert_ne!(uid_first, uid_second);
    }

    #[test]
    fn destroy_deletes_the_message_and_unindexes_it() {
        let ctx = test_context_with_account();
        let account = ctx.account_id as u32;
        let doc = seed_email(
            &ctx,
            b"Subject: doomed zanzibar\r\nFrom: a@example.net\r\n\r\nthe zanzibar body\r\n",
        );
        assert!(load_data(ctx.store.as_ref(), account, doc)
            .unwrap()
            .is_some());

        let response = email_set(
            &ctx,
            &json!({"accountId": ctx.account_id.to_string(), "destroy": [doc.to_string()]}),
            "c0",
        );

        assert_eq!(response.arguments()["destroyed"], json!([doc.to_string()]));
        assert!(response.arguments()["notDestroyed"]
            .as_object()
            .unwrap()
            .is_empty());
        assert!(load_data(ctx.store.as_ref(), account, doc)
            .unwrap()
            .is_none());
        let hits = FtsIndex::new(ctx.store.as_ref())
            .search(account, Collection::Email, &Query::term("zanzibar"), &[doc])
            .unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn set_states_advance_with_the_email_changelog() {
        let ctx = test_context_with_account();
        let doc = seed_email(
            &ctx,
            b"Subject: gone\r\nFrom: a@example.net\r\n\r\nbody\r\n",
        );

        let response = email_set(
            &ctx,
            &json!({"accountId": ctx.account_id.to_string(), "destroy": [doc.to_string()]}),
            "c0",
        );

        let old_state = response.arguments()["oldState"].as_str().unwrap();
        let new_state = response.arguments()["newState"].as_str().unwrap();
        assert_ne!(old_state, new_state);

        let delta = crate::email_changes(
            &ctx,
            &json!({"accountId": ctx.account_id.to_string(), "sinceState": old_state}),
            "c1",
        );
        let destroyed = delta.arguments()["destroyed"].as_array().unwrap().clone();
        assert!(
            destroyed.contains(&json!(doc.to_string())),
            "destroyed: {destroyed:?}"
        );
    }

    #[test]
    fn a_stale_if_in_state_is_rejected_with_state_mismatch() {
        let ctx = test_context_with_account();
        let account = ctx.account_id as u32;
        let doc = seed_email(
            &ctx,
            b"Subject: keep\r\nFrom: a@example.net\r\n\r\nbody\r\n",
        );

        let stale = email_set(
            &ctx,
            &json!({
                "accountId": ctx.account_id.to_string(),
                "ifInState": "999999",
                "destroy": [doc.to_string()],
            }),
            "c0",
        );
        assert_eq!(stale.name(), "error");
        assert_eq!(stale.arguments()["type"], "stateMismatch");
        assert!(load_data(ctx.store.as_ref(), account, doc)
            .unwrap()
            .is_some());

        let current = email_set(
            &ctx,
            &json!({"accountId": ctx.account_id.to_string()}),
            "c1",
        )
        .arguments()["newState"]
            .as_str()
            .unwrap()
            .to_string();
        let fresh = email_set(
            &ctx,
            &json!({
                "accountId": ctx.account_id.to_string(),
                "ifInState": current,
                "destroy": [doc.to_string()],
            }),
            "c2",
        );
        assert_eq!(fresh.arguments()["destroyed"], json!([doc.to_string()]));
    }

    #[test]
    fn destroying_an_absent_or_malformed_id_reports_not_found() {
        let ctx = test_context_with_account();
        let response = email_set(
            &ctx,
            &json!({"accountId": ctx.account_id.to_string(), "destroy": ["424242", "abc"]}),
            "c0",
        );
        assert_eq!(response.arguments()["destroyed"], json!([]));
        assert_eq!(
            response.arguments()["notDestroyed"]["424242"]["type"],
            "notFound"
        );
        assert_eq!(
            response.arguments()["notDestroyed"]["abc"]["type"],
            "notFound"
        );
    }

    #[test]
    fn updating_an_absent_message_reports_not_found() {
        let ctx = test_context();
        let response = email_set(
            &ctx,
            &json!({"accountId": "1", "update": {"7": {"keywords/$seen": true}}}),
            "c0",
        );
        assert!(response.arguments()["notUpdated"]["7"].is_object());
        assert_eq!(response.arguments()["notUpdated"]["7"]["type"], "notFound");
    }

    #[test]
    fn the_set_shape_is_present() {
        let ctx = test_context();
        let response = email_set(&ctx, &json!({"accountId": "1"}), "c0");
        assert!(response.arguments()["updated"].is_object());
        assert!(response.arguments()["destroyed"].is_array());
    }
}
