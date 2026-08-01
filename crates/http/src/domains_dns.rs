use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use irixmail_dns::{domain_records, DomainRecordsInput};

use crate::app::{error_response, AppState};
use crate::validate::parse_id;

pub async fn dns(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(id) = parse_id(&id) else {
        return error_response(StatusCode::NOT_FOUND, "domain not found");
    };
    let domain = match state.directory.domains().get(id) {
        Ok(domain) => domain,
        Err(_) => return error_response(StatusCode::NOT_FOUND, "domain not found"),
    };
    let dkim_keys = match state.directory.dkim().get(id) {
        Ok(Some(key)) => vec![key],
        Ok(None) => Vec::new(),
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
    let zone = irixmail_dns::zone_file(&domain.name, &records);
    Json(json!({
        "domain": domain.name,
        "status": domain.dns_status,
        "records": records,
        "zone": zone,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request};
    use tower::ServiceExt;

    use crate::app::router;
    use crate::tests_support::{admin_token, state, TempDir};

    #[tokio::test]
    async fn the_required_records_are_generated() {
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
                    .uri(format!("/api/domains/{}/dns", domain.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value["records"].as_array().unwrap().len() >= 3);
    }

    #[tokio::test]
    async fn the_bundle_carries_address_records_for_the_detected_public_ip() {
        let dir = TempDir::new();
        let mut shared = state(&dir);
        shared.public_ipv4 = Some("198.51.100.7".parse().unwrap());
        shared.public_ipv6 = Some("2001:db8::7".parse().unwrap());
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
                    .uri(format!("/api/domains/{}/dns", domain.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let records = value["records"].as_array().unwrap();
        assert!(
            records.iter().any(|r| r["record_type"] == "A"
                && r["name"] == "mail.example.com"
                && r["value"] == "198.51.100.7"),
            "the bundle must include an A record for the detected public IPv4: {records:?}"
        );
        assert!(
            records.iter().any(|r| r["record_type"] == "AAAA"
                && r["name"] == "mail.example.com"
                && r["value"] == "2001:db8::7"),
            "the bundle must include an AAAA record for the detected public IPv6: {records:?}"
        );
    }

    #[tokio::test]
    async fn the_dns_bundle_includes_a_zone_file() {
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
                    .uri(format!("/api/domains/{}/dns", domain.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let zone = value["zone"].as_str().unwrap();
        assert!(zone.contains("$ORIGIN example.com."), "{zone}");
        assert!(zone.contains("IN\tMX"), "{zone}");
    }
}
