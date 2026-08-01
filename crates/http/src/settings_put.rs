use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use irixmail_store::WriteOp;

use crate::app::{error_response, AppState};
use crate::settings_get::settings_key;

pub async fn put(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let mut stored = match state.store.get(&settings_key()) {
        Ok(Some(bytes)) => serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({})),
        _ => json!({}),
    };
    crate::settings_get::merge(&mut stored, &body);
    let Ok(bytes) = serde_json::to_vec(&stored) else {
        return error_response(StatusCode::BAD_REQUEST, "invalid settings");
    };
    let op = WriteOp::Set {
        key: settings_key(),
        value: bytes,
    };
    match state.store.batch(&[op]) {
        Ok(()) => Json(json!({ "ok": true, "settings": stored })).into_response(),
        Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "could not save settings"),
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

    async fn put_json(app: &axum::Router, token: &str, body: &str) -> StatusCode {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/settings")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        response.status()
    }

    async fn get_settings(app: &axum::Router, token: &str) -> Value {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn submitted_settings_are_persisted_and_read_back() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);

        let status = put_json(
            &app,
            &token,
            r#"{"antiSpam":{"greylistWindowSeconds":300}}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let value = get_settings(&app, &token).await;
        assert_eq!(value["antiSpam"]["greylistWindowSeconds"], 300);
    }

    #[tokio::test]
    async fn a_partial_put_preserves_the_untouched_sections() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);

        put_json(
            &app,
            &token,
            r#"{"antiSpam":{"greylistWindowSeconds":300}}"#,
        )
        .await;

        let value = get_settings(&app, &token).await;
        assert_eq!(
            value["rateLimits"]["maxConnectionsPerIp"],
            serde_json::json!(irixmail_smtp::DEFAULT_MAX_CONNECTIONS),
            "a partial PUT must not truncate the sections it did not name"
        );
    }

    #[tokio::test]
    async fn successive_partial_puts_merge_instead_of_replacing() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);

        put_json(&app, &token, r#"{"rateLimits":{"maxConnectionsPerIp":5}}"#).await;
        put_json(
            &app,
            &token,
            r#"{"rateLimits":{"maxMessagesPerConnection":7}}"#,
        )
        .await;

        let value = get_settings(&app, &token).await;
        assert_eq!(value["rateLimits"]["maxConnectionsPerIp"], 5);
        assert_eq!(value["rateLimits"]["maxMessagesPerConnection"], 7);
    }
}
