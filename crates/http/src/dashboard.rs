use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use irixmail_directory::DnsStatus;
use irixmail_tls::{inspect, needs_renewal, RenewalSchedule};

use crate::app::AppState;

pub async fn dashboard(State(state): State<AppState>) -> Json<Value> {
    let domains = state.directory.domains().list().unwrap_or_default();
    let accounts = state
        .directory
        .accounts()
        .list()
        .map(|list| list.len())
        .unwrap_or(0);
    let now = irixmail_tls::acme_http01::unix_now();
    let queue_depth = irixmail_smtp::scan_all(state.store.as_ref())
        .map(|entries| entries.len())
        .unwrap_or(0);
    let messages_in = irixmail_smtp::inbound_total(state.store.as_ref(), now).unwrap_or(0);
    let messages_out = irixmail_smtp::outbound_total(state.store.as_ref(), now).unwrap_or(0);
    let storage_bytes = state.blobs.usage_bytes().unwrap_or(0);
    let services: Vec<Value> = state
        .services
        .get()
        .map(|names| {
            names
                .iter()
                .map(|name| json!({ "name": name, "status": "running" }))
                .collect()
        })
        .unwrap_or_default();
    let latest = state
        .update_available
        .read()
        .ok()
        .and_then(|slot| slot.clone());
    let update_available = latest.is_some();
    Json(json!({
        "version": {
            "current": env!("CARGO_PKG_VERSION"),
            "latest": latest,
            "updateAvailable": update_available,
        },
        "domains": domains.len(),
        "accounts": accounts,
        "messagesInToday": messages_in,
        "messagesOutToday": messages_out,
        "queueDepth": queue_depth,
        "storageBytes": storage_bytes,
        "recentLogEntries": state.logs.len(),
        "certificate": certificate_json(&state, now),
        "dns": { "status": dns_status(&domains) },
        "services": services,
    }))
}

fn certificate_json(state: &AppState, now: u64) -> Value {
    let summary = state
        .tls
        .as_ref()
        .and_then(|tls| tls.cert_store.as_ref())
        .and_then(|store| store.load(&state.hostname).ok().flatten())
        .and_then(|material| inspect(&material));
    match summary {
        Some(summary) => {
            let status = if summary.self_signed {
                "self-signed"
            } else if needs_renewal(
                summary.not_after.max(0) as u64,
                now,
                RenewalSchedule::default().renew_before,
            ) {
                "expiring"
            } else {
                "valid"
            };
            json!({ "status": status, "expiresAt": summary.not_after * 1000 })
        }
        None => json!({ "status": "none", "expiresAt": Value::Null }),
    }
}

fn dns_status(domains: &[irixmail_directory::Domain]) -> &'static str {
    if domains.is_empty() {
        return "unknown";
    }
    if domains
        .iter()
        .any(|domain| matches!(domain.dns_status, DnsStatus::Failing { .. }))
    {
        return "failed";
    }
    if domains.iter().all(|domain| domain.dns_status.is_verified()) {
        return "ok";
    }
    "unverified"
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    use crate::app::{router, TokenInfo};
    use crate::tests_support::{state, TempDir};

    #[tokio::test]
    async fn the_dashboard_requires_authentication() {
        let dir = TempDir::new();
        let app = router(state(&dir));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/dashboard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn the_dashboard_reports_real_queue_storage_and_message_stats() {
        use std::sync::Arc;

        let dir = TempDir::new();
        let shared = state(&dir);
        let token = crate::tests_support::admin_token(&shared);
        let store = Arc::clone(&shared.store);
        let blobs = Arc::clone(&shared.blobs);
        let now = irixmail_tls::acme_http01::unix_now();
        irixmail_smtp::record_inbound(store.as_ref(), now).unwrap();
        irixmail_smtp::record_outbound(store.as_ref(), now).unwrap();
        let recipients = vec![(
            "rcpt@remote.example".to_string(),
            irixmail_smtp::Expiry::Attempts(5),
        )];
        let request = irixmail_smtp::Enqueue {
            created: now,
            return_path: "sender@local.example",
            recipients: &recipients,
            first_due: now + 3_600,
        };
        irixmail_smtp::enqueue(store.as_ref(), blobs.as_ref(), b"queued body", &request).unwrap();
        shared
            .services
            .set(vec!["smtp:25".to_string(), "http:80".to_string()])
            .unwrap();

        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/dashboard")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["queueDepth"], 1);
        assert_eq!(value["messagesInToday"], 1);
        assert_eq!(value["messagesOutToday"], 1);
        assert!(value["storageBytes"].as_u64().unwrap() > 0);
        assert_eq!(value["services"][0]["name"], "smtp:25");
        assert_eq!(value["services"][0]["status"], "running");
        assert_eq!(value["dns"]["status"], "unknown");
        assert_eq!(value["certificate"]["status"], "none");
    }

    #[tokio::test]
    async fn the_dashboard_reports_the_stored_certificate_and_domain_dns_state() {
        use std::sync::Arc;

        use irixmail_tls::CertStore;

        use crate::app::TlsHandles;

        let dir = TempDir::new();
        let mut shared = state(&dir);
        let cert_store = CertStore::new(dir.path.join("certs"));
        let material = irixmail_tls::self_signed::generate(vec![shared.hostname.clone()]).unwrap();
        cert_store
            .save(
                &shared.hostname,
                &material,
                irixmail_tls::CertSource::SelfSigned,
            )
            .unwrap();
        shared.tls = Some(TlsHandles {
            cert_store: Some(Arc::new(cert_store)),
            ..TlsHandles::default()
        });
        shared
            .directory
            .domains()
            .create("one.example", Vec::new())
            .unwrap();
        let token = crate::tests_support::admin_token(&shared);

        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/dashboard")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["certificate"]["status"], "self-signed");
        let not_after = irixmail_tls::inspect(&material).unwrap().not_after;
        assert_eq!(value["certificate"]["expiresAt"], not_after * 1000);
        assert_eq!(value["dns"]["status"], "unverified");
    }

    #[tokio::test]
    async fn the_dashboard_reports_the_version_and_available_update() {
        let dir = TempDir::new();
        let shared = state(&dir);
        *shared.update_available.write().unwrap() = Some("v9.9.9".into());
        let token = crate::tests_support::admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/dashboard")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["version"]["current"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["version"]["latest"], "v9.9.9");
        assert_eq!(value["version"]["updateAvailable"], true);
    }

    #[tokio::test]
    async fn an_admin_sees_the_dashboard_counts() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = shared.tokens.issue(TokenInfo {
            account_id: 1,
            username: "admin@example.com".into(),
            is_admin: true,
        });
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/dashboard")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["domains"], 0);
        assert!(value["services"].is_array());
    }
}
