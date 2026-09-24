use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::app::{error_response, AppState};
use crate::auth_mw::{authenticate_request, AuthMethod};

pub async fn session(State(state): State<AppState>, request: Request) -> Response {
    match authenticate_request(&state, &request).await {
        Some(identity) if identity.method == AuthMethod::Session => (
            StatusCode::OK,
            Json(json!({
                "username": identity.username,
                "isAdmin": identity.is_admin,
                "kind": identity.kind.map(|kind| kind.as_str()),
            })),
        )
            .into_response(),
        _ => error_response(StatusCode::UNAUTHORIZED, "authentication required"),
    }
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    use crate::app::router;
    use crate::tests_support::{admin_token, state, webmail_token, TempDir};

    async fn fetch(app: axum::Router, token: &str) -> (StatusCode, serde_json::Value) {
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/auth/session")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn a_session_token_describes_itself() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let (status, value) = fetch(router(shared.clone()), &admin_token(&shared)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["kind"], "admin");
        assert_eq!(value["isAdmin"], true);
        let (status, value) = fetch(router(shared.clone()), &webmail_token(&shared, 2)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["kind"], "webmail");
    }

    #[tokio::test]
    async fn an_api_key_or_unknown_token_is_unauthorized() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let (_, plaintext) = shared
            .directory
            .api_keys()
            .create("ci", &shared.secrets)
            .unwrap();
        let (status, _) = fetch(router(shared.clone()), &plaintext).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _) = fetch(router(shared), "nope").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}
