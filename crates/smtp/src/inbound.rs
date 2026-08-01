use std::net::IpAddr;

use mail_auth::{AuthenticatedMessage, DmarcResult, SpfResult};

use irixmail_mail::{
    ArcVerdict, AuthResults, DkimVerdict as ResultDkim, DmarcVerdict as ResultDmarc, MethodResult,
    SpfIdentity, SpfVerdict as ResultSpf,
};

use crate::arc::ArcDecision;
use crate::dkim_verify::{DkimDecision, DkimVerdict};
use crate::dmarc::DmarcDecision;
use crate::dnsbl::DnsblDecision;
use crate::loop_detect::{self, LoopConfig, LoopDecision};
use crate::session_services::InboundServices;
use crate::spam_decision::{self, AuthSummary, ReputationSummary, SpamDecision};
use crate::spf::SpfDecision;

pub struct GauntletOutcome {
    pub verdict: SpamDecision,
    pub auth_results: String,
}

#[allow(clippy::too_many_arguments)]
pub async fn run_gauntlet(
    services: &InboundServices,
    sid: u64,
    peer_ip: IpAddr,
    helo_domain: &str,
    mail_from: &str,
    raw_message: &[u8],
    dnsbl: &DnsblDecision,
) -> GauntletOutcome {
    let spf_output = services
        .spf()
        .mail_from_output(peer_ip, helo_domain, mail_from)
        .await;
    let spf = SpfDecision::mail_from(&spf_output);
    tracing::info!(
        target: "irixmail::smtp::inbound",
        sid,
        result = ?spf.result,
        domain = %spf.domain,
        "spf verdict"
    );

    let parsed = AuthenticatedMessage::parse(raw_message).map(|mut message| {
        crate::dkim_verify::demote_insecure(&mut message);
        message
    });
    let (dkim, arc, dmarc) = match &parsed {
        Some(message) => {
            let dkim_output = services.dkim().outputs(message).await;
            let dkim = DkimDecision::from_outputs(&dkim_output);
            let arc = services.arc().verify_parsed(message).await;
            let mail_from_domain = domain_of(mail_from).unwrap_or(helo_domain);
            let dmarc = services
                .dmarc()
                .verify(message, &dkim_output, mail_from_domain, &spf_output)
                .await;
            (dkim, arc, dmarc)
        }
        None => (
            DkimDecision::default(),
            ArcDecision {
                result: DkimVerdict::None,
                instances: 0,
            },
            unevaluated_dmarc(),
        ),
    };
    tracing::info!(
        target: "irixmail::smtp::inbound",
        sid,
        signatures = dkim.signatures.len(),
        result = ?dkim.signatures.first().map(|signature| signature.result),
        "dkim verdict"
    );
    tracing::info!(
        target: "irixmail::smtp::inbound",
        sid,
        result = ?dmarc.result,
        action = ?dmarc.action,
        domain = %dmarc.domain,
        "dmarc verdict"
    );

    let auth_results = synthesize_header(services.spf().host_domain(), &spf, &dkim, &dmarc, &arc);

    if let LoopDecision::Reject(reply) = loop_detect::check(raw_message, LoopConfig::default()) {
        return GauntletOutcome {
            verdict: SpamDecision::Defer(reply),
            auth_results,
        };
    }

    let auth = AuthSummary {
        spf: &spf,
        dkim: &dkim,
        dmarc: &dmarc,
        arc: &arc,
    };
    let reputation = ReputationSummary { dnsbl };
    let verdict = spam_decision::decide(false, &auth, &reputation);

    GauntletOutcome {
        verdict,
        auth_results,
    }
}

fn unevaluated_dmarc() -> DmarcDecision {
    DmarcDecision {
        domain: String::new(),
        result: DmarcResult::None,
        policy: mail_auth::dmarc::Policy::Unspecified,
        action: crate::dmarc::DmarcAction::Pass,
        temp_error: false,
    }
}

