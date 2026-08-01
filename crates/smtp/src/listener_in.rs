use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use irixmail_core::registry::Registry;
use irixmail_core::{Error, Result};

use crate::cmd_starttls::upgrade;
use crate::session::{Flow, Session};
use crate::session_services::{local_domains, InboundServices};

pub struct InboundListener {
    listener: TcpListener,
}

impl InboundListener {
    pub async fn bind(addr: SocketAddr) -> Result<Self> {
        let listener = TcpListener::bind(addr).await.map_err(|err| {
            Error::internal(format!(
                "could not bind the inbound SMTP listener on {addr}: {err}"
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
            let (stream, peer) = self
                .listener
                .accept()
                .await
                .map_err(|err| Error::internal(format!("inbound SMTP accept failed: {err}")))?;
            let handler = handler.clone();
            tokio::spawn(async move {
                handler(stream, peer).await;
            });
        }
    }
}

pub fn register_inbound(
    registry: &Registry,
    listener: InboundListener,
    acceptor: TlsAcceptor,
    services: InboundServices,
) {
    let acceptor = Arc::new(acceptor);
    registry.register_listener("smtp:25", move || async move {
        let result = listener
            .serve(move |stream, peer| {
                let acceptor = acceptor.clone();
                let services = services.clone();
                async move {
                    if let Err(err) = handle_connection(stream, peer, &acceptor, services).await {
                        tracing::debug!(%peer, error = %err, "inbound SMTP connection ended");
                    }
                }
            })
            .await;
        if let Err(err) = result {
            tracing::error!(error = %err, "inbound SMTP listener stopped");
        }
    });
}

async fn handle_connection(
    mut stream: TcpStream,
    peer: SocketAddr,
    acceptor: &TlsAcceptor,
    services: InboundServices,
) -> Result<()> {
    if crate::ip_guard::is_blocked(services.directory(), peer.ip()) {
        use tokio::io::AsyncWriteExt;
        let _ = stream.write_all(b"554 5.7.1 access denied\r\n").await;
        return Ok(());
    }
    let services = services.for_connection();
    let domains = local_domains(services.directory());
    let mut session = Session::new(stream, peer)
        .with_local_domains(domains.clone())
        .with_inbound_services(services.clone());
    if session.run().await? != Flow::Upgrade {
        return Ok(());
    }

    let sid = session.session_id();
    let secured = upgrade(acceptor, session.into_inner()).await?;
    let mut secured_session = Session::new(secured, peer)
        .with_session_id(sid)
        .with_local_domains(domains)
        .with_starttls_upgrade()
        .with_inbound_services(services);
    secured_session.run().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

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

    #[derive(Debug)]
    struct NoCert;

    impl ResolvesServerCert for NoCert {
        fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
            None
        }
    }

    fn acceptor() -> TlsAcceptor {
        let provider = Arc::new(default_provider());
        build_acceptor(provider, Arc::new(NoCert)).unwrap()
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

    fn tls_acceptor() -> TlsAcceptor {
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

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "irixmail-smtp-listener-in-{}-{unique}",
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

    async fn recv_reply<S: AsyncReadExt + Unpin>(client: &mut S) -> String {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 256];
        loop {
            let read = tokio::time::timeout(Duration::from_secs(10), client.read(&mut chunk))
                .await
                .expect("timed out waiting for a reply")
                .unwrap();
            assert!(
                read > 0,
                "connection closed early: {:?}",
                String::from_utf8_lossy(&buf)
            );
            buf.extend_from_slice(&chunk[..read]);
            if reply_complete(&buf) {
                break;
            }
        }
        String::from_utf8(buf).unwrap()
    }

    fn reply_complete(buf: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(buf) else {
            return false;
        };
        if !text.ends_with("\r\n") {
            return false;
        }
        text.lines()
            .next_back()
            .map(|line| line.len() >= 4 && line.as_bytes()[3] == b' ')
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn an_accepted_connection_reaches_the_handler() {
        let listener = InboundListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        tokio::spawn(async move {
            let _ = listener
                .serve(move |_stream, peer| {
                    let tx = tx.clone();
                    async move {
                        let _ = tx.send(peer).await;
                    }
                })
                .await;
        });

        let _client = TcpStream::connect(addr).await.unwrap();
        let received = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .unwrap();
        assert!(received.is_some());
    }

    #[tokio::test]
    async fn registering_appends_one_listener() {
        let registry = Registry::new();
        let listener = InboundListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let dir = TempDir::new();

        register_inbound(&registry, listener, acceptor(), inbound_services(&dir));

        assert_eq!(registry.len(), 1);
        assert_eq!(registry.registered()[0].0, "smtp:25");
    }

    #[tokio::test]
    async fn a_blocked_ip_is_refused_at_accept_before_the_greeting() {
        use irixmail_directory::IpAction;

        let registry = Registry::new();
        let listener = InboundListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let dir = TempDir::new();
        let services = inbound_services(&dir);
        services
            .directory()
            .ip_rules()
            .create("127.0.0.1", IpAction::Block)
            .unwrap();

        register_inbound(&registry, listener, acceptor(), services);
        let mut tasks = registry.start_all();

        let mut client = TcpStream::connect(addr).await.unwrap();
        let mut reply = Vec::new();
        client.read_to_end(&mut reply).await.unwrap();
        let text = String::from_utf8(reply).unwrap();
        assert!(text.starts_with("554 5.7.1 access denied"), "got: {text:?}");
        assert!(!text.contains("220 "));

        tasks.abort_all();
    }

    #[tokio::test]
    async fn a_domain_created_after_registration_accepts_inbound_mail_without_restart() {
        use irixmail_directory::{AddressEntry, Role};

        let registry = Registry::new();
        let listener = InboundListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let dir = TempDir::new();
        let services = inbound_services(&dir);
        let directory = services.directory().clone();

        register_inbound(&registry, listener, acceptor(), services);
        let mut tasks = registry.start_all();

        let domain = directory
            .domains()
            .create("late.example", Vec::new())
            .unwrap();
        let account = directory
            .accounts()
            .create("bob", domain.id, "Bob", Role::User)
            .unwrap();
        directory
            .addresses()
            .set(AddressEntry::account("bob@late.example", account.id))
            .unwrap();

        let mut client = TcpStream::connect(addr).await.unwrap();
        assert!(recv_reply(&mut client).await.starts_with("220 "));
        client.write_all(b"EHLO relay.example\r\n").await.unwrap();
        recv_reply(&mut client).await;
        client
            .write_all(b"MAIL FROM:<sender@remote.example>\r\n")
            .await
            .unwrap();
        assert!(recv_reply(&mut client).await.starts_with("250"));
        client
            .write_all(b"RCPT TO:<bob@late.example>\r\n")
            .await
            .unwrap();
        let rcpt = recv_reply(&mut client).await;
        assert!(
            rcpt.starts_with("250"),
            "a domain created after boot must be deliverable without a restart: {rcpt}"
        );

        tasks.abort_all();
    }

    #[tokio::test]
    async fn a_starttls_upgrade_serves_the_transaction_without_a_second_greeting() {
        use irixmail_directory::{AddressEntry, Role};

        let registry = Registry::new();
        let listener = InboundListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let dir = TempDir::new();
        let services = inbound_services(&dir);
        let directory = services.directory().clone();
        let domain = directory
            .domains()
            .create("late.example", Vec::new())
            .unwrap();
        let account = directory
            .accounts()
            .create("bob", domain.id, "Bob", Role::User)
            .unwrap();
        directory
            .addresses()
            .set(AddressEntry::account("bob@late.example", account.id))
            .unwrap();

        register_inbound(&registry, listener, tls_acceptor(), services);
        let mut tasks = registry.start_all();

        let mut client = TcpStream::connect(addr).await.unwrap();
        assert!(recv_reply(&mut client).await.starts_with("220 "));
        client.write_all(b"EHLO client\r\n").await.unwrap();
        assert!(recv_reply(&mut client).await.contains("STARTTLS"));
        client.write_all(b"STARTTLS\r\n").await.unwrap();
        assert!(recv_reply(&mut client).await.starts_with("220 2.0.0"));

        let server_name = ServerName::try_from("localhost").unwrap();
        let mut secured = connector().connect(server_name, client).await.unwrap();

        secured.write_all(b"EHLO client\r\n").await.unwrap();
        let ehlo = recv_reply(&mut secured).await;
        assert!(
            ehlo.starts_with("250"),
            "the server must not greet again after STARTTLS: {ehlo:?}"
        );

        secured
            .write_all(b"MAIL FROM:<sender@remote.example>\r\n")
            .await
            .unwrap();
        assert!(recv_reply(&mut secured).await.starts_with("250"));
        secured
            .write_all(b"RCPT TO:<bob@late.example>\r\n")
            .await
            .unwrap();
        assert!(recv_reply(&mut secured).await.starts_with("250"));
        secured.write_all(b"DATA\r\n").await.unwrap();
        let data = recv_reply(&mut secured).await;
        assert!(data.starts_with("354"), "got: {data:?}");
        secured
            .write_all(b"Subject: hello\r\n\r\nover starttls\r\n.\r\n")
            .await
            .unwrap();
        let accepted = recv_reply(&mut secured).await;
        assert!(accepted.starts_with("250"), "got: {accepted:?}");
        secured.write_all(b"QUIT\r\n").await.unwrap();
        assert!(recv_reply(&mut secured).await.starts_with("221"));

        tasks.abort_all();
    }

    #[tokio::test]
    async fn saved_rate_limit_settings_govern_new_connections_without_restart() {
        let registry = Registry::new();
        let listener = InboundListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let dir = TempDir::new();
        let (services, store) = inbound_services_with_store(&dir);

        register_inbound(&registry, listener, acceptor(), services);
        let mut tasks = registry.start_all();

        let mut first = TcpStream::connect(addr).await.unwrap();
        assert!(recv_reply(&mut first).await.starts_with("220 "));

        let strict = serde_json::json!({ "rateLimits": { "maxConnectionsPerIp": 1 } });
        store
            .put(
                &irixmail_store::settings_key(),
                strict.to_string().as_bytes(),
            )
            .unwrap();
        let mut second = TcpStream::connect(addr).await.unwrap();
        let refusal = recv_reply(&mut second).await;
        assert!(
            refusal.starts_with("421"),
            "saved settings must govern the very next connection: {refusal}"
        );

        let relaxed = serde_json::json!({ "rateLimits": { "maxConnectionsPerIp": 100 } });
        store
            .put(
                &irixmail_store::settings_key(),
                relaxed.to_string().as_bytes(),
            )
            .unwrap();
        let mut third = TcpStream::connect(addr).await.unwrap();
        assert!(recv_reply(&mut third).await.starts_with("220 "));

        tasks.abort_all();
    }

    #[tokio::test]
    async fn the_registered_listener_greets_and_serves_a_session() {
        let registry = Registry::new();
        let listener = InboundListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let dir = TempDir::new();

        register_inbound(&registry, listener, acceptor(), inbound_services(&dir));
        let mut tasks = registry.start_all();

        let mut client = TcpStream::connect(addr).await.unwrap();
        let mut greeting = [0u8; 64];
        let read = client.read(&mut greeting).await.unwrap();
        assert!(greeting[..read].starts_with(b"220 "));

        client.write_all(b"QUIT\r\n").await.unwrap();
        let mut farewell = [0u8; 64];
        let read = client.read(&mut farewell).await.unwrap();
        assert!(farewell[..read].starts_with(b"221 "));

        tasks.abort_all();
    }
}
