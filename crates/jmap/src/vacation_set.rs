use serde_json::{json, Map, Value};

use irixmail_core::{Error, Result};

use crate::context::JmapContext;
use crate::reply::{account_id, STATE};
use crate::request::Invocation;
use crate::utc_date;

pub fn vacation_set(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let mut updated = Map::new();
    let mut not_updated = Map::new();

    if let Some(update) = args.get("update").and_then(Value::as_object) {
        for (id, patch) in update {
            if id != "singleton" {
                not_updated.insert(id.clone(), set_error("notFound"));
                continue;
            }
            match apply(ctx, patch) {
                Ok(()) => {
                    updated.insert(id.clone(), Value::Null);
                }
                Err(Error::InvalidInput(_)) => {
                    not_updated.insert(id.clone(), set_error("invalidProperties"));
                }
                Err(_) => {
                    not_updated.insert(id.clone(), set_error("serverFail"));
                }
            }
        }
    }

    Invocation::new(
        "VacationResponse/set",
        json!({
            "accountId": account_id(args),
            "oldState": STATE,
            "newState": STATE,
            "created": {},
            "updated": updated,
            "destroyed": [],
            "notCreated": {},
            "notUpdated": not_updated,
            "notDestroyed": {},
        }),
        call_id,
    )
}

fn apply(ctx: &JmapContext, patch: &Value) -> Result<()> {
    let mut account = ctx.directory.accounts().get(ctx.account_id)?;
    if let Some(enabled) = patch.get("isEnabled").and_then(Value::as_bool) {
        account.vacation.enabled = enabled;
    }
    if patch.get("subject").is_some() {
        account.vacation.subject = patch
            .get("subject")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    }
    if patch.get("textBody").is_some() {
        account.vacation.body = patch
            .get("textBody")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    }
    if patch.get("fromDate").is_some() {
        account.vacation.active_from = date_field(patch, "fromDate")?;
    }
    if patch.get("toDate").is_some() {
        account.vacation.active_to = date_field(patch, "toDate")?;
    }
    ctx.directory.accounts().update(account)
}

fn date_field(patch: &Value, field: &str) -> Result<Option<u64>> {
    match patch.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => utc_date::parse(text)
            .map(Some)
            .ok_or_else(|| Error::invalid_input(format!("{field} is not a UTC date"))),
        Some(_) => Err(Error::invalid_input(format!("{field} is not a UTC date"))),
    }
}

fn set_error(kind: &str) -> Value {
    json!({ "type": kind })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{test_context, test_context_with_account};
    use crate::vacation_get::vacation_get;

    #[test]
    fn updating_an_unknown_id_is_not_found() {
        let ctx = test_context();
        let response = vacation_set(
            &ctx,
            &json!({"accountId": "1", "update": {"other": {"isEnabled": true}}}),
            "c0",
        );
        assert_eq!(
            response.arguments()["notUpdated"]["other"]["type"],
            "notFound"
        );
    }

    #[test]
    fn vacation_dates_persist_on_set_and_round_trip_on_get() {
        let ctx = test_context_with_account();
        let response = vacation_set(
            &ctx,
            &json!({"accountId": "1", "update": {"singleton": {
                "isEnabled": true,
                "subject": "Away",
                "textBody": "Back soon",
                "fromDate": "2026-07-10T00:00:00.000Z",
                "toDate": "2026-07-20T00:00:00Z"
            }}}),
            "c0",
        );
        assert!(response.arguments()["updated"]
            .as_object()
            .unwrap()
            .contains_key("singleton"));

        let account = ctx.directory.accounts().get(ctx.account_id).unwrap();
        assert_eq!(account.vacation.active_from, Some(1_783_641_600));
        assert_eq!(account.vacation.active_to, Some(1_784_505_600));

        let got = vacation_get(&ctx, &json!({"accountId": "1", "ids": null}), "c0");
        assert_eq!(
            got.arguments()["list"][0]["fromDate"],
            "2026-07-10T00:00:00Z"
        );
        assert_eq!(got.arguments()["list"][0]["toDate"], "2026-07-20T00:00:00Z");
    }

    #[test]
    fn null_vacation_dates_clear_the_stored_window() {
        let ctx = test_context_with_account();
        vacation_set(
            &ctx,
            &json!({"accountId": "1", "update": {"singleton": {
                "fromDate": "2026-07-10T00:00:00Z",
                "toDate": "2026-07-20T00:00:00Z"
            }}}),
            "c0",
        );
        let stored = ctx.directory.accounts().get(ctx.account_id).unwrap();
        assert_eq!(stored.vacation.active_from, Some(1_783_641_600));

        vacation_set(
            &ctx,
            &json!({"accountId": "1", "update": {"singleton": {
                "fromDate": null,
                "toDate": null
            }}}),
            "c0",
        );
        let account = ctx.directory.accounts().get(ctx.account_id).unwrap();
        assert_eq!(account.vacation.active_from, None);
        assert_eq!(account.vacation.active_to, None);
    }

    #[test]
    fn an_unparseable_date_is_invalid_properties() {
        let ctx = test_context_with_account();
        let response = vacation_set(
            &ctx,
            &json!({"accountId": "1", "update": {"singleton": {
                "fromDate": "next tuesday"
            }}}),
            "c0",
        );
        assert_eq!(
            response.arguments()["notUpdated"]["singleton"]["type"],
            "invalidProperties"
        );
    }
}
