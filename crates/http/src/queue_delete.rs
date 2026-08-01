use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::app::{error_response, AppState};

pub async fn delete(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Ok(id) = id.parse::<u32>() else {
        return error_response(StatusCode::NOT_FOUND, "queued message not found");
    };
    match irixmail_smtp::load(state.store.as_ref(), id) {
        Ok(Some(_)) => match irixmail_smtp::remove(state.store.as_ref(), id) {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(_) => error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not delete the queued message",
            ),
        },
        Ok(None) => error_response(StatusCode::NOT_FOUND, "queued message not found"),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not delete the queued message",
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
    async fn deleting_a_queued_message_removes_its_record() {
        use std::sync::Arc;

        let dir = TempDir::new();
        let shared = state(&dir);
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
                    .method("DELETE")
                    .uri(format!("/api/queue/{}", enqueued.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(irixmail_smtp::load(store.as_ref(), enqueued.id)
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn deleting_an_unknown_message_is_not_found() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/queue/abc")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
