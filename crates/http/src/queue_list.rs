use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use irixmail_smtp::{DueMessage, QueueRecipient, RecipientStatus};
use irixmail_store::BlobStore;

use crate::app::{error_response, AppState};

pub async fn list(State(state): State<AppState>) -> Response {
    match irixmail_smtp::scan_all(state.store.as_ref()) {
        Ok(entries) => {
            let queue: Vec<Value> = entries
                .iter()
                .map(|entry| message_json(state.blobs.as_ref(), entry))
                .collect();
            Json(json!({ "queue": queue })).into_response()
        }
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not list the queue",
        ),
    }
}

fn message_json(blobs: &dyn BlobStore, entry: &DueMessage) -> Value {
    let message = &entry.message;
    let subject = blobs
        .get_all(&message.blob_hash())
        .ok()
        .flatten()
        .and_then(|raw| irixmail_mail::message_text(&raw).ok())
        .map(|text| text.subject)
        .filter(|subject| !subject.is_empty());
    json!({
        "id": entry.id.to_string(),
        "sender": message.return_path,
        "subject": subject,
        "status": "pending",
        "createdAt": message.created * 1000,
        "nextAttemptAt": message.next_due().map(|due| due * 1000),
        "recipients": message.recipients.iter().map(recipient_json).collect::<Vec<_>>(),
    })
}

fn recipient_json(recipient: &QueueRecipient) -> Value {
    let (status, last_error) = match &recipient.status {
        RecipientStatus::Scheduled => ("scheduled", None),
        RecipientStatus::Delivered => ("delivered", None),
        RecipientStatus::Deferred(reason) => ("deferred", Some(reason.as_str())),
        RecipientStatus::Bounced(reason) => ("bounced", Some(reason.as_str())),
    };
    json!({
        "address": recipient.address,
        "status": status,
        "lastError": last_error,
    })
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use serde_json::Value;
    use std::sync::Arc;
    use tower::ServiceExt;

    use crate::app::router;
    use crate::tests_support::{admin_token, state, TempDir};

    #[tokio::test]
    async fn the_queue_lists_a_real_message_with_its_recipients() {
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
        let enqueued = irixmail_smtp::enqueue(
            store.as_ref(),
            blobs.as_ref(),
            b"Subject: Hi there\r\nFrom: sender@local.example\r\n\r\nBody.\r\n",
            &request,
        )
        .unwrap();

        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/queue")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&bytes).unwrap();
        let queue = json["queue"].as_array().unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0]["id"], enqueued.id.to_string());
        assert_eq!(queue[0]["sender"], "sender@local.example");
        assert_eq!(queue[0]["subject"], "Hi there");
        assert_eq!(queue[0]["createdAt"], 1_000_000);
        assert_eq!(queue[0]["recipients"][0]["address"], "rcpt@remote.example");
        assert_eq!(queue[0]["recipients"][0]["status"], "scheduled");
    }

    #[tokio::test]
    async fn the_queue_requires_authentication() {
        let dir = TempDir::new();
        let app = router(state(&dir));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/queue")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
