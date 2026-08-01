use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use rand::RngCore;
use serde_json::json;

use irixmail_core::LogBuffer;
use irixmail_directory::{Directory, SecretCipher};
use irixmail_dns::Resolver;
use irixmail_store::{BlobStore, ChangeNotifier, Store};
use irixmail_tls::rustls::crypto::CryptoProvider;
use irixmail_tls::{CertStore, Http01Challenges, SniResolver};
use tokio::sync::mpsc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenInfo {
    pub account_id: u64,
    pub username: String,
    pub is_admin: bool,
}

const TOKEN_TTL: Duration = Duration::from_secs(24 * 60 * 60);

struct StoredToken {
    info: TokenInfo,
    expires_at: Instant,
}

#[derive(Default)]
pub struct SessionTokens {
    inner: Mutex<HashMap<String, StoredToken>>,
}

impl SessionTokens {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn issue(&self, info: TokenInfo) -> String {
        self.issue_with_ttl(info, TOKEN_TTL)
    }

    pub fn issue_with_ttl(&self, info: TokenInfo, ttl: Duration) -> String {
        let token = random_token();
        let stored = StoredToken {
            info,
            expires_at: Instant::now() + ttl,
        };
        self.inner.lock().unwrap().insert(token.clone(), stored);
        token
    }

    pub fn validate(&self, token: &str) -> Option<TokenInfo> {
        let mut inner = self.inner.lock().unwrap();
        let snapshot = inner
            .get(token)
            .map(|stored| (stored.info.clone(), Instant::now() < stored.expires_at));
        match snapshot {
            Some((info, true)) => Some(info),
            Some((_, false)) => {
                inner.remove(token);
                None
            }
            None => None,
        }
    }

    pub fn revoke(&self, token: &str) -> bool {
        self.inner.lock().unwrap().remove(token).is_some()
    }
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

const PENDING_TOTP_TTL: Duration = Duration::from_secs(5 * 60);

const PENDING_TOTP_ATTEMPTS: u8 = 5;

struct PendingChallenge {
    account_id: u64,
    expires_at: Instant,
    attempts_left: u8,
}

#[derive(Default)]
pub struct PendingChallenges {
    inner: Mutex<HashMap<String, PendingChallenge>>,
}

impl PendingChallenges {
    pub fn begin(&self, username: &str, account_id: u64) {
        self.inner.lock().unwrap().insert(
            challenge_key(username),
            PendingChallenge {
                account_id,
                expires_at: Instant::now() + PENDING_TOTP_TTL,
                attempts_left: PENDING_TOTP_ATTEMPTS,
            },
        );
    }

    pub fn take_attempt(&self, username: &str) -> Option<u64> {
        let key = challenge_key(username);
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.get_mut(&key)?;
        if Instant::now() >= entry.expires_at || entry.attempts_left == 0 {
            inner.remove(&key);
            return None;
        }
        entry.attempts_left -= 1;
        Some(entry.account_id)
    }

    pub fn complete(&self, username: &str) {
        self.inner.lock().unwrap().remove(&challenge_key(username));
    }
}

fn challenge_key(username: &str) -> String {
    username.trim().to_ascii_lowercase()
}

#[derive(Clone)]
pub struct AppState {
    pub directory: Directory,
    pub logs: LogBuffer,
    pub tokens: Arc<SessionTokens>,
    pub totp_pending: Arc<PendingChallenges>,
    pub store: Arc<dyn Store>,
    pub blobs: Arc<dyn BlobStore>,
    pub notifier: Arc<ChangeNotifier>,
    pub queue_wakeups: Option<mpsc::Sender<()>>,
    pub submitter: Option<irixmail_jmap::Submitter>,
    pub hostname: String,
    pub listeners: irixmail_core::config::ListenersConfig,
    pub public_ipv4: Option<Ipv4Addr>,
    pub public_ipv6: Option<Ipv6Addr>,
    pub resolver: Resolver,
    pub secrets: SecretCipher,
    pub tls: Option<TlsHandles>,
    pub services: Arc<OnceLock<Vec<String>>>,
    pub ready: Arc<AtomicBool>,
    pub update_available: Arc<RwLock<Option<String>>>,
}

#[derive(Clone, Default)]
pub struct TlsHandles {
    pub http01: Http01Challenges,
    pub cert_store: Option<Arc<CertStore>>,
    pub provider: Option<Arc<CryptoProvider>>,
    pub sni_resolver: Option<Arc<SniResolver>>,
    pub reissue: Option<mpsc::Sender<()>>,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        directory: Directory,
        logs: LogBuffer,
        store: Arc<dyn Store>,
        blobs: Arc<dyn BlobStore>,
        notifier: Arc<ChangeNotifier>,
        hostname: impl Into<String>,
        resolver: Resolver,
        secrets: SecretCipher,
    ) -> Self {
        Self {
            directory,
            logs,
            tokens: Arc::new(SessionTokens::new()),
            totp_pending: Arc::new(PendingChallenges::default()),
            store,
            blobs,
            notifier,
            queue_wakeups: None,
            submitter: None,
            hostname: hostname.into(),
            listeners: irixmail_core::config::ListenersConfig::default(),
            public_ipv4: None,
            public_ipv6: None,
            resolver,
            secrets,
            tls: None,
            services: Arc::new(OnceLock::new()),
            ready: Arc::new(AtomicBool::new(false)),
            update_available: Arc::new(RwLock::new(None)),
        }
    }
}

pub fn error_response(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(json!({ "error": { "status": status.as_u16(), "message": message } })),
    )
        .into_response()
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(crate::jmap_mount::routes(state.clone()))
        .merge(crate::dav_mount::routes(state.clone()))
        .merge(crate::api::routes(state.clone()))
        .fallback(crate::static_assets::spa_fallback)
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    use crate::tests_support::{state, TempDir};

    #[test]
    fn tokens_round_trip_and_revoke() {
        let tokens = SessionTokens::new();
        let info = TokenInfo {
            account_id: 7,
            username: "alice@example.com".into(),
            is_admin: true,
        };
        let token = tokens.issue(info.clone());
        assert_eq!(token.len(), 64);
        assert_eq!(tokens.validate(&token), Some(info));
        assert!(tokens.revoke(&token));
        assert_eq!(tokens.validate(&token), None);
    }

    #[test]
    fn a_token_within_its_ttl_validates() {
        let tokens = SessionTokens::new();
        let info = TokenInfo {
            account_id: 3,
            username: "alice@example.com".into(),
            is_admin: false,
        };
        let token = tokens.issue_with_ttl(info.clone(), Duration::from_secs(60));
        assert_eq!(tokens.validate(&token), Some(info));
    }

    #[test]
    fn an_expired_token_is_rejected_and_evicted() {
        let tokens = SessionTokens::new();
        let info = TokenInfo {
            account_id: 3,
            username: "alice@example.com".into(),
            is_admin: false,
        };
        let token = tokens.issue_with_ttl(info, Duration::from_secs(0));
        assert_eq!(tokens.validate(&token), None);
        assert!(!tokens.revoke(&token));
    }

    #[test]
    fn issued_tokens_are_distinct() {
        let tokens = SessionTokens::new();
        let info = TokenInfo {
            account_id: 1,
            username: "a".into(),
            is_admin: false,
        };
        let first = tokens.issue(info.clone());
        let second = tokens.issue(info);
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn an_unknown_api_route_is_a_json_404() {
        let dir = TempDir::new();
        let app = router(state(&dir));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_browser_route_serves_the_spa() {
        let dir = TempDir::new();
        let app = router(state(&dir));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/dashboard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
