use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::aws_lc_rs::default_provider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::{TlsAcceptor, TlsConnector};

use irixmail_core::registry::Registry;
use irixmail_core::IdGenerator;
use irixmail_directory::{AddressEntry, Directory, Role};
use irixmail_dns::Resolver;
use irixmail_mail::MailServices;
use irixmail_store::{
    BlobStore, ChangeNotifier, Flow, FsBlobStore, KeyPrefix, RocksdbStore, Store, Subspace,
    TtlStore,
};

use irixmail_smtp::{
    build_acceptor, enqueue_submission, register_inbound, register_submission_465, Greylist,
    GreylistConfig, ImplicitTlsListener, InboundListener, InboundServices, RateLimiter, SpfConfig,
    SpfVerifier, Submission, SubmissionServices,
};
use irixmail_smtp::{ArcVerifier, DkimVerifier, DmarcVerifier, DnsblConfig};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "irixmail-smtp-listener-wiring-{}-{unique}",
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

fn authenticator() -> mail_auth::MessageAuthenticator {
    use mail_auth::hickory_resolver::config::{ResolverConfig, ResolverOpts};
    mail_auth::MessageAuthenticator::new(ResolverConfig::default(), ResolverOpts::default())
        .unwrap()
}

fn dns_resolver() -> Resolver {
    use hickory_resolver::config::{ResolverConfig, ResolverOpts};
    Resolver::from_config(ResolverConfig::default(), ResolverOpts::default())
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
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
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

fn seed_local_account(directory: &Directory) {
    let domain = directory.domains().create("d.example", Vec::new()).unwrap();
    let account = directory
        .accounts()
        .create("c", domain.id, "", Role::User)
        .unwrap();
    directory
        .addresses()
        .set(AddressEntry::account("c@d.example", account.id))
        .unwrap();
}

fn inbound_services(dir: &TempDir) -> InboundServices {
    inbound_services_with_window(dir, Duration::ZERO)
}

fn inbound_services_with_window(dir: &TempDir, window: Duration) -> InboundServices {
    let store: Arc<dyn Store> = Arc::new(RocksdbStore::open(dir.path.join("db")).unwrap());
    let blobs: Arc<dyn BlobStore> = Arc::new(FsBlobStore::open(dir.path.join("blobs")).unwrap());
    let notifier = Arc::new(ChangeNotifier::new());
    let ttl = Arc::new(TtlStore::new());
    let directory = Directory::new(Arc::clone(&store), Arc::new(IdGenerator::new(0)), None);
    seed_local_account(&directory);
    InboundServices::new(
        directory,
        authenticator(),
        dns_resolver(),
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
            GreylistConfig { window },
        )),
        Arc::new(RateLimiter::new(ttl, Default::default())),
        MailServices::new(store, blobs, notifier),
    )
}

fn submission_services(dir: &TempDir) -> SubmissionServices {
    let store: Arc<dyn Store> = Arc::new(RocksdbStore::open(dir.path.join("db")).unwrap());
    let blobs: Arc<dyn BlobStore> = Arc::new(FsBlobStore::open(dir.path.join("blobs")).unwrap());
    let directory = Directory::new(Arc::clone(&store), Arc::new(IdGenerator::new(0)), None);
    let domain = directory.domains().create("d.example", Vec::new()).unwrap();
    let account = directory
        .accounts()
        .create("alice", domain.id, "", Role::User)
        .unwrap();
    directory
        .addresses()
        .set(AddressEntry::account("alice@d.example", account.id))
        .unwrap();
    SubmissionServices::new(directory, store, blobs)
}

async fn read_reply<S>(stream: &mut S) -> String
where
    S: AsyncReadExt + Unpin,
{
    let mut collected = Vec::new();
    let mut buffer = [0u8; 512];
    loop {
        let read = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        if read == 0 {
            break;
        }
        collected.extend_from_slice(&buffer[..read]);
        if let Some(line) = collected
            .split(|byte| *byte == b'\n')
            .rfind(|line| !line.is_empty())
        {
            if line.len() >= 4 && line[3] == b' ' {
                break;
            }
        }
    }
    String::from_utf8_lossy(&collected).into_owned()
}

