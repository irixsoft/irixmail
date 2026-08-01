use crate::arc::ArcDecision;
use crate::dkim_verify::DkimDecision;
use crate::dmarc::{DmarcAction, DmarcDecision};
use crate::dnsbl::DnsblDecision;
use crate::spf::SpfDecision;

const REJECTED: &[u8] = b"550 5.7.1 Message rejected by sender policy\r\n";
const DEFERRED: &[u8] = b"451 4.7.1 Message temporarily deferred, please retry\r\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    Inbox,
    Spam,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpamDecision {
    Accept(Disposition),
    Defer(&'static [u8]),
    Reject(&'static [u8]),
}

impl SpamDecision {
    pub fn is_accepted(&self) -> bool {
        matches!(self, SpamDecision::Accept(_))
    }

    pub fn disposition(&self) -> Option<Disposition> {
        match self {
            SpamDecision::Accept(disposition) => Some(*disposition),
            _ => None,
        }
    }
}

pub struct AuthSummary<'a> {
    pub spf: &'a SpfDecision,
    pub dkim: &'a DkimDecision,
    pub dmarc: &'a DmarcDecision,
    pub arc: &'a ArcDecision,
}

pub struct ReputationSummary<'a> {
    pub dnsbl: &'a DnsblDecision,
}

