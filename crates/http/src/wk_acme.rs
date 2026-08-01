use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::app::{error_response, AppState};

pub async fn acme_challenge(State(state): State<AppState>, Path(token): Path<String>) -> Response {
    match state.tls.as_ref().and_then(|tls| {
        tls.http01
            .get(&token, irixmail_tls::acme_http01::unix_now())
    }) {
        Some(key_authorization) => (StatusCode::OK, key_authorization).into_response(),
        None => error_response(StatusCode::NOT_FOUND, "no active ACME challenge"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    use axum::body::to_bytes;

    use crate::app::TlsHandles;
    use crate::tests_support::{state, TempDir};

    #[tokio::test]
    async fn an_inactive_challenge_token_is_not_found() {
        let dir = TempDir::new();
        let app = Router::new()
            .route("/.well-known/acme-challenge/{token}", get(acme_challenge))
            .with_state(state(&dir));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/.well-known/acme-challenge/abc123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn an_active_challenge_token_is_served_with_its_key_authorization() {
        let dir = TempDir::new();
        let mut shared = state(&dir);
        let handles = TlsHandles::default();
        handles.http01.insert(
            "valid-token",
            "valid-token.keyauth",
            irixmail_tls::acme_http01::unix_now(),
        );
        shared.tls = Some(handles);
        let app = Router::new()
            .route("/.well-known/acme-challenge/{token}", get(acme_challenge))
            .with_state(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/.well-known/acme-challenge/valid-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(bytes.as_ref(), b"valid-token.keyauth");
    }
}
