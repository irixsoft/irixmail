use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use irixmail_core::config::ListenersConfig;

pub use irixmail_store::settings_key;

use crate::app::AppState;

pub fn defaults(hostname: &str, listeners: &ListenersConfig) -> Value {
    json!({
        "hostname": hostname,
        "antiSpam": {
            "dnsblZones": Vec::<String>::new(),
            "greylistWindowSeconds": irixmail_smtp::GreylistConfig::default().window.as_secs(),
        },
        "rateLimits": {
            "maxConnectionsPerIp": irixmail_smtp::DEFAULT_MAX_CONNECTIONS,
            "maxMessagesPerConnection": irixmail_smtp::DEFAULT_MAX_MESSAGES,
            "maxMessagesPerSenderPerHour": irixmail_smtp::DEFAULT_MAX_PER_SENDER,
            "maxMessagesPerDomainPerHour": irixmail_smtp::DEFAULT_MAX_PER_DOMAIN,
        },
        "listeners": listeners_json(listeners),
    })
}

fn listeners_json(listeners: &ListenersConfig) -> Value {
    json!({
        "smtp": listeners.smtp.plain,
        "submission": [listeners.submission.plain, listeners.submission.tls],
        "imap": [listeners.imap.plain, listeners.imap.tls],
        "pop3": [listeners.pop3.plain, listeners.pop3.tls],
        "managesieve": listeners.managesieve.plain,
        "https": listeners.http.tls,
        "http": listeners.http.plain,
    })
}

pub(crate) fn merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(base), Value::Object(patch)) => {
            for (key, value) in patch {
                merge(base.entry(key).or_insert(Value::Null), value);
            }
        }
        (slot, patch) => *slot = patch.clone(),
    }
}

pub async fn get(State(state): State<AppState>) -> Json<Value> {
    let mut value = defaults(&state.hostname, &state.listeners);
    if let Ok(Some(bytes)) = state.store.get(&settings_key()) {
        if let Ok(stored) = serde_json::from_slice::<Value>(&bytes) {
            merge(&mut value, &stored);
        }
    }
    value["hostname"] = json!(state.hostname);
    value["listeners"] = listeners_json(&state.listeners);
    Json(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    use irixmail_core::config::ProtocolListener;

    use crate::app::router;
    use crate::tests_support::{admin_token, state, TempDir};

    #[tokio::test]
    async fn the_settings_carry_the_hostname() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["hostname"], "mail.example.com");
    }

    #[test]
    fn the_advertised_defaults_match_the_live_configuration() {
        let value = defaults("mail.example.com", &ListenersConfig::default());
        assert_eq!(
            value["antiSpam"]["greylistWindowSeconds"],
            json!(irixmail_smtp::GreylistConfig::default().window.as_secs())
        );
        assert_eq!(value["antiSpam"]["dnsblZones"], json!([] as [&str; 0]));
        assert_eq!(
            value["rateLimits"]["maxConnectionsPerIp"],
            json!(irixmail_smtp::DEFAULT_MAX_CONNECTIONS)
        );
        assert_eq!(
            value["rateLimits"]["maxMessagesPerConnection"],
            json!(irixmail_smtp::DEFAULT_MAX_MESSAGES)
        );
        assert_eq!(
            value["rateLimits"]["maxMessagesPerSenderPerHour"],
            json!(irixmail_smtp::DEFAULT_MAX_PER_SENDER)
        );
        assert_eq!(
            value["rateLimits"]["maxMessagesPerDomainPerHour"],
            json!(irixmail_smtp::DEFAULT_MAX_PER_DOMAIN)
        );
    }

    #[tokio::test]
    async fn the_advertised_listeners_come_from_the_running_configuration() {
        let dir = TempDir::new();
        let mut shared = state(&dir);
        shared.listeners.smtp = ProtocolListener {
            plain: Some(2525),
            tls: None,
        };
        shared.listeners.http = ProtocolListener {
            plain: Some(8080),
            tls: Some(8443),
        };
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["listeners"]["smtp"], json!(2525));
        assert_eq!(value["listeners"]["http"], json!(8080));
        assert_eq!(value["listeners"]["https"], json!(8443));
    }

    #[tokio::test]
    async fn a_stored_hostname_does_not_shadow_the_real_one() {
        let dir = TempDir::new();
        let shared = state(&dir);
        shared
            .store
            .put(
                &settings_key(),
                br#"{"hostname":"spoof.example","antiSpam":{"greylistWindowSeconds":0}}"#,
            )
            .unwrap();
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["hostname"], "mail.example.com");
    }
}
