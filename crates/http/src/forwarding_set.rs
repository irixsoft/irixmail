use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use irixmail_directory::Forwarding;

use crate::app::{error_response, AppState};
use crate::validate::{bad_request, is_valid_email, parse_id};

#[derive(Deserialize)]
pub struct ForwardingBody {
    pub destinations: Vec<String>,
    #[serde(rename = "keepLocalCopy", default)]
    pub keep_local_copy: bool,
}

pub async fn set(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ForwardingBody>,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return error_response(StatusCode::NOT_FOUND, "account not found");
    };
    for destination in &body.destinations {
        if !is_valid_email(destination) {
            return bad_request("invalid forwarding address");
        }
    }
    let mut account = match state.directory.accounts().get(id) {
        Ok(account) => account,
        Err(_) => return error_response(StatusCode::NOT_FOUND, "account not found"),
    };
    account.forwarding = Forwarding {
        destinations: body.destinations,
        keep_local_copy: body.keep_local_copy,
    };
    match state.directory.accounts().update(account.clone()) {
        Ok(()) => Json(json!({ "forwarding": account.forwarding })).into_response(),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not update forwarding",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request};
    use tower::ServiceExt;

    use irixmail_directory::Role;

    use crate::app::router;
    use crate::tests_support::{admin_token, state, TempDir};

    #[tokio::test]
    async fn forwarding_destinations_are_set() {
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
            .create("alice", domain.id, "Alice", Role::User)
            .unwrap();
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/accounts/{}/forwarding", account.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"destinations":["other@elsewhere.com"],"keepLocalCopy":true}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["forwarding"]["keep_local_copy"], true);
    }

    #[tokio::test]
    async fn an_invalid_destination_is_rejected() {
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
            .create("alice", domain.id, "Alice", Role::User)
            .unwrap();
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/accounts/{}/forwarding", account.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"destinations":["nope"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
