use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use irixmail_directory::app_password;

use crate::app::{error_response, AppState};
use crate::validate::parse_id;

pub async fn list(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(id) = parse_id(&id) else {
        return error_response(StatusCode::NOT_FOUND, "account not found");
    };
    if state.directory.accounts().get(id).is_err() {
        return error_response(StatusCode::NOT_FOUND, "account not found");
    }
    let Ok(stored) = state.directory.credentials().list_app_passwords(id) else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not read app passwords",
        );
    };
    let listed: Vec<_> = app_password::list(&stored)
        .into_iter()
        .map(|record| {
            json!({
                "id": record.id.to_string(),
                "name": record.name,
                "createdAt": record.created_at,
                "lastUsedAt": record.last_used_at,
            })
        })
        .collect();
    Json(json!({ "appPasswords": listed })).into_response()
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
    async fn app_passwords_are_listed_for_an_account() {
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
        let minted = app_password::generate(11, "Thunderbird", 1_000).unwrap();
        shared
            .directory
            .credentials()
            .add_app_password(account.id, minted.record.clone())
            .unwrap();
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/accounts/{}/app-passwords", account.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let entries = value["appPasswords"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["id"], "11");
        assert_eq!(entries[0]["name"], "Thunderbird");
        let rendered = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(!rendered.contains(&minted.record.hash));
    }
}
