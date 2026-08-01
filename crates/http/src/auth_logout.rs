use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::app::AppState;

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|header| header.strip_prefix("Bearer "))
    {
        state.tokens.revoke(token.trim());
    }
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    use crate::app::{router, TokenInfo};
    use crate::tests_support::{state, TempDir};

    #[tokio::test]
    async fn logging_out_revokes_the_token() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = shared.tokens.issue(TokenInfo {
            account_id: 1,
            username: "a@b.com".into(),
            is_admin: true,
        });
        assert!(shared.tokens.validate(&token).is_some());

        let app = router(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/logout")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(shared.tokens.validate(&token).is_none());
    }
}
