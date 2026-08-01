use serde_json::{json, Value};

use crate::context::JmapContext;
use crate::push_store::load_subscriptions;
use crate::reply::requested_ids;
use crate::request::Invocation;

pub fn push_get(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let now = crate::push_set::now_seconds();
    let subscriptions =
        load_subscriptions(ctx.store.as_ref(), ctx.account_id as u32, now).unwrap_or_default();
    let wanted = requested_ids(args);
    let mut list = Vec::new();
    let mut not_found = Vec::new();
    for subscription in &subscriptions {
        let id = subscription.id.to_string();
        if wanted.as_ref().is_none_or(|ids| ids.contains(&id)) {
            list.push(json!({
                "id": id,
                "deviceClientId": subscription.device_client_id,
                "verificationCode": Value::Null,
                "verified": subscription.verified,
                "expires": crate::utc_date::format(subscription.expires),
                "types": if subscription.types.is_empty() {
                    Value::Null
                } else {
                    json!(subscription.types)
                },
            }));
        }
    }
    if let Some(ids) = wanted {
        for id in ids {
            if !subscriptions
                .iter()
                .any(|subscription| subscription.id.to_string() == id)
            {
                not_found.push(Value::String(id));
            }
        }
    }
    Invocation::new(
        "PushSubscription/get",
        json!({
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
    use crate::push_store::{save_subscriptions, PushSubscriptionRecord};

    #[test]
    fn subscriptions_list_without_url_or_keys() {
        let ctx = test_context();
        save_subscriptions(
            ctx.store.as_ref(),
            &ctx.notifier,
            1,
            &[PushSubscriptionRecord {
                id: 1,
                device_client_id: "device-a".to_string(),
                url: "https://push.example.com/x".to_string(),
                keys: None,
                verification_code: "secret".to_string(),
                verified: true,
                expires: u64::MAX,
                types: Vec::new(),
                verification_sends: 0,
            }],
        )
        .unwrap();

        let response = push_get(&ctx, &json!({"ids": null}), "c0");
        let list = response.arguments()["list"].as_array().unwrap().clone();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["deviceClientId"], "device-a");
        assert!(list[0].get("url").is_none());
        assert!(list[0].get("keys").is_none());
        assert_eq!(list[0]["verificationCode"], Value::Null);
        assert_eq!(list[0]["verified"], json!(true));
    }

    #[test]
    fn an_unverified_subscription_reports_verified_false() {
        let ctx = test_context();
        save_subscriptions(
            ctx.store.as_ref(),
            &ctx.notifier,
            1,
            &[PushSubscriptionRecord {
                id: 1,
                device_client_id: "device-a".to_string(),
                url: "https://push.example.com/x".to_string(),
                keys: None,
                verification_code: "secret".to_string(),
                verified: false,
                expires: u64::MAX,
                types: Vec::new(),
                verification_sends: 0,
            }],
        )
        .unwrap();

        let response = push_get(&ctx, &json!({"ids": null}), "c0");
        assert_eq!(response.arguments()["list"][0]["verified"], json!(false));
    }

    #[test]
    fn unknown_ids_are_reported_as_not_found() {
        let ctx = test_context();
        let response = push_get(&ctx, &json!({"ids": ["9"]}), "c0");
        assert_eq!(response.arguments()["notFound"], json!(["9"]));
    }
}
