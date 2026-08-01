use std::sync::Arc;

use rustls::crypto::CryptoProvider;
use rustls::server::ResolvesServerCert;
use rustls::ServerConfig;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;

use irixmail_core::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StlsReply {
    Ready,
    AlreadySecure,
    NotAvailable,
}

impl StlsReply {
    pub fn line(self) -> &'static [u8] {
        match self {
            StlsReply::Ready => b"+OK begin TLS negotiation\r\n",
            StlsReply::AlreadySecure => b"-ERR TLS already active\r\n",
            StlsReply::NotAvailable => b"-ERR TLS not available\r\n",
        }
    }

    pub fn upgrades(self) -> bool {
        matches!(self, StlsReply::Ready)
    }
}

pub fn stls_reply(is_tls: bool, has_acceptor: bool) -> StlsReply {
    if is_tls {
        StlsReply::AlreadySecure
    } else if !has_acceptor {
        StlsReply::NotAvailable
    } else {
        StlsReply::Ready
    }
}

pub fn build_acceptor(
    provider: Arc<CryptoProvider>,
    resolver: Arc<dyn ResolvesServerCert>,
) -> Result<TlsAcceptor> {
    let config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|err| Error::internal(format!("TLS provider error: {err}")))?
        .with_no_client_auth()
        .with_cert_resolver(resolver);
    Ok(TlsAcceptor::from(Arc::new(config)))
}

pub async fn upgrade<S>(acceptor: &TlsAcceptor, stream: S) -> Result<TlsStream<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    acceptor
        .accept(stream)
        .await
        .map_err(|err| Error::protocol(format!("STLS handshake failed: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plaintext_session_with_a_cert_is_ready() {
        let reply = stls_reply(false, true);
        assert_eq!(reply, StlsReply::Ready);
        assert!(reply.upgrades());
        assert!(reply.line().starts_with(b"+OK"));
    }

    #[test]
    fn an_encrypted_session_is_refused() {
        let reply = stls_reply(true, true);
        assert_eq!(reply, StlsReply::AlreadySecure);
        assert!(!reply.upgrades());
        assert!(reply.line().starts_with(b"-ERR"));
    }

    #[test]
    fn a_missing_acceptor_declines() {
        assert_eq!(stls_reply(false, false), StlsReply::NotAvailable);
    }

    #[test]
    fn an_acceptor_is_built_from_a_resolver() {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let resolver: Arc<dyn ResolvesServerCert> = Arc::new(EmptyResolver);
        let acceptor = build_acceptor(provider, resolver).unwrap();
        assert!(Arc::strong_count(acceptor.config()) >= 1);
    }

    #[derive(Debug)]
    struct EmptyResolver;

    impl ResolvesServerCert for EmptyResolver {
        fn resolve(
            &self,
            _client_hello: rustls::server::ClientHello<'_>,
        ) -> Option<Arc<rustls::sign::CertifiedKey>> {
            None
        }
    }
}
