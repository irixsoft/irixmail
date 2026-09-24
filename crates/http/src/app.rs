use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::json;

use irixmail_core::LogBuffer;
use irixmail_directory::{Directory, SecretCipher};
use irixmail_dns::Resolver;
use irixmail_store::{BlobStore, ChangeNotifier, Store};
use irixmail_tls::rustls::crypto::CryptoProvider;
use irixmail_tls::{CertStore, Http01Challenges, SniResolver};
use tokio::sync::mpsc;

use crate::sessions::{SessionKind, Sessions};

const PENDING_TOTP_TTL: Duration = Duration::from_secs(5 * 60);

const PENDING_TOTP_ATTEMPTS: u8 = 5;

struct PendingChallenge {
    account_id: u64,
    kind: SessionKind,
    expires_at: Instant,
    attempts_left: u8,
}

#[derive(Default)]
pub struct PendingChallenges {
    inner: Mutex<HashMap<String, PendingChallenge>>,
}

impl PendingChallenges {
    pub fn begin(&self, username: &str, account_id: u64, kind: SessionKind) {
        self.inner.lock().unwrap().insert(
            challenge_key(username),
            PendingChallenge {
                account_id,
                kind,
                expires_at: Instant::now() + PENDING_TOTP_TTL,
                attempts_left: PENDING_TOTP_ATTEMPTS,
            },
        );
    }

    pub fn take_attempt(&self, username: &str) -> Option<(u64, SessionKind)> {
        let key = challenge_key(username);
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.get_mut(&key)?;
        if Instant::now() >= entry.expires_at || entry.attempts_left == 0 {
            inner.remove(&key);
            return None;
        }
        entry.attempts_left -= 1;
        Some((entry.account_id, entry.kind))
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
    pub tokens: Arc<Sessions>,
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
            tokens: Arc::new(Sessions::new(Arc::clone(&store))),
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
    fn a_pending_challenge_keeps_the_requested_session_kind() {
        let pending = PendingChallenges::default();
        pending.begin("Alice@Example.com", 7, SessionKind::Admin);
        assert_eq!(
            pending.take_attempt("alice@example.com"),
            Some((7, SessionKind::Admin))
        );
        pending.complete("alice@example.com");
        assert_eq!(pending.take_attempt("alice@example.com"), None);
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
