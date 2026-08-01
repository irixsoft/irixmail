use serde_json::{json, Map, Value};

use irixmail_mail::{
    assign_uid_validity, create_mailbox, delete_mailbox, load_mailboxes, provision_mailboxes,
    rename_mailbox, MailboxDelete, FIRST_USER_MAILBOX_ID,
};

use irixmail_store::Collection;

use crate::context::JmapContext;
use crate::reply::{account_id, collection_state};
use crate::request::{method_error, Invocation};

pub fn mailbox_set(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let account = ctx.account_id as u32;
    let old_state = collection_state(ctx.store.as_ref(), account, Collection::Mailbox);
    if let Some(expected) = args.get("ifInState").and_then(Value::as_str) {
        if expected != old_state {
            return method_error("stateMismatch", call_id);
        }
    }

    let mut created = Map::new();
    let mut not_created = Map::new();
    let mut updated = Map::new();
    let mut not_updated = Map::new();
    let mut destroyed: Vec<Value> = Vec::new();
    let mut not_destroyed = Map::new();

    if let Some(create) = args.get("create").and_then(Value::as_object) {
        for (creation_id, object) in create {
            match create_one(ctx, account, object) {
                Ok(id) => {
                    created.insert(creation_id.clone(), json!({ "id": id.to_string() }));
                }
                Err(err) => {
                    not_created.insert(creation_id.clone(), err);
                }
            }
        }
    }

    if let Some(update) = args.get("update").and_then(Value::as_object) {
        for (id, patch) in update {
            match update_one(ctx, account, id, patch) {
                Ok(()) => {
                    updated.insert(id.clone(), Value::Null);
                }
                Err(err) => {
                    not_updated.insert(id.clone(), err);
                }
            }
        }
    }

    if let Some(destroy) = args.get("destroy").and_then(Value::as_array) {
        let remove_emails = args
            .get("onDestroyRemoveEmails")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        for id in destroy.iter().filter_map(Value::as_str) {
            match destroy_one(ctx, account, id, remove_emails) {
                Ok(()) => destroyed.push(Value::String(id.to_string())),
                Err(err) => {
                    not_destroyed.insert(id.to_string(), err);
                }
            }
        }
    }

    Invocation::new(
        "Mailbox/set",
        json!({
            "accountId": account_id(args),
            "oldState": old_state,
            "newState": collection_state(ctx.store.as_ref(), account, Collection::Mailbox),
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

fn create_one(ctx: &JmapContext, account: u32, object: &Value) -> Result<u32, Value> {
    let name = folder_name(object.get("name"))?;
    if name_taken(ctx, account, &name, None) {
        return Err(invalid_properties("name"));
    }
    let uid_validity = assign_uid_validity(created_at(ctx));
    create_mailbox(
        ctx.store.as_ref(),
        ctx.notifier.as_ref(),
        account,
        &name,
        uid_validity,
    )
    .map(|mailbox| mailbox.id)
    .map_err(|_| set_error("serverFail"))
}

fn update_one(ctx: &JmapContext, account: u32, id: &str, patch: &Value) -> Result<(), Value> {
    let mailbox_id = id.parse::<u32>().map_err(|_| set_error("notFound"))?;
    if mailbox_id < FIRST_USER_MAILBOX_ID {
        return Err(set_error("forbidden"));
    }
    let name = folder_name(patch.get("name"))?;
    if name_taken(ctx, account, &name, Some(mailbox_id)) {
        return Err(invalid_properties("name"));
    }
    match rename_mailbox(
        ctx.store.as_ref(),
        ctx.notifier.as_ref(),
        account,
        mailbox_id,
        &name,
    ) {
        Ok(true) => Ok(()),
        Ok(false) => Err(set_error("notFound")),
        Err(_) => Err(set_error("serverFail")),
    }
}

fn destroy_one(
    ctx: &JmapContext,
    account: u32,
    id: &str,
    remove_emails: bool,
) -> Result<(), Value> {
    let mailbox_id = id.parse::<u32>().map_err(|_| set_error("notFound"))?;
    if mailbox_id < FIRST_USER_MAILBOX_ID {
        return Err(set_error("forbidden"));
    }
    match delete_mailbox(
        ctx.store.as_ref(),
        ctx.blobs.as_ref(),
        ctx.notifier.as_ref(),
        account,
        mailbox_id,
        remove_emails,
    ) {
        Ok(MailboxDelete::Deleted) => Ok(()),
        Ok(MailboxDelete::HasMail) => Err(set_error("mailboxHasEmail")),
        Err(_) => Err(set_error("serverFail")),
    }
}

fn folder_name(value: Option<&Value>) -> Result<String, Value> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .filter(|name| name.len() <= crate::session::MAX_SIZE_MAILBOX_NAME)
        .filter(|name| name.split('/').count() <= crate::session::MAX_MAILBOX_DEPTH)
        .map(str::to_string)
        .ok_or_else(|| invalid_properties("name"))
}

// Reject a name already used by a default or persisted mailbox (excluding the one being renamed).
fn name_taken(ctx: &JmapContext, account: u32, name: &str, exclude: Option<u32>) -> bool {
    let mut mailboxes = provision_mailboxes(created_at(ctx));
    if let Ok(persisted) = load_mailboxes(ctx.store.as_ref(), account) {
        mailboxes.extend(
            persisted
                .into_iter()
                .filter(|m| m.id >= FIRST_USER_MAILBOX_ID),
        );
    }
    mailboxes
        .into_iter()
        .filter(|m| Some(m.id) != exclude)
        .any(|m| m.name.eq_ignore_ascii_case(name))
}

fn created_at(ctx: &JmapContext) -> u64 {
    ctx.directory
        .accounts()
        .get(ctx.account_id)
        .map(|account| account.created_at)
        .unwrap_or(0)
}

fn set_error(kind: &str) -> Value {
    json!({ "type": kind })
}

fn invalid_properties(property: &str) -> Value {
    json!({ "type": "invalidProperties", "properties": [property] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_context;
    use irixmail_mail::MessageData;
    use irixmail_store::{serialize, Collection, Key, Subspace};

    fn create(ctx: &JmapContext, name: &str) -> String {
        let response = mailbox_set(
            ctx,
            &json!({"accountId": "1", "create": {"a": {"name": name}}}),
            "c0",
        );
        response.arguments()["created"]["a"]["id"]
            .as_str()
            .expect("created id")
            .to_string()
    }

    #[test]
    fn create_persists_a_user_mailbox_and_returns_its_id() {
        let ctx = test_context();
        let id = create(&ctx, "Projects").parse::<u32>().unwrap();
        let rows = load_mailboxes(ctx.store.as_ref(), 1).unwrap();
        assert!(rows.iter().any(|m| m.id == id && m.name == "Projects"));
    }

    #[test]
    fn set_states_are_the_mailbox_changelog_head() {
        let ctx = test_context();
        let response = mailbox_set(
            &ctx,
            &json!({"accountId": "1", "create": {"a": {"name": "Projects"}}}),
            "c0",
        );
        let id = response.arguments()["created"]["a"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let old_state = response.arguments()["oldState"].as_str().unwrap();
        let new_state = response.arguments()["newState"]
            .as_str()
            .unwrap()
            .to_string();
        assert_ne!(old_state, new_state);

        let delta = crate::mailbox_changes(
            &ctx,
            &json!({"accountId": "1", "sinceState": old_state}),
            "c1",
        );
        let created = delta.arguments()["created"].as_array().unwrap().clone();
        assert!(created.contains(&json!(id)), "created: {created:?}");

        let get = crate::mailbox_get(&ctx, &json!({"accountId": "1", "ids": null}), "c2");
        assert_eq!(get.arguments()["state"], json!(new_state));

        let query = crate::mailbox_query(&ctx, &json!({"accountId": "1"}), "c3");
        assert_eq!(query.arguments()["queryState"], json!(new_state));
    }

    #[test]
    fn a_stale_if_in_state_is_rejected_with_state_mismatch() {
        let ctx = test_context();
        let response = mailbox_set(
            &ctx,
            &json!({
                "accountId": "1",
                "ifInState": "999999",
                "create": {"a": {"name": "Never"}},
            }),
            "c0",
        );
        assert_eq!(response.name(), "error");
        assert_eq!(response.arguments()["type"], "stateMismatch");
        let rows = load_mailboxes(ctx.store.as_ref(), 1).unwrap();
        assert!(!rows.iter().any(|m| m.name == "Never"));
    }

    #[test]
    fn a_set_over_the_object_limit_is_request_too_large() {
        use crate::request::{Invocation, Request, Router};

        let ctx = test_context();
        let mut router = Router::new();
        router.register_stateful("Mailbox/set", mailbox_set);
        let destroy: Vec<String> = (0..=500).map(|id| id.to_string()).collect();
        let request = Request {
            using: Vec::new(),
            method_calls: vec![Invocation::new(
                "Mailbox/set",
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
    fn a_name_deeper_than_the_advertised_mailbox_depth_is_rejected() {
        let ctx = test_context();
        let name = ["a"; 11].join("/");
        let response = mailbox_set(
            &ctx,
            &json!({"accountId": "1", "create": {"deep": {"name": name}}}),
            "c0",
        );
        assert_eq!(
            response.arguments()["notCreated"]["deep"]["type"],
            "invalidProperties"
        );
    }

    #[test]
    fn a_name_longer_than_the_advertised_limit_is_rejected() {
        let ctx = test_context();
        let name = "x".repeat(256);
        let response = mailbox_set(
            &ctx,
            &json!({"accountId": "1", "create": {"long": {"name": name}}}),
            "c0",
        );
        assert_eq!(
            response.arguments()["notCreated"]["long"]["type"],
            "invalidProperties"
        );
    }

    #[test]
    fn creating_a_duplicate_name_is_rejected() {
        let ctx = test_context();
        create(&ctx, "Dup");
        let response = mailbox_set(
            &ctx,
            &json!({"accountId": "1", "create": {"b": {"name": "Dup"}}}),
            "c1",
        );
        assert_eq!(
            response.arguments()["notCreated"]["b"]["type"],
            "invalidProperties"
        );
    }

    #[test]
    fn creating_a_system_folder_name_is_rejected() {
        let ctx = test_context();
        let response = mailbox_set(
            &ctx,
            &json!({"accountId": "1", "create": {"b": {"name": "Inbox"}}}),
            "c1",
        );
        assert_eq!(
            response.arguments()["notCreated"]["b"]["type"],
            "invalidProperties"
        );
    }

    #[test]
    fn renaming_a_user_mailbox_updates_the_name() {
        let ctx = test_context();
        let id = create(&ctx, "Old");
        let mut update = Map::new();
        update.insert(id.clone(), json!({ "name": "New" }));
        let response = mailbox_set(
            &ctx,
            &json!({"accountId": "1", "update": Value::Object(update)}),
            "c1",
        );
        assert!(response.arguments()["updated"].get(&id).is_some());
        let rows = load_mailboxes(ctx.store.as_ref(), 1).unwrap();
        assert!(rows.iter().any(|m| m.name == "New"));
        assert!(!rows.iter().any(|m| m.name == "Old"));
    }

    #[test]
    fn destroying_a_non_empty_mailbox_requires_on_destroy_remove_emails() {
        let ctx = test_context();
        let id = create(&ctx, "Keep");
        let mailbox_id: u32 = id.parse().unwrap();
        let mut data = MessageData::new(7, 100);
        data.add_mailbox(mailbox_id, 1);
        let key = Key::new(Subspace::Property, 1, Collection::Email, 7).encode();
        ctx.store
            .put(&key, &serialize::archive(&data).unwrap())
            .unwrap();

        let refused = mailbox_set(
            &ctx,
            &json!({"accountId": "1", "destroy": [id.clone()]}),
            "c1",
        );
        assert_eq!(
            refused.arguments()["notDestroyed"][&id]["type"],
            "mailboxHasEmail"
        );
        let rows = load_mailboxes(ctx.store.as_ref(), 1).unwrap();
        assert!(rows.iter().any(|m| m.name == "Keep"));
        assert!(ctx.store.get(&key).unwrap().is_some());

        let allowed = mailbox_set(
            &ctx,
            &json!({"accountId": "1", "destroy": [id.clone()], "onDestroyRemoveEmails": true}),
            "c2",
        );
        let destroyed = allowed.arguments()["destroyed"].as_array().unwrap();
        assert!(destroyed
            .iter()
            .any(|value| value == &Value::String(id.clone())));
        assert!(ctx.store.get(&key).unwrap().is_none());
    }

    #[test]
    fn destroying_a_user_mailbox_removes_it_and_system_folders_are_forbidden() {
        let ctx = test_context();
        let id = create(&ctx, "Temp");
        let response = mailbox_set(
            &ctx,
            &json!({"accountId": "1", "destroy": [id.clone(), "1"]}),
            "c1",
        );
        let destroyed = response.arguments()["destroyed"].as_array().unwrap();
        assert!(destroyed
            .iter()
            .any(|value| value == &Value::String(id.clone())));
        assert_eq!(
            response.arguments()["notDestroyed"]["1"]["type"],
            "forbidden"
        );
        let rows = load_mailboxes(ctx.store.as_ref(), 1).unwrap();
        assert!(!rows.iter().any(|m| m.name == "Temp"));
    }
}
