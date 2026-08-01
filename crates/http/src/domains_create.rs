use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::app::{error_response, AppState};
use crate::domains_list::domain_json;
use crate::validate::{bad_request, is_valid_domain};

#[derive(Deserialize)]
pub struct CreateBody {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}

pub async fn create(State(state): State<AppState>, Json(body): Json<CreateBody>) -> Response {
    if !is_valid_domain(&body.name) {
        return bad_request("invalid domain name");
    }
    if state
        .directory
        .domains()
        .get_by_name(&body.name)
        .ok()
        .flatten()
        .is_some()
    {
        return error_response(StatusCode::CONFLICT, "domain already exists");
    }
    let domain = match state.directory.domains().create(&body.name, body.aliases) {
        Ok(domain) => domain,
        Err(_) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "could not create domain")
        }
    };
    if state
        .directory
        .dkim()
        .get_or_create(domain.id, "default")
        .is_err()
    {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not generate the DKIM key",
        );
    }
    (
        StatusCode::CREATED,
        Json(json!({ "domain": domain_json(&domain) })),
    )
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

    fn post(token: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/domains")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn a_valid_domain_is_created() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(post(&token, r#"{"name":"example.com"}"#))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let id = value["domain"]["id"].as_str().unwrap();
        assert!(id.parse::<u64>().unwrap() > (1u64 << 53));
    }

    #[tokio::test]
    async fn an_invalid_domain_is_rejected() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(post(&token, r#"{"name":"localhost"}"#))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_duplicate_domain_is_a_conflict() {
        let dir = TempDir::new();
        let shared = state(&dir);
        shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(post(&token, r#"{"name":"example.com"}"#))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }
}
