use std::path::PathBuf;
use std::sync::Arc;

use rustls::crypto::CryptoProvider;
use rustls::server::ResolvesServerCert;
use rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

use irixmail_core::{Error, Result, Server};

use crate::acme_alpn::AlpnChallenges;
use crate::acme_http01::Http01Challenges;
use crate::cert_store::CertStore;
use crate::resolver::SniResolver;

pub struct TlsServices {
    provider: Arc<CryptoProvider>,
    sni_resolver: Arc<SniResolver>,
    alpn_challenges: Arc<AlpnChallenges>,
    http01_challenges: Http01Challenges,
    cert_store: Arc<CertStore>,
}

impl TlsServices {
    pub fn new(provider: Arc<CryptoProvider>, certs_dir: PathBuf) -> Self {
        Self {
            sni_resolver: Arc::new(SniResolver::new(provider.clone())),
            alpn_challenges: Arc::new(AlpnChallenges::new(provider.clone())),
            http01_challenges: Http01Challenges::new(),
            cert_store: Arc::new(CertStore::new(certs_dir)),
            provider,
        }
    }

    pub fn from_server(server: &Server) -> Result<Self> {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        Ok(Self::new(provider, certs_dir(server)))
    }

    pub fn provider(&self) -> &Arc<CryptoProvider> {
        &self.provider
    }

    pub fn sni_resolver(&self) -> &Arc<SniResolver> {
        &self.sni_resolver
    }

    pub fn alpn_challenges(&self) -> &Arc<AlpnChallenges> {
        &self.alpn_challenges
    }

    pub fn http01_challenges(&self) -> &Http01Challenges {
        &self.http01_challenges
    }

    pub fn cert_store(&self) -> &CertStore {
        &self.cert_store
    }

    pub fn cert_store_handle(&self) -> Arc<CertStore> {
        Arc::clone(&self.cert_store)
    }

    pub fn sni_resolver_handle(&self) -> Arc<SniResolver> {
        Arc::clone(&self.sni_resolver)
    }

    pub fn acceptor(&self) -> Result<TlsAcceptor> {
        let resolver = self.sni_resolver.clone() as Arc<dyn ResolvesServerCert>;
        let config = ServerConfig::builder_with_provider(self.provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|err| Error::internal(format!("TLS provider error: {err}")))?
            .with_no_client_auth()
            .with_cert_resolver(resolver);
        Ok(TlsAcceptor::from(Arc::new(config)))
    }
}

fn certs_dir(server: &Server) -> PathBuf {
    let config = server.config();
    config
        .bootstrap
        .paths
        .db
        .parent()
        .map(|parent| parent.join("certs"))
        .unwrap_or_else(|| PathBuf::from("certs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_starts_without_a_certificate_or_challenge() {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let tls = TlsServices::new(provider, std::env::temp_dir().join("irixmail-tls-test"));
        assert!(!tls.sni_resolver().has_certificate());
        assert!(!tls.alpn_challenges().has_challenge("example.com"));
    }
}
