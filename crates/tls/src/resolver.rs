use std::sync::Arc;

use arc_swap::ArcSwapOption;
use rustls::crypto::CryptoProvider;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;

use irixmail_core::{Error, Result};

use crate::cert_store::CertMaterial;

pub struct SniResolver {
    provider: Arc<CryptoProvider>,
    current: ArcSwapOption<CertifiedKey>,
}

impl SniResolver {
    pub fn new(provider: Arc<CryptoProvider>) -> Self {
        Self {
            provider,
            current: ArcSwapOption::empty(),
        }
    }

    pub fn set(&self, material: &CertMaterial) -> Result<()> {
        let key = self
            .provider
            .key_provider
            .load_private_key(material.key.clone_key())
            .map_err(|err| Error::internal(format!("invalid certificate private key: {err}")))?;
        self.current.store(Some(Arc::new(CertifiedKey::new(
            material.chain.clone(),
            key,
        ))));
        Ok(())
    }

    pub fn has_certificate(&self) -> bool {
        self.current.load().is_some()
    }
}

impl std::fmt::Debug for SniResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SniResolver")
            .field("has_certificate", &self.has_certificate())
            .finish()
    }
}

impl ResolvesServerCert for SniResolver {
    fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.current.load_full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::self_signed;

    #[test]
    fn setting_material_makes_a_certificate_resolvable() {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let resolver = SniResolver::new(provider);
        assert!(!resolver.has_certificate());

        let material = self_signed::generate(vec!["mail.example.com".to_string()]).unwrap();
        resolver.set(&material).unwrap();

        assert!(resolver.has_certificate());
        assert!(resolver.current.load_full().is_some());
    }
}
