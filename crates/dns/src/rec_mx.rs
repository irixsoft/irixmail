use serde::{Deserialize, Serialize};

pub const DEFAULT_TTL: u32 = 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DnsRecordKind {
    Mx,
    A,
    Aaaa,
    Spf,
    Dkim,
    Dmarc,
    MtaSts,
    TlsRpt,
    Autoconfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsRecord {
    pub kind: DnsRecordKind,
    pub name: String,
    pub record_type: String,
    pub value: String,
    pub ttl: u32,
    pub in_zone: bool,
}

impl DnsRecord {
    pub fn new(
        kind: DnsRecordKind,
        name: impl Into<String>,
        record_type: &str,
        value: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            name: name.into(),
            record_type: record_type.to_string(),
            value: value.into(),
            ttl: DEFAULT_TTL,
            in_zone: true,
        }
    }

    pub fn txt(kind: DnsRecordKind, name: impl Into<String>, value: impl Into<String>) -> Self {
        Self::new(kind, name, "TXT", value)
    }
}

pub(crate) fn fqdn(host: &str) -> String {
    if host.ends_with('.') {
        host.to_string()
    } else {
        format!("{host}.")
    }
}

pub fn mx_record(domain: &str, mail_host: &str, preference: u16) -> DnsRecord {
    DnsRecord::new(
        DnsRecordKind::Mx,
        domain,
        "MX",
        format!("{preference} {}", fqdn(mail_host)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mx_record_points_the_domain_at_a_qualified_mail_host() {
        let record = mx_record("example.com", "mail.example.com", 10);
        assert_eq!(record.kind, DnsRecordKind::Mx);
        assert_eq!(record.name, "example.com");
        assert_eq!(record.record_type, "MX");
        assert_eq!(record.value, "10 mail.example.com.");
        assert_eq!(record.ttl, DEFAULT_TTL);
    }

    #[test]
    fn fqdn_appends_only_a_missing_root() {
        assert_eq!(fqdn("mail.example.com"), "mail.example.com.");
        assert_eq!(fqdn("mail.example.com."), "mail.example.com.");
    }
}
