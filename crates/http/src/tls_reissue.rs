use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use tokio::sync::mpsc::error::TrySendError;

use crate::app::{error_response, AppState};

pub async fn reissue(State(state): State<AppState>) -> Response {
    let Some(sender) = state.tls.as_ref().and_then(|tls| tls.reissue.clone()) else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "TLS management is not available",
        );
    };
    match sender.try_send(()) {
        Ok(()) | Err(TrySendError::Full(())) => {
            (StatusCode::ACCEPTED, Json(json!({ "status": "reissuing" }))).into_response()
        }
        Err(TrySendError::Closed(())) => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "the TLS reissue worker is not running",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt;

    use crate::app::{router, TlsHandles};
    use crate::tests_support::{admin_token, state, TempDir};

    fn post(token: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/tls/reissue")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn a_reissue_signals_the_issuance_worker() {
        let dir = TempDir::new();
        let mut shared = state(&dir);
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        shared.tls = Some(TlsHandles {
            reissue: Some(sender),
            ..TlsHandles::default()
        });
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app.oneshot(post(&token)).await.unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        assert!(receiver.try_recv().is_ok());
    }

    #[tokio::test]
    async fn a_reissue_without_a_worker_is_unavailable() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app.oneshot(post(&token)).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
