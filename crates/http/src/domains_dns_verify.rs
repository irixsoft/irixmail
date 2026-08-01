use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use irixmail_dns::{domain_records, verify_all, CheckStatus, DomainRecordsInput};

use crate::app::{error_response, AppState};
use crate::dns_status::{persist_status, status_from_checks};
use crate::validate::parse_id;

pub async fn verify(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(id) = parse_id(&id) else {
        return error_response(StatusCode::NOT_FOUND, "domain not found");
    };
    let domain = match state.directory.domains().get(id) {
        Ok(domain) => domain,
        Err(_) => return error_response(StatusCode::NOT_FOUND, "domain not found"),
    };
    let dkim_keys = match state.directory.dkim().get_or_create(domain.id, "default") {
        Ok(key) => vec![key],
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not load the DKIM key",
            )
        }
    };
    let mtasts_id = domain.created_at.to_string();
    let input = DomainRecordsInput {
        domain: &domain.name,
        mail_host: &state.hostname,
        ipv4: state.public_ipv4,
        ipv6: state.public_ipv6,
        dkim_keys: &dkim_keys,
        mtasts_id: &mtasts_id,
        mx_preference: 10,
    };
    let records = domain_records(&input);
    let checks = match verify_all(&state.resolver, &records).await {
        Ok(checks) => checks,
        Err(_) => return error_response(StatusCode::BAD_GATEWAY, "DNS verification failed"),
    };
    let results: Vec<_> = checks
        .iter()
        .map(|check| {
            json!({
                "record": check.record,
                "status": status_label(check.status),
                "observed": check.observed,
            })
        })
        .collect();
    let all_green = checks.iter().all(|check| check.status == CheckStatus::Pass);
    let status = status_from_checks(&checks, irixmail_tls::acme_http01::unix_now());
    if let Err(err) = persist_status(&state.directory, &domain, status.clone()) {
        tracing::warn!(domain = %domain.name, error = %err, "could not store the dns status");
    }
    Json(json!({
        "domain": domain.name,
        "results": results,
        "allGreen": all_green,
        "status": status,
    }))
    .into_response()
}

fn status_label(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Pass => "pass",
        CheckStatus::Mismatch => "mismatch",
        CheckStatus::Missing => "missing",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request};
    use irixmail_directory::DnsStatus;
    use tower::ServiceExt;

    use crate::app::router;
    use crate::dns_status::persist_status;
    use crate::tests_support::{admin_token, state, TempDir};

    #[tokio::test]
    async fn verification_reports_real_record_statuses_not_a_placeholder() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/domains/{}/dns/verify", domain.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["allGreen"], false);
        let statuses: Vec<&str> = value["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["status"].as_str().unwrap())
            .collect();
        assert!(!statuses.is_empty());
        assert!(statuses.iter().all(|status| *status == "missing"));
        assert!(value["results"][0]["observed"].is_array());
    }

    #[tokio::test]
    async fn verification_checks_the_dkim_selector_record() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let token = admin_token(&shared);
        let app = router(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/domains/{}/dns/verify", domain.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let dkim: Vec<&serde_json::Value> = value["results"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| {
                entry["record"]["name"]
                    .as_str()
                    .unwrap()
                    .contains("_domainkey.example.com")
            })
            .collect();
        assert_eq!(dkim.len(), 1, "verify must check the DKIM selector record");
        assert_eq!(dkim[0]["status"], "missing");
        assert_eq!(value["allGreen"], false);
        let key = shared.directory.dkim().get(domain.id).unwrap();
        assert!(
            key.is_some(),
            "verify must mint the domain's DKIM key when absent"
        );
    }

    async fn verify_request(shared: &AppState, id: u64) -> serde_json::Value {
        let token = admin_token(shared);
        let app = router(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/domains/{id}/dns/verify"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn verifying_persists_the_failing_status_on_the_domain() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        verify_request(&shared, domain.id).await;
        let stored = shared.directory.domains().get(domain.id).unwrap();
        match stored.dns_status {
            DnsStatus::Failing {
                checked_at,
                ref missing,
            } => {
                assert!(checked_at > 0);
                assert!(!missing.is_empty());
            }
            other => panic!("expected a failing status, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_verify_response_carries_the_new_status() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let value = verify_request(&shared, domain.id).await;
        assert_eq!(value["status"]["state"], "failing");
    }

    #[tokio::test]
    async fn a_verified_domain_is_marked_failing_again_when_records_disappear() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let mut domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        domain.dns_status = DnsStatus::Verified {
            checked_at: 1_700_000_000,
        };
        shared.directory.domains().update(domain.clone()).unwrap();
        verify_request(&shared, domain.id).await;
        let stored = shared.directory.domains().get(domain.id).unwrap();
        assert!(matches!(stored.dns_status, DnsStatus::Failing { .. }));
    }

    #[tokio::test]
    async fn persisting_an_unchanged_status_is_a_no_op() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let status = DnsStatus::Verified { checked_at: 99 };
        persist_status(&shared.directory, &domain, status.clone()).unwrap();
        let stored = shared.directory.domains().get(domain.id).unwrap();
        persist_status(&shared.directory, &stored, status.clone()).unwrap();
        let again = shared.directory.domains().get(domain.id).unwrap();
        assert_eq!(again.dns_status, status);
        assert_eq!(again.name, "example.com");
    }
}
