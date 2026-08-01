use serde_json::{json, Value};

use crate::context::JmapContext;
use crate::reply::{account_id, requested_ids, STATE};
use crate::request::Invocation;

pub fn vacation_get(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let singleton = match ctx.directory.accounts().get(ctx.account_id) {
        Ok(account) => json!({
            "id": "singleton",
            "isEnabled": account.vacation.enabled,
            "fromDate": date_or_null(account.vacation.active_from),
            "toDate": date_or_null(account.vacation.active_to),
            "subject": optional(&account.vacation.subject),
            "textBody": optional(&account.vacation.body),
            "htmlBody": null,
        }),
        Err(_) => json!({
            "id": "singleton",
            "isEnabled": false,
            "fromDate": null,
            "toDate": null,
            "subject": null,
            "textBody": null,
            "htmlBody": null,
        }),
    };
    let not_found: Vec<Value> = match requested_ids(args) {
        Some(ids) => ids
            .into_iter()
            .filter(|id| id != "singleton")
            .map(Value::String)
            .collect(),
        None => Vec::new(),
    };
    Invocation::new(
        "VacationResponse/get",
        json!({
            "accountId": account_id(args),
            "state": STATE,
            "list": [singleton],
            "notFound": not_found,
        }),
        call_id,
    )
}

fn optional(value: &str) -> Value {
    if value.is_empty() {
        Value::Null
    } else {
        Value::String(value.to_string())
    }
}

fn date_or_null(value: Option<u64>) -> Value {
    match value {
        Some(seconds) => Value::String(crate::utc_date::format(seconds)),
        None => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_context;

    #[test]
    fn the_singleton_is_always_returned() {
        let ctx = test_context();
        let response = vacation_get(&ctx, &json!({"accountId": "1", "ids": null}), "c0");
        assert_eq!(response.name(), "VacationResponse/get");
        assert_eq!(response.arguments()["list"][0]["id"], "singleton");
        assert_eq!(response.arguments()["list"][0]["isEnabled"], false);
    }

    #[test]
    fn other_requested_ids_are_not_found() {
        let ctx = test_context();
        let response = vacation_get(
            &ctx,
            &json!({"accountId": "1", "ids": ["singleton", "x"]}),
            "c0",
        );
        assert_eq!(response.arguments()["notFound"], json!(["x"]));
    }
}
