use mail_auth::common::crypto::Algorithm;
use mail_auth::common::verify::VerifySignature;
use mail_auth::{AuthenticatedMessage, DkimOutput, DkimResult, MessageAuthenticator};

use irixmail_core::{Error, Result};

// RFC 8301: rsa-sha1 and RSA keys under 1024 bits must not verify; a 1024-bit
// RSA signature is 128 bytes, so a shorter b= betrays a weaker key.
pub(crate) fn demote_insecure(message: &mut AuthenticatedMessage<'_>) {
    for header in message.dkim_headers.iter_mut() {
        if let Ok(signature) = &header.header {
            if signature.a == Algorithm::RsaSha1
                || (signature.a == Algorithm::RsaSha256 && signature.b.len() < 128)
            {
                header.header = Err(mail_auth::Error::CryptoError(
                    "insecure DKIM signature".to_string(),
                ));
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DkimSignatureResult {
    pub domain: String,
    pub result: DkimVerdict,
}

impl DkimSignatureResult {
    pub fn passed(&self) -> bool {
        self.result == DkimVerdict::Pass
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DkimVerdict {
    Pass,
    Neutral,
    Fail,
    PermError,
    TempError,
    None,
}

impl From<&DkimResult> for DkimVerdict {
    fn from(result: &DkimResult) -> Self {
        match result {
            DkimResult::Pass => DkimVerdict::Pass,
            DkimResult::Neutral(_) => DkimVerdict::Neutral,
            DkimResult::Fail(_) => DkimVerdict::Fail,
            DkimResult::PermError(_) => DkimVerdict::PermError,
            DkimResult::TempError(_) => DkimVerdict::TempError,
            DkimResult::None => DkimVerdict::None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DkimDecision {
    pub signatures: Vec<DkimSignatureResult>,
}

impl DkimDecision {
    pub fn from_outputs(outputs: &[DkimOutput<'_>]) -> Self {
        DkimDecision {
            signatures: outputs
                .iter()
                .map(|output| DkimSignatureResult {
                    domain: output
                        .signature()
                        .map(|signature| signature.domain().to_string())
                        .unwrap_or_default(),
                    result: output.result().into(),
                })
                .collect(),
        }
    }

    pub fn pass(&self) -> bool {
        self.signatures.iter().any(DkimSignatureResult::passed)
    }

    pub fn temp_error(&self) -> bool {
        !self.pass()
            && self
                .signatures
                .iter()
                .any(|s| s.result == DkimVerdict::TempError)
    }
}

pub struct DkimVerifier {
    authenticator: MessageAuthenticator,
}

impl DkimVerifier {
    pub fn new(authenticator: MessageAuthenticator) -> Self {
        Self { authenticator }
    }

    pub fn from_system() -> Result<Self> {
        let authenticator = MessageAuthenticator::new_system_conf().map_err(|err| {
            Error::internal(format!("could not initialize the DKIM resolver: {err}"))
        })?;
        Ok(Self::new(authenticator))
    }

    pub async fn verify(&self, raw_message: &[u8]) -> Result<DkimDecision> {
        let mut message = AuthenticatedMessage::parse(raw_message).ok_or_else(|| {
            Error::internal("the message could not be parsed for DKIM verification")
        })?;
        demote_insecure(&mut message);
        Ok(DkimDecision::from_outputs(&self.outputs(&message).await))
    }

    pub async fn outputs<'x>(&self, message: &'x AuthenticatedMessage<'x>) -> Vec<DkimOutput<'x>> {
        self.authenticator.verify_dkim(message).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::borrow::Borrow;
    use std::collections::HashMap;
    use std::hash::Hash;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use mail_auth::common::crypto::{RsaKey, Sha256};
    use mail_auth::common::headers::HeaderWriter;
    use mail_auth::common::parse::TxtRecordParser;
    use mail_auth::common::verify::DomainKey;
    use mail_auth::dkim::DkimSigner;
    use mail_auth::hickory_resolver::config::{ResolverConfig, ResolverOpts};
    use mail_auth::{Parameters, ResolverCache, Txt};

    fn verifier() -> DkimVerifier {
        DkimVerifier::new(authenticator())
    }

    fn authenticator() -> MessageAuthenticator {
        MessageAuthenticator::new(ResolverConfig::default(), ResolverOpts::default()).unwrap()
    }

    #[derive(Default)]
    struct MapCache {
        map: Mutex<HashMap<Box<str>, Txt>>,
    }

    impl ResolverCache<Box<str>, Txt> for MapCache {
        fn get<Q>(&self, name: &Q) -> Option<Txt>
        where
            Box<str>: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.map.lock().unwrap().get(name).cloned()
        }

        fn remove<Q>(&self, name: &Q) -> Option<Txt>
        where
            Box<str>: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.map.lock().unwrap().remove(name)
        }

        fn insert(&self, key: Box<str>, value: Txt, _valid_until: Instant) {
            self.map.lock().unwrap().insert(key, value);
        }
    }

    const AUTHORED: &[u8] =
        b"From: alice@legacy.example\r\nTo: bob@host.example\r\nSubject: hello\r\n\r\nbody\r\n";

    const RSA_SHA1_SIGNED: &[u8] = b"DKIM-Signature: v=1; a=rsa-sha1; d=legacy.example; s=weak; c=simple/simple; h=From:To:Subject; bh=7HMbuhnXn9NaJou/ifx09cPS/D8=; b=vJ4ItENNviwViYwgurJ5UnpKtux0pRZOF3oJ39Fk/LY0XulhuKRRwmGDj9g0iahaV2+RZemarThQTm9Yimo3s/brK7l30XzEGz8ePLj+y8QAP+ReaiBAV5v5f+2OHiT9fbQ4YvwvnA4ou2iku+SicgcHYu1xMderZZTuk68rfXo=\r\nFrom: alice@legacy.example\r\nTo: bob@host.example\r\nSubject: hello\r\n\r\nbody\r\n";

    const RSA_1024_PUBLIC_B64: &str = "MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDN4jwEfPwTlYZWAF5KGRJ0Vz3hp02JCZCG6fvdx4IVOR0axGjVshOzthteSTmh5lDfIM8Qj0jZF84jddthD9wB6QRVYwEz/mKtx67rZgkXZL2pW99vcqJ0SMoEBBTAjhmzvvO3aqU5wCPx1/TLwblWRg58n5yBymelDD+Jw7rL8QIDAQAB";

    fn published(selector: &str, public_key_b64: &str) -> MapCache {
        let record = format!("v=DKIM1; k=rsa; p={public_key_b64}");
        let cache = MapCache::default();
        cache.insert(
            format!("{selector}._domainkey.legacy.example.").into(),
            Txt::from(DomainKey::parse(record.as_bytes()).unwrap()),
            Instant::now() + Duration::from_secs(3600),
        );
        cache
    }

    fn pkcs8(der: &[u8]) -> rustls::pki_types::PrivateKeyDer<'_> {
        rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(der))
    }

    #[tokio::test]
    async fn a_legacy_rsa_sha1_signature_never_counts_as_a_dkim_pass() {
        let cache = published("weak", RSA_1024_PUBLIC_B64);
        let message = AuthenticatedMessage::parse(RSA_SHA1_SIGNED).unwrap();
        let raw = authenticator()
            .verify_dkim(Parameters::new(&message).with_txt_cache(&cache))
            .await;
        assert!(
            raw.iter()
                .any(|output| matches!(output.result(), DkimResult::Pass)),
            "setup: the rsa-sha1 signature must be cryptographically valid: {raw:?}"
        );

        let decision = verifier().verify(RSA_SHA1_SIGNED).await.unwrap();
        assert!(
            !decision.pass(),
            "a legacy signature must never authenticate: {decision:?}"
        );
        assert_eq!(
            decision.signatures[0].result,
            DkimVerdict::Neutral,
            "{decision:?}"
        );
    }

    #[tokio::test]
    async fn a_modern_rsa_sha256_signature_survives_the_insecure_filter() {
        let key = irixmail_dns::dkim_keys::generate_rsa("mail").unwrap();
        let signing = RsaKey::<Sha256>::from_key_der(pkcs8(&key.private_pkcs8_der)).unwrap();
        let signature = DkimSigner::from_key(signing)
            .domain("legacy.example")
            .selector("mail")
            .headers(["From", "To", "Subject"])
            .sign(AUTHORED)
            .unwrap();
        let mut signed = Vec::new();
        signature.write_header(&mut signed);
        signed.extend_from_slice(AUTHORED);

        let cache = published("mail", &key.public_key_b64);
        let mut message = AuthenticatedMessage::parse(&signed).unwrap();
        demote_insecure(&mut message);
        let raw = authenticator()
            .verify_dkim(Parameters::new(&message).with_txt_cache(&cache))
            .await;
        assert!(
            raw.iter()
                .any(|output| matches!(output.result(), DkimResult::Pass)),
            "a healthy rsa-sha256 signature must still verify: {raw:?}"
        );
    }

    #[tokio::test]
    async fn a_sub_1024_bit_rsa_signature_is_demoted_without_a_dns_lookup() {
        let key = irixmail_dns::dkim_keys::generate_rsa("s").unwrap();
        let signing = RsaKey::<Sha256>::from_key_der(pkcs8(&key.private_pkcs8_der)).unwrap();
        let signature = DkimSigner::from_key(signing)
            .domain("legacy.example")
            .selector("s")
            .headers(["From", "To", "Subject"])
            .sign(AUTHORED)
            .unwrap();
        let mut header = Vec::new();
        signature.write_header(&mut header);
        let text = String::from_utf8(header).unwrap();
        let pos = text.rfind("b=").unwrap();
        let short_b = "A".repeat(84) + "AA==";
        let mut signed = format!("{}{}\r\n", &text[..pos + 2], short_b).into_bytes();
        signed.extend_from_slice(AUTHORED);

        let decision = verifier().verify(&signed).await.unwrap();
        assert!(!decision.pass());
        assert_eq!(
            decision.signatures[0].result,
            DkimVerdict::Neutral,
            "a sub-1024-bit RSA signature must be refused before verification: {decision:?}"
        );
    }

    #[test]
    fn each_dkim_result_maps_to_its_verdict() {
        use mail_auth::Error as AuthError;
        let cases = [
            (DkimResult::Pass, DkimVerdict::Pass),
            (
                DkimResult::Neutral(AuthError::FailedBodyHashMatch),
                DkimVerdict::Neutral,
            ),
            (
                DkimResult::Fail(AuthError::FailedBodyHashMatch),
                DkimVerdict::Fail,
            ),
            (
                DkimResult::PermError(AuthError::UnsupportedAlgorithm),
                DkimVerdict::PermError,
            ),
            (
                DkimResult::TempError(AuthError::FailedBodyHashMatch),
                DkimVerdict::TempError,
            ),
            (DkimResult::None, DkimVerdict::None),
        ];
        for (result, expected) in cases {
            assert_eq!(DkimVerdict::from(&result), expected);
        }
    }

    #[test]
    fn only_a_pass_signature_authenticates_the_message() {
        let decision = DkimDecision {
            signatures: vec![
                DkimSignatureResult {
                    domain: "fail.example".to_string(),
                    result: DkimVerdict::Fail,
                },
                DkimSignatureResult {
                    domain: "good.example".to_string(),
                    result: DkimVerdict::Pass,
                },
            ],
        };
        assert!(decision.pass());
        assert!(!decision.temp_error());
    }

    #[test]
    fn a_transient_failure_is_reported_without_a_pass() {
        let decision = DkimDecision {
            signatures: vec![DkimSignatureResult {
                domain: "sender.example".to_string(),
                result: DkimVerdict::TempError,
            }],
        };
        assert!(!decision.pass());
        assert!(decision.temp_error());
    }

    #[test]
    fn an_empty_decision_neither_passes_nor_defers() {
        let decision = DkimDecision::default();
        assert!(!decision.pass());
        assert!(!decision.temp_error());
    }

    #[tokio::test]
    async fn an_unsigned_message_yields_no_signatures() {
        let raw = b"From: sender@sender.example\r\nTo: rcpt@host.example\r\nSubject: test\r\n\r\nbody\r\n";
        let decision = verifier().verify(raw).await.unwrap();
        assert!(decision.signatures.is_empty());
        assert!(!decision.pass());
    }

    #[tokio::test]
    async fn an_unparseable_message_is_an_error() {
        let raw = b"";
        assert!(verifier().verify(raw).await.is_err());
    }
}
