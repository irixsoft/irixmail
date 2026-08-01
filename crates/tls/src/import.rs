use std::io::BufReader;
use std::sync::Arc;

use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::ServerConfig;

use irixmail_core::{Error, Result};

use crate::cert_store::CertMaterial;

pub fn import_pem(
    provider: Arc<CryptoProvider>,
    cert_pem: &str,
    key_pem: &str,
) -> Result<CertMaterial> {
    let chain = parse_certs(cert_pem)?;
    let key = parse_key(key_pem)?;
    validate(provider, chain.clone(), key.clone_key())?;
    Ok(CertMaterial { chain, key })
}

fn parse_certs(pem: &str) -> Result<Vec<CertificateDer<'static>>> {
    let mut reader = BufReader::new(pem.as_bytes());
    let chain = rustls_pemfile::certs(&mut reader).collect::<std::result::Result<Vec<_>, _>>()?;
    if chain.is_empty() {
        return Err(Error::invalid_input(
            "no certificate found in the uploaded PEM",
        ));
    }
    Ok(chain)
}

fn parse_key(pem: &str) -> Result<PrivateKeyDer<'static>> {
    let mut reader = BufReader::new(pem.as_bytes());
    rustls_pemfile::private_key(&mut reader)?
        .ok_or_else(|| Error::invalid_input("no private key found in the uploaded PEM"))
}

fn validate(
    provider: Arc<CryptoProvider>,
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> Result<()> {
    ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|err| Error::internal(format!("TLS provider error: {err}")))?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(|err| {
            Error::invalid_input(format!(
                "the certificate and private key do not match: {err}"
            ))
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> Arc<CryptoProvider> {
        Arc::new(rustls::crypto::aws_lc_rs::default_provider())
    }

    fn self_signed() -> (String, String) {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        (certified.cert.pem(), certified.key_pair.serialize_pem())
    }

    #[test]
    fn a_matching_certificate_and_key_import() {
        let (cert, key) = self_signed();
        assert!(import_pem(provider(), &cert, &key).is_ok());
    }

    #[test]
    fn a_mismatched_key_is_rejected() {
        let (cert, _) = self_signed();
        let (_, other_key) = self_signed();
        assert!(import_pem(provider(), &cert, &other_key).is_err());
    }

    #[test]
    fn malformed_input_is_rejected() {
        assert!(import_pem(provider(), "not a cert", "not a key").is_err());
    }
}
