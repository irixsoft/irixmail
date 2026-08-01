use std::net::SocketAddr;

use tokio_rustls::TlsAcceptor;

use irixmail_core::registry::Registry;
use irixmail_core::Result;

use crate::listener_sub_465::{register_submission_465, ImplicitTlsListener};
use crate::listener_sub_587::{register_submission_587, SubmissionListener};
use crate::session_services::SubmissionServices;

pub async fn register_submission(
    registry: &Registry,
    starttls: Option<SocketAddr>,
    implicit: Option<SocketAddr>,
    acceptor: TlsAcceptor,
    services: SubmissionServices,
) -> Result<usize> {
    let mut registered = 0;

    if let Some(addr) = starttls {
        let listener = SubmissionListener::bind(addr).await?;
        register_submission_587(registry, listener, acceptor.clone(), services.clone());
        registered += 1;
    }

    if let Some(addr) = implicit {
        let listener = ImplicitTlsListener::bind(addr).await?;
        register_submission_465(registry, listener, acceptor, services);
        registered += 1;
    }

    Ok(registered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use rustls::server::{ClientHello, ResolvesServerCert};
    use rustls::sign::CertifiedKey;

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
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        build_acceptor(provider, Arc::new(NoCert)).unwrap()
    }

    fn loopback() -> SocketAddr {
        "127.0.0.1:0".parse().unwrap()
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "irixmail-smtp-listener-sub-{}-{unique}",
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
    async fn both_ports_register_two_listeners() {
        let registry = Registry::new();
        let dir = TempDir::new();

        let count = register_submission(
            &registry,
            Some(loopback()),
            Some(loopback()),
            acceptor(),
            submission_services(&dir),
        )
        .await
        .unwrap();

        assert_eq!(count, 2);
        let names: Vec<String> = registry
            .registered()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(names, vec!["smtp:587", "smtp:465"]);
    }

    #[tokio::test]
    async fn the_starttls_port_can_be_disabled() {
        let registry = Registry::new();
        let dir = TempDir::new();

        let count = register_submission(
            &registry,
            None,
            Some(loopback()),
            acceptor(),
            submission_services(&dir),
        )
        .await
        .unwrap();

        assert_eq!(count, 1);
        assert_eq!(registry.registered()[0].0, "smtp:465");
    }

    #[tokio::test]
    async fn the_implicit_port_can_be_disabled() {
        let registry = Registry::new();
        let dir = TempDir::new();

        let count = register_submission(
            &registry,
            Some(loopback()),
            None,
            acceptor(),
            submission_services(&dir),
        )
        .await
        .unwrap();

        assert_eq!(count, 1);
        assert_eq!(registry.registered()[0].0, "smtp:587");
    }

    #[tokio::test]
    async fn no_ports_register_nothing() {
        let registry = Registry::new();
        let dir = TempDir::new();

        let count =
            register_submission(&registry, None, None, acceptor(), submission_services(&dir))
                .await
                .unwrap();

        assert_eq!(count, 0);
        assert!(registry.is_empty());
    }
}