async fn read_reply_within<S>(stream: &mut S, timeout: Duration) -> Option<String>
where
    S: AsyncReadExt + Unpin,
{
    let mut collected = Vec::new();
    let mut buffer = [0u8; 512];
    loop {
        let read = match tokio::time::timeout(timeout, stream.read(&mut buffer)).await {
            Ok(Ok(read)) => read,
            _ => return None,
        };
        if read == 0 {
            return Some(String::from_utf8_lossy(&collected).into_owned());
        }
        collected.extend_from_slice(&buffer[..read]);
        if let Some(line) = collected
            .split(|byte| *byte == b'\n')
            .rfind(|line| !line.is_empty())
        {
            if line.len() >= 4 && line[3] == b' ' {
                return Some(String::from_utf8_lossy(&collected).into_owned());
            }
        }
    }
}

#[tokio::test]
async fn the_inbound_listener_defers_a_greylisted_recipient_at_rcpt() {
    let dir = TempDir::new();
    let registry = Registry::new();
    let listener = InboundListener::bind("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    register_inbound(
        &registry,
        listener,
        acceptor(),
        inbound_services_with_window(&dir, Duration::from_secs(3600)),
    );
    let mut tasks = registry.start_all();

    let mut client = TcpStream::connect(addr).await.unwrap();
    assert!(read_reply(&mut client).await.starts_with("220 "));

    client.write_all(b"EHLO client\r\n").await.unwrap();
    assert!(read_reply(&mut client).await.contains("250"));

    client
        .write_all(b"MAIL FROM:<a@b.example>\r\n")
        .await
        .unwrap();
    assert!(read_reply(&mut client).await.starts_with("250"));

    client
        .write_all(b"RCPT TO:<c@d.example>\r\n")
        .await
        .unwrap();
    let rcpt = read_reply(&mut client).await;
    assert!(rcpt.contains("452 4.2.2 Greylisted"), "got: {rcpt}");

    client.write_all(b"DATA\r\n").await.unwrap();
    let data = read_reply(&mut client).await;
    assert!(
        data.starts_with("503"),
        "no body may transfer for a deferred recipient: {data}"
    );

    client
        .write_all(b"RCPT TO:<c@d.example>\r\n")
        .await
        .unwrap();
    let retry = read_reply(&mut client).await;
    assert!(
        retry.starts_with("250"),
        "the retried pair must be admitted: {retry}"
    );

    client.write_all(b"QUIT\r\n").await.unwrap();
    tasks.abort_all();
}

async fn open_submission(addr: SocketAddr) -> tokio_rustls::client::TlsStream<TcpStream> {
    let stream = TcpStream::connect(addr).await.unwrap();
    let server_name = ServerName::try_from("localhost").unwrap();
    let mut client = connector().connect(server_name, stream).await.unwrap();
    assert!(read_reply(&mut client).await.starts_with("220 "));
    client.write_all(b"EHLO client\r\n").await.unwrap();
    assert!(read_reply(&mut client).await.contains("250"));
    client
}

