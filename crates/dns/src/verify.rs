use hickory_resolver::proto::rr::{RData, RecordType};

use irixmail_core::Result;

use crate::rec_mx::{fqdn, DnsRecord};
use crate::resolver::Resolver;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Pass,
    Mismatch,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordCheck {
    pub record: DnsRecord,
    pub status: CheckStatus,
    pub observed: Vec<String>,
}

pub async fn verify_record(resolver: &Resolver, record: &DnsRecord) -> Result<RecordCheck> {
    let observed = match record_type_of(&record.record_type) {
        Some(rtype) => match resolver.lookup(&record.name, rtype).await? {
            Some(lookup) => lookup.iter().filter_map(observed_value).collect(),
            None => Vec::new(),
        },
        None => Vec::new(),
    };
    let status = decide(&record.record_type, &record.value, &observed);
    Ok(RecordCheck {
        record: record.clone(),
        status,
        observed,
    })
}

pub async fn verify_all(resolver: &Resolver, records: &[DnsRecord]) -> Result<Vec<RecordCheck>> {
    let mut checks = Vec::with_capacity(records.len());
    for record in records {
        checks.push(verify_record(resolver, record).await?);
    }
    Ok(checks)
}

fn record_type_of(record_type: &str) -> Option<RecordType> {
    match record_type {
        "A" => Some(RecordType::A),
        "AAAA" => Some(RecordType::AAAA),
        "CNAME" => Some(RecordType::CNAME),
        "MX" => Some(RecordType::MX),
        "TXT" => Some(RecordType::TXT),
        "SRV" => Some(RecordType::SRV),
        _ => None,
    }
}

fn observed_value(rdata: &RData) -> Option<String> {
    match rdata {
        RData::A(a) => Some(a.0.to_string()),
        RData::AAAA(a) => Some(a.0.to_string()),
        RData::CNAME(cname) => Some(fqdn(&cname.0.to_string())),
        RData::MX(mx) => Some(format!(
            "{} {}",
            mx.preference(),
            fqdn(&mx.exchange().to_string())
        )),
        RData::SRV(srv) => Some(format!(
            "{} {} {} {}",
            srv.priority(),
            srv.weight(),
            srv.port(),
            fqdn(&srv.target().to_string())
        )),
        RData::TXT(txt) => Some(
            txt.txt_data()
                .iter()
                .map(|chunk| String::from_utf8_lossy(chunk))
                .collect::<String>(),
        ),
        _ => None,
    }
}

fn decide(record_type: &str, expected: &str, observed: &[String]) -> CheckStatus {
    let fold_case = record_type != "TXT";
    if observed.is_empty() {
        CheckStatus::Missing
    } else if observed
        .iter()
        .any(|value| normalize(value, fold_case) == normalize(expected, fold_case))
    {
        CheckStatus::Pass
    } else {
        CheckStatus::Mismatch
    }
}

fn normalize(value: &str, fold_case: bool) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if fold_case {
        collapsed.to_ascii_lowercase()
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_resolver::proto::rr::rdata::{A, MX};
    use hickory_resolver::proto::rr::Name;
    use std::str::FromStr;

    #[test]
    fn record_types_map_from_their_names() {
        assert_eq!(record_type_of("MX"), Some(RecordType::MX));
        assert_eq!(record_type_of("TXT"), Some(RecordType::TXT));
        assert_eq!(record_type_of("nonsense"), None);
    }

    #[test]
    fn observed_values_render_in_presentation_form() {
        let mx = RData::MX(MX::new(10, Name::from_str("mail.example.com.").unwrap()));
        assert_eq!(observed_value(&mx).unwrap(), "10 mail.example.com.");
        let a = RData::A(A::new(192, 0, 2, 1));
        assert_eq!(observed_value(&a).unwrap(), "192.0.2.1");
    }

    #[test]
    fn cname_records_resolve_and_render() {
        assert_eq!(record_type_of("CNAME"), Some(RecordType::CNAME));
        let cname = RData::CNAME(hickory_resolver::proto::rr::rdata::CNAME(
            Name::from_str("mail.example.com.").unwrap(),
        ));
        assert_eq!(observed_value(&cname).unwrap(), "mail.example.com.");
    }

    #[test]
    fn a_host_name_published_with_different_case_still_passes() {
        assert_eq!(
            decide(
                "MX",
                "10 mail.example.com.",
                &["10 MAIL.EXAMPLE.COM.".into()]
            ),
            CheckStatus::Pass
        );
        assert_eq!(
            decide("CNAME", "mail.example.com.", &["Mail.Example.Com.".into()]),
            CheckStatus::Pass
        );
        assert_eq!(
            decide(
                "SRV",
                "0 1 993 mail.example.com.",
                &["0 1 993 MAIL.example.com.".into()]
            ),
            CheckStatus::Pass
        );
    }

    #[test]
    fn a_txt_value_with_different_case_is_still_a_mismatch() {
        assert_eq!(
            decide("TXT", "v=DKIM1; p=AbCd", &["v=DKIM1; p=abcd".into()]),
            CheckStatus::Mismatch
        );
    }

    #[test]
    fn decide_distinguishes_missing_mismatch_and_pass() {
        assert_eq!(
            decide("MX", "10 mail.example.com.", &[]),
            CheckStatus::Missing
        );
        assert_eq!(
            decide(
                "MX",
                "10 mail.example.com.",
                &["20 other.example.com.".into()]
            ),
            CheckStatus::Mismatch
        );
        assert_eq!(
            decide(
                "MX",
                "10 mail.example.com.",
                &["10  mail.example.com.".into()]
            ),
            CheckStatus::Pass
        );
    }
}
