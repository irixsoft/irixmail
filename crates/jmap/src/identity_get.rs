use serde_json::{json, Value};

use crate::context::JmapContext;
use crate::reply::{account_id, requested_ids, STATE};
use crate::request::Invocation;

pub fn identity_get(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let mut list = Vec::new();
    let mut not_found = Vec::new();

    if let Ok(account) = ctx.directory.accounts().get(ctx.account_id) {
        let id = account.id.to_string();
        let wanted = requested_ids(args);
        let include = wanted.as_ref().is_none_or(|ids| ids.contains(&id));
        if include {
            let domain = ctx
                .directory
                .domains()
                .get(account.domain_id)
                .map(|domain| domain.name)
                .unwrap_or_default();
            list.push(json!({
                "id": id,
                "name": account.display_name,
                "email": format!("{}@{}", account.local_part, domain),
                "replyTo": Value::Null,
                "bcc": Value::Null,
                "textSignature": account.signature,
                "htmlSignature": "",
                "mayDelete": false,
            }));
        }
        if let Some(ids) = wanted {
            for requested in ids {
                if requested != id {
                    not_found.push(Value::String(requested));
                }
            }
        }
    }

    Invocation::new(
        "Identity/get",
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
    fn an_account_without_a_record_yields_no_identity() {
        let ctx = test_context();
        let response = identity_get(&ctx, &json!({"accountId": "1", "ids": null}), "c0");
        assert_eq!(response.name(), "Identity/get");
        assert_eq!(response.arguments()["list"], json!([]));
    }
}
