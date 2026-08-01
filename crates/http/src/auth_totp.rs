use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use irixmail_directory::totp_flow::{self, ChallengeOutcome};
use irixmail_directory::{Credential, Role};

use crate::app::{error_response, AppState, TokenInfo};

#[derive(Deserialize)]
pub struct TotpBody {
    pub username: String,
    pub code: String,
}

pub async fn totp(State(state): State<AppState>, Json(body): Json<TotpBody>) -> Response {
    let code = body.code.trim();
    if code.is_empty() || code.len() > 64 {
        return error_response(StatusCode::BAD_REQUEST, "a verification code is required");
    }
    let Some(account_id) = state.totp_pending.take_attempt(&body.username) else {
        return error_response(StatusCode::UNAUTHORIZED, "invalid or expired code");
    };
    let Ok(account) = state.directory.accounts().get(account_id) else {
        return error_response(StatusCode::UNAUTHORIZED, "invalid or expired code");
    };
    let Ok(mut stored) = state.directory.credentials().list(account.id) else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not read credentials",
        );
    };
    match totp_flow::complete(&mut stored, code, unix_now(), |secret| {
        state.secrets.decrypt(secret)
    }) {
        Ok(ChallengeOutcome::Granted) => {
            let consumed = stored.into_iter().find_map(|credential| match credential {
                Credential::Totp(totp) => Some(totp),
                _ => None,
            });
            if let Some(totp) = consumed {
                if state
                    .directory
                    .credentials()
                    .set_totp(account.id, totp)
                    .is_err()
                {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "could not update credentials",
                    );
                }
            }
            state.totp_pending.complete(&body.username);
            let is_admin = account.role == Role::Admin;
            let token = state.tokens.issue(TokenInfo {
                account_id: account.id,
                username: body.username,
                is_admin,
            });
            (
                StatusCode::OK,
                Json(json!({ "token": token, "isAdmin": is_admin })),
            )
                .into_response()
        }
        Ok(ChallengeOutcome::Denied) => {
            error_response(StatusCode::UNAUTHORIZED, "invalid or expired code")
        }
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not verify the code",
        ),
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request};
    use tower::ServiceExt;

    use irixmail_directory::{password, totp as totp_service, Role, Totp};

    use crate::app::{router, AppState};
    use crate::tests_support::{state, TempDir};

    fn enroll(shared: &AppState, account_id: u64, secret: &[u8], recovery_codes: Vec<String>) {
        shared
            .directory
            .credentials()
            .set_totp(
                account_id,
                Totp {
                    secret: shared.secrets.encrypt(secret).unwrap(),
                    enabled: true,
                    recovery_codes,
                    enrolled_at: 0,
                },
            )
            .unwrap();
    }

    fn account_with_password(shared: &AppState, plaintext: &str) -> u64 {
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
        let hash = password::hash(plaintext).unwrap();
        shared
            .directory
            .credentials()
            .set_primary_password(account.id, hash)
            .unwrap();
        account.id
    }

    fn unix_time() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn current_code(secret: &[u8], unix_time: u64) -> String {
        let code = totp_rs::TOTP::new(
            totp_rs::Algorithm::SHA1,
            6,
            1,
            30,
            secret.to_vec(),
            None,
            String::new(),
        )
        .unwrap()
        .generate(unix_time);
        assert!(totp_service::verify_code(secret, &code, unix_time).unwrap());
        code
    }

    fn wrong_code(secret: &[u8], unix_time: u64) -> String {
        if current_code(secret, unix_time) == "000000" {
            "000001".to_string()
        } else {
            "000000".to_string()
        }
    }

    async fn post_json(app: axum::Router, uri: &str, body: String) -> axum::http::Response<Body> {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn json_value(response: axum::http::Response<Body>) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn login(shared: &AppState) -> serde_json::Value {
        let response = post_json(
            router(shared.clone()),
            "/api/auth/login",
            r#"{"username":"alice@example.com","password":"correct horse"}"#.to_string(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        json_value(response).await
    }

    async fn challenge(shared: &AppState, code: &str) -> axum::http::Response<Body> {
        post_json(
            router(shared.clone()),
            "/api/auth/totp",
            format!(r#"{{"username":"alice@example.com","code":"{code}"}}"#),
        )
        .await
    }

    #[tokio::test]
    async fn a_valid_code_after_login_mints_a_session_token() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let account_id = account_with_password(&shared, "correct horse");
        let secret = totp_service::generate_secret().unwrap();
        enroll(&shared, account_id, &secret, Vec::new());

        let login_reply = login(&shared).await;
        assert_eq!(login_reply["totpRequired"], true);
        assert!(
            login_reply["token"].is_null(),
            "no token before the second factor"
        );

        let response = challenge(&shared, &current_code(&secret, unix_time())).await;
        assert_eq!(response.status(), StatusCode::OK);
        let value = json_value(response).await;
        let token = value["token"].as_str().expect("a session token is minted");
        assert!(!token.is_empty());
        assert_eq!(value["isAdmin"], false);

        let me = router(shared.clone())
            .oneshot(
                Request::builder()
                    .uri("/api/me/app-passwords")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(me.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn a_correct_code_without_a_prior_login_is_unauthorized() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let account_id = account_with_password(&shared, "correct horse");
        let secret = totp_service::generate_secret().unwrap();
        enroll(&shared, account_id, &secret, Vec::new());

        let response = challenge(&shared, &current_code(&secret, unix_time())).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_wrong_code_is_unauthorized() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let account_id = account_with_password(&shared, "correct horse");
        let secret = totp_service::generate_secret().unwrap();
        enroll(&shared, account_id, &secret, Vec::new());

        login(&shared).await;
        let response = challenge(&shared, &wrong_code(&secret, unix_time())).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_recovery_code_completes_login_and_is_single_use() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let account_id = account_with_password(&shared, "correct horse");
        let secret = totp_service::generate_secret().unwrap();
        let codes = totp_service::generate_recovery_codes().unwrap();
        let enrolled_hashes = codes.hashes[..2].to_vec();
        enroll(&shared, account_id, &secret, enrolled_hashes.clone());

        login(&shared).await;
        let response = challenge(&shared, &codes.plaintext[0]).await;
        assert_eq!(response.status(), StatusCode::OK);

        let remaining = shared
            .directory
            .credentials()
            .list(account_id)
            .unwrap()
            .into_iter()
            .find_map(|credential| match credential {
                irixmail_directory::Credential::Totp(totp) => Some(totp.recovery_codes.len()),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            remaining,
            enrolled_hashes.len() - 1,
            "the used code is consumed"
        );

        login(&shared).await;
        let replay = challenge(&shared, &codes.plaintext[0]).await;
        assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn attempts_are_capped_per_challenge() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let account_id = account_with_password(&shared, "correct horse");
        let secret = totp_service::generate_secret().unwrap();
        enroll(&shared, account_id, &secret, Vec::new());

        login(&shared).await;
        let wrong = wrong_code(&secret, unix_time());
        for _ in 0..5 {
            let response = challenge(&shared, &wrong).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        let response = challenge(&shared, &current_code(&secret, unix_time())).await;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "an exhausted challenge must not accept even the right code"
        );
    }

    #[tokio::test]
    async fn an_empty_code_is_a_bad_request() {
        let dir = TempDir::new();
        let app = router(state(&dir));
        let body = r#"{"username":"a@b.com","code":"  "}"#;
        let response = post_json(app, "/api/auth/totp", body.to_string()).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_well_formed_code_without_an_enrolled_secret_is_unauthorized() {
        let dir = TempDir::new();
        let app = router(state(&dir));
        let body = r#"{"username":"a@b.com","code":"123456"}"#;
        let response = post_json(app, "/api/auth/totp", body.to_string()).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
