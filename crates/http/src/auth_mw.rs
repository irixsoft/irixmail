use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{header, Extensions, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

use irixmail_directory::{attempt_login_blocking, LoginAttempt, LoginPurpose, Role};

use crate::app::{error_response, AppState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthMethod {
    Session,
    ApiKey,
    MailPassword,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthIdentity {
    pub account_id: u64,
    pub username: String,
    pub is_admin: bool,
    pub method: AuthMethod,
}

impl AuthIdentity {
    pub fn interactive(&self) -> bool {
        self.method == AuthMethod::Session
    }
}

pub struct ClientIp(pub Option<IpAddr>);

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(client_ip_of(&parts.extensions))
    }
}

pub fn client_ip_of(extensions: &Extensions) -> ClientIp {
    ClientIp(
        extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0.ip().to_canonical()),
    )
}

pub async fn require_auth(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    match authenticate_request(&state, &request).await {
        Some(identity) => {
            request.extensions_mut().insert(identity);
            next.run(request).await
        }
        None => error_response(StatusCode::UNAUTHORIZED, "authentication required"),
    }
}

pub async fn require_interactive(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    match authenticate_request(&state, &request).await {
        Some(identity) if identity.interactive() => {
            request.extensions_mut().insert(identity);
            next.run(request).await
        }
        Some(_) => error_response(StatusCode::UNAUTHORIZED, "session authentication required"),
        None => error_response(StatusCode::UNAUTHORIZED, "authentication required"),
    }
}

pub async fn require_admin(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    match authenticate_request(&state, &request).await {
        Some(identity) if identity.method == AuthMethod::MailPassword => {
            error_response(StatusCode::UNAUTHORIZED, "session authentication required")
        }
        Some(identity) if identity.is_admin => {
            request.extensions_mut().insert(identity);
            next.run(request).await
        }
        Some(_) => error_response(StatusCode::FORBIDDEN, "administrator access required"),
        None => error_response(StatusCode::UNAUTHORIZED, "authentication required"),
    }
}

pub fn authenticate_request<'a>(
    state: &'a AppState,
    request: &Request,
) -> impl std::future::Future<Output = Option<AuthIdentity>> + Send + 'a {
    let authorization = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let ip = client_ip_of(request.extensions()).0;
    async move {
        let header = authorization?;
        if let Some(token) = header.strip_prefix("Bearer ") {
            let token = token.trim();
            if let Some(info) = state.tokens.validate(token) {
                return Some(AuthIdentity {
                    account_id: info.account_id,
                    username: info.username,
                    is_admin: info.is_admin,
                    method: AuthMethod::Session,
                });
            }
            if let Ok(Some(key)) = state.directory.api_keys().verify(token, &state.secrets) {
                return Some(AuthIdentity {
                    account_id: 0,
                    username: format!("api-key:{}", key.name),
                    is_admin: true,
                    method: AuthMethod::ApiKey,
                });
            }
            return None;
        }
        if let Some(encoded) = header.strip_prefix("Basic ") {
            let decoded = STANDARD.decode(encoded.trim()).ok()?;
            let text = String::from_utf8(decoded).ok()?;
            let (user, password) = text.split_once(':')?;
            let ip = ip.map(|ip| ip.to_string());
            return resolve_basic(state, ip.as_deref(), user, password).await;
        }
        None
    }
}

