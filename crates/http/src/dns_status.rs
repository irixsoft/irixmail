use std::net::{Ipv4Addr, Ipv6Addr};

use irixmail_core::Result;
use irixmail_directory::{Directory, DnsRecordKind, DnsStatus, Domain};
use irixmail_dns::{
    domain_records, verify_all, CheckStatus, DomainRecordsInput, RecordCheck, Resolver,
};
use irixmail_tls::acme_http01::unix_now;

pub struct RecheckInput<'a> {
    pub directory: &'a Directory,
    pub resolver: &'a Resolver,
    pub hostname: &'a str,
    pub ipv4: Option<Ipv4Addr>,
    pub ipv6: Option<Ipv6Addr>,
}

pub fn status_from_checks(checks: &[RecordCheck], now: u64) -> DnsStatus {
    let mut missing: Vec<DnsRecordKind> = Vec::new();
    for check in checks.iter().filter(|c| c.status != CheckStatus::Pass) {
        if let Some(kind) = missing_kind(check.record.kind) {
            if !missing.contains(&kind) {
                missing.push(kind);
            }
        }
    }
    if missing.is_empty() {
        DnsStatus::Verified { checked_at: now }
    } else {
        DnsStatus::Failing {
            checked_at: now,
            missing,
        }
    }
}

// autoconfig SRV records are advisory and never hold a domain back from verified
fn missing_kind(kind: irixmail_dns::DnsRecordKind) -> Option<DnsRecordKind> {
    match kind {
        irixmail_dns::DnsRecordKind::Mx => Some(DnsRecordKind::Mx),
        irixmail_dns::DnsRecordKind::A => Some(DnsRecordKind::A),
        irixmail_dns::DnsRecordKind::Aaaa => Some(DnsRecordKind::Aaaa),
        irixmail_dns::DnsRecordKind::Spf => Some(DnsRecordKind::Spf),
        irixmail_dns::DnsRecordKind::Dkim => Some(DnsRecordKind::Dkim),
        irixmail_dns::DnsRecordKind::Dmarc => Some(DnsRecordKind::Dmarc),
        irixmail_dns::DnsRecordKind::MtaSts => Some(DnsRecordKind::MtaSts),
        irixmail_dns::DnsRecordKind::TlsRpt => Some(DnsRecordKind::TlsRpt),
        irixmail_dns::DnsRecordKind::Autoconfig => None,
    }
}

pub fn persist_status(directory: &Directory, domain: &Domain, status: DnsStatus) -> Result<()> {
    if domain.dns_status == status {
        return Ok(());
    }
    let mut updated = domain.clone();
    updated.dns_status = status;
    directory.domains().update(updated)
}

pub async fn recheck_domain(input: &RecheckInput<'_>, domain: &Domain) -> Result<DnsStatus> {
    let Some(key) = input.directory.dkim().get(domain.id)? else {
        return Ok(domain.dns_status.clone());
    };
    let dkim_keys = [key];
    let mtasts_id = domain.created_at.to_string();
    let records = domain_records(&DomainRecordsInput {
        domain: &domain.name,
        mail_host: input.hostname,
        ipv4: input.ipv4,
        ipv6: input.ipv6,
        dkim_keys: &dkim_keys,
        mtasts_id: &mtasts_id,
        mx_preference: 10,
    });
    let checks = verify_all(input.resolver, &records).await?;
    Ok(status_from_checks(&checks, unix_now()))
}

pub async fn recheck_all(input: &RecheckInput<'_>) -> usize {
    let domains = match input.directory.domains().list() {
        Ok(domains) => domains,
        Err(err) => {
            tracing::warn!(error = %err, "could not list domains for the dns recheck");
            return 0;
        }
    };
    let mut updated = 0;
    for domain in domains {
        let status = match recheck_domain(input, &domain).await {
            Ok(status) => status,
            Err(err) => {
                tracing::warn!(domain = %domain.name, error = %err, "the dns recheck failed");
                continue;
            }
        };
        if status == domain.dns_status {
            continue;
        }
        match persist_status(input.directory, &domain, status) {
            Ok(()) => updated += 1,
            Err(err) => {
                tracing::warn!(domain = %domain.name, error = %err, "could not store the dns status")
            }
        }
    }
    updated
}

