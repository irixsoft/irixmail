use std::net::{Ipv4Addr, Ipv6Addr};

use crate::dkim_keys::DkimKey;
use crate::rec_a::{a_record, aaaa_record};
use crate::rec_autoconfig::autoconfig_records;
use crate::rec_dkim::dkim_record;
use crate::rec_dmarc::dmarc_record;
use crate::rec_mtasts::{mtasts_host_record, mtasts_record};
use crate::rec_mx::{mx_record, DnsRecord};
use crate::rec_spf::spf_record;
use crate::rec_tlsrpt::tlsrpt_record;

pub struct DomainRecordsInput<'a> {
    pub domain: &'a str,
    pub mail_host: &'a str,
    pub ipv4: Option<Ipv4Addr>,
    pub ipv6: Option<Ipv6Addr>,
    pub dkim_keys: &'a [DkimKey],
    pub mtasts_id: &'a str,
    pub mx_preference: u16,
}

fn host_in_zone(host: &str, domain: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let domain = domain.trim_end_matches('.').to_ascii_lowercase();
    host == domain || host.ends_with(&format!(".{domain}"))
}

pub fn domain_records(input: &DomainRecordsInput) -> Vec<DnsRecord> {
    let mail_host_in_zone = host_in_zone(input.mail_host, input.domain);
    let mut records = vec![mx_record(
        input.domain,
        input.mail_host,
        input.mx_preference,
    )];
    if let Some(v4) = input.ipv4 {
        let mut record = a_record(input.mail_host, v4);
        record.in_zone = mail_host_in_zone;
        records.push(record);
    }
    if let Some(v6) = input.ipv6 {
        let mut record = aaaa_record(input.mail_host, v6);
        record.in_zone = mail_host_in_zone;
        records.push(record);
    }
    records.push(spf_record(input.domain, input.mail_host));
    for key in input.dkim_keys {
        records.push(dkim_record(input.domain, key));
    }
    records.push(dmarc_record(input.domain));
    records.push(mtasts_record(input.domain, input.mtasts_id));
    records.push(mtasts_host_record(input.domain, input.mail_host));
    records.push(tlsrpt_record(input.domain));
    records.extend(autoconfig_records(input.domain, input.mail_host));
    records
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dkim_keys::generate_ed25519;
    use crate::rec_mx::DnsRecordKind;

    #[test]
    fn the_full_set_covers_every_required_record() {
        let keys = vec![generate_ed25519("mail").unwrap()];
        let input = DomainRecordsInput {
            domain: "example.com",
            mail_host: "mail.example.com",
            ipv4: Some(Ipv4Addr::new(192, 0, 2, 1)),
            ipv6: Some(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
            dkim_keys: &keys,
            mtasts_id: "20260101000000",
            mx_preference: 10,
        };
        let records = domain_records(&input);

        assert!(
            records
                .iter()
                .any(|r| r.name == "mta-sts.example.com" && r.record_type == "CNAME"),
            "the bundle must include the mta-sts policy host record"
        );

        let kinds: Vec<_> = records.iter().map(|r| r.kind).collect();
        for required in [
            DnsRecordKind::Mx,
            DnsRecordKind::A,
            DnsRecordKind::Aaaa,
            DnsRecordKind::Spf,
            DnsRecordKind::Dkim,
            DnsRecordKind::Dmarc,
            DnsRecordKind::MtaSts,
            DnsRecordKind::TlsRpt,
            DnsRecordKind::Autoconfig,
        ] {
            assert!(kinds.contains(&required), "missing {required:?}");
        }
    }

    #[test]
    fn address_records_for_a_mail_host_in_another_zone_are_marked_out_of_zone() {
        let input = DomainRecordsInput {
            domain: "msaeedsakib.com",
            mail_host: "mail.irixsoft.co.uk",
            ipv4: Some(Ipv4Addr::new(192, 0, 2, 1)),
            ipv6: Some(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
            dkim_keys: &[],
            mtasts_id: "1",
            mx_preference: 10,
        };
        for record in domain_records(&input) {
            let addressed_the_mail_host =
                matches!(record.kind, DnsRecordKind::A | DnsRecordKind::Aaaa);
            assert_eq!(record.in_zone, !addressed_the_mail_host, "{record:?}");
        }
    }

    #[test]
    fn a_mail_host_inside_the_domain_leaves_every_record_in_zone() {
        let input = DomainRecordsInput {
            domain: "example.com",
            mail_host: "MAIL.Example.com.",
            ipv4: Some(Ipv4Addr::new(192, 0, 2, 1)),
            ipv6: Some(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
            dkim_keys: &[],
            mtasts_id: "1",
            mx_preference: 10,
        };
        assert!(domain_records(&input).iter().all(|r| r.in_zone));
    }

    #[test]
    fn a_mail_host_that_only_shares_a_suffix_with_the_domain_is_out_of_zone() {
        let input = DomainRecordsInput {
            domain: "example.com",
            mail_host: "mail.notexample.com",
            ipv4: Some(Ipv4Addr::new(192, 0, 2, 1)),
            ipv6: None,
            dkim_keys: &[],
            mtasts_id: "1",
            mx_preference: 10,
        };
        let records = domain_records(&input);
        let a = records.iter().find(|r| r.kind == DnsRecordKind::A).unwrap();
        assert!(!a.in_zone);
    }

    #[test]
    fn address_records_follow_the_available_families() {
        let input = DomainRecordsInput {
            domain: "example.com",
            mail_host: "mail.example.com",
            ipv4: Some(Ipv4Addr::new(192, 0, 2, 1)),
            ipv6: None,
            dkim_keys: &[],
            mtasts_id: "1",
            mx_preference: 10,
        };
        let records = domain_records(&input);
        assert!(records.iter().any(|r| r.kind == DnsRecordKind::A));
        assert!(!records.iter().any(|r| r.kind == DnsRecordKind::Aaaa));
        assert!(!records.iter().any(|r| r.kind == DnsRecordKind::Dkim));
    }
}
