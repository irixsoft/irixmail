use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Domain {
    pub id: u64,
    pub name: String,
    pub aliases: Vec<String>,
    pub enabled: bool,
    pub catch_all_account_id: Option<u64>,
    pub dkim_key_ids: Vec<u64>,
    pub dns_status: DnsStatus,
    pub created_at: u64,
}

impl Domain {
    pub fn matches_name(&self, name: &str) -> bool {
        if self.name.eq_ignore_ascii_case(name) {
            return true;
        }
        self.aliases
            .iter()
            .any(|alias| alias.eq_ignore_ascii_case(name))
    }

    pub fn accepts_mail(&self) -> bool {
        self.enabled
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum DnsStatus {
    #[default]
    Unverified,
    Verified {
        checked_at: u64,
    },
    Failing {
        checked_at: u64,
        missing: Vec<DnsRecordKind>,
    },
}

impl DnsStatus {
    pub fn is_verified(&self) -> bool {
        matches!(self, DnsStatus::Verified { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_domain() -> Domain {
        Domain {
            id: 42,
            name: "irixsoft.com".to_string(),
            aliases: vec!["irixmail.com".to_string()],
            enabled: true,
            catch_all_account_id: None,
            dkim_key_ids: Vec::new(),
            dns_status: DnsStatus::Unverified,
            created_at: 1_700_000_000_000,
        }
    }

    #[test]
    fn matches_the_canonical_name_regardless_of_case() {
        let domain = sample_domain();
        assert!(domain.matches_name("irixsoft.com"));
        assert!(domain.matches_name("IriXSoft.CoM"));
    }

    #[test]
    fn matches_an_alias_but_not_an_unrelated_name() {
        let domain = sample_domain();
        assert!(domain.matches_name("irixmail.com"));
        assert!(!domain.matches_name("example.org"));
    }

    #[test]
    fn a_disabled_domain_does_not_accept_mail() {
        let mut domain = sample_domain();
        assert!(domain.accepts_mail());
        domain.enabled = false;
        assert!(!domain.accepts_mail());
    }

    #[test]
    fn dns_status_defaults_to_unverified_and_is_not_verified() {
        assert_eq!(DnsStatus::default(), DnsStatus::Unverified);
        assert!(!DnsStatus::default().is_verified());
        assert!(!DnsStatus::Failing {
            checked_at: 1,
            missing: vec![DnsRecordKind::Spf],
        }
        .is_verified());
        assert!(DnsStatus::Verified { checked_at: 1 }.is_verified());
    }

    #[test]
    fn a_domain_round_trips_through_json() {
        let mut domain = sample_domain();
        domain.catch_all_account_id = Some(7);
        domain.dkim_key_ids = vec![100, 101];
        domain.dns_status = DnsStatus::Failing {
            checked_at: 1_700_000_500_000,
            missing: vec![DnsRecordKind::Dkim, DnsRecordKind::Dmarc],
        };

        let encoded = serde_json::to_string(&domain).expect("domain serializes");
        let decoded: Domain = serde_json::from_str(&encoded).expect("domain deserializes");
        assert_eq!(decoded, domain);
    }
}