#[tokio::test]
async fn the_submission_listener_requires_authentication_before_it_admits_mail() {
    let dir = TempDir::new();
    let registry = Registry::new();
    let listener = ImplicitTlsListener::bind("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    register_submission_465(&registry, listener, acceptor(), submission_services(&dir));
    let mut tasks = registry.start_all();

    let mut client = open_submission(addr).await;

    client
        .write_all(b"MAIL FROM:<alice@d.example>\r\n")
        .await
        .unwrap();
    let reply = read_reply(&mut client).await;
    assert!(
        reply.contains("530 5.7.0 Authentication required"),
        "got: {reply}"
    );

    client.write_all(b"QUIT\r\n").await.unwrap();
    tasks.abort_all();
}

#[tokio::test]
async fn the_submission_listener_verifies_credentials_against_the_directory() {
    let dir = TempDir::new();
    let registry = Registry::new();
    let listener = ImplicitTlsListener::bind("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    register_submission_465(&registry, listener, acceptor(), submission_services(&dir));
    let mut tasks = registry.start_all();

    let mut client = open_submission(addr).await;

    let payload = STANDARD.encode(b"\0alice@d.example\0secret");
    client
        .write_all(format!("AUTH PLAIN {payload}\r\n").as_bytes())
        .await
        .unwrap();
    let reply = read_reply(&mut client).await;
    assert!(reply.contains("535 5.7.8"), "got: {reply}");

    client
        .write_all(b"MAIL FROM:<alice@d.example>\r\n")
        .await
        .unwrap();
    assert!(read_reply(&mut client)
        .await
        .contains("530 5.7.0 Authentication required"));

    client.write_all(b"QUIT\r\n").await.unwrap();
    tasks.abort_all();
}

#[tokio::test]
async fn a_submission_is_signed_for_the_domain_and_filed_into_the_queue() {
    let dir = TempDir::new();
    let services = submission_services(&dir);
    let signer = services.signer("d.example").expect("the domain is signed");

    let message =
        b"From: alice@d.example\r\nTo: bob@remote.example\r\nSubject: hi\r\n\r\nbody\r\n".to_vec();
    let signed = signer.sign_message(&message).unwrap();
    let stamped = String::from_utf8(signed.clone()).unwrap();
    assert!(stamped.starts_with("DKIM-Signature: "));
    assert!(stamped.contains("d=d.example"));

    let recipients = vec!["bob@remote.example".to_string()];
    let submission = Submission {
        return_path: "alice@d.example",
        recipients: &recipients,
    };
    enqueue_submission(
        services.store().as_ref(),
        services.blobs().as_ref(),
        &signed,
        &submission,
    )
    .unwrap();

    let mut found = 0;
    services
        .store()
        .iterate(
            &KeyPrefix::subspace(Subspace::Queue),
            &mut |_key, _value| {
                found += 1;
                Ok(Flow::Continue)
            },
        )
        .unwrap();
    assert_eq!(found, 1);
}

#[tokio::test]
async fn an_oversized_bdat_chunk_is_refused_before_the_server_allocates_for_it() {
    let dir = TempDir::new();
    let registry = Registry::new();
    let listener = InboundListener::bind("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    register_inbound(&registry, listener, acceptor(), inbound_services(&dir));
    let mut tasks = registry.start_all();

    let mut client = TcpStream::connect(addr).await.unwrap();
    assert!(read_reply(&mut client).await.starts_with("220 "));
    client.write_all(b"EHLO client\r\n").await.unwrap();
    assert!(read_reply(&mut client).await.contains("250"));
    client
        .write_all(b"MAIL FROM:<a@b.example>\r\n")
        .await
        .unwrap();
    assert!(read_reply(&mut client).await.starts_with("250"));
    client
        .write_all(b"RCPT TO:<c@d.example>\r\n")
        .await
        .unwrap();
    assert!(read_reply(&mut client).await.starts_with("250"));

    // Declares a 30 MB chunk (over the 25 MB limit) but sends no payload: a correct server
    // rejects on the declared size before committing memory, so the reply must arrive promptly.
    client.write_all(b"BDAT 30000000 LAST\r\n").await.unwrap();
    let reply = read_reply_within(&mut client, Duration::from_secs(3)).await;
    tasks.abort_all();

    assert!(
        reply.as_deref().is_some_and(|reply| reply.contains("552")),
        "expected a prompt 552 rejection of the oversized BDAT chunk, got: {reply:?}"
    );
}
