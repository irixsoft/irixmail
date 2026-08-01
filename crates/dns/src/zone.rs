use std::fmt::Write;

use crate::rec_mx::{fqdn, DnsRecord, DEFAULT_TTL};

const MAX_TXT_CHUNK: usize = 255;

pub fn zone_file(origin: &str, records: &[DnsRecord]) -> String {
    let mut out = format!(
        "; irixmail DNS records for {origin}\n$ORIGIN {}\n$TTL {DEFAULT_TTL}\n",
        fqdn(origin)
    );
    for record in records.iter().filter(|record| record.in_zone) {
        let value = if record.record_type == "TXT" {
            txt_value(&record.value)
        } else {
            record.value.clone()
        };
        let _ = writeln!(
            out,
            "{}\t{}\tIN\t{}\t{value}",
            fqdn(&record.name),
            record.ttl,
            record.record_type
        );
    }
    out
}

fn txt_value(value: &str) -> String {
    split_chunks(value, MAX_TXT_CHUNK)
        .into_iter()
        .map(|chunk| format!("\"{}\"", escape(chunk)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn split_chunks(value: &str, limit: usize) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut end = 0;
    for ch in value.chars() {
        if end - start + ch.len_utf8() > limit {
            chunks.push(&value[start..end]);
            start = end;
        }
        end += ch.len_utf8();
    }
    if start < end || chunks.is_empty() {
        chunks.push(&value[start..end]);
    }
    chunks
}

fn escape(chunk: &str) -> String {
    chunk.replace('\\', r"\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    use crate::dkim_keys::generate_ed25519;
    use crate::rec_mx::{DnsRecord, DnsRecordKind, DEFAULT_TTL};
    use crate::records::{domain_records, DomainRecordsInput};

    fn record_lines(zone: &str) -> Vec<&str> {
        zone.lines()
            .filter(|line| !line.is_empty() && !line.starts_with(';') && !line.starts_with('$'))
            .collect()
    }

    fn quoted_parts(line: &str) -> Vec<String> {
        let value = line.split('\t').nth(4).unwrap();
        value
            .split("\" \"")
            .map(|part| part.trim_matches('"').to_string())
            .collect()
    }

    #[test]
    fn the_zone_opens_with_the_origin_and_default_ttl() {
        let zone = zone_file("example.com", &[]);
        assert!(zone.starts_with(
            "; irixmail DNS records for example.com\n$ORIGIN example.com.\n$TTL 3600\n"
        ));
    }

    #[test]
    fn every_record_name_is_absolute() {
        let records = vec![
            crate::rec_mx::mx_record("example.com", "mail.example.com", 10),
            DnsRecord::txt(
                DnsRecordKind::Dmarc,
                "_dmarc.example.com",
                "v=DMARC1; p=none",
            ),
        ];
        let zone = zone_file("example.com", &records);
        for line in record_lines(&zone) {
            let name = line.split('\t').next().unwrap();
            assert!(name.ends_with('.'), "name is not absolute: {line}");
        }
    }

    #[test]
    fn a_long_txt_value_is_split_into_quoted_strings_of_at_most_255_characters() {
        let value = "a".repeat(400);
        let records = vec![DnsRecord::txt(
            DnsRecordKind::Dkim,
            "mail._domainkey.example.com",
            value.clone(),
        )];
        let zone = zone_file("example.com", &records);
        let line = record_lines(&zone)[0];
        let parts = quoted_parts(line);
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|part| part.len() <= 255));
        assert_eq!(parts.concat(), value);
    }

    #[test]
    fn a_short_txt_value_stays_a_single_quoted_string() {
        let records = vec![DnsRecord::txt(
            DnsRecordKind::Spf,
            "example.com",
            "v=spf1 mx -all",
        )];
        let zone = zone_file("example.com", &records);
        let line = record_lines(&zone)[0];
        assert!(line.ends_with("\tTXT\t\"v=spf1 mx -all\""), "{line}");
    }

    #[test]
    fn quotes_and_backslashes_in_a_txt_value_are_escaped() {
        let records = vec![DnsRecord::txt(
            DnsRecordKind::Spf,
            "example.com",
            r#"he said "hi" \ bye"#,
        )];
        let zone = zone_file("example.com", &records);
        let line = record_lines(&zone)[0];
        assert!(line.contains(r#"\"hi\""#), "{line}");
        assert!(line.contains(r"\\"), "{line}");
    }

    #[test]
    fn mx_and_srv_values_keep_their_preference_and_priority() {
        let mut records = vec![crate::rec_mx::mx_record(
            "example.com",
            "mail.example.com",
            10,
        )];
        records.extend(crate::rec_autoconfig::autoconfig_records(
            "example.com",
            "mail.example.com",
        ));
        let zone = zone_file("example.com", &records);
        assert!(zone.contains("example.com.\t3600\tIN\tMX\t10 mail.example.com."));
        assert!(zone.contains("_imaps._tcp.example.com.\t3600\tIN\tSRV\t0 1 993 mail.example.com."));
    }

    #[test]
    fn the_generated_bundle_produces_one_line_per_record() {
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
        let zone = zone_file("example.com", &records);
        assert_eq!(zone.lines().count(), 3 + records.len());
        for record_type in ["A", "AAAA", "CNAME", "SRV"] {
            assert!(
                zone.contains(&format!("\tIN\t{record_type}\t")),
                "missing {record_type} in {zone}"
            );
        }
    }

    #[test]
    fn a_mail_host_in_another_zone_leaves_its_address_records_out_of_the_zone_file() {
        let input = DomainRecordsInput {
            domain: "msaeedsakib.com",
            mail_host: "mail.irixsoft.co.uk",
            ipv4: Some(Ipv4Addr::new(192, 0, 2, 1)),
            ipv6: Some(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
            dkim_keys: &[],
            mtasts_id: "20260101000000",
            mx_preference: 10,
        };
        let zone = zone_file("msaeedsakib.com", &domain_records(&input));
        assert!(!zone.contains("mail.irixsoft.co.uk.\t"), "{zone}");
        assert!(!zone.contains("\tIN\tA\t"), "{zone}");
        assert!(!zone.contains("\tIN\tAAAA\t"), "{zone}");
        assert!(zone.contains("msaeedsakib.com.\t3600\tIN\tMX\t10 mail.irixsoft.co.uk."));
    }

    #[test]
    fn a_domain_that_owns_its_mail_host_keeps_the_address_records_in_the_zone_file() {
        let input = DomainRecordsInput {
            domain: "example.com",
            mail_host: "mail.example.com",
            ipv4: Some(Ipv4Addr::new(192, 0, 2, 1)),
            ipv6: Some(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
            dkim_keys: &[],
            mtasts_id: "20260101000000",
            mx_preference: 10,
        };
        let zone = zone_file("example.com", &domain_records(&input));
        assert!(
            zone.contains("mail.example.com.\t3600\tIN\tA\t192.0.2.1"),
            "{zone}"
        );
        assert!(
            zone.contains("mail.example.com.\t3600\tIN\tAAAA\t2001:db8::1"),
            "{zone}"
        );
    }

    #[test]
    fn a_record_with_a_custom_ttl_keeps_it() {
        let mut record = crate::rec_mx::mx_record("example.com", "mail.example.com", 10);
        record.ttl = 60;
        let zone = zone_file("example.com", &[record]);
        assert!(zone.contains("example.com.\t60\tIN\tMX\t"), "{zone}");
        assert!(zone.contains(&format!("$TTL {DEFAULT_TTL}")));
    }
}
