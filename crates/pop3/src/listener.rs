use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use irixmail_core::registry::Registry;
use irixmail_core::{Error, Result};
use irixmail_directory::Directory;
use irixmail_store::{BlobStore, ChangeNotifier};

use crate::cmd_stls::upgrade;
use crate::session::{Flow, Session};

pub struct Pop3Listener {
    listener: TcpListener,
}

impl Pop3Listener {
    pub async fn bind(addr: SocketAddr) -> Result<Self> {
        let listener = TcpListener::bind(addr).await.map_err(|err| {
            Error::internal(format!("could not bind the POP3 listener on {addr}: {err}"))
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
                .map_err(|err| Error::internal(format!("POP3 accept failed: {err}")))?;
            let handler = handler.clone();
            tokio::spawn(async move {
                handler(stream, peer).await;
            });
        }
    }
}

pub async fn register_pop3(
    registry: &Registry,
    stls: Option<SocketAddr>,
    implicit: Option<SocketAddr>,
    acceptor: TlsAcceptor,
    directory: Directory,
    blobs: Arc<dyn BlobStore>,
    notifier: Arc<ChangeNotifier>,
) -> Result<usize> {
    let mut registered = 0;

    if let Some(addr) = stls {
        let listener = Pop3Listener::bind(addr).await?;
        register_pop3_plain(
            registry,
            listener,
            acceptor.clone(),
            directory.clone(),
            blobs.clone(),
            notifier.clone(),
        );
        registered += 1;
    }

    if let Some(addr) = implicit {
        let listener = Pop3Listener::bind(addr).await?;
        register_pop3_implicit(registry, listener, acceptor, directory, blobs, notifier);
        registered += 1;
    }

    Ok(registered)
}

pub fn register_pop3_plain(
    registry: &Registry,
    listener: Pop3Listener,
    acceptor: TlsAcceptor,
    directory: Directory,
    blobs: Arc<dyn BlobStore>,
    notifier: Arc<ChangeNotifier>,
) {
    let acceptor = Arc::new(acceptor);
    registry.register_listener("pop3:110", move || async move {
        let result = listener
            .serve(move |stream, peer| {
                let acceptor = acceptor.clone();
                let directory = directory.clone();
                let blobs = blobs.clone();
                let notifier = notifier.clone();
                async move {
                    if let Err(err) =
                        handle_plain(stream, peer, &acceptor, directory, blobs, notifier).await
                    {
                        tracing::debug!(%peer, error = %err, "POP3 connection ended");
                    }
                }
            })
            .await;
        if let Err(err) = result {
            tracing::error!(error = %err, "POP3 110 listener stopped");
        }
    });
}

pub fn register_pop3_implicit(
    registry: &Registry,
    listener: Pop3Listener,
    acceptor: TlsAcceptor,
    directory: Directory,
    blobs: Arc<dyn BlobStore>,
    notifier: Arc<ChangeNotifier>,
) {
    let acceptor = Arc::new(acceptor);
    registry.register_listener("pop3:995", move || async move {
        let result = listener
            .serve(move |stream, peer| {
                let acceptor = acceptor.clone();
                let directory = directory.clone();
                let blobs = blobs.clone();
                let notifier = notifier.clone();
                async move {
                    if let Err(err) =
                        handle_implicit(stream, peer, &acceptor, directory, blobs, notifier).await
                    {
                        tracing::debug!(%peer, error = %err, "POP3 connection ended");
                    }
                }
            })
            .await;
        if let Err(err) = result {
            tracing::error!(error = %err, "POP3 995 listener stopped");
        }
    });
}

const BLOCKED_REPLY: &[u8] = b"-ERR access denied\r\n";

async fn handle_plain(
    mut stream: TcpStream,
    peer: SocketAddr,
    acceptor: &TlsAcceptor,
    directory: Directory,
    blobs: Arc<dyn BlobStore>,
    notifier: Arc<ChangeNotifier>,
) -> Result<()> {
    if directory.ip_rules().blocks(peer.ip()) {
        use tokio::io::AsyncWriteExt;
        let _ = stream.write_all(BLOCKED_REPLY).await;
        return Ok(());
    }
    let mut session = Session::new(stream, peer)
        .with_directory(directory.clone())
        .with_blobs(blobs.clone())
        .with_notifier(notifier.clone());
    if session.run().await? != Flow::Upgrade {
        return Ok(());
    }
    let sid = session.session_id();
    let secured = upgrade(acceptor, session.into_inner()).await?;
    let mut secured = Session::new(secured, peer)
        .with_session_id(sid)
        .with_tls()
        .without_greeting()
        .with_directory(directory)
        .with_blobs(blobs)
        .with_notifier(notifier);
    secured.run().await?;
    Ok(())
}

async fn handle_implicit(
    stream: TcpStream,
    peer: SocketAddr,
    acceptor: &TlsAcceptor,
    directory: Directory,
    blobs: Arc<dyn BlobStore>,
    notifier: Arc<ChangeNotifier>,
) -> Result<()> {
    let secured = acceptor
        .accept(stream)
        .await
        .map_err(|err| Error::protocol(format!("POP3 implicit TLS handshake failed: {err}")))?;
    if directory.ip_rules().blocks(peer.ip()) {
        use tokio::io::AsyncWriteExt;
        let mut secured = secured;
        let _ = secured.write_all(BLOCKED_REPLY).await;
        return Ok(());
    }
    let mut session = Session::new(secured, peer)
        .with_tls()
        .with_directory(directory)
        .with_blobs(blobs)
        .with_notifier(notifier);
    session.run().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::crypto::aws_lc_rs::default_provider;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
    use rustls::server::{ClientHello, ResolvesServerCert};
    use rustls::sign::CertifiedKey;
    use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::TlsConnector;

    use irixmail_core::IdGenerator;
    use irixmail_store::{BlobStore, ChangeNotifier, FsBlobStore, RocksdbStore, Store};

    use crate::cmd_stls::build_acceptor;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "irixmail-pop3-listener-{}-{unique}",
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

    fn directory(dir: &TempDir) -> Directory {
        let store: Arc<dyn Store> = Arc::new(RocksdbStore::open(dir.path.join("db")).unwrap());
        Directory::new(store, Arc::new(IdGenerator::new(0)), None)
    }

    fn blobs(dir: &TempDir) -> Arc<dyn BlobStore> {
        std::fs::create_dir_all(dir.path.join("blobs")).unwrap();
        Arc::new(FsBlobStore::open(dir.path.join("blobs")).unwrap())
    }

    fn notifier() -> Arc<ChangeNotifier> {
        Arc::new(ChangeNotifier::new())
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

    fn loopback() -> SocketAddr {
        "127.0.0.1:0".parse().unwrap()
    }

    #[tokio::test]
    async fn both_ports_register_two_listeners() {
        let registry = Registry::new();
        let dir = TempDir::new();
        let count = register_pop3(
            &registry,
            Some(loopback()),
            Some(loopback()),
            acceptor(),
            directory(&dir),
            blobs(&dir),
            notifier(),
        )
        .await
        .unwrap();

        assert_eq!(count, 2);
        let names: Vec<String> = registry
            .registered()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(names, vec!["pop3:110", "pop3:995"]);
    }

    #[tokio::test]
    async fn the_plaintext_listener_greets_and_quits() {
        let registry = Registry::new();
        let dir = TempDir::new();
        let listener = Pop3Listener::bind(loopback()).await.unwrap();
        let addr = listener.local_addr().unwrap();
        register_pop3_plain(
            &registry,
            listener,
            acceptor(),
            directory(&dir),
            blobs(&dir),
            notifier(),
        );
        let mut tasks = registry.start_all();

        let mut client = TcpStream::connect(addr).await.unwrap();
        let mut greeting = [0u8; 64];
        let read = client.read(&mut greeting).await.unwrap();
        assert!(greeting[..read].starts_with(b"+OK"));

        client.write_all(b"QUIT\r\n").await.unwrap();
        let mut farewell = [0u8; 64];
        let read = client.read(&mut farewell).await.unwrap();
        assert!(farewell[..read].starts_with(b"+OK"));

        tasks.abort_all();
    }

    #[tokio::test]
    async fn a_blocked_ip_is_refused_instead_of_the_greeting() {
        use irixmail_directory::IpAction;

        let registry = Registry::new();
        let dir = TempDir::new();
        let listener = Pop3Listener::bind(loopback()).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let directory = directory(&dir);
        directory
            .ip_rules()
            .create("127.0.0.1", IpAction::Block)
            .unwrap();
        register_pop3_plain(
            &registry,
            listener,
            acceptor(),
            directory,
            blobs(&dir),
            notifier(),
        );
        let mut tasks = registry.start_all();

        let mut client = TcpStream::connect(addr).await.unwrap();
        let mut reply = Vec::new();
        client.read_to_end(&mut reply).await.unwrap();
        let text = String::from_utf8(reply).unwrap();
        assert!(text.starts_with("-ERR access denied"), "got: {text:?}");
        assert!(!text.contains("+OK"));

        tasks.abort_all();
    }

    #[tokio::test]
    async fn the_implicit_listener_greets_after_the_handshake() {
        let registry = Registry::new();
        let dir = TempDir::new();
        let listener = Pop3Listener::bind(loopback()).await.unwrap();
        let addr = listener.local_addr().unwrap();
        register_pop3_implicit(
            &registry,
            listener,
            acceptor(),
            directory(&dir),
            blobs(&dir),
            notifier(),
        );
        let mut tasks = registry.start_all();

        let stream = TcpStream::connect(addr).await.unwrap();
        let server_name = ServerName::try_from("localhost").unwrap();
        let mut client = connector().connect(server_name, stream).await.unwrap();

        let mut greeting = [0u8; 64];
        let read = client.read(&mut greeting).await.unwrap();
        assert!(greeting[..read].starts_with(b"+OK"));

        tasks.abort_all();
    }

    async fn read_until<S: AsyncReadExt + Unpin>(client: &mut S, marker: &str) -> String {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let read =
                tokio::time::timeout(std::time::Duration::from_secs(10), client.read(&mut chunk))
                    .await
                    .expect("read did not time out")
                    .expect("read succeeds");
            if read == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..read]);
            if String::from_utf8_lossy(&buf).contains(marker) {
                break;
            }
        }
        String::from_utf8_lossy(&buf).into_owned()
    }

    async fn exchange<S: AsyncReadExt + AsyncWriteExt + Unpin>(
        client: &mut S,
        command: &[u8],
        done: &str,
    ) -> String {
        client.write_all(command).await.unwrap();
        read_until(client, done).await
    }

    async fn connect(addr: SocketAddr) -> tokio_rustls::client::TlsStream<TcpStream> {
        let stream = TcpStream::connect(addr).await.unwrap();
        let server_name = ServerName::try_from("localhost").unwrap();
        connector().connect(server_name, stream).await.unwrap()
    }

    #[tokio::test]
    async fn gate2_app_password_retr_returns_real_bytes_and_dele_expunges() {
        use irixmail_directory::{app_password, Role};
        use irixmail_mail::{deliver, provision_mailboxes, DeliveryRequest};

        let dir = TempDir::new();
        std::fs::create_dir_all(dir.path.join("blobs")).unwrap();
        let store: Arc<dyn Store> = Arc::new(RocksdbStore::open(dir.path.join("db")).unwrap());
        let directory = Directory::new(store.clone(), Arc::new(IdGenerator::new(0)), None);
        let domain = directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let account = directory
            .accounts()
            .create("alice", domain.id, "Alice", Role::User)
            .unwrap();
        let minted = app_password::generate(1, "gate2", 0).unwrap();
        directory
            .credentials()
            .add_app_password(account.id, minted.record)
            .unwrap();

        let blobs: Arc<dyn BlobStore> =
            Arc::new(FsBlobStore::open(dir.path.join("blobs")).unwrap());
        let notifier = Arc::new(ChangeNotifier::new());
        let message: &[u8] = b"Subject: Gate2\r\nFrom: bob@example.net\r\n\r\npop3 body line\r\n";
        let mailboxes = provision_mailboxes(account.created_at);
        deliver(
            store.as_ref(),
            blobs.as_ref(),
            &notifier,
            &DeliveryRequest {
                account: &account,
                mailboxes: &mailboxes,
                sieve: None,
                mail_from: "bob@example.net",
                recipient: "alice@example.com",
                document_id: 1,
                raw: message,
                target_override: None,
                received_at: 1_700_000_000,
            },
        )
        .unwrap();

        let registry = Registry::new();
        let listener = Pop3Listener::bind(loopback()).await.unwrap();
        let addr = listener.local_addr().unwrap();
        register_pop3_implicit(&registry, listener, acceptor(), directory, blobs, notifier);
        let mut tasks = registry.start_all();

        let pass = format!("PASS {}\r\n", minted.plaintext);

        let mut client = connect(addr).await;
        let _ = read_until(&mut client, "\r\n").await;
        let _ = exchange(&mut client, b"USER alice@example.com\r\n", "\r\n").await;
        let out = exchange(&mut client, pass.as_bytes(), "\r\n").await;
        assert!(out.contains("+OK"), "app-password PASS: {out}");
        let out = exchange(&mut client, b"RETR 1\r\n", "\r\n.\r\n").await;
        assert!(
            out.contains(&format!("+OK {} octets", message.len())),
            "RETR size: {out}"
        );
        assert!(out.contains("pop3 body line"), "RETR body: {out}");
        let out = exchange(&mut client, b"DELE 1\r\n", "\r\n").await;
        assert!(out.contains("+OK"), "DELE: {out}");
        let _ = exchange(&mut client, b"QUIT\r\n", "\r\n").await;

        let mut second = connect(addr).await;
        let _ = read_until(&mut second, "\r\n").await;
        let _ = exchange(&mut second, b"USER alice@example.com\r\n", "\r\n").await;
        let _ = exchange(&mut second, pass.as_bytes(), "\r\n").await;
        let out = exchange(&mut second, b"STAT\r\n", "\r\n").await;
        assert!(
            out.contains("+OK 0 0"),
            "maildrop empty after DELE+QUIT: {out}"
        );
        let _ = exchange(&mut second, b"QUIT\r\n", "\r\n").await;

        tasks.abort_all();
    }
}
