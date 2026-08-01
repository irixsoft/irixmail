use std::sync::Arc;

use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tower::Service;

use irixmail_core::registry::Registry;
use irixmail_core::ShutdownSignal;

use crate::app::{router, AppState};

pub fn register_http(
    registry: &Registry,
    listener: TcpListener,
    state: AppState,
    mut shutdown: ShutdownSignal,
) {
    registry.register_listener("http:80", move || async move {
        let app = router(state).into_make_service_with_connect_info::<std::net::SocketAddr>();
        let serve =
            axum::serve(listener, app).with_graceful_shutdown(async move { shutdown.recv().await });
        if let Err(err) = serve.await {
            tracing::error!(error = %err, "HTTP :80 listener stopped");
        }
    });
}

pub fn redirect_router(state: AppState, https_port: u16) -> axum::Router {
    use axum::http::{header, StatusCode, Uri};
    use axum::response::IntoResponse;

    let authority = if https_port == 443 {
        state.hostname.clone()
    } else {
        format!("{}:{https_port}", state.hostname)
    };
    crate::api::public_routes()
        .fallback(move |uri: Uri| {
            let target = format!(
                "https://{authority}{}",
                uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/")
            );
            async move {
                (StatusCode::PERMANENT_REDIRECT, [(header::LOCATION, target)]).into_response()
            }
        })
        .with_state(state)
}

pub fn register_http_redirect(
    registry: &Registry,
    listener: TcpListener,
    state: AppState,
    https_port: u16,
    mut shutdown: ShutdownSignal,
) {
    registry.register_listener("http:80", move || async move {
        let app = redirect_router(state, https_port)
            .into_make_service_with_connect_info::<std::net::SocketAddr>();
        let serve =
            axum::serve(listener, app).with_graceful_shutdown(async move { shutdown.recv().await });
        if let Err(err) = serve.await {
            tracing::error!(error = %err, "HTTP :80 redirect listener stopped");
        }
    });
}

