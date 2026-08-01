use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::app::{error_response, AppState};

pub async fn retry(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Ok(id) = id.parse::<u32>() else {
        return error_response(StatusCode::NOT_FOUND, "queued message not found");
    };
    let now = irixmail_tls::acme_http01::unix_now();
    match irixmail_smtp::retry_now(state.store.as_ref(), id, now) {
        Ok(true) => {
            if let Some(wakeups) = &state.queue_wakeups {
                let _ = wakeups.try_send(());
            }
            Json(json!({ "retried": true })).into_response()
        }
        Ok(false) => error_response(StatusCode::NOT_FOUND, "queued message not found"),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not schedule the retry",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt;

    use crate::app::router;
    use crate::tests_support::{admin_token, state, TempDir};

    #[tokio::test]
    async fn a_retry_makes_the_message_due_and_wakes_the_manager() {
        use std::sync::Arc;

        let dir = TempDir::new();
        let mut shared = state(&dir);
        let (wakeup, mut wakeups) = irixmail_smtp::wakeup_channel();
        shared.queue_wakeups = Some(wakeup);
        let token = admin_token(&shared);
        let store = Arc::clone(&shared.store);
        let blobs = Arc::clone(&shared.blobs);
        let recipients = vec![(
            "rcpt@remote.example".to_string(),
            irixmail_smtp::Expiry::Attempts(5),
        )];
        let request = irixmail_smtp::Enqueue {
            created: 1_000,
            return_path: "sender@local.example",
            recipients: &recipients,
            first_due: 99_999_999_999,
        };
        let enqueued =
            irixmail_smtp::enqueue(store.as_ref(), blobs.as_ref(), b"body", &request).unwrap();

        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/queue/{}/retry", enqueued.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let message = irixmail_smtp::load(store.as_ref(), enqueued.id)
            .unwrap()
            .unwrap();
        assert!(message.next_due().unwrap() < 99_999_999_999);
        assert!(wakeups.try_recv().is_ok());
    }

    #[tokio::test]
    async fn retrying_an_unknown_message_is_not_found() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/queue/abc/retry")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