fn synthesize_header(
    authserv_id: &str,
    spf: &SpfDecision,
    dkim: &DkimDecision,
    dmarc: &DmarcDecision,
    arc: &ArcDecision,
) -> String {
    let mut results = AuthResults::new();
    for signature in &dkim.signatures {
        results = results.with_dkim(ResultDkim {
            result: dkim_method(signature.result),
            domain: signature.domain.clone(),
            selector: None,
        });
    }
    results = results.with_spf(ResultSpf {
        result: spf_method(spf.result),
        identity: SpfIdentity::MailFrom,
        value: spf.domain.clone(),
    });
    results = results.with_dmarc(ResultDmarc {
        result: dmarc_method(&dmarc.result),
        from_domain: dmarc.domain.clone(),
    });
    if arc.present() {
        results = results.with_arc(ArcVerdict {
            result: dkim_method(arc.result),
        });
    }
    results.to_header_value(authserv_id)
}

fn spf_method(result: SpfResult) -> MethodResult {
    match result {
        SpfResult::Pass => MethodResult::Pass,
        SpfResult::Fail => MethodResult::Fail,
        SpfResult::SoftFail => MethodResult::SoftFail,
        SpfResult::Neutral => MethodResult::Neutral,
        SpfResult::None => MethodResult::None,
        SpfResult::TempError => MethodResult::TempError,
        SpfResult::PermError => MethodResult::PermError,
    }
}

fn dkim_method(verdict: DkimVerdict) -> MethodResult {
    match verdict {
        DkimVerdict::Pass => MethodResult::Pass,
        DkimVerdict::Neutral => MethodResult::Neutral,
        DkimVerdict::Fail => MethodResult::Fail,
        DkimVerdict::PermError => MethodResult::PermError,
        DkimVerdict::TempError => MethodResult::TempError,
        DkimVerdict::None => MethodResult::None,
    }
}

fn dmarc_method(result: &DmarcResult) -> MethodResult {
    match result {
        DmarcResult::Pass => MethodResult::Pass,
        DmarcResult::Fail(_) => MethodResult::Fail,
        DmarcResult::TempError(_) => MethodResult::TempError,
        DmarcResult::PermError(_) => MethodResult::PermError,
        DmarcResult::None => MethodResult::None,
    }
}

pub fn build_received(
    helo: &str,
    remote_ip: IpAddr,
    host: &str,
    tls: bool,
    id: u64,
    now: u64,
) -> Vec<u8> {
    let date = crate::dsn::Rfc822Date::from_timestamp(now as i64).to_string();
    let with = if tls { "ESMTPS" } else { "ESMTP" };
    let mut out = Vec::with_capacity(helo.len() + host.len() + date.len() + 96);
    out.extend_from_slice(b"Received: from ");
    out.extend_from_slice(helo.as_bytes());
    out.extend_from_slice(b" [");
    out.extend_from_slice(remote_ip.to_string().as_bytes());
    out.extend_from_slice(b"]\r\n\tby ");
    out.extend_from_slice(host.as_bytes());
    out.extend_from_slice(b" (IRIXMAIL) with ");
    out.extend_from_slice(with.as_bytes());
    out.extend_from_slice(b" id ");
    out.extend_from_slice(format!("{id:X}").as_bytes());
    out.extend_from_slice(b";\r\n\t");
    out.extend_from_slice(date.as_bytes());
    out.extend_from_slice(b"\r\n");
    out
}

pub fn prepend_header(field: &str, value: &str, raw_message: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(field.len() + value.len() + raw_message.len() + 4);
    out.extend_from_slice(field.as_bytes());
    out.extend_from_slice(b": ");
    out.extend_from_slice(value.as_bytes());
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(raw_message);
    out
}

