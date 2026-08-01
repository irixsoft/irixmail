use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use irixmail_core::registry::Registry;
use irixmail_core::{Error, Result};

use crate::session::Session;
use crate::session_services::{local_domains, InboundServices};

pub struct InboundTlsListener {
    listener: TcpListener,
}

impl InboundTlsListener {
    pub async fn bind(addr: SocketAddr) -> Result<Self> {
        let listener = TcpListener::bind(addr).await.map_err(|err| {
            Error::internal(format!(
                "could not bind the implicit TLS inbound SMTP listener on {addr}: {err}"
            ))
        })?;
        Ok(Self { listener })
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.listener.local_addr()?)
    }

    pub async fn serve<H, Fut>(self, handler: H) -> Result<()>
    where
        H: Fn(TcpStream, SocketAddr) -> Fut + Clone + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        loop {
            let (stream, peer) = self.listener.accept().await.map_err(|err| {
                Error::internal(format!("implicit TLS inbound SMTP accept failed: {err}"))
            })?;
            let handler = handler.clone();
            tokio::spawn(async move {
                handler(stream, peer).await;
            });
        }
    }
}

pub fn register_inbound_tls(
    registry: &Registry,
    listener: InboundTlsListener,
    acceptor: TlsAcceptor,
    services: InboundServices,
) {
    let acceptor = Arc::new(acceptor);
    registry.register_listener("smtps", move || async move {
        let result = listener
            .serve(move |stream, peer| {
                let acceptor = acceptor.clone();
                let services = services.clone();
                async move {
                    if let Err(err) = handle_connection(stream, peer, &acceptor, services).await {
                        tracing::debug!(%peer, error = %err, "implicit TLS inbound SMTP connection ended");
                    }
                }
            })
            .await;
        if let Err(err) = result {
            tracing::error!(error = %err, "implicit TLS inbound SMTP listener stopped");
        }
    });
}

