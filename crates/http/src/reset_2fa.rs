use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::app::{error_response, AppState};
use crate::validate::parse_id;

pub async fn reset(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(id) = parse_id(&id) else {
        return error_response(StatusCode::NOT_FOUND, "account not found");
    };
    if state.directory.accounts().get(id).is_err() {
        return error_response(StatusCode::NOT_FOUND, "account not found");
    }
    if state.directory.credentials().clear_totp(id).is_err() {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not clear the enrollment",
        );
    }
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "twoFactorEnabled": false })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt;

    use irixmail_directory::Role;

    use crate::app::router;
    use crate::tests_support::{admin_token, state, TempDir};

    #[tokio::test]
    async fn resetting_two_factor_succeeds() {
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
                    .method("POST")
                    .uri(format!("/api/accounts/{}/reset-2fa", account.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn resetting_two_factor_clears_the_stored_enrollment() {
        use irixmail_directory::{totp as totp_service, Credential, Totp};

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
        let secret = totp_service::generate_secret().unwrap();
        shared
            .directory
            .credentials()
            .set_totp(
                account.id,
                Totp {
                    secret: shared.secrets.encrypt(&secret).unwrap(),
                    enabled: true,
                    recovery_codes: Vec::new(),
                    enrolled_at: 0,
                },
            )
            .unwrap();

        let token = admin_token(&shared);
        let app = router(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/accounts/{}/reset-2fa", account.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let remaining = shared.directory.credentials().list(account.id).unwrap();
        assert!(
            !remaining
                .iter()
                .any(|credential| matches!(credential, Credential::Totp(_))),
            "the enrollment must be deleted so the user can re-enroll"
        );
    }
}