pub fn decide(
    authenticated: bool,
    auth: &AuthSummary<'_>,
    reputation: &ReputationSummary<'_>,
) -> SpamDecision {
    if authenticated {
        return SpamDecision::Accept(Disposition::Inbox);
    }

    if let DnsblDecision::Reject { reply, .. } = reputation.dnsbl {
        return SpamDecision::Reject(reply);
    }

    match auth.dmarc.action {
        DmarcAction::Reject => {
            if auth.dmarc.temp_error {
                return SpamDecision::Defer(DEFERRED);
            }
            SpamDecision::Reject(REJECTED)
        }
        DmarcAction::Quarantine => SpamDecision::Accept(Disposition::Spam),
        DmarcAction::Pass => SpamDecision::Accept(Disposition::Inbox),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dkim_verify::{DkimSignatureResult, DkimVerdict};
    use crate::dnsbl::DEFAULT_ZONE;
    use irixmail_dns::dnsbl::DnsblListing;
    use mail_auth::dmarc::Policy;
    use mail_auth::{DmarcResult, SpfResult};

    fn spf(result: SpfResult) -> SpfDecision {
        SpfDecision {
            stage: crate::spf::SpfStage::MailFrom,
            domain: "sender.example".to_string(),
            result,
        }
    }

    fn dkim(verdict: DkimVerdict) -> DkimDecision {
        DkimDecision {
            signatures: vec![DkimSignatureResult {
                domain: "sender.example".to_string(),
                result: verdict,
            }],
        }
    }

    fn dmarc(action: DmarcAction, temp_error: bool) -> DmarcDecision {
        DmarcDecision {
            domain: "sender.example".to_string(),
            result: if action == DmarcAction::Pass {
                DmarcResult::Pass
            } else {
                DmarcResult::Fail(mail_auth::Error::FailedVerification)
            },
            policy: match action {
                DmarcAction::Reject => Policy::Reject,
                DmarcAction::Quarantine => Policy::Quarantine,
                DmarcAction::Pass => Policy::None,
            },
            action,
            temp_error,
        }
    }

    fn arc(verdict: DkimVerdict, instances: usize) -> ArcDecision {
        ArcDecision {
            result: verdict,
            instances,
        }
    }

    fn listed() -> DnsblDecision {
        DnsblDecision::Reject {
            zone: DEFAULT_ZONE.to_string(),
            listing: DnsblListing {
                codes: vec![std::net::Ipv4Addr::new(127, 0, 0, 2)],
                reason: Some("listed".to_string()),
            },
            reply: b"554 5.7.1 Connection refused, your address is on a public blocklist\r\n",
        }
    }

    fn judge(
        authenticated: bool,
        spf_result: SpfResult,
        dkim_verdict: DkimVerdict,
        dmarc_action: DmarcAction,
        dmarc_temp: bool,
        dnsbl: &DnsblDecision,
    ) -> SpamDecision {
        let spf = spf(spf_result);
        let dkim = dkim(dkim_verdict);
        let dmarc = dmarc(dmarc_action, dmarc_temp);
        let arc = arc(DkimVerdict::None, 0);
        let auth = AuthSummary {
            spf: &spf,
            dkim: &dkim,
            dmarc: &dmarc,
            arc: &arc,
        };
        let reputation = ReputationSummary { dnsbl };
        decide(authenticated, &auth, &reputation)
    }

    #[test]
    fn an_aligned_message_with_no_adverse_signal_is_delivered_to_the_inbox() {
        let decision = judge(
            false,
            SpfResult::Pass,
            DkimVerdict::Pass,
            DmarcAction::Pass,
            false,
            &DnsblDecision::Allow,
        );
        assert_eq!(decision, SpamDecision::Accept(Disposition::Inbox));
        assert!(decision.is_accepted());
        assert_eq!(decision.disposition(), Some(Disposition::Inbox));
    }

    #[test]
    fn an_authenticated_session_skips_the_gauntlet_and_reaches_the_inbox() {
        let decision = judge(
            true,
            SpfResult::Fail,
            DkimVerdict::Fail,
            DmarcAction::Reject,
            false,
            &listed(),
        );
        assert_eq!(decision, SpamDecision::Accept(Disposition::Inbox));
    }

    #[test]
    fn a_blocklisted_source_is_refused_permanently() {
        let dnsbl = listed();
        let decision = judge(
            false,
            SpfResult::Pass,
            DkimVerdict::Pass,
            DmarcAction::Pass,
            false,
            &dnsbl,
        );
        match decision {
            SpamDecision::Reject(reply) => assert!(reply.starts_with(b"554")),
            other => panic!("expected a permanent rejection, got {other:?}"),
        }
    }

    #[test]
    fn a_listing_is_decided_before_the_senders_own_quarantine_wish() {
        let dnsbl = listed();
        let decision = judge(
            false,
            SpfResult::Fail,
            DkimVerdict::Fail,
            DmarcAction::Quarantine,
            false,
            &dnsbl,
        );
        assert!(matches!(decision, SpamDecision::Reject(_)));
    }

    #[test]
    fn an_unaligned_message_under_reject_is_refused() {
        let decision = judge(
            false,
            SpfResult::Fail,
            DkimVerdict::Fail,
            DmarcAction::Reject,
            false,
            &DnsblDecision::Allow,
        );
        match decision {
            SpamDecision::Reject(reply) => assert!(reply.starts_with(b"550")),
            other => panic!("expected a permanent rejection, got {other:?}"),
        }
    }

    #[test]
    fn a_reject_policy_resting_on_a_temporary_dns_failure_is_deferred() {
        let decision = judge(
            false,
            SpfResult::TempError,
            DkimVerdict::None,
            DmarcAction::Reject,
            true,
            &DnsblDecision::Allow,
        );
        match decision {
            SpamDecision::Defer(reply) => assert!(reply.starts_with(b"451")),
            other => panic!("expected a transient deferral, got {other:?}"),
        }
        assert!(!decision.is_accepted());
        assert_eq!(decision.disposition(), None);
    }

    #[test]
    fn an_unaligned_message_under_quarantine_is_filed_in_the_spam_folder() {
        let decision = judge(
            false,
            SpfResult::Fail,
            DkimVerdict::Fail,
            DmarcAction::Quarantine,
            false,
            &DnsblDecision::Allow,
        );
        assert_eq!(decision, SpamDecision::Accept(Disposition::Spam));
        assert_eq!(decision.disposition(), Some(Disposition::Spam));
    }

    #[test]
    fn the_reject_and_defer_replies_are_the_expected_codes() {
        assert!(REJECTED.starts_with(b"550"));
        assert!(DEFERRED.starts_with(b"451"));
    }
}
