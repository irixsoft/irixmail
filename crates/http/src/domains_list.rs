use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use irixmail_directory::Domain;

use crate::app::AppState;

pub async fn list(State(state): State<AppState>) -> Json<Value> {
    let domains = state.directory.domains().list().unwrap_or_default();
    let domains: Vec<Value> = domains.iter().map(domain_json).collect();
    Json(json!({ "domains": domains }))
}

pub fn domain_json(domain: &Domain) -> Value {
    let mut value = serde_json::to_value(domain).unwrap_or_else(|_| json!({}));
    if let Some(fields) = value.as_object_mut() {
        fields.insert("id".into(), json!(domain.id.to_string()));
        fields.insert(
            "catch_all_account_id".into(),
            match domain.catch_all_account_id {
                Some(id) => json!(id.to_string()),
                None => Value::Null,
            },
        );
        let key_ids: Vec<String> = domain
            .dkim_key_ids
            .iter()
            .map(|id| id.to_string())
            .collect();
        fields.insert("dkim_key_ids".into(), json!(key_ids));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    use crate::app::router;
    use crate::tests_support::{admin_token, state, TempDir};

    #[tokio::test]
    async fn an_admin_lists_domains() {
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
                    .uri("/api/domains")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["domains"][0]["name"], "example.com");
        assert!(domain.id > (1u64 << 53));
        let id = value["domains"][0]["id"].as_str().unwrap();
        assert_eq!(id, domain.id.to_string());
        assert_eq!(id.parse::<u64>().unwrap(), domain.id);
        assert!(value["domains"][0]["catch_all_account_id"].is_null());
        assert!(value["domains"][0]["dkim_key_ids"].is_array());
    }

    #[tokio::test]
    async fn listing_requires_admin() {
        let dir = TempDir::new();
        let app = router(state(&dir));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/domains")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
