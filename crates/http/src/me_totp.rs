use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;

use irixmail_directory::{totp, Credential, Totp};

use crate::app::{error_response, AppState};
use crate::auth_mw::AuthIdentity;

const ISSUER: &str = "IRIXMAIL";

pub async fn status(
    State(state): State<AppState>,
    Extension(identity): Extension<AuthIdentity>,
) -> Response {
    match enrollment(&state, identity.account_id) {
        Ok(stored) => {
            let enabled = stored.is_some_and(|totp| totp.enabled);
            Json(json!({ "enabled": enabled })).into_response()
        }
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not read credentials",
        ),
    }
}

pub async fn setup(
    State(state): State<AppState>,
    Extension(identity): Extension<AuthIdentity>,
) -> Response {
    match enrollment(&state, identity.account_id) {
        Ok(Some(stored)) if stored.enabled => {
            return error_response(
                StatusCode::CONFLICT,
                "two-factor authentication is already enabled",
            )
        }
        Ok(_) => {}
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not read credentials",
            )
        }
    }
    let Ok(secret) = totp::generate_secret() else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not generate a secret",
        );
    };
    let (Ok(encoded), Ok(url)) = (
        totp::secret_base32(&secret),
        totp::provisioning_uri(&secret, ISSUER, &identity.username),
    ) else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not encode the secret",
        );
    };
    let Ok(recovery) = totp::generate_recovery_codes() else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not generate recovery codes",
        );
    };
    let Ok(sealed) = state.secrets.encrypt(&secret) else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not protect the secret",
        );
    };
    let enrollment = Totp {
        secret: sealed,
        enabled: false,
        recovery_codes: recovery.hashes,
        enrolled_at: now_millis(),
    };
    if state
        .directory
        .credentials()
        .set_totp(identity.account_id, enrollment)
        .is_err()
    {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not store the enrollment",
        );
    }
    let Ok(qr) = qr_data_url(&url) else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not render the qr code",
        );
    };
    Json(json!({
        "secret": encoded,
        "otpauthUrl": url,
        "qr": qr,
        "recoveryCodes": recovery.plaintext,
    }))
    .into_response()
}

fn qr_data_url(url: &str) -> Result<String, qrcode::types::QrError> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;

    let image = qrcode::QrCode::new(url.as_bytes())?
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(200, 200)
        .build();
    Ok(format!(
        "data:image/svg+xml;base64,{}",
        STANDARD.encode(image)
    ))
}

#[derive(Deserialize)]
pub struct VerifyBody {
    #[serde(default)]
    pub code: String,
}

pub async fn verify(
    State(state): State<AppState>,
    Extension(identity): Extension<AuthIdentity>,
    Json(body): Json<VerifyBody>,
) -> Response {
    let code = body.code.trim();
    if code.is_empty() || code.len() > 64 {
        return error_response(StatusCode::BAD_REQUEST, "a verification code is required");
    }
    let mut stored = match enrollment(&state, identity.account_id) {
        Ok(Some(stored)) => stored,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "no enrollment is pending"),
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not read credentials",
            )
        }
    };
    let Ok(secret) = state.secrets.decrypt(&stored.secret) else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not read the secret",
        );
    };
    match totp::verify_code(&secret, code, unix_now()) {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::FORBIDDEN, "the code did not match"),
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not verify the code",
            )
        }
    }
    stored.enabled = true;
    match state
        .directory
        .credentials()
        .set_totp(identity.account_id, stored)
    {
        Ok(()) => Json(json!({ "ok": true, "enabled": true })).into_response(),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not store the enrollment",
        ),
    }
}

pub async fn disable(
    State(state): State<AppState>,
    Extension(identity): Extension<AuthIdentity>,
) -> Response {
    match state
        .directory
        .credentials()
        .clear_totp(identity.account_id)
    {
        Ok(()) => Json(json!({ "ok": true, "enabled": false })).into_response(),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not clear the enrollment",
        ),
    }
}

