use mail_auth::common::crypto::{Ed25519Key, RsaKey, Sha256};
use mail_auth::common::headers::HeaderWriter;
use mail_auth::dkim::{Canonicalization, DkimSigner as Builder, Done, Signature};

use irixmail_core::{Error, Result};
use irixmail_dns::dkim_keys::{DkimAlgorithm, DkimKey};

const SIGNED_HEADERS: &[&str] = &["From", "To", "Cc", "Subject", "Date", "Message-ID"];

pub enum DomainSigner {
    Rsa(Builder<RsaKey<Sha256>, Done>),
    Ed25519(Builder<Ed25519Key, Done>),
}

impl DomainSigner {
    pub fn from_key(domain: &str, key: &DkimKey) -> Result<Self> {
        match key.algorithm {
            DkimAlgorithm::Rsa => {
                let signing_key = RsaKey::<Sha256>::from_key_der(pkcs8(&key.private_pkcs8_der)?)
                    .map_err(|err| {
                        Error::internal(format!("could not load the RSA DKIM key: {err}"))
                    })?;
                Ok(DomainSigner::Rsa(template(
                    Builder::from_key(signing_key),
                    domain,
                    &key.selector,
                )))
            }
            DkimAlgorithm::Ed25519 => {
                let signing_key =
                    Ed25519Key::from_pkcs8_der(&key.private_pkcs8_der).map_err(|err| {
                        Error::internal(format!("could not load the Ed25519 DKIM key: {err}"))
                    })?;
                Ok(DomainSigner::Ed25519(template(
                    Builder::from_key(signing_key),
                    domain,
                    &key.selector,
                )))
            }
        }
    }

    fn sign(&self, message: &[u8]) -> Result<Signature> {
        let signed = match self {
            DomainSigner::Rsa(signer) => signer.sign(message),
            DomainSigner::Ed25519(signer) => signer.sign(message),
        };
        signed.map_err(|err| Error::internal(format!("could not DKIM-sign the message: {err}")))
    }

    pub fn sign_message(&self, message: &[u8]) -> Result<Vec<u8>> {
        let signature = self.sign(message)?;
        let mut header = Vec::with_capacity(message.len() + 256);
        signature.write_header(&mut header);
        header.extend_from_slice(message);
        Ok(header)
    }
}

fn template<T>(
    builder: Builder<T, mail_auth::dkim::NeedDomain>,
    domain: &str,
    selector: &str,
) -> Builder<T, Done>
where
    T: mail_auth::common::crypto::SigningKey,
{
    builder
        .domain(domain)
        .selector(selector)
        .headers(SIGNED_HEADERS.iter().copied())
        .header_canonicalization(Canonicalization::Relaxed)
        .body_canonicalization(Canonicalization::Relaxed)
}

fn pkcs8(der: &[u8]) -> Result<rustls::pki_types::PrivateKeyDer<'_>> {
    Ok(rustls::pki_types::PrivateKeyDer::Pkcs8(
        rustls::pki_types::PrivatePkcs8KeyDer::from(der),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use irixmail_dns::dkim_keys::{generate_ed25519, generate_rsa};

    const MESSAGE: &[u8] =
        b"From: alice@irixsoft.com\r\nTo: bob@example.org\r\nSubject: hello\r\n\r\nbody\r\n";

    fn rsa_key() -> DkimKey {
        generate_rsa("mail").unwrap()
    }

    #[test]
    fn an_ed25519_signature_is_prepended_and_names_the_domain() {
        let key = generate_ed25519("mail").unwrap();
        let signer = DomainSigner::from_key("irixsoft.com", &key).unwrap();
        let signed = signer.sign_message(MESSAGE).unwrap();

        let header = String::from_utf8(signed.clone()).unwrap();
        assert!(header.starts_with("DKIM-Signature: "));
        assert!(header.contains("d=irixsoft.com"));
        assert!(header.contains("s=mail"));
        assert!(header.contains("a=ed25519-sha256"));
        assert!(signed.ends_with(MESSAGE));
    }

    #[test]
    fn an_rsa_signature_uses_the_rsa_algorithm() {
        let signer = DomainSigner::from_key("irixsoft.com", &rsa_key()).unwrap();
        let signed = signer.sign_message(MESSAGE).unwrap();
        let header = String::from_utf8(signed).unwrap();
        assert!(header.contains("a=rsa-sha256"));
        assert!(header.contains("c=relaxed/relaxed"));
    }

    #[test]
    fn the_signature_binds_the_author_and_subject_headers() {
        let signer = DomainSigner::from_key("irixsoft.com", &rsa_key()).unwrap();
        let signed = signer.sign_message(MESSAGE).unwrap();
        let header = String::from_utf8(signed).unwrap();
        for name in ["From", "To", "Subject"] {
            assert!(header.contains(name));
        }
    }

    #[test]
    fn a_corrupt_key_is_rejected() {
        let mut key = rsa_key();
        key.private_pkcs8_der = vec![0, 1, 2, 3];
        assert!(DomainSigner::from_key("irixsoft.com", &key).is_err());
    }

    #[test]
    fn the_same_key_signs_the_same_message_repeatably() {
        let signer = DomainSigner::from_key("irixsoft.com", &rsa_key()).unwrap();
        let first = signer.sign_message(MESSAGE).unwrap();
        let second = signer.sign_message(MESSAGE).unwrap();
        assert!(first.ends_with(MESSAGE));
        assert!(second.ends_with(MESSAGE));
        assert!(String::from_utf8(first)
            .unwrap()
            .starts_with("DKIM-Signature: "));
    }
}
