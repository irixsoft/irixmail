use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use rcgen::{CertificateParams, CustomExtension, KeyPair};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::PrivateKeyDer;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;

use irixmail_core::{Error, Result};

const ACME_TLS_ALPN: &[u8] = b"acme-tls/1";

pub struct AlpnChallenges {
    provider: Arc<CryptoProvider>,
    certs: Arc<RwLock<HashMap<String, Arc<CertifiedKey>>>>,
}

impl AlpnChallenges {
    pub fn new(provider: Arc<CryptoProvider>) -> Self {
        Self {
            provider,
            certs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn set(&self, domain: &str, key_authorization: &str) -> Result<()> {
        let digest = ring::digest::digest(&ring::digest::SHA256, key_authorization.as_bytes());

        let mut params = CertificateParams::new(vec![domain.to_string()])
            .map_err(|err| Error::internal(format!("ALPN challenge params: {err}")))?;
        params.custom_extensions = vec![CustomExtension::new_acme_identifier(digest.as_ref())];

        let key_pair = KeyPair::generate()
            .map_err(|err| Error::internal(format!("ALPN challenge key: {err}")))?;
        let cert = params
            .self_signed(&key_pair)
            .map_err(|err| Error::internal(format!("ALPN challenge cert: {err}")))?;

        let signing_key = self
            .provider
            .key_provider
            .load_private_key(PrivateKeyDer::Pkcs8(key_pair.serialize_der().into()))
            .map_err(|err| Error::internal(format!("ALPN challenge signing key: {err}")))?;

        let certified = CertifiedKey::new(vec![cert.der().clone()], signing_key);
        self.certs
            .write()
            .unwrap()
            .insert(domain.to_string(), Arc::new(certified));
        Ok(())
    }

    pub fn remove(&self, domain: &str) {
        self.certs.write().unwrap().remove(domain);
    }

    pub fn has_challenge(&self, domain: &str) -> bool {
        self.certs.read().unwrap().contains_key(domain)
    }

    fn is_acme_alpn(hello: &ClientHello) -> bool {
        hello
            .alpn()
            .map(|mut protocols| protocols.any(|proto| proto == ACME_TLS_ALPN))
            .unwrap_or(false)
    }
}

impl std::fmt::Debug for AlpnChallenges {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlpnChallenges").finish_non_exhaustive()
    }
}

impl ResolvesServerCert for AlpnChallenges {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        if !Self::is_acme_alpn(&client_hello) {
            return None;
        }
        let domain = client_hello.server_name()?;
        self.certs.read().unwrap().get(domain).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_a_challenge_records_a_certificate_for_the_domain() {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let challenges = AlpnChallenges::new(provider);
        assert!(!challenges.has_challenge("example.com"));

        challenges.set("example.com", "token.keyauth").unwrap();
        assert!(challenges.has_challenge("example.com"));

        challenges.remove("example.com");
        assert!(!challenges.has_challenge("example.com"));
    }
}
