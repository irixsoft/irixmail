use mail_auth::dmarc::verify::DmarcParameters;
use mail_auth::dmarc::Policy;
use mail_auth::{AuthenticatedMessage, DkimOutput, DmarcResult, MessageAuthenticator, SpfOutput};

use irixmail_core::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DmarcAction {
    Pass,
    Quarantine,
    Reject,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DmarcDecision {
    pub domain: String,
    pub result: DmarcResult,
    pub policy: Policy,
    pub action: DmarcAction,
    pub temp_error: bool,
}

impl DmarcDecision {
    pub fn pass(&self) -> bool {
        matches!(self.result, DmarcResult::Pass)
    }
}

pub struct DmarcVerifier {
    authenticator: MessageAuthenticator,
}

impl DmarcVerifier {
    pub fn new(authenticator: MessageAuthenticator) -> Self {
        Self { authenticator }
    }

    pub fn from_system() -> Result<Self> {
        let authenticator = MessageAuthenticator::new_system_conf().map_err(|err| {
            Error::internal(format!("could not initialize the DMARC resolver: {err}"))
        })?;
        Ok(Self::new(authenticator))
    }

    pub async fn verify(
        &self,
        message: &AuthenticatedMessage<'_>,
        dkim_output: &[DkimOutput<'_>],
        mail_from_domain: &str,
        spf_output: &SpfOutput,
    ) -> DmarcDecision {
        let output = self
            .authenticator
            .verify_dmarc(DmarcParameters::new(
                message,
                dkim_output,
                mail_from_domain,
                spf_output,
            ))
            .await;

        let aligned = matches!(output.spf_result(), DmarcResult::Pass)
            || matches!(output.dkim_result(), DmarcResult::Pass);
        let temp_error = matches!(output.spf_result(), DmarcResult::TempError(_))
            || matches!(output.dkim_result(), DmarcResult::TempError(_));

        let result = if aligned {
            DmarcResult::Pass
        } else if output.spf_result() != &DmarcResult::None {
            output.spf_result().clone()
        } else if output.dkim_result() != &DmarcResult::None {
            output.dkim_result().clone()
        } else {
            DmarcResult::None
        };

        let policy = output.policy();
        let action = action_for(policy, aligned);

        DmarcDecision {
            domain: output.domain().to_string(),
            result,
            policy,
            action,
            temp_error,
        }
    }
}

fn action_for(policy: Policy, aligned: bool) -> DmarcAction {
    if aligned {
        return DmarcAction::Pass;
    }
    match policy {
        Policy::Reject => DmarcAction::Reject,
        Policy::Quarantine => DmarcAction::Quarantine,
        Policy::None | Policy::Unspecified => DmarcAction::Pass,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_auth::hickory_resolver::config::{ResolverConfig, ResolverOpts};

    fn verifier() -> DmarcVerifier {
        let authenticator =
            MessageAuthenticator::new(ResolverConfig::default(), ResolverOpts::default()).unwrap();
        DmarcVerifier::new(authenticator)
    }

    #[test]
    fn an_aligned_message_passes_under_every_policy() {
        for policy in [
            Policy::None,
            Policy::Quarantine,
            Policy::Reject,
            Policy::Unspecified,
        ] {
            assert_eq!(action_for(policy, true), DmarcAction::Pass);
        }
    }

    #[test]
    fn an_unaligned_message_follows_the_published_policy() {
        assert_eq!(action_for(Policy::Reject, false), DmarcAction::Reject);
        assert_eq!(
            action_for(Policy::Quarantine, false),
            DmarcAction::Quarantine
        );
        assert_eq!(action_for(Policy::None, false), DmarcAction::Pass);
        assert_eq!(action_for(Policy::Unspecified, false), DmarcAction::Pass);
    }

    #[test]
    fn a_pass_decision_reports_alignment() {
        let decision = DmarcDecision {
            domain: "sender.example".to_string(),
            result: DmarcResult::Pass,
            policy: Policy::Reject,
            action: DmarcAction::Pass,
            temp_error: false,
        };
        assert!(decision.pass());
    }

    #[test]
    fn a_non_pass_result_does_not_report_alignment() {
        let decision = DmarcDecision {
            domain: "sender.example".to_string(),
            result: DmarcResult::None,
            policy: Policy::None,
            action: DmarcAction::Pass,
            temp_error: false,
        };
        assert!(!decision.pass());
    }

    #[tokio::test]
    async fn a_message_without_a_dmarc_record_is_accepted_with_a_pass_action() {
        let raw = b"From: sender@dmarc-absent.invalid\r\nTo: rcpt@host.example\r\nSubject: t\r\n\r\nbody\r\n";
        let message = AuthenticatedMessage::parse(raw).unwrap();
        let spf = SpfOutput::default();
        let decision = verifier()
            .verify(&message, &[], "dmarc-absent.invalid", &spf)
            .await;
        assert_eq!(decision.action, DmarcAction::Pass);
        assert!(!decision.pass());
    }
}
