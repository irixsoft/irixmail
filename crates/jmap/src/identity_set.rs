use serde_json::{json, Map, Value};

use irixmail_core::Result;

use crate::context::JmapContext;
use crate::reply::{account_id, STATE};
use crate::request::Invocation;

pub fn identity_set(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let mut created = Map::new();
    let mut not_created = Map::new();
    let mut updated = Map::new();
    let mut not_updated = Map::new();

    if let Some(create) = args.get("create").and_then(Value::as_object) {
        for (creation_id, patch) in create {
            match apply(ctx, patch) {
                Ok(()) => {
                    created.insert(
                        creation_id.clone(),
                        json!({ "id": ctx.account_id.to_string() }),
                    );
                }
                Err(_) => {
                    not_created.insert(creation_id.clone(), set_error("serverFail"));
                }
            }
        }
    }

    if let Some(update) = args.get("update").and_then(Value::as_object) {
        for (id, patch) in update {
            match apply(ctx, patch) {
                Ok(()) => {
                    updated.insert(id.clone(), Value::Null);
                }
                Err(_) => {
                    not_updated.insert(id.clone(), set_error("serverFail"));
                }
            }
        }
    }

    Invocation::new(
        "Identity/set",
        json!({
            "accountId": account_id(args),
            "oldState": STATE,
            "newState": STATE,
            "created": created,
            "updated": updated,
            "destroyed": [],
            "notCreated": not_created,
            "notUpdated": not_updated,
            "notDestroyed": {},
        }),
        call_id,
    )
}

fn apply(ctx: &JmapContext, patch: &Value) -> Result<()> {
    let mut account = ctx.directory.accounts().get(ctx.account_id)?;
    if let Some(name) = patch.get("name").and_then(Value::as_str) {
        account.display_name = name.to_string();
    }
    if let Some(signature) = patch.get("textSignature").and_then(Value::as_str) {
        account.signature = signature.to_string();
    }
    ctx.directory.accounts().update(account)
}

fn set_error(kind: &str) -> Value {
    json!({ "type": kind })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_context;

    #[test]
    fn updating_an_absent_account_is_reported() {
        let ctx = test_context();
        let response = identity_set(
            &ctx,
            &json!({"accountId": "1", "update": {"1": {"name": "Alice"}}}),
            "c0",
        );
        assert_eq!(response.name(), "Identity/set");
        assert!(response.arguments()["notUpdated"]["1"].is_object());
    }
}