fn enrollment(state: &AppState, account_id: u64) -> irixmail_core::Result<Option<Totp>> {
    Ok(state
        .directory
        .credentials()
        .list(account_id)?
        .into_iter()
        .find_map(|credential| match credential {
            Credential::Totp(totp) => Some(totp),
            _ => None,
        }))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    use irixmail_directory::{totp as totp_service, Credential, Role, Totp};

    use crate::app::{router, AppState, TokenInfo};
    use crate::tests_support::{state, TempDir};

    fn account(shared: &AppState) -> u64 {
        let domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        shared
            .directory
            .accounts()
            .create("alice", domain.id, "Alice", Role::User)
            .unwrap()
            .id
    }

    fn token(shared: &AppState, account_id: u64) -> String {
        shared.tokens.issue(TokenInfo {
            account_id,
            username: "alice@example.com".into(),
            is_admin: false,
        })
    }

    async fn request(
        shared: &AppState,
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: Option<String>,
    ) -> axum::http::Response<Body> {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let request = match body {
            Some(body) => builder
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        router(shared.clone()).oneshot(request).await.unwrap()
    }

    async fn json_value(response: axum::http::Response<Body>) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn stored_totp(shared: &AppState, account_id: u64) -> Option<Totp> {
        shared
            .directory
            .credentials()
            .list(account_id)
            .unwrap()
            .into_iter()
            .find_map(|credential| match credential {
                Credential::Totp(totp) => Some(totp),
                _ => None,
            })
    }

    fn current_code(shared: &AppState, account_id: u64) -> String {
        let stored = stored_totp(shared, account_id).expect("an enrollment exists");
        let secret = shared.secrets.decrypt(&stored.secret).unwrap();
        let unix_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        totp_rs::TOTP::new(
            totp_rs::Algorithm::SHA1,
            6,
            1,
            30,
            secret,
            None,
            String::new(),
        )
        .unwrap()
        .generate(unix_time)
    }

    #[tokio::test]
    async fn the_full_enrollment_flow_enables_totp() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let account_id = account(&shared);
        let token = token(&shared, account_id);

        let status = request(&shared, "GET", "/api/me/totp", Some(&token), None).await;
        assert_eq!(status.status(), StatusCode::OK);
        assert_eq!(json_value(status).await["enabled"], false);

        let setup = request(&shared, "POST", "/api/me/totp/setup", Some(&token), None).await;
        assert_eq!(setup.status(), StatusCode::OK);
        let setup = json_value(setup).await;
        let secret = setup["secret"].as_str().expect("a manual-entry secret");
        assert!(!secret.is_empty());
        assert!(secret
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || (b'2'..=b'7').contains(&byte)));
        let url = setup["otpauthUrl"].as_str().expect("a provisioning url");
        assert!(url.starts_with("otpauth://totp/"), "got {url}");
        assert!(url.contains("alice"), "got {url}");
        assert_eq!(setup["recoveryCodes"].as_array().unwrap().len(), 10);

        let pending = stored_totp(&shared, account_id).expect("the enrollment persisted");
        assert!(
            !pending.enabled,
            "the factor must not arm before verification"
        );

        let code = current_code(&shared, account_id);
        let verify = request(
            &shared,
            "POST",
            "/api/me/totp/verify",
            Some(&token),
            Some(format!(r#"{{"code":"{code}"}}"#)),
        )
        .await;
        assert_eq!(verify.status(), StatusCode::OK);

        let status = request(&shared, "GET", "/api/me/totp", Some(&token), None).await;
        assert_eq!(json_value(status).await["enabled"], true);
        assert!(stored_totp(&shared, account_id).unwrap().enabled);
    }

    #[tokio::test]
    async fn the_setup_response_includes_a_scannable_qr() {
        use base64::Engine;

        let dir = TempDir::new();
        let shared = state(&dir);
        let account_id = account(&shared);
        let token = token(&shared, account_id);

        let setup = request(&shared, "POST", "/api/me/totp/setup", Some(&token), None).await;
        assert_eq!(setup.status(), StatusCode::OK);
        let setup = json_value(setup).await;
        let qr = setup["qr"].as_str().expect("a qr image");
        let payload = qr
            .strip_prefix("data:image/svg+xml;base64,")
            .expect("an svg data url");
        let svg = base64::engine::general_purpose::STANDARD
            .decode(payload)
            .unwrap();
        let svg = String::from_utf8(svg).unwrap();
        assert!(svg.contains("<svg"), "got: {svg:.>60}");
    }

    #[tokio::test]
    async fn a_wrong_code_does_not_enable_the_factor() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let account_id = account(&shared);
        let token = token(&shared, account_id);

        let setup = request(&shared, "POST", "/api/me/totp/setup", Some(&token), None).await;
        assert_eq!(setup.status(), StatusCode::OK);

        let code = current_code(&shared, account_id);
        let wrong = if code == "000000" { "000001" } else { "000000" };
        let verify = request(
            &shared,
            "POST",
            "/api/me/totp/verify",
            Some(&token),
            Some(format!(r#"{{"code":"{wrong}"}}"#)),
        )
        .await;
        assert_eq!(verify.status(), StatusCode::FORBIDDEN);
        assert!(!stored_totp(&shared, account_id).unwrap().enabled);
    }

    #[tokio::test]
    async fn verifying_without_a_pending_enrollment_is_not_found() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let account_id = account(&shared);
        let token = token(&shared, account_id);

        let verify = request(
            &shared,
            "POST",
            "/api/me/totp/verify",
            Some(&token),
            Some(r#"{"code":"123456"}"#.to_string()),
        )
        .await;
        assert_eq!(verify.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn disable_clears_the_enrollment() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let account_id = account(&shared);
        let token = token(&shared, account_id);

        let secret = totp_service::generate_secret().unwrap();
        shared
            .directory
            .credentials()
            .set_totp(
                account_id,
                Totp {
                    secret: shared.secrets.encrypt(&secret).unwrap(),
                    enabled: true,
                    recovery_codes: Vec::new(),
                    enrolled_at: 0,
                },
            )
            .unwrap();

        let disable = request(&shared, "POST", "/api/me/totp/disable", Some(&token), None).await;
        assert_eq!(disable.status(), StatusCode::OK);
        assert!(stored_totp(&shared, account_id).is_none());

        let status = request(&shared, "GET", "/api/me/totp", Some(&token), None).await;
        assert_eq!(json_value(status).await["enabled"], false);
    }

    #[tokio::test]
    async fn setup_while_enabled_is_a_conflict() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let account_id = account(&shared);
        let token = token(&shared, account_id);

        let secret = totp_service::generate_secret().unwrap();
        shared
            .directory
            .credentials()
            .set_totp(
                account_id,
                Totp {
                    secret: shared.secrets.encrypt(&secret).unwrap(),
                    enabled: true,
                    recovery_codes: Vec::new(),
                    enrolled_at: 0,
                },
            )
            .unwrap();

        let setup = request(&shared, "POST", "/api/me/totp/setup", Some(&token), None).await;
        assert_eq!(setup.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn the_totp_routes_require_a_session() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let response = request(&shared, "GET", "/api/me/totp", None, None).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