async fn resolve_basic(
    state: &AppState,
    ip: Option<&str>,
    user: &str,
    password: &str,
) -> Option<AuthIdentity> {
    match attempt_login_blocking(&state.directory, ip, user, password, LoginPurpose::Mail)
        .await
        .ok()?
    {
        LoginAttempt::Granted(account, _) => Some(AuthIdentity {
            account_id: account.id,
            username: user.to_string(),
            is_admin: account.role == Role::Admin,
            method: AuthMethod::MailPassword,
        }),
        LoginAttempt::Denied | LoginAttempt::Throttled => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    use crate::app::TokenInfo;
    use crate::tests_support::{state, TempDir};

    async fn protected() -> &'static str {
        "ok"
    }

    fn admin_router(state: AppState) -> Router {
        Router::new()
            .route("/api/x", get(protected))
            .route_layer(axum::middleware::from_fn_with_state(
                state.clone(),
                require_admin,
            ))
            .with_state(state)
    }

    #[tokio::test]
    async fn a_request_without_credentials_is_unauthorized() {
        let dir = TempDir::new();
        let app = admin_router(state(&dir));
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/x")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_valid_admin_bearer_token_passes() {
        let dir = TempDir::new();
        let state = state(&dir);
        let token = state.tokens.issue(TokenInfo {
            account_id: 1,
            username: "admin@example.com".into(),
            is_admin: true,
        });
        let app = admin_router(state);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/x")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn a_non_admin_token_is_forbidden() {
        let dir = TempDir::new();
        let state = state(&dir);
        let token = state.tokens.issue(TokenInfo {
            account_id: 2,
            username: "user@example.com".into(),
            is_admin: false,
        });
        let app = admin_router(state);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/x")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn an_admin_primary_password_over_basic_auth_cannot_reach_admin_routes() {
        use irixmail_directory::{password, Role};

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
            .create("root", domain.id, "Root", Role::Admin)
            .unwrap();
        let hash = password::hash("hunter2").unwrap();
        shared
            .directory
            .credentials()
            .set_primary_password(account.id, hash)
            .unwrap();
        let encoded = STANDARD.encode("root@example.com:hunter2");
        let app = admin_router(shared);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/x")
                    .header(header::AUTHORIZATION, format!("Basic {encoded}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn an_admin_app_password_over_basic_auth_cannot_reach_admin_routes() {
        use irixmail_directory::{app_password, Role};

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
            .create("root", domain.id, "Root", Role::Admin)
            .unwrap();
        let minted = app_password::generate(1, "phone", 0).unwrap();
        shared
            .directory
            .credentials()
            .add_app_password(account.id, minted.record)
            .unwrap();
        let encoded = STANDARD.encode(format!("root@example.com:{}", minted.plaintext));
        let app = admin_router(shared);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/x")
                    .header(header::AUTHORIZATION, format!("Basic {encoded}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn basic_auth_with_a_stored_app_password_resolves() {
        use irixmail_directory::{app_password, Role};

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
        let minted = app_password::generate(1, "client", 0).unwrap();
        let plaintext = minted.plaintext.clone();
        shared
            .directory
            .credentials()
            .add_app_password(account.id, minted.record)
            .unwrap();
        let encoded = STANDARD.encode(format!("alice@example.com:{plaintext}"));
        let request = HttpRequest::builder()
            .header(header::AUTHORIZATION, format!("Basic {encoded}"))
            .body(Body::empty())
            .unwrap();
        let identity = authenticate_request(&shared, &request).await.unwrap();
        assert_eq!(identity.account_id, account.id);
        assert_eq!(identity.username, "alice@example.com");
        assert!(!identity.is_admin);
        assert_eq!(
            identity.method,
            AuthMethod::MailPassword,
            "basic auth must not mint an interactive identity"
        );
    }

    #[tokio::test]
    async fn an_admin_api_key_bearer_token_reaches_admin_routes() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let (record, plaintext) = shared
            .directory
            .api_keys()
            .create("ci", &shared.secrets)
            .unwrap();
        let app = admin_router(shared.clone());
        let request = |token: &str| {
            HttpRequest::builder()
                .uri("/api/x")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap()
        };

        let response = app.clone().oneshot(request(&plaintext)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        assert!(shared.directory.api_keys().revoke(record.id).unwrap());
        let revoked = app.clone().oneshot(request(&plaintext)).await.unwrap();
        assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);

        let wrong = app.oneshot(request("not-a-key")).await.unwrap();
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn an_api_key_cannot_use_interactive_routes() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let (_, plaintext) = shared
            .directory
            .api_keys()
            .create("ci", &shared.secrets)
            .unwrap();
        let app = Router::new()
            .route("/api/x", get(protected))
            .route_layer(axum::middleware::from_fn_with_state(
                shared.clone(),
                require_interactive,
            ))
            .with_state(shared);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/x")
                    .header(header::AUTHORIZATION, format!("Bearer {plaintext}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn repeated_failed_basic_logins_lock_the_account() {
        use irixmail_directory::{password, Role};

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
        let hash = password::hash("hunter2").unwrap();
        shared
            .directory
            .credentials()
            .set_primary_password(account.id, hash)
            .unwrap();
        let app = Router::new()
            .route("/api/x", get(protected))
            .route_layer(axum::middleware::from_fn_with_state(
                shared.clone(),
                require_auth,
            ))
            .with_state(shared);

        let request = |password: &str| {
            let encoded = STANDARD.encode(format!("alice@example.com:{password}"));
            HttpRequest::builder()
                .uri("/api/x")
                .header(header::AUTHORIZATION, format!("Basic {encoded}"))
                .body(Body::empty())
                .unwrap()
        };
        for _ in 0..5 {
            let response = app.clone().oneshot(request("wrong")).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        let response = app.clone().oneshot(request("hunter2")).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn an_unknown_bearer_token_resolves_to_nothing() {
        let dir = TempDir::new();
        let state = state(&dir);
        let request = HttpRequest::builder()
            .header(header::AUTHORIZATION, "Bearer deadbeef")
            .body(Body::empty())
            .unwrap();
        assert_eq!(authenticate_request(&state, &request).await, None);
        let _ = Arc::new(());
    }
}
