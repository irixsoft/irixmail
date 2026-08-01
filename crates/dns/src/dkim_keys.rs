use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use rand::rngs::OsRng;
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey};
use rsa::RsaPrivateKey;
use serde::{Deserialize, Serialize};

use irixmail_core::{Error, Result};

const RSA_KEY_BITS: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DkimAlgorithm {
    Rsa,
    Ed25519,
}

impl DkimAlgorithm {
    pub fn record_tag(self) -> &'static str {
        match self {
            DkimAlgorithm::Rsa => "rsa",
            DkimAlgorithm::Ed25519 => "ed25519",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DkimKey {
    pub selector: String,
    pub algorithm: DkimAlgorithm,
    pub private_pkcs8_der: Vec<u8>,
    pub public_key_b64: String,
}

pub fn generate_rsa(selector: impl Into<String>) -> Result<DkimKey> {
    generate_rsa_with_bits(selector, RSA_KEY_BITS)
}

fn generate_rsa_with_bits(selector: impl Into<String>, bits: usize) -> Result<DkimKey> {
    let mut rng = OsRng;
    let private = RsaPrivateKey::new(&mut rng, bits)
        .map_err(|err| Error::internal(format!("could not generate the RSA DKIM key: {err}")))?;

    let private_der = private
        .to_pkcs8_der()
        .map_err(|err| {
            Error::internal(format!("could not encode the RSA DKIM private key: {err}"))
        })?
        .as_bytes()
        .to_vec();

    let public_der = private.to_public_key().to_public_key_der().map_err(|err| {
        Error::internal(format!("could not encode the RSA DKIM public key: {err}"))
    })?;

    Ok(DkimKey {
        selector: selector.into(),
        algorithm: DkimAlgorithm::Rsa,
        private_pkcs8_der: private_der,
        public_key_b64: STANDARD.encode(public_der.as_bytes()),
    })
}

pub fn generate_ed25519(selector: impl Into<String>) -> Result<DkimKey> {
    let rng = SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng)
        .map_err(|_| Error::internal("could not generate the Ed25519 DKIM key".to_string()))?;
    let keypair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).map_err(|err| {
        Error::internal(format!(
            "could not load the generated Ed25519 DKIM key: {err}"
        ))
    })?;

    Ok(DkimKey {
        selector: selector.into(),
        algorithm: DkimAlgorithm::Ed25519,
        private_pkcs8_der: pkcs8.as_ref().to_vec(),
        public_key_b64: STANDARD.encode(keypair.public_key().as_ref()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs8::DecodePrivateKey;

    #[test]
    fn the_record_tag_matches_the_algorithm() {
        assert_eq!(DkimAlgorithm::Rsa.record_tag(), "rsa");
        assert_eq!(DkimAlgorithm::Ed25519.record_tag(), "ed25519");
    }

    #[test]
    fn an_ed25519_key_publishes_a_32_byte_public_key_and_reloads() {
        let key = generate_ed25519("mail").unwrap();

        assert_eq!(key.selector, "mail");
        assert_eq!(key.algorithm, DkimAlgorithm::Ed25519);
        assert!(!key.private_pkcs8_der.is_empty());

        let public = STANDARD.decode(&key.public_key_b64).unwrap();
        assert_eq!(public.len(), 32);

        Ed25519KeyPair::from_pkcs8(&key.private_pkcs8_der).expect("ed25519 key reloads");
    }

    #[test]
    fn an_rsa_key_reloads_and_publishes_a_non_empty_public_key() {
        let key = generate_rsa_with_bits("mail", 1024).unwrap();

        assert_eq!(key.algorithm, DkimAlgorithm::Rsa);
        RsaPrivateKey::from_pkcs8_der(&key.private_pkcs8_der).expect("rsa key reloads");

        let public = STANDARD.decode(&key.public_key_b64).unwrap();
        assert!(!public.is_empty());
    }
}
