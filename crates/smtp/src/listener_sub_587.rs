use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use irixmail_core::registry::Registry;
use irixmail_core::{Error, Result};

use crate::cmd_starttls::upgrade;
use crate::session::{Flow, Session};
use crate::session_services::{local_domains, SubmissionServices};

pub struct SubmissionListener {
    listener: TcpListener,
}

impl SubmissionListener {
    pub async fn bind(addr: SocketAddr) -> Result<Self> {
        let listener = TcpListener::bind(addr).await.map_err(|err| {
            Error::internal(format!(
                "could not bind the submission listener on {addr}: {err}"
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
                .map_err(|err| Error::internal(format!("submission accept failed: {err}")))?;
            let handler = handler.clone();
            tokio::spawn(async move {
                handler(stream, peer).await;
            });
        }
    }
}

pub fn register_submission_587(
    registry: &Registry,
    listener: SubmissionListener,
    acceptor: TlsAcceptor,
    services: SubmissionServices,
) {
    let acceptor = Arc::new(acceptor);
    registry.register_listener("smtp:587", move || async move {
        let result = listener
            .serve(move |stream, peer| {
                let acceptor = acceptor.clone();
                let services = services.clone();
                async move {
                    if let Err(err) = handle_connection(stream, peer, &acceptor, services).await {
                        tracing::debug!(%peer, error = %err, "submission connection ended");
                    }
                }
            })
            .await;
        if let Err(err) = result {
            tracing::error!(error = %err, "submission listener stopped");
        }
    });
}

async fn handle_connection(
    mut stream: TcpStream,
    peer: SocketAddr,
    acceptor: &TlsAcceptor,
    services: SubmissionServices,
) -> Result<()> {
    if crate::ip_guard::is_blocked(services.directory(), peer.ip()) {
        use tokio::io::AsyncWriteExt;
        let _ = stream.write_all(b"554 5.7.1 access denied\r\n").await;
        return Ok(());
    }
    let domains = local_domains(services.directory());
    let mut session = Session::new(stream, peer)
        .with_local_domains(domains.clone())
        .with_submission_services(services.clone());
    if session.run().await? != Flow::Upgrade {
        return Ok(());
    }

    let secured = upgrade(acceptor, session.into_inner()).await?;
    let mut secured_session = Session::new(secured, peer)
        .with_local_domains(domains)
        .with_starttls_upgrade()
        .with_submission_services(services);
    secured_session.run().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

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
    use irixmail_store::{BlobStore, FsBlobStore, RocksdbStore, Store};

    use crate::cmd_starttls::build_acceptor;

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
            let text = String::from_utf8_lossy(&buf);
            if text.ends_with("\r\n")
                && text
                    .lines()
                    .next_back()
                    .map(|line| line.len() >= 4 && line.as_bytes()[3] == b' ')
                    .unwrap_or(false)
            {
                break;
            }
        }
        String::from_utf8(buf).unwrap()
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "irixmail-smtp-listener-sub587-{}-{unique}",
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

    fn submission_services(dir: &TempDir) -> SubmissionServices {
        let store: Arc<dyn Store> = Arc::new(RocksdbStore::open(dir.path.join("db")).unwrap());
        let blobs: Arc<dyn BlobStore> =
            Arc::new(FsBlobStore::open(dir.path.join("blobs")).unwrap());
        let directory = Directory::new(Arc::clone(&store), Arc::new(IdGenerator::new(0)), None);
        SubmissionServices::new(directory, store, blobs)
    }

    #[tokio::test]
    async fn an_accepted_connection_reaches_the_handler() {
        let listener = SubmissionListener::bind("127.0.0.1:0".parse().unwrap())
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
        let listener = SubmissionListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let dir = TempDir::new();

        register_submission_587(&registry, listener, acceptor(), submission_services(&dir));

        assert_eq!(registry.len(), 1);
        assert_eq!(registry.registered()[0].0, "smtp:587");
    }

    #[tokio::test]
    async fn a_blocked_ip_is_refused_at_accept_before_the_greeting() {
        use irixmail_directory::IpAction;

        let registry = Registry::new();
        let listener = SubmissionListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let dir = TempDir::new();
        let services = submission_services(&dir);
        services
            .directory()
            .ip_rules()
            .create("127.0.0.1", IpAction::Block)
            .unwrap();

        register_submission_587(&registry, listener, acceptor(), services);
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
    async fn a_starttls_upgrade_does_not_send_a_second_greeting() {
        let registry = Registry::new();
        let listener = SubmissionListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let dir = TempDir::new();

        register_submission_587(
            &registry,
            listener,
            tls_acceptor(),
            submission_services(&dir),
        );
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

        secured.write_all(b"QUIT\r\n").await.unwrap();
        assert!(recv_reply(&mut secured).await.starts_with("221"));

        tasks.abort_all();
    }

    #[tokio::test]
    async fn the_registered_listener_greets_and_serves_a_session() {
        let registry = Registry::new();
        let listener = SubmissionListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let dir = TempDir::new();

        register_submission_587(&registry, listener, acceptor(), submission_services(&dir));
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