fn domain_of(address: &str) -> Option<&str> {
    match address.rsplit_once('@') {
        Some((_, domain)) if !domain.is_empty() => Some(domain),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dkim_verify::DkimSignatureResult;
    use mail_auth::dmarc::Policy;

    fn spf(result: SpfResult) -> SpfDecision {
        SpfDecision {
            stage: crate::spf::SpfStage::MailFrom,
            domain: "sender.example".to_string(),
            result,
        }
    }

    fn dkim(verdict: DkimVerdict, domain: &str) -> DkimDecision {
        DkimDecision {
            signatures: vec![DkimSignatureResult {
                domain: domain.to_string(),
                result: verdict,
            }],
        }
    }

    fn dmarc(result: DmarcResult) -> DmarcDecision {
        DmarcDecision {
            domain: "sender.example".to_string(),
            result,
            policy: Policy::None,
            action: crate::dmarc::DmarcAction::Pass,
            temp_error: false,
        }
    }

    #[test]
    fn the_header_names_each_method_with_its_translated_result() {
        let header = synthesize_header(
            "mx.irixmail.test",
            &spf(SpfResult::Pass),
            &dkim(DkimVerdict::Pass, "sender.example"),
            &dmarc(DmarcResult::Pass),
            &ArcDecision {
                result: DkimVerdict::None,
                instances: 0,
            },
        );
        assert!(header.starts_with("mx.irixmail.test"));
        assert!(header.contains("dkim=pass header.d=sender.example"));
        assert!(header.contains("spf=pass smtp.mailfrom=sender.example"));
        assert!(header.contains("dmarc=pass header.from=sender.example"));
        assert!(!header.contains("\tarc="));
    }

    #[test]
    fn a_present_arc_chain_contributes_its_clause() {
        let header = synthesize_header(
            "mx.irixmail.test",
            &spf(SpfResult::Fail),
            &dkim(DkimVerdict::Fail, "sender.example"),
            &dmarc(DmarcResult::Fail(mail_auth::Error::FailedVerification)),
            &ArcDecision {
                result: DkimVerdict::Pass,
                instances: 2,
            },
        );
        assert!(header.contains("spf=fail"));
        assert!(header.contains("dmarc=fail"));
        assert!(header.contains("arc=pass"));
    }

    #[test]
    fn each_spf_result_maps_to_its_method_keyword() {
        assert_eq!(spf_method(SpfResult::Pass), MethodResult::Pass);
        assert_eq!(spf_method(SpfResult::SoftFail), MethodResult::SoftFail);
        assert_eq!(spf_method(SpfResult::PermError), MethodResult::PermError);
    }

    #[test]
    fn each_dkim_verdict_maps_to_its_method_keyword() {
        assert_eq!(dkim_method(DkimVerdict::Pass), MethodResult::Pass);
        assert_eq!(dkim_method(DkimVerdict::TempError), MethodResult::TempError);
        assert_eq!(dkim_method(DkimVerdict::None), MethodResult::None);
    }

    #[test]
    fn a_received_header_records_the_hop_over_tls_as_esmtps() {
        let line = build_received(
            "client.example",
            "198.51.100.7".parse().unwrap(),
            "mx.irix.example",
            true,
            0x2AB,
            1_700_000_000,
        );
        let text = String::from_utf8(line).unwrap();
        assert!(text.starts_with("Received: from client.example [198.51.100.7]\r\n\t"));
        assert!(text.contains("by mx.irix.example (IRIXMAIL) with ESMTPS id 2AB;"));
        assert!(text.ends_with("Tue, 14 Nov 2023 22:13:20 +0000\r\n"));
    }

    #[test]
    fn the_prepended_header_leads_the_message() {
        let raw = b"From: a@b.example\r\nSubject: hi\r\n\r\nbody\r\n";
        let out = prepend_header("Authentication-Results", "mx.irixmail.test; none", raw);
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("Authentication-Results: mx.irixmail.test; none\r\nFrom:"));
        assert!(text.ends_with("body\r\n"));
    }

    #[test]
    fn a_domain_is_taken_from_the_last_at() {
        assert_eq!(domain_of("alice@sender.example"), Some("sender.example"));
        assert_eq!(domain_of("postmaster"), None);
        assert_eq!(domain_of("trailing@"), None);
    }
}
