use std::net::IpAddr;

use hickory_resolver::proto::rr::{RData, RecordType};

use irixmail_core::Result;

use crate::resolver::Resolver;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailExchange {
    pub preference: u16,
    pub host: String,
}

pub async fn mx_hosts(resolver: &Resolver, domain: &str) -> Result<Vec<MailExchange>> {
    let mut exchanges = Vec::new();
    if let Some(lookup) = resolver.lookup(domain, RecordType::MX).await? {
        for rdata in lookup.iter() {
            if let Some(exchange) = exchange_from_rdata(rdata) {
                exchanges.push(exchange);
            }
        }
    }
    sort_exchanges(&mut exchanges);
    Ok(exchanges)
}

pub async fn host_ips(resolver: &Resolver, host: &str) -> Result<Vec<IpAddr>> {
    match resolver.lookup_ip(host).await? {
        Some(ips) => Ok(ips.iter().collect()),
        None => Ok(Vec::new()),
    }
}

fn exchange_from_rdata(rdata: &RData) -> Option<MailExchange> {
    match rdata {
        RData::MX(mx) => Some(MailExchange {
            preference: mx.preference(),
            host: trim_root(&mx.exchange().to_string()),
        }),
        _ => None,
    }
}

fn sort_exchanges(exchanges: &mut [MailExchange]) {
    exchanges.sort_by(|a, b| {
        a.preference
            .cmp(&b.preference)
            .then_with(|| a.host.cmp(&b.host))
    });
}

fn trim_root(name: &str) -> String {
    name.strip_suffix('.').unwrap_or(name).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_resolver::proto::rr::rdata::MX;
    use hickory_resolver::proto::rr::Name;
    use std::str::FromStr;

    fn mx(preference: u16, exchange: &str) -> RData {
        RData::MX(MX::new(preference, Name::from_str(exchange).unwrap()))
    }

    #[test]
    fn an_mx_record_becomes_a_trimmed_mail_exchange() {
        let exchange = exchange_from_rdata(&mx(10, "mail.example.com.")).unwrap();
        assert_eq!(
            exchange,
            MailExchange {
                preference: 10,
                host: "mail.example.com".to_string(),
            }
        );
    }

    #[test]
    fn a_non_mx_record_is_ignored() {
        let a = RData::A(hickory_resolver::proto::rr::rdata::A::new(192, 0, 2, 1));
        assert_eq!(exchange_from_rdata(&a), None);
    }

    #[test]
    fn exchanges_sort_by_preference_then_host() {
        let mut exchanges = vec![
            MailExchange {
                preference: 20,
                host: "backup.example.com".into(),
            },
            MailExchange {
                preference: 10,
                host: "b.example.com".into(),
            },
            MailExchange {
                preference: 10,
                host: "a.example.com".into(),
            },
        ];
        sort_exchanges(&mut exchanges);
        let order: Vec<_> = exchanges.iter().map(|e| e.host.as_str()).collect();
        assert_eq!(
            order,
            ["a.example.com", "b.example.com", "backup.example.com"]
        );
    }

    #[test]
    fn trim_root_removes_only_a_trailing_dot() {
        assert_eq!(trim_root("mail.example.com."), "mail.example.com");
        assert_eq!(trim_root("mail.example.com"), "mail.example.com");
    }
}
