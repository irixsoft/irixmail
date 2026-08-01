use std::net::IpAddr;

use mail_auth::spf::verify::SpfParameters;
use mail_auth::{MessageAuthenticator, SpfOutput, SpfResult};

use irixmail_core::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpfConfig {
    pub host_domain: String,
}

impl SpfConfig {
    pub fn new(host_domain: impl Into<String>) -> Self {
        Self {
            host_domain: host_domain.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpfStage {
    Ehlo,
    MailFrom,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpfDecision {
    pub stage: SpfStage,
    pub domain: String,
    pub result: SpfResult,
}

impl SpfDecision {
    pub fn passed(&self) -> bool {
        matches!(self.result, SpfResult::Pass)
    }

    pub fn mail_from(output: &SpfOutput) -> Self {
        SpfDecision {
            stage: SpfStage::MailFrom,
            domain: output.domain().to_string(),
            result: output.result(),
        }
    }
}

pub struct SpfVerifier {
    authenticator: MessageAuthenticator,
    host_domain: String,
}

impl SpfVerifier {
    pub fn new(authenticator: MessageAuthenticator, config: SpfConfig) -> Self {
        Self {
            authenticator,
            host_domain: config.host_domain,
        }
    }

    pub fn from_system(config: SpfConfig) -> Result<Self> {
        let authenticator = MessageAuthenticator::new_system_conf().map_err(|err| {
            Error::internal(format!("could not initialize the SPF resolver: {err}"))
        })?;
        Ok(Self::new(authenticator, config))
    }

    pub fn host_domain(&self) -> &str {
        &self.host_domain
    }

    pub async fn verify_ehlo(&self, ip: IpAddr, helo_domain: &str) -> SpfDecision {
        let output = self
            .authenticator
            .verify_spf(SpfParameters::verify_ehlo(
                ip,
                helo_domain,
                &self.host_domain,
            ))
            .await;
        SpfDecision {
            stage: SpfStage::Ehlo,
            domain: output.domain().to_string(),
            result: output.result(),
        }
    }

    pub async fn verify_mail_from(
        &self,
        ip: IpAddr,
        helo_domain: &str,
        sender: &str,
    ) -> SpfDecision {
        SpfDecision::mail_from(&self.mail_from_output(ip, helo_domain, sender).await)
    }

    pub async fn mail_from_output(&self, ip: IpAddr, helo_domain: &str, sender: &str) -> SpfOutput {
        self.authenticator
            .verify_spf(SpfParameters::verify_mail_from(
                ip,
                helo_domain,
                &self.host_domain,
                sender,
            ))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_auth::hickory_resolver::config::{ResolverConfig, ResolverOpts};

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn verifier() -> SpfVerifier {
        let authenticator =
            MessageAuthenticator::new(ResolverConfig::default(), ResolverOpts::default()).unwrap();
        SpfVerifier::new(authenticator, SpfConfig::new("mail.example"))
    }

    #[test]
    fn the_config_carries_the_host_domain() {
        let config = SpfConfig::new("mail.example");
        assert_eq!(config.host_domain, "mail.example");
    }

    #[test]
    fn a_verifier_reports_the_host_domain_it_was_built_with() {
        let v = verifier();
        assert_eq!(v.host_domain(), "mail.example");
    }

    #[test]
    fn only_a_pass_is_treated_as_authorised() {
        let domain = "sender.example".to_string();
        for (result, expected) in [
            (SpfResult::Pass, true),
            (SpfResult::Fail, false),
            (SpfResult::SoftFail, false),
            (SpfResult::Neutral, false),
            (SpfResult::None, false),
            (SpfResult::TempError, false),
            (SpfResult::PermError, false),
        ] {
            let decision = SpfDecision {
                stage: SpfStage::MailFrom,
                domain: domain.clone(),
                result,
            };
            assert_eq!(decision.passed(), expected);
        }
    }

    #[tokio::test]
    async fn an_ehlo_check_records_the_ehlo_stage() {
        let v = verifier();
        let decision = v.verify_ehlo(ip("192.0.2.10"), "sender.example").await;
        assert_eq!(decision.stage, SpfStage::Ehlo);
    }

    #[tokio::test]
    async fn a_mail_from_check_records_the_sender_domain() {
        let v = verifier();
        let decision = v
            .verify_mail_from(ip("192.0.2.10"), "relay.example", "user@sender.example")
            .await;
        assert_eq!(decision.stage, SpfStage::MailFrom);
        assert_eq!(decision.domain, "sender.example");
    }

    #[tokio::test]
    async fn an_empty_reverse_path_falls_back_to_the_ehlo_domain() {
        let v = verifier();
        let decision = v
            .verify_mail_from(ip("192.0.2.10"), "relay.example", "")
            .await;
        assert_eq!(decision.stage, SpfStage::MailFrom);
        assert_eq!(decision.domain, "relay.example");
    }
}
