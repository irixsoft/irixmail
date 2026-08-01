use std::net::IpAddr;

use irixmail_dns::dnsbl::{check_ip, DnsblListing};
use irixmail_dns::Resolver;

const LISTED: &[u8] = b"554 5.7.1 Connection refused, your address is on a public blocklist\r\n";

pub const DEFAULT_ZONE: &str = "zen.spamhaus.org";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DnsblConfig {
    pub zones: Vec<String>,
}

impl Default for DnsblConfig {
    fn default() -> Self {
        Self {
            zones: vec![DEFAULT_ZONE.to_string()],
        }
    }
}

impl DnsblConfig {
    pub fn is_empty(&self) -> bool {
        self.zones.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DnsblDecision {
    Allow,
    Reject {
        zone: String,
        listing: DnsblListing,
        reply: &'static [u8],
    },
}

impl DnsblDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, DnsblDecision::Allow)
    }
}

pub async fn check(config: &DnsblConfig, resolver: &Resolver, ip: IpAddr) -> DnsblDecision {
    for zone in &config.zones {
        match check_ip(resolver, ip, zone).await {
            Ok(Some(listing)) => {
                return DnsblDecision::Reject {
                    zone: zone.clone(),
                    listing,
                    reply: LISTED,
                };
            }
            Ok(None) => {}
            Err(_) => {}
        }
    }
    DnsblDecision::Allow
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_config_checks_the_spamhaus_zone() {
        let config = DnsblConfig::default();
        assert_eq!(config.zones, vec![DEFAULT_ZONE.to_string()]);
        assert!(!config.is_empty());
    }

    #[test]
    fn an_empty_zone_list_is_reported_as_empty() {
        let config = DnsblConfig { zones: Vec::new() };
        assert!(config.is_empty());
    }

    #[test]
    fn the_allow_decision_is_permitted() {
        assert!(DnsblDecision::Allow.is_allowed());
    }

    #[test]
    fn a_reject_decision_is_not_permitted_and_is_a_permanent_refusal() {
        let decision = DnsblDecision::Reject {
            zone: DEFAULT_ZONE.to_string(),
            listing: DnsblListing {
                codes: vec![std::net::Ipv4Addr::new(127, 0, 0, 2)],
                reason: Some("listed".to_string()),
            },
            reply: LISTED,
        };
        assert!(!decision.is_allowed());
        match decision {
            DnsblDecision::Reject { reply, .. } => assert!(reply.starts_with(b"554")),
            DnsblDecision::Allow => unreachable!(),
        }
    }
}
