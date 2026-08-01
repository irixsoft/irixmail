use rand::distr::Alphanumeric;
use rand::Rng;
use serde_json::{json, Map, Value};

use crate::context::JmapContext;
use crate::push_store::{
    load_subscriptions, save_subscriptions, PushKeys, PushSubscriptionRecord, MAX_EXPIRES_SECS,
    MAX_SUBSCRIPTIONS,
};
use crate::request::Invocation;

pub(crate) fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

pub fn push_set(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let account_id = ctx.account_id as u32;
    let now = now_seconds();
    let mut subscriptions =
        load_subscriptions(ctx.store.as_ref(), account_id, now).unwrap_or_default();
    let mut created = Map::new();
    let mut not_created = Map::new();
    let mut updated = Map::new();
    let mut not_updated = Map::new();
    let mut destroyed = Vec::new();
    let mut not_destroyed = Map::new();
    let mut dirty = false;

    if let Some(create) = args.get("create").and_then(Value::as_object) {
        for (creation_id, patch) in create {
            if subscriptions.len() >= MAX_SUBSCRIPTIONS {
                not_created.insert(
                    creation_id.clone(),
                    set_error("overQuota", "too many push subscriptions"),
                );
                continue;
            }
            match build_subscription(patch, ctx.directory.ids().generate(), now) {
                Ok(record) => {
                    tracing::info!(
                        target: "irixmail::jmap",
                        account = account_id,
                        subscription = record.id,
                        device = %record.device_client_id,
                        "push subscription created"
                    );
                    created.insert(
                        creation_id.clone(),
                        json!({
                            "id": record.id.to_string(),
                            "expires": crate::utc_date::format(record.expires),
                        }),
                    );
                    subscriptions.push(record);
                    dirty = true;
                }
                Err(reason) => {
                    not_created.insert(creation_id.clone(), set_error("invalidProperties", reason));
                }
            }
        }
    }

    if let Some(update) = args.get("update").and_then(Value::as_object) {
        for (id, patch) in update {
            let Some(record) = id
                .parse::<u64>()
                .ok()
                .and_then(|id| subscriptions.iter_mut().find(|record| record.id == id))
            else {
                tracing::warn!(
                    target: "irixmail::jmap",
                    account = account_id,
                    subscription = %id,
                    "push subscription update rejected: no such subscription"
                );
                not_updated.insert(id.clone(), set_error("notFound", "no such subscription"));
                continue;
            };
            match apply_update(record, patch, now) {
                Ok(became_verified) => {
                    if became_verified {
                        tracing::info!(
                            target: "irixmail::jmap",
                            account = account_id,
                            subscription = record.id,
                            "push subscription verified"
                        );
                    }
                    updated.insert(id.clone(), Value::Null);
                    dirty = true;
                }
                Err(reason) => {
                    tracing::warn!(
                        target: "irixmail::jmap",
                        account = account_id,
                        subscription = %id,
                        reason,
                        "push subscription update rejected"
                    );
                    not_updated.insert(id.clone(), set_error("invalidProperties", reason));
                }
            }
        }
    }

    if let Some(destroy) = args.get("destroy").and_then(Value::as_array) {
        for id in destroy.iter().filter_map(Value::as_str) {
            let before = subscriptions.len();
            if let Ok(numeric) = id.parse::<u64>() {
                subscriptions.retain(|record| record.id != numeric);
            }
            if subscriptions.len() < before {
                tracing::info!(
                    target: "irixmail::jmap",
                    account = account_id,
                    subscription = %id,
                    "push subscription destroyed"
                );
                destroyed.push(Value::String(id.to_string()));
                dirty = true;
            } else {
                not_destroyed.insert(
                    id.to_string(),
                    set_error("notFound", "no such subscription"),
                );
            }
        }
    }

    if dirty {
        if let Err(error) = save_subscriptions(
            ctx.store.as_ref(),
            &ctx.notifier,
            account_id,
            &subscriptions,
        ) {
            tracing::warn!(
                target: "irixmail::jmap",
                account = account_id,
                error = %error,
                "push subscription save failed"
            );
        }
    }

    Invocation::new(
        "PushSubscription/set",
        json!({
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

fn build_subscription(
    patch: &Value,
    id: u64,
    now: u64,
) -> Result<PushSubscriptionRecord, &'static str> {
    let device_client_id = patch
        .get("deviceClientId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() < 256)
        .ok_or("deviceClientId is required")?
        .to_string();
    let url = patch
        .get("url")
        .and_then(Value::as_str)
        .filter(|value| value.starts_with("https://") && value.len() < 512)
        .ok_or("url must be an https address")?
        .to_string();
    let keys = match patch.get("keys") {
        None | Some(Value::Null) => None,
        Some(keys) => {
            let p256dh = keys.get("p256dh").and_then(Value::as_str).unwrap_or("");
            let auth = keys.get("auth").and_then(Value::as_str).unwrap_or("");
            if decode_key(p256dh).is_none() || decode_key(auth).is_none() {
                return Err("keys must be base64url p256dh and auth values");
            }
            Some(PushKeys {
                p256dh: p256dh.to_string(),
                auth: auth.to_string(),
            })
        }
    };
    let expires = clamp_expires(patch.get("expires"), now)?;
    let types = parse_types(patch.get("types"))?;
    let verification_code: String = rand::rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();
    Ok(PushSubscriptionRecord {
        id,
        device_client_id,
        url,
        keys,
        verification_code,
        verified: false,
        expires,
        types,
        verification_sends: 0,
    })
}

fn apply_update(
    record: &mut PushSubscriptionRecord,
    patch: &Value,
    now: u64,
) -> Result<bool, &'static str> {
    for key in patch
        .as_object()
        .map(|map| map.keys())
        .into_iter()
        .flatten()
    {
        if !matches!(key.as_str(), "verificationCode" | "expires" | "types") {
            return Err("only verificationCode, expires and types may be updated");
        }
    }
    let mut became_verified = false;
    if let Some(code) = patch.get("verificationCode").and_then(Value::as_str) {
        if code != record.verification_code {
            return Err("verification code does not match");
        }
        if !record.verified {
            record.verified = true;
            became_verified = true;
        }
    }
    if patch.get("expires").is_some() {
        record.expires = clamp_expires(patch.get("expires"), now)?;
    }
    if patch.get("types").is_some() {
        record.types = parse_types(patch.get("types"))?;
    }
    Ok(became_verified)
}

fn clamp_expires(value: Option<&Value>, now: u64) -> Result<u64, &'static str> {
    let ceiling = now + MAX_EXPIRES_SECS;
    match value {
        None | Some(Value::Null) => Ok(ceiling),
        Some(Value::String(text)) => match crate::utc_date::parse(text) {
            Some(stamp) => Ok(stamp.min(ceiling)),
            None => Err("expires is not a valid UTCDate"),
        },
        Some(_) => Err("expires is not a valid UTCDate"),
    }
}

fn parse_types(value: Option<&Value>) -> Result<Vec<String>, &'static str> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => Ok(items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()),
        Some(_) => Err("types must be an array of type names"),
    }
}