#[cfg(test)]
mod tests {
    use super::*;
    use irixmail_dns::{DnsRecord, DnsRecordKind as WireKind};

    use crate::tests_support::{state, TempDir};

    fn check(kind: WireKind, status: CheckStatus) -> RecordCheck {
        RecordCheck {
            record: DnsRecord::new(kind, "example.com", "TXT", "value"),
            status,
            observed: Vec::new(),
        }
    }

    #[test]
    fn all_passing_checks_mark_the_domain_verified() {
        let checks = vec![
            check(WireKind::Mx, CheckStatus::Pass),
            check(WireKind::Spf, CheckStatus::Pass),
            check(WireKind::Dkim, CheckStatus::Pass),
        ];
        assert_eq!(
            status_from_checks(&checks, 42),
            DnsStatus::Verified { checked_at: 42 }
        );
    }

    #[test]
    fn a_missing_mx_record_marks_the_domain_failing() {
        let checks = vec![
            check(WireKind::Mx, CheckStatus::Missing),
            check(WireKind::Spf, CheckStatus::Pass),
        ];
        assert_eq!(
            status_from_checks(&checks, 42),
            DnsStatus::Failing {
                checked_at: 42,
                missing: vec![DnsRecordKind::Mx],
            }
        );
    }

    #[test]
    fn a_mismatched_record_is_listed_as_missing() {
        let checks = vec![check(WireKind::Dmarc, CheckStatus::Mismatch)];
        assert_eq!(
            status_from_checks(&checks, 7),
            DnsStatus::Failing {
                checked_at: 7,
                missing: vec![DnsRecordKind::Dmarc],
            }
        );
    }

    #[test]
    fn autoconfig_failures_do_not_block_verification() {
        let checks = vec![
            check(WireKind::Mx, CheckStatus::Pass),
            check(WireKind::Autoconfig, CheckStatus::Missing),
            check(WireKind::Autoconfig, CheckStatus::Mismatch),
        ];
        assert_eq!(
            status_from_checks(&checks, 5),
            DnsStatus::Verified { checked_at: 5 }
        );
    }

    #[test]
    fn repeated_kinds_are_listed_once() {
        let checks = vec![
            check(WireKind::Dkim, CheckStatus::Missing),
            check(WireKind::Dkim, CheckStatus::Mismatch),
        ];
        assert_eq!(
            status_from_checks(&checks, 9),
            DnsStatus::Failing {
                checked_at: 9,
                missing: vec![DnsRecordKind::Dkim],
            }
        );
    }

    #[tokio::test]
    async fn rechecking_writes_a_status_for_every_domain() {
        let dir = TempDir::new();
        let shared = state(&dir);
        for name in ["one.example.com", "two.example.com"] {
            let domain = shared.directory.domains().create(name, Vec::new()).unwrap();
            shared
                .directory
                .dkim()
                .get_or_create(domain.id, "default")
                .unwrap();
        }
        let input = RecheckInput {
            directory: &shared.directory,
            resolver: &shared.resolver,
            hostname: "mail.example.com",
            ipv4: None,
            ipv6: None,
        };
        assert_eq!(recheck_all(&input).await, 2);
        for domain in shared.directory.domains().list().unwrap() {
            match domain.dns_status {
                DnsStatus::Failing {
                    checked_at,
                    ref missing,
                } => {
                    assert!(checked_at > 0);
                    assert!(!missing.is_empty());
                }
                other => panic!("expected a failing status, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn a_domain_without_a_dkim_key_is_skipped() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let input = RecheckInput {
            directory: &shared.directory,
            resolver: &shared.resolver,
            hostname: "mail.example.com",
            ipv4: None,
            ipv6: None,
        };
        assert_eq!(recheck_all(&input).await, 0);
        let stored = shared.directory.domains().get(domain.id).unwrap();
        assert_eq!(stored.dns_status, DnsStatus::Unverified);
    }
}
