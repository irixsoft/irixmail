use std::net::IpAddr;

use irixmail_core::Result;
use irixmail_dns::lookup::{host_ips, mx_hosts};
use irixmail_dns::Resolver;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MxTarget {
    pub host: String,
    pub ips: Vec<IpAddr>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MxResolution {
    Targets(Vec<MxTarget>),
    NoMailAccepted,
    Unresolvable,
}

pub async fn resolve(resolver: &Resolver, domain: &str) -> Result<MxResolution> {
    let exchanges = mx_hosts(resolver, domain).await?;

    if is_null_mx(&exchanges) {
        return Ok(MxResolution::NoMailAccepted);
    }

    let hosts: Vec<String> = if exchanges.is_empty() {
        vec![domain.to_string()]
    } else {
        exchanges
            .into_iter()
            .map(|exchange| exchange.host)
            .collect()
    };

    let mut targets = Vec::new();
    for host in hosts {
        let ips = host_ips(resolver, &host).await?;
        if !ips.is_empty() {
            targets.push(MxTarget { host, ips });
        }
    }

    if targets.is_empty() {
        Ok(MxResolution::Unresolvable)
    } else {
        Ok(MxResolution::Targets(targets))
    }
}

fn is_null_mx(exchanges: &[irixmail_dns::lookup::MailExchange]) -> bool {
    matches!(
        exchanges,
        [exchange] if exchange.preference == 0 && (exchange.host.is_empty() || exchange.host == ".")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use irixmail_dns::lookup::MailExchange;

    fn exchange(preference: u16, host: &str) -> MailExchange {
        MailExchange {
            preference,
            host: host.to_string(),
        }
    }

    #[test]
    fn a_root_exchange_at_zero_preference_is_a_null_mx() {
        assert!(is_null_mx(&[exchange(0, ".")]));
        assert!(is_null_mx(&[exchange(0, "")]));
    }

    #[test]
    fn an_ordinary_exchange_is_not_a_null_mx() {
        assert!(!is_null_mx(&[exchange(10, "mail.example.com")]));
    }

    #[test]
    fn a_root_exchange_at_nonzero_preference_is_not_a_null_mx() {
        assert!(!is_null_mx(&[exchange(10, ".")]));
    }

    #[test]
    fn several_exchanges_are_never_a_null_mx() {
        assert!(!is_null_mx(&[
            exchange(0, "."),
            exchange(10, "mail.example.com")
        ]));
    }

    #[test]
    fn an_empty_exchange_list_is_not_a_null_mx() {
        assert!(!is_null_mx(&[]));
    }

    #[test]
    fn a_target_pairs_a_host_with_its_addresses() {
        let target = MxTarget {
            host: "mail.example.com".to_string(),
            ips: vec![IpAddr::from([192, 0, 2, 1])],
        };
        assert_eq!(target.host, "mail.example.com");
        assert_eq!(target.ips, vec![IpAddr::from([192, 0, 2, 1])]);
    }

    #[test]
    fn the_resolution_variants_compare_by_value() {
        assert_eq!(MxResolution::NoMailAccepted, MxResolution::NoMailAccepted);
        assert_eq!(MxResolution::Unresolvable, MxResolution::Unresolvable);
        assert_ne!(MxResolution::NoMailAccepted, MxResolution::Unresolvable);
    }
}
