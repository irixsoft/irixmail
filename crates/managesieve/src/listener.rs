use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use irixmail_core::registry::Registry;
use irixmail_core::{Error, Result};
use irixmail_directory::Directory;

use crate::cmd_starttls::upgrade;
use crate::session::{Flow, Session};

pub struct ManageSieveListener {
    listener: TcpListener,
}

impl ManageSieveListener {
    pub async fn bind(addr: SocketAddr) -> Result<Self> {
        let listener = TcpListener::bind(addr).await.map_err(|err| {
            Error::internal(format!(
                "could not bind the ManageSieve listener on {addr}: {err}"
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
                .map_err(|err| Error::internal(format!("ManageSieve accept failed: {err}")))?;
            let handler = handler.clone();
            tokio::spawn(async move {
                handler(stream, peer).await;
            });
        }
    }
}

pub async fn register_managesieve(
    registry: &Registry,
    addr: Option<SocketAddr>,
    acceptor: TlsAcceptor,
    directory: Directory,
) -> Result<usize> {
    let Some(addr) = addr else {
        return Ok(0);
    };
    let listener = ManageSieveListener::bind(addr).await?;
    let acceptor = Arc::new(acceptor);
    registry.register_listener("managesieve:4190", move || async move {
        let result = listener
            .serve(move |stream, peer| {
                let acceptor = acceptor.clone();
                let directory = directory.clone();
                async move {
                    if let Err(err) = handle_plain(stream, peer, &acceptor, directory).await {
                        tracing::debug!(%peer, error = %err, "ManageSieve connection ended");
                    }
                }
            })
            .await;
        if let Err(err) = result {
            tracing::error!(error = %err, "ManageSieve 4190 listener stopped");
        }
    });
    Ok(1)
}

const BLOCKED_REPLY: &[u8] = b"BYE \"access denied\"\r\n";

async fn handle_plain(
    mut stream: TcpStream,
    peer: SocketAddr,
    acceptor: &TlsAcceptor,
    directory: Directory,
) -> Result<()> {
    if directory.ip_rules().blocks(peer.ip()) {
        use tokio::io::AsyncWriteExt;
        let _ = stream.write_all(BLOCKED_REPLY).await;
        return Ok(());
    }
    let mut session = Session::new(stream, peer).with_directory(directory.clone());
    if session.run().await? != Flow::Upgrade {
        return Ok(());
    }
    let sid = session.session_id();
    let secured = upgrade(acceptor, session.into_inner()).await?;
    let mut secured = Session::new(secured, peer)
        .with_session_id(sid)
        .with_tls()
        .with_directory(directory);
    secured.run().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use irixmail_core::IdGenerator;
    use irixmail_store::{RocksdbStore, Store};

    fn directory() -> (Directory, std::path::PathBuf) {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "irixmail-managesieve-listener-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        let store: Arc<dyn Store> = Arc::new(RocksdbStore::open(path.join("db")).unwrap());
        (
            Directory::new(store, Arc::new(IdGenerator::new(0)), None),
            path,
        )
    }

    #[tokio::test]
    async fn the_listener_greets_a_connection_with_capabilities() {
        let (directory, path) = directory();
        let listener = ManageSieveListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = listener
                .serve(move |stream, peer| {
                    let directory = directory.clone();
                    async move {
                        let mut session = Session::new(stream, peer).with_directory(directory);
                        let _ = session.run().await;
                    }
                })
                .await;
        });

        let mut client = TcpStream::connect(addr).await.unwrap();
        client.write_all(b"LOGOUT\r\n").await.unwrap();
        let mut reply = String::new();
        client.read_to_string(&mut reply).await.unwrap();
        assert!(reply.contains("\"IMPLEMENTATION\" \"IRIXMAIL\""));
        assert!(reply.contains("\"STARTTLS\""));
        assert!(reply.contains("OK \"Bye\""));
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn a_blocked_ip_is_told_bye_before_any_greeting() {
        let (directory, path) = directory();
        directory
            .ip_rules()
            .create("127.0.0.1/32", irixmail_directory::IpAction::Block)
            .unwrap();
        let listener = ManageSieveListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(NoCert));
        let acceptor = TlsAcceptor::from(Arc::new(config));
        tokio::spawn(async move {
            let _ = listener
                .serve(move |stream, peer| {
                    let directory = directory.clone();
                    let acceptor = acceptor.clone();
                    async move {
                        let _ = handle_plain(stream, peer, &acceptor, directory).await;
                    }
                })
                .await;
        });

        let mut client = TcpStream::connect(addr).await.unwrap();
        let mut reply = String::new();
        client.read_to_string(&mut reply).await.unwrap();
        assert_eq!(reply, "BYE \"access denied\"\r\n");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[derive(Debug)]
    struct NoCert;

    impl rustls::server::ResolvesServerCert for NoCert {
        fn resolve(
            &self,
            _hello: rustls::server::ClientHello<'_>,
        ) -> Option<Arc<rustls::sign::CertifiedKey>> {
            None
        }
    }
}
