use mail_auth::{AuthenticatedMessage, MessageAuthenticator};

use irixmail_core::{Error, Result};

use crate::dkim_verify::DkimVerdict;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArcDecision {
    pub result: DkimVerdict,
    pub instances: usize,
}

impl ArcDecision {
    pub fn passed(&self) -> bool {
        self.result == DkimVerdict::Pass
    }

    pub fn temp_error(&self) -> bool {
        self.result == DkimVerdict::TempError
    }

    pub fn present(&self) -> bool {
        self.instances > 0
    }
}

pub struct ArcVerifier {
    authenticator: MessageAuthenticator,
}

impl ArcVerifier {
    pub fn new(authenticator: MessageAuthenticator) -> Self {
        Self { authenticator }
    }

    pub fn from_system() -> Result<Self> {
        let authenticator = MessageAuthenticator::new_system_conf().map_err(|err| {
            Error::internal(format!("could not initialize the ARC resolver: {err}"))
        })?;
        Ok(Self::new(authenticator))
    }

    pub async fn verify(&self, raw_message: &[u8]) -> Result<ArcDecision> {
        let message = AuthenticatedMessage::parse(raw_message).ok_or_else(|| {
            Error::internal("the message could not be parsed for ARC verification")
        })?;
        Ok(self.verify_parsed(&message).await)
    }

    pub async fn verify_parsed(&self, message: &AuthenticatedMessage<'_>) -> ArcDecision {
        let output = self.authenticator.verify_arc(message).await;
        ArcDecision {
            result: output.result().into(),
            instances: output.sets().len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_auth::hickory_resolver::config::{ResolverConfig, ResolverOpts};

    fn verifier() -> ArcVerifier {
        let authenticator =
            MessageAuthenticator::new(ResolverConfig::default(), ResolverOpts::default()).unwrap();
        ArcVerifier::new(authenticator)
    }

    #[test]
    fn only_a_pass_chain_authenticates_the_message() {
        let decision = ArcDecision {
            result: DkimVerdict::Pass,
            instances: 2,
        };
        assert!(decision.passed());
        assert!(decision.present());
        assert!(!decision.temp_error());
    }

    #[test]
    fn a_transient_failure_is_reported_without_a_pass() {
        let decision = ArcDecision {
            result: DkimVerdict::TempError,
            instances: 1,
        };
        assert!(!decision.passed());
        assert!(decision.temp_error());
    }

    #[test]
    fn an_empty_chain_is_neither_present_nor_passing() {
        let decision = ArcDecision {
            result: DkimVerdict::None,
            instances: 0,
        };
        assert!(!decision.present());
        assert!(!decision.passed());
    }

    #[tokio::test]
    async fn a_message_without_arc_headers_yields_an_empty_chain() {
        let raw = b"From: sender@sender.example\r\nTo: rcpt@host.example\r\nSubject: test\r\n\r\nbody\r\n";
        let decision = verifier().verify(raw).await.unwrap();
        assert_eq!(decision.instances, 0);
        assert!(!decision.present());
        assert!(!decision.passed());
    }

    #[tokio::test]
    async fn an_unparseable_message_is_an_error() {
        let raw = b"";
        assert!(verifier().verify(raw).await.is_err());
    }
}
