use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use irixmail_directory::password;

use crate::app::{error_response, AppState};
use crate::validate::{bad_request, parse_id};

#[derive(Deserialize)]
pub struct PasswordBody {
    pub password: String,
}

pub async fn set(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PasswordBody>,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return error_response(StatusCode::NOT_FOUND, "account not found");
    };
    if state.directory.accounts().get(id).is_err() {
        return error_response(StatusCode::NOT_FOUND, "account not found");
    }
    if body.password.trim().is_empty() {
        return bad_request("a password is required");
    }
    let Ok(hash) = password::hash(&body.password) else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not hash the password",
        );
    };
    match state.directory.credentials().set_primary_password(id, hash) {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not store the password",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt;

    use irixmail_directory::{authenticate, Authentication, LoginPurpose, Role};

    use crate::app::router;
    use crate::tests_support::{admin_token, state, TempDir};

    #[tokio::test]
    async fn setting_a_password_lets_the_account_authenticate() {
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
        let app = router(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/accounts/{}/password", account.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"password":"correct horse"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let stored = shared.directory.credentials().list(account.id).unwrap();
        let outcome = authenticate(&account, &stored, LoginPurpose::Mail, "correct horse").unwrap();
        assert!(matches!(outcome, Authentication::Granted(_)));
    }

    #[tokio::test]
    async fn an_empty_password_is_rejected() {
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
                    .uri(format!("/api/accounts/{}/password", account.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"password":"   "}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn an_unknown_account_is_not_found() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/accounts/999/password")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"password":"whatever"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
