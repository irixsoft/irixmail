use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use irixmail_directory::{attempt_login_blocking, LoginAttempt, LoginPurpose, Role};

use crate::app::{error_response, AppState};
use crate::auth_mw::ClientIp;
use crate::sessions::{SessionKind, TokenInfo};

#[derive(Deserialize)]
pub struct LoginBody {
    pub kind: SessionKind,
    pub username: String,
    pub password: String,
}

pub fn session_response(state: &AppState, info: TokenInfo) -> Response {
    let is_admin = info.is_admin;
    match state.tokens.issue(info) {
        Ok(token) => (
            StatusCode::OK,
            Json(json!({ "token": token, "isAdmin": is_admin })),
        )
            .into_response(),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not store the session",
        ),
    }
}

fn admin_required() -> Response {
    error_response(StatusCode::FORBIDDEN, "administrator access required")
}

pub async fn login(
    State(state): State<AppState>,
    ClientIp(client_ip): ClientIp,
    Json(body): Json<LoginBody>,
) -> Response {
    let ip = client_ip.map(|ip| ip.to_string());
    let ip = ip.as_deref();
    let throttle = state.directory.throttle();
    if throttle.is_locked(ip, None) {
        return error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "too many failed login attempts",
        );
    }
    if let Some(admin) = state.directory.recovery_admin() {
        if admin.matches(&body.username) {
            let verified = {
                let admin = admin.clone();
                let username = body.username.clone();
                let password = body.password.clone();
                tokio::task::spawn_blocking(move || admin.verify(&username, &password)).await
            };
            return match verified {
                Ok(Ok(true)) => {
                    throttle.record_success(ip, None);
                    if body.kind != SessionKind::Admin {
                        return admin_required();
                    }
                    session_response(
                        &state,
                        TokenInfo {
                            account_id: 0,
                            username: body.username,
                            is_admin: true,
                            kind: SessionKind::Admin,
                        },
                    )
                }
                _ => {
                    throttle.record_failure(ip, None);
                    error_response(StatusCode::UNAUTHORIZED, "invalid credentials")
                }
            };
        }
    }
    match attempt_login_blocking(
        &state.directory,
        ip,
        &body.username,
        &body.password,
        LoginPurpose::Interactive,
    )
    .await
    {
        Ok(LoginAttempt::Throttled) => error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "too many failed login attempts",
        ),
        Ok(LoginAttempt::Granted(account, _))
            if body.kind == SessionKind::Admin && account.role != Role::Admin =>
        {
            admin_required()
        }
        Ok(LoginAttempt::Granted(account, by)) if by.mfa_required() => {
            state
                .totp_pending
                .begin(&body.username, account.id, body.kind);
            (StatusCode::OK, Json(json!({ "totpRequired": true }))).into_response()
        }
        Ok(LoginAttempt::Granted(account, _)) => session_response(
            &state,
            TokenInfo {
                account_id: account.id,
                username: body.username,
                is_admin: account.role == Role::Admin,
                kind: body.kind,
            },
        ),
        _ => error_response(StatusCode::UNAUTHORIZED, "invalid credentials"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    use irixmail_directory::{password, RecoveryAdmin};

    use crate::app::router;
    use crate::tests_support::{state, state_with_recovery, TempDir};

    fn recovery_admin(user: &str, secret: &str) -> RecoveryAdmin {
        let hash = password::hash(secret).unwrap();
        RecoveryAdmin::parse(&format!("{user}:{hash}")).unwrap()
    }

    async fn login_request(app: axum::Router, body: &str) -> axum::http::Response<Body> {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn json_value(response: axum::http::Response<Body>) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn provision(shared: &AppState, local: &str, role: Role, secret: &str) -> u64 {
        let domain = match shared.directory.domains().get_by_name("example.com").unwrap() {
            Some(existing) => existing,
            None => shared
                .directory
                .domains()
                .create("example.com", Vec::new())
                .unwrap(),
        };
        let account = shared
            .directory
            .accounts()
            .create(local, domain.id, local, role)
            .unwrap();
        shared
            .directory
            .credentials()
            .set_primary_password(account.id, password::hash(secret).unwrap())
            .unwrap();
        account.id
    }

    #[tokio::test]
    async fn the_recovery_admin_logs_in_without_a_stored_account() {
        let dir = TempDir::new();
        let shared = state_with_recovery(&dir, recovery_admin("root", "break-glass"));
        let app = router(shared);
        let response = login_request(
            app,
            r#"{"kind":"admin","username":"root","password":"break-glass"}"#,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let value = json_value(response).await;
        assert!(value["token"]
            .as_str()
            .is_some_and(|token| !token.is_empty()));
        assert_eq!(value["isAdmin"], true);
    }

    #[tokio::test]
    async fn the_recovery_admin_cannot_open_a_webmail_session() {
        let dir = TempDir::new();
        let shared = state_with_recovery(&dir, recovery_admin("root", "break-glass"));
        let app = router(shared);
        let response = login_request(
            app,
            r#"{"kind":"webmail","username":"root","password":"break-glass"}"#,
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn the_recovery_admin_with_a_wrong_secret_is_unauthorized() {
        let dir = TempDir::new();
        let shared = state_with_recovery(&dir, recovery_admin("root", "break-glass"));
        let app = router(shared);
        let response = login_request(
            app,
            r#"{"kind":"admin","username":"root","password":"wrong"}"#,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_valid_login_returns_a_session_token_of_the_requested_kind() {
        let dir = TempDir::new();
        let shared = state(&dir);
        provision(&shared, "alice", Role::User, "correct horse");
        let response = login_request(
            router(shared.clone()),
            r#"{"kind":"webmail","username":"alice@example.com","password":"correct horse"}"#,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let value = json_value(response).await;
        let token = value["token"].as_str().unwrap();
        assert_eq!(
            shared.tokens.validate(token).map(|info| info.kind),
            Some(SessionKind::Webmail)
        );
    }

    #[tokio::test]
    async fn a_non_admin_cannot_open_an_admin_session() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let account_id = provision(&shared, "alice", Role::User, "correct horse");
        let response = login_request(
            router(shared.clone()),
            r#"{"kind":"admin","username":"alice@example.com","password":"correct horse"}"#,
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(shared.tokens.revoke_account(account_id).unwrap(), 0);
    }

    #[tokio::test]
    async fn an_admin_account_opens_an_admin_session() {
        let dir = TempDir::new();
        let shared = state(&dir);
        provision(&shared, "root", Role::Admin, "hunter2");
        let response = login_request(
            router(shared.clone()),
            r#"{"kind":"admin","username":"root@example.com","password":"hunter2"}"#,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let value = json_value(response).await;
        assert_eq!(value["isAdmin"], true);
        let token = value["token"].as_str().unwrap();
        assert_eq!(
            shared.tokens.validate(token).map(|info| info.kind),
            Some(SessionKind::Admin)
        );
    }

    #[tokio::test]
    async fn a_wrong_password_is_unauthorized() {
        let dir = TempDir::new();
        let shared = state(&dir);
        provision(&shared, "alice", Role::User, "correct horse");
        let response = login_request(
            router(shared),
            r#"{"kind":"webmail","username":"alice@example.com","password":"wrong"}"#,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn an_unknown_account_is_unauthorized() {
        let dir = TempDir::new();
        let app = router(state(&dir));
        let response = login_request(
            app,
            r#"{"kind":"webmail","username":"ghost@example.com","password":"x"}"#,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_login_without_a_kind_is_rejected() {
        let dir = TempDir::new();
        let shared = state(&dir);
        provision(&shared, "alice", Role::User, "correct horse");
        let response = login_request(
            router(shared),
            r#"{"username":"alice@example.com","password":"correct horse"}"#,
        )
        .await;
        assert!(response.status().is_client_error());
    }

    async fn provisioned_app(dir: &TempDir) -> axum::Router {
        let shared = state(dir);
        provision(&shared, "alice", Role::User, "correct horse");
        router(shared)
    }

    #[tokio::test]
    async fn repeated_failed_logins_lock_the_account() {
        let dir = TempDir::new();
        let app = provisioned_app(&dir).await;
        let wrong = r#"{"kind":"webmail","username":"alice@example.com","password":"wrong"}"#;
        for _ in 0..5 {
            let response = login_request(app.clone(), wrong).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        let right =
            r#"{"kind":"webmail","username":"alice@example.com","password":"correct horse"}"#;
        let response = login_request(app.clone(), right).await;
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn an_unknown_account_is_not_a_timing_oracle() {
        async fn timed(app: &axum::Router, body: &str) -> std::time::Duration {
            let started = std::time::Instant::now();
            let response = login_request(app.clone(), body).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            started.elapsed()
        }

        fn median(mut samples: Vec<std::time::Duration>) -> std::time::Duration {
            samples.sort();
            samples[samples.len() / 2]
        }

        let dir = TempDir::new();
        let app = provisioned_app(&dir).await;
        let wrong = r#"{"kind":"webmail","username":"alice@example.com","password":"wrong"}"#;
        let unknown = r#"{"kind":"webmail","username":"ghost@example.com","password":"wrong"}"#;
        timed(&app, wrong).await;
        timed(&app, unknown).await;

        let mut known_samples = Vec::new();
        let mut unknown_samples = Vec::new();
        for _ in 0..3 {
            known_samples.push(timed(&app, wrong).await);
            unknown_samples.push(timed(&app, unknown).await);
        }
        let known = median(known_samples);
        let unknown = median(unknown_samples);
        assert!(
            unknown * 3 >= known,
            "an unknown account answers in {unknown:?} while a wrong password takes {known:?}"
        );
    }

    #[tokio::test]
    async fn a_malformed_body_is_rejected() {
        let dir = TempDir::new();
        let app = router(state(&dir));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from("not json"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response.status().is_client_error());
    }
}
