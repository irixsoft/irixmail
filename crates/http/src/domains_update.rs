use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::app::{error_response, AppState};
use crate::domains_list::domain_json;
use crate::validate::parse_id;

#[derive(Deserialize)]
pub struct UpdateBody {
    pub enabled: Option<bool>,
    pub aliases: Option<Vec<String>>,
    #[serde(
        rename = "catchAllAccountId",
        default,
        deserialize_with = "present_or_null"
    )]
    pub catch_all_account_id: Option<Option<String>>,
}

fn present_or_null<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::deserialize(deserializer).map(Some)
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateBody>,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return error_response(StatusCode::NOT_FOUND, "domain not found");
    };
    let mut domain = match state.directory.domains().get(id) {
        Ok(domain) => domain,
        Err(_) => return error_response(StatusCode::NOT_FOUND, "domain not found"),
    };
    if let Some(enabled) = body.enabled {
        domain.enabled = enabled;
    }
    if let Some(aliases) = body.aliases {
        domain.aliases = aliases;
    }
    if let Some(catch_all) = body.catch_all_account_id {
        domain.catch_all_account_id = match catch_all {
            Some(raw) => match parse_id(&raw) {
                Some(account_id) => Some(account_id),
                None => return error_response(StatusCode::NOT_FOUND, "account not found"),
            },
            None => None,
        };
    }
    match state.directory.domains().update(domain.clone()) {
        Ok(()) => Json(json!({ "domain": domain_json(&domain) })).into_response(),
        Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "could not update domain"),
    }
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
    async fn a_domain_can_be_disabled() {
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
                    .method("PUT")
                    .uri(format!("/api/domains/{}", domain.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"enabled":false}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["domain"]["enabled"], false);
        assert_eq!(
            value["domain"]["id"].as_str().unwrap(),
            domain.id.to_string()
        );
    }

    #[tokio::test]
    async fn a_string_catch_all_account_id_is_accepted() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let account = shared
            .directory
            .accounts()
            .create("alice", domain.id, "Alice", irixmail_directory::Role::User)
            .unwrap();
        let token = admin_token(&shared);
        let app = router(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/domains/{}", domain.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"catchAllAccountId":"{}"}}"#,
                        account.id
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value["domain"]["catch_all_account_id"].as_str().unwrap(),
            account.id.to_string()
        );
        assert_eq!(
            shared
                .directory
                .domains()
                .get(domain.id)
                .unwrap()
                .catch_all_account_id,
            Some(account.id)
        );
    }

    #[tokio::test]
    async fn a_null_catch_all_account_id_clears_it() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let mut domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let account = shared
            .directory
            .accounts()
            .create("alice", domain.id, "Alice", irixmail_directory::Role::User)
            .unwrap();
        domain.catch_all_account_id = Some(account.id);
        shared.directory.domains().update(domain.clone()).unwrap();
        let token = admin_token(&shared);
        let app = router(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/domains/{}", domain.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"catchAllAccountId":null}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value["domain"]["catch_all_account_id"].is_null());
        assert_eq!(
            shared
                .directory
                .domains()
                .get(domain.id)
                .unwrap()
                .catch_all_account_id,
            None
        );
    }

    #[tokio::test]
    async fn a_non_numeric_catch_all_account_id_is_not_found() {
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
                    .method("PUT")
                    .uri(format!("/api/domains/{}", domain.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"catchAllAccountId":"nope"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_non_numeric_domain_id_is_not_found() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/domains/not-a-number")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn updating_an_unknown_domain_is_not_found() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/domains/999")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