fn decode_key(value: &str) -> Option<Vec<u8>> {
    use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
    use base64::Engine;
    if value.is_empty() {
        return None;
    }
    URL_SAFE_NO_PAD
        .decode(value)
        .or_else(|_| URL_SAFE.decode(value))
        .ok()
}

fn set_error(kind: &str, description: &str) -> Value {
    json!({ "type": kind, "description": description })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_context;

    fn create_args(url: &str) -> Value {
        json!({
            "create": {
                "k1": {
                    "deviceClientId": "device-a",
                    "url": url,
                    "types": ["Email"],
                }
            }
        })
    }

    #[test]
    fn creating_a_subscription_returns_its_id_and_clamped_expires() {
        let ctx = test_context();
        let response = push_set(&ctx, &create_args("https://push.example.com/x"), "c0");
        let created = &response.arguments()["created"]["k1"];
        assert!(created["id"].as_str().unwrap().parse::<u64>().is_ok());
        assert!(created["expires"].is_string());

        let stored = load_subscriptions(ctx.store.as_ref(), 1, 0).unwrap();
        assert_eq!(stored.len(), 1);
        assert!(!stored[0].verified);
        assert_eq!(stored[0].verification_code.len(), 32);
        assert!(stored[0].expires <= now_seconds() + MAX_EXPIRES_SECS);
    }

    #[test]
    fn created_ids_are_snowflake_style() {
        let ctx = test_context();
        let response = push_set(&ctx, &create_args("https://push.example.com/x"), "c0");
        let id: u64 = response.arguments()["created"]["k1"]["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!(id > 1 << 22);
    }

    #[test]
    fn a_plain_http_url_is_rejected() {
        let ctx = test_context();
        let response = push_set(&ctx, &create_args("http://push.example.com/x"), "c0");
        assert_eq!(
            response.arguments()["notCreated"]["k1"]["type"],
            "invalidProperties"
        );
    }

    #[test]
    fn the_matching_verification_code_marks_the_subscription_verified() {
        let ctx = test_context();
        push_set(&ctx, &create_args("https://push.example.com/x"), "c0");
        let stored = load_subscriptions(ctx.store.as_ref(), 1, 0).unwrap();
        let id = stored[0].id.to_string();
        let code = stored[0].verification_code.clone();

        let wrong = push_set(
            &ctx,
            &json!({"update": {id.clone(): {"verificationCode": "nope"}}}),
            "c1",
        );
        assert!(wrong.arguments()["notUpdated"][&id].is_object());

        let right = push_set(
            &ctx,
            &json!({"update": {id.clone(): {"verificationCode": code}}}),
            "c2",
        );
        assert!(right.arguments()["updated"].get(&id).is_some());
        assert!(load_subscriptions(ctx.store.as_ref(), 1, 0).unwrap()[0].verified);
    }

    #[test]
    fn destroy_removes_the_subscription() {
        let ctx = test_context();
        push_set(&ctx, &create_args("https://push.example.com/x"), "c0");
        let id = load_subscriptions(ctx.store.as_ref(), 1, 0).unwrap()[0]
            .id
            .to_string();
        let response = push_set(&ctx, &json!({"destroy": [id.clone()]}), "c1");
        assert_eq!(response.arguments()["destroyed"], json!([id]));
        assert!(load_subscriptions(ctx.store.as_ref(), 1, 0)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn the_subscription_count_is_capped() {
        let ctx = test_context();
        for index in 0..MAX_SUBSCRIPTIONS {
            let args = json!({
                "create": { "k": {
                    "deviceClientId": format!("d{index}"),
                    "url": "https://push.example.com/x",
                }}
            });
            let response = push_set(&ctx, &args, "c");
            assert!(response.arguments()["created"]["k"].is_object(), "{index}");
        }
        let response = push_set(&ctx, &create_args("https://push.example.com/x"), "c");
        assert_eq!(
            response.arguments()["notCreated"]["k1"]["type"],
            "overQuota"
        );
    }
}