async fn handle_connection(
    stream: TcpStream,
    peer: SocketAddr,
    acceptor: &TlsAcceptor,
    services: InboundServices,
) -> Result<()> {
    let secured = acceptor.accept(stream).await.map_err(|err| {
        Error::protocol(format!("implicit TLS inbound SMTP handshake failed: {err}"))
    })?;
    if crate::ip_guard::is_blocked(services.directory(), peer.ip()) {
        use tokio::io::AsyncWriteExt;
        let mut secured = secured;
        let _ = secured.write_all(b"554 5.7.1 access denied\r\n").await;
        return Ok(());
    }
    let services = services.for_connection();
    let mut session = Session::new(secured, peer)
        .with_local_domains(local_domains(services.directory()))
        .with_tls()
        .with_inbound_services(services);
    session.run().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use mail_auth::hickory_resolver::config::{ResolverConfig, ResolverOpts};
    use mail_auth::MessageAuthenticator;
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::crypto::aws_lc_rs::default_provider;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
    use rustls::server::{ClientHello, ResolvesServerCert};
    use rustls::sign::CertifiedKey;
    use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::TlsConnector;

    use irixmail_core::IdGenerator;
    use irixmail_directory::Directory;
    use irixmail_dns::Resolver;
    use irixmail_mail::MailServices;
    use irixmail_store::{BlobStore, ChangeNotifier, FsBlobStore, RocksdbStore, Store, TtlStore};

    use crate::arc::ArcVerifier;
    use crate::cmd_starttls::build_acceptor;
    use crate::dkim_verify::DkimVerifier;
    use crate::dmarc::DmarcVerifier;
    use crate::dnsbl::DnsblConfig;
    use crate::greylist::Greylist;
    use crate::ratelimit_in::RateLimiter;
    use crate::spf::{SpfConfig, SpfVerifier};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "irixmail-smtp-listener-in-tls-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn authenticator() -> MessageAuthenticator {
        MessageAuthenticator::new(ResolverConfig::default(), ResolverOpts::default()).unwrap()
    }

    fn inbound_services(dir: &TempDir) -> InboundServices {
        inbound_services_with_store(dir).0
    }

    fn inbound_services_with_store(dir: &TempDir) -> (InboundServices, Arc<dyn Store>) {
        use hickory_resolver::config::{ResolverConfig as DnsConfig, ResolverOpts as DnsOpts};

        let store: Arc<dyn Store> = Arc::new(RocksdbStore::open(dir.path.join("db")).unwrap());
        let blobs: Arc<dyn BlobStore> =
            Arc::new(FsBlobStore::open(dir.path.join("blobs")).unwrap());
        let notifier = Arc::new(ChangeNotifier::new());
        let ttl = Arc::new(TtlStore::new());
        let directory = Directory::new(Arc::clone(&store), Arc::new(IdGenerator::new(0)), None);
        let services = InboundServices::new(
            directory,
            authenticator(),
            Resolver::from_config(DnsConfig::default(), DnsOpts::default()),
            Arc::new(SpfVerifier::new(
                authenticator(),
                SpfConfig::new("mx.d.example"),
            )),
            Arc::new(DkimVerifier::new(authenticator())),
            Arc::new(DmarcVerifier::new(authenticator())),
            Arc::new(ArcVerifier::new(authenticator())),
            DnsblConfig { zones: Vec::new() },
            Arc::new(Greylist::new(
                Arc::new(irixmail_store::ExpiringStore::new(Arc::clone(&store))),
                Default::default(),
            )),
            Arc::new(RateLimiter::new(ttl, Default::default())),
            MailServices::new(Arc::clone(&store), blobs, notifier),
        );
        (services, store)
    }

    fn certified_key() -> Arc<CertifiedKey> {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let key = PrivateKeyDer::Pkcs8(certified.key_pair.serialize_der().into());
        let signing_key = default_provider()
            .key_provider
            .load_private_key(key)
            .unwrap();
        Arc::new(CertifiedKey::new(
            vec![certified.cert.der().clone()],
            signing_key,
        ))
    }

    #[derive(Debug)]
    struct StaticCert(Arc<CertifiedKey>);

    impl ResolvesServerCert for StaticCert {
        fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
            Some(self.0.clone())
        }
    }

    fn acceptor() -> TlsAcceptor {
        let provider = Arc::new(default_provider());
        build_acceptor(provider, Arc::new(StaticCert(certified_key()))).unwrap()
    }

    #[derive(Debug)]
    struct TrustAny;

    impl ServerCertVerifier for TrustAny {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> std::result::Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            default_provider()
                .signature_verification_algorithms
                .supported_schemes()
        }
    }

    fn connector() -> TlsConnector {
        let config = ClientConfig::builder_with_provider(Arc::new(default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(TrustAny))
            .with_no_client_auth();
        TlsConnector::from(Arc::new(config))
    }

    #[tokio::test]
    async fn registering_appends_one_listener() {
        let registry = Registry::new();
        let listener = InboundTlsListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let dir = TempDir::new();

        register_inbound_tls(&registry, listener, acceptor(), inbound_services(&dir));

        assert_eq!(registry.len(), 1);
        assert_eq!(registry.registered()[0].0, "smtps");
    }

    #[tokio::test]
    async fn the_registered_listener_greets_after_the_handshake() {
        let registry = Registry::new();
        let listener = InboundTlsListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let dir = TempDir::new();

        register_inbound_tls(&registry, listener, acceptor(), inbound_services(&dir));
        let mut tasks = registry.start_all();

        let stream = TcpStream::connect(addr).await.unwrap();
        let server_name = ServerName::try_from("localhost").unwrap();
        let mut client = connector().connect(server_name, stream).await.unwrap();

        let mut greeting = [0u8; 64];
        let read = client.read(&mut greeting).await.unwrap();
        assert!(greeting[..read].starts_with(b"220 "));

        client.write_all(b"QUIT\r\n").await.unwrap();
        let mut farewell = [0u8; 64];
        let read = client.read(&mut farewell).await.unwrap();
        assert!(farewell[..read].starts_with(b"221 "));

        tasks.abort_all();
    }

    #[tokio::test]
    async fn an_implicit_tls_connection_counts_toward_the_connection_limit() {
        let registry = Registry::new();
        let listener = InboundTlsListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let dir = TempDir::new();
        let (services, store) = inbound_services_with_store(&dir);

        register_inbound_tls(&registry, listener, acceptor(), services);
        let mut tasks = registry.start_all();

        let strict = serde_json::json!({ "rateLimits": { "maxConnectionsPerIp": 1 } });
        store
            .put(
                &irixmail_store::settings_key(),
                strict.to_string().as_bytes(),
            )
            .unwrap();

        let server_name = ServerName::try_from("localhost").unwrap();
        let stream = TcpStream::connect(addr).await.unwrap();
        let mut first = connector()
            .connect(server_name.clone(), stream)
            .await
            .unwrap();
        let mut greeting = [0u8; 64];
        let read = first.read(&mut greeting).await.unwrap();
        assert!(greeting[..read].starts_with(b"220 "));

        let stream = TcpStream::connect(addr).await.unwrap();
        let mut second = connector().connect(server_name, stream).await.unwrap();
        let mut refusal = [0u8; 64];
        let read = second.read(&mut refusal).await.unwrap();
        assert!(
            refusal[..read].starts_with(b"421 "),
            "an implicit TLS connection must be rate limited: {:?}",
            String::from_utf8_lossy(&refusal[..read])
        );

        tasks.abort_all();
    }
}
