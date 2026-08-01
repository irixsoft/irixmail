use std::sync::Arc;

use rustls::crypto::CryptoProvider;
use rustls::server::ResolvesServerCert;
use rustls::ServerConfig;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;

use irixmail_core::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartTlsReply {
    Ready,
    AlreadySecure,
    NotAvailable,
}

impl StartTlsReply {
    pub fn status(self) -> &'static str {
        match self {
            StartTlsReply::Ready => "OK",
            StartTlsReply::AlreadySecure => "BAD",
            StartTlsReply::NotAvailable => "NO",
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            StartTlsReply::Ready => "Begin TLS negotiation now",
            StartTlsReply::AlreadySecure => "TLS already active",
            StartTlsReply::NotAvailable => "TLS not available",
        }
    }

    pub fn upgrades(self) -> bool {
        matches!(self, StartTlsReply::Ready)
    }
}

pub fn starttls_reply(is_tls: bool, has_acceptor: bool) -> StartTlsReply {
    if is_tls {
        StartTlsReply::AlreadySecure
    } else if !has_acceptor {
        StartTlsReply::NotAvailable
    } else {
        StartTlsReply::Ready
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
        .map_err(|err| Error::protocol(format!("STARTTLS handshake failed: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plaintext_session_with_a_cert_is_ready_to_upgrade() {
        let reply = starttls_reply(false, true);
        assert_eq!(reply, StartTlsReply::Ready);
        assert!(reply.upgrades());
        assert_eq!(reply.status(), "OK");
    }

    #[test]
    fn an_encrypted_session_is_rejected() {
        let reply = starttls_reply(true, true);
        assert_eq!(reply, StartTlsReply::AlreadySecure);
        assert!(!reply.upgrades());
        assert_eq!(reply.status(), "BAD");
    }

    #[test]
    fn a_missing_acceptor_declines() {
        let reply = starttls_reply(false, false);
        assert_eq!(reply, StartTlsReply::NotAvailable);
        assert!(!reply.upgrades());
        assert_eq!(reply.status(), "NO");
    }

    #[test]
    fn an_already_secure_check_wins_over_a_missing_acceptor() {
        assert_eq!(starttls_reply(true, false), StartTlsReply::AlreadySecure);
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