pub fn register_https(
    registry: &Registry,
    listener: TcpListener,
    acceptor: TlsAcceptor,
    state: AppState,
    mut shutdown: ShutdownSignal,
) {
    let acceptor = Arc::new(acceptor);
    registry.register_listener("https:443", move || async move {
        let app = router(state);
        loop {
            let accepted = tokio::select! {
                biased;
                _ = shutdown.recv() => return,
                accepted = listener.accept() => accepted,
            };
            let (stream, peer) = match accepted {
                Ok(connection) => connection,
                Err(err) => {
                    tracing::error!(error = %err, "HTTPS accept failed");
                    continue;
                }
            };
            let acceptor = acceptor.clone();
            let app = app.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(stream).await else {
                    return;
                };
                let io = TokioIo::new(tls);
                let service =
                    hyper::service::service_fn(move |mut request: hyper::Request<Incoming>| {
                        request
                            .extensions_mut()
                            .insert(axum::extract::ConnectInfo(peer));
                        app.clone().call(request)
                    });
                let _ = Builder::new(TokioExecutor::new())
                    .serve_connection_with_upgrades(io, service)
                    .await;
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    use rustls::crypto::aws_lc_rs::default_provider;
    use rustls::pki_types::PrivateKeyDer;
    use rustls::server::{ClientHello, ResolvesServerCert};
    use rustls::sign::CertifiedKey;
    use rustls::ServerConfig;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use crate::tests_support::{state, TempDir};

    fn acceptor() -> TlsAcceptor {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let key = PrivateKeyDer::Pkcs8(certified.key_pair.serialize_der().into());
        let signing_key = default_provider()
            .key_provider
            .load_private_key(key)
            .unwrap();
        let certified_key = Arc::new(CertifiedKey::new(
            vec![certified.cert.der().clone()],
            signing_key,
        ));

        #[derive(Debug)]
        struct Static(Arc<CertifiedKey>);
        impl ResolvesServerCert for Static {
            fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
                Some(self.0.clone())
            }
        }

        let config = ServerConfig::builder_with_provider(Arc::new(default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(Static(certified_key)));
        TlsAcceptor::from(Arc::new(config))
    }

    fn connector() -> tokio_rustls::TlsConnector {
        use rustls::client::danger::{
            HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
        };
        use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
        use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};

        #[derive(Debug)]
        struct TrustAny;
        impl ServerCertVerifier for TrustAny {
            fn verify_server_cert(
                &self,
                _e: &CertificateDer<'_>,
                _i: &[CertificateDer<'_>],
                _n: &ServerName<'_>,
                _o: &[u8],
                _t: UnixTime,
            ) -> std::result::Result<ServerCertVerified, rustls::Error> {
                Ok(ServerCertVerified::assertion())
            }
            fn verify_tls12_signature(
                &self,
                _m: &[u8],
                _c: &CertificateDer<'_>,
                _d: &DigitallySignedStruct,
            ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
                Ok(HandshakeSignatureValid::assertion())
            }
            fn verify_tls13_signature(
                &self,
                _m: &[u8],
                _c: &CertificateDer<'_>,
                _d: &DigitallySignedStruct,
            ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
                Ok(HandshakeSignatureValid::assertion())
            }
            fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
                default_provider()
                    .signature_verification_algorithms
                    .supported_schemes()
            }
        }

        let config = ClientConfig::builder_with_provider(Arc::new(default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(TrustAny))
            .with_no_client_auth();
        tokio_rustls::TlsConnector::from(Arc::new(config))
    }

    fn temp() -> TempDir {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let _ = COUNTER.fetch_add(1, Ordering::Relaxed);
        TempDir::new()
    }

    #[tokio::test]
    async fn the_http_listener_serves_a_plain_request() {
        let dir = temp();
        let registry = Registry::new();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let shutdown = irixmail_core::Shutdown::new();
        register_http(&registry, listener, state(&dir), shutdown.subscribe());
        assert_eq!(registry.registered()[0].0, "http:80");
        let mut tasks = registry.start_all();

        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(
                b"GET /healthz/live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let text = String::from_utf8_lossy(&buf);
        assert!(text.contains("200"));
        assert!(text.contains("\"status\""));

        tasks.abort_all();
    }

    #[tokio::test]
    async fn the_http_listener_completes_on_its_own_when_shutdown_triggers() {
        let dir = temp();
        let registry = Registry::new();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let shutdown = irixmail_core::Shutdown::new();
        register_http(&registry, listener, state(&dir), shutdown.subscribe());
        let mut tasks = registry.start_all();

        tokio::task::yield_now().await;
        shutdown.trigger(irixmail_core::ShutdownCause::Terminate);
        let joined = tokio::time::timeout(std::time::Duration::from_secs(2), tasks.join_next())
            .await
            .expect("the listener must stop when shutdown triggers");
        assert!(joined.is_some());
    }

    #[tokio::test]
    async fn the_https_listener_completes_on_its_own_when_shutdown_triggers() {
        let dir = temp();
        let registry = Registry::new();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let shutdown = irixmail_core::Shutdown::new();
        register_https(
            &registry,
            listener,
            acceptor(),
            state(&dir),
            shutdown.subscribe(),
        );
        let mut tasks = registry.start_all();

        tokio::task::yield_now().await;
        shutdown.trigger(irixmail_core::ShutdownCause::Terminate);
        let joined = tokio::time::timeout(std::time::Duration::from_secs(2), tasks.join_next())
            .await
            .expect("the listener must stop when shutdown triggers");
        assert!(joined.is_some());
    }

    #[tokio::test]
    async fn the_redirect_router_redirects_app_paths_to_https() {
        use axum::body::Body;
        use axum::http::{header, Request, StatusCode};
        use tower::ServiceExt;

        let dir = temp();
        let app = redirect_router(state(&dir), 443);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
        assert_eq!(
            response.headers()[header::LOCATION],
            "https://mail.example.com/admin"
        );
    }

    #[tokio::test]
    async fn the_redirect_keeps_a_nonstandard_https_port_and_the_query() {
        use axum::body::Body;
        use axum::http::{header, Request, StatusCode};
        use tower::ServiceExt;

        let dir = temp();
        let app = redirect_router(state(&dir), 8443);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/webmail/inbox?page=2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
        assert_eq!(
            response.headers()[header::LOCATION],
            "https://mail.example.com:8443/webmail/inbox?page=2"
        );
    }

    #[tokio::test]
    async fn the_redirect_router_still_serves_the_public_well_known_surface() {
        use axum::body::{to_bytes, Body};
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        use crate::app::TlsHandles;

        let dir = temp();
        let mut shared = state(&dir);
        let tls = TlsHandles::default();
        tls.http01.insert(
            "tok".to_string(),
            "tok.keyauth".to_string(),
            irixmail_tls::acme_http01::unix_now(),
        );
        shared.tls = Some(tls);
        let app = redirect_router(shared, 443);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/.well-known/acme-challenge/tok")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], b"tok.keyauth");

        let live = app
            .oneshot(
                Request::builder()
                    .uri("/healthz/live")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(live.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn the_https_listener_serves_a_request_over_tls() {
        use rustls::pki_types::ServerName;

        let dir = temp();
        let registry = Registry::new();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let shutdown = irixmail_core::Shutdown::new();
        register_https(
            &registry,
            listener,
            acceptor(),
            state(&dir),
            shutdown.subscribe(),
        );
        assert_eq!(registry.registered()[0].0, "https:443");
        let mut tasks = registry.start_all();

        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let name = ServerName::try_from("localhost").unwrap();
        let mut tls = connector().connect(name, stream).await.unwrap();
        tls.write_all(
            b"GET /healthz/live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        let mut buf = Vec::new();
        tls.read_to_end(&mut buf).await.unwrap();
        let text = String::from_utf8_lossy(&buf);
        assert!(text.contains("200"));
        assert!(text.contains("\"status\""));

        tasks.abort_all();
    }
}
