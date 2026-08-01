use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;

use irixmail_directory::{
    app_password, authenticate_blocking, password, Authentication, LoginPurpose,
};

use crate::app::{error_response, AppState};
use crate::auth_mw::AuthIdentity;
use crate::validate::{bad_request, parse_id};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasswordBody {
    #[serde(default)]
    pub current_password: String,
    #[serde(default)]
    pub new_password: String,
}

pub async fn change_password(
    State(state): State<AppState>,
    Extension(identity): Extension<AuthIdentity>,
    Json(body): Json<PasswordBody>,
) -> Response {
    if body.new_password.trim().is_empty() {
        return bad_request("a new password is required");
    }
    let Ok(account) = state.directory.accounts().get(identity.account_id) else {
        return error_response(StatusCode::NOT_FOUND, "account not found");
    };
    let Ok(stored) = state.directory.credentials().list(identity.account_id) else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not read credentials",
        );
    };
    match authenticate_blocking(
        &account,
        &stored,
        LoginPurpose::Interactive,
        &body.current_password,
    )
    .await
    {
        Ok(Authentication::Granted(_)) => {}
        _ => return error_response(StatusCode::FORBIDDEN, "the current password is incorrect"),
    }
    let Ok(hash) = password::hash(&body.new_password) else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not hash the password",
        );
    };
    match state
        .directory
        .credentials()
        .set_primary_password(identity.account_id, hash)
    {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not store the password",
        ),
    }
}

pub async fn list_app_passwords(
    State(state): State<AppState>,
    Extension(identity): Extension<AuthIdentity>,
) -> Response {
    let Ok(stored) = state
        .directory
        .credentials()
        .list_app_passwords(identity.account_id)
    else {
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

#[derive(Deserialize)]
pub struct CreateAppPasswordBody {
    #[serde(default)]
    pub name: String,
}

pub async fn create_app_password(
    State(state): State<AppState>,
    Extension(identity): Extension<AuthIdentity>,
    Json(body): Json<CreateAppPasswordBody>,
) -> Response {
    let name = if body.name.trim().is_empty() {
        "app password".to_string()
    } else {
        body.name
    };
    let Ok(generated) = app_password::generate(rand::random::<u64>(), &name, now_millis()) else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not generate app password",
        );
    };
    if state
        .directory
        .credentials()
        .add_app_password(identity.account_id, generated.record.clone())
        .is_err()
    {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not store the app password",
        );
    }
    (
        StatusCode::CREATED,
        Json(json!({
            "id": generated.record.id.to_string(),
            "name": generated.record.name,
            "password": generated.plaintext,
        })),
    )
        .into_response()
}

pub async fn delete_app_password(
    State(state): State<AppState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(pid): Path<String>,
) -> Response {
    let Some(pid) = parse_id(&pid) else {
        return error_response(StatusCode::NOT_FOUND, "app password not found");
    };
    match state
        .directory
        .credentials()
        .revoke_app_password(identity.account_id, pid)
    {
        Ok(true) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "app password not found"),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not revoke the app password",
        ),
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request};
    use tower::ServiceExt;

    use irixmail_directory::Role;

    use crate::app::router;
    use crate::app::TokenInfo;
    use crate::tests_support::{state, TempDir};

    fn user_token(shared: &AppState, account_id: u64) -> String {
        shared.tokens.issue(TokenInfo {
            account_id,
            username: "alice@example.com".into(),
            is_admin: false,
        })
    }

    #[tokio::test]
    async fn a_user_changes_their_own_password() {
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
        let hash = irixmail_directory::password::hash("old secret").unwrap();
        shared
            .directory
            .credentials()
            .set_primary_password(account.id, hash)
            .unwrap();
        let token = user_token(&shared, account.id);
        let app = router(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/me/password")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"currentPassword":"old secret","newPassword":"new secret"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn a_wrong_current_password_is_rejected() {
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
        let hash = irixmail_directory::password::hash("old secret").unwrap();
        shared
            .directory
            .credentials()
            .set_primary_password(account.id, hash)
            .unwrap();
        let token = user_token(&shared, account.id);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/me/password")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"currentPassword":"wrong","newPassword":"new secret"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn an_app_password_cannot_change_the_primary_password_over_basic_auth() {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine as _;
        use irixmail_directory::{app_password, authenticate, Authentication, LoginPurpose};

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
        let hash = irixmail_directory::password::hash("old secret").unwrap();
        shared
            .directory
            .credentials()
            .set_primary_password(account.id, hash)
            .unwrap();
        let minted = app_password::generate(1, "phone", 0).unwrap();
        shared
            .directory
            .credentials()
            .add_app_password(account.id, minted.record)
            .unwrap();

        let encoded = STANDARD.encode(format!("alice@example.com:{}", minted.plaintext));
        let body = format!(
            r#"{{"currentPassword":"{}","newPassword":"attacker"}}"#,
            minted.plaintext
        );
        let app = router(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/me/password")
                    .header(header::AUTHORIZATION, format!("Basic {encoded}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let stored = shared.directory.credentials().list(account.id).unwrap();
        let outcome = authenticate(
            &shared.directory.accounts().get(account.id).unwrap(),
            &stored,
            LoginPurpose::Interactive,
            "old secret",
        )
        .unwrap();
        assert!(
            matches!(outcome, Authentication::Granted(_)),
            "the primary password must be unchanged"
        );
    }

    #[tokio::test]
    async fn an_app_password_is_not_accepted_as_the_current_password() {
        use irixmail_directory::app_password;

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
        let hash = irixmail_directory::password::hash("old secret").unwrap();
        shared
            .directory
            .credentials()
            .set_primary_password(account.id, hash)
            .unwrap();
        let minted = app_password::generate(1, "phone", 0).unwrap();
        shared
            .directory
            .credentials()
            .add_app_password(account.id, minted.record)
            .unwrap();

        let token = user_token(&shared, account.id);
        let body = format!(
            r#"{{"currentPassword":"{}","newPassword":"attacker"}}"#,
            minted.plaintext
        );
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/me/password")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn an_app_password_round_trips_for_the_authenticated_user() {
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
        let token = user_token(&shared, account.id);
        let created = router(shared.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/me/app-passwords")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"Phone"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let bytes = to_bytes(created.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value["password"].as_str().is_some());
        let id = value["id"].as_str().unwrap().to_string();

        let listed = shared
            .directory
            .credentials()
            .list_app_passwords(account.id)
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(id.parse::<u64>().unwrap(), listed[0].id);

        let response = router(shared.clone())
            .oneshot(
                Request::builder()
                    .uri("/api/me/app-passwords")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["appPasswords"][0]["id"].as_str().unwrap(), id);

        let revoked = router(shared.clone())
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/me/app-passwords/{id}"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoked.status(), StatusCode::OK);
        assert!(shared
            .directory
            .credentials()
            .list_app_passwords(account.id)
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn a_non_numeric_app_password_id_is_not_found() {
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
        let token = user_token(&shared, account.id);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/me/app-passwords/not-a-number")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
