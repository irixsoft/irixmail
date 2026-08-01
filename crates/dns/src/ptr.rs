use std::net::IpAddr;

use hickory_resolver::proto::rr::{RData, RecordType};

use irixmail_core::Result;

use crate::lookup::host_ips;
use crate::resolver::Resolver;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtrStatus {
    Missing,
    Mismatch,
    Unconfirmed,
    Ok,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtrCheck {
    pub names: Vec<String>,
    pub matches_expected: bool,
    pub forward_confirmed: bool,
}

impl PtrCheck {
    pub fn status(&self) -> PtrStatus {
        if self.names.is_empty() {
            PtrStatus::Missing
        } else if !self.matches_expected {
            PtrStatus::Mismatch
        } else if !self.forward_confirmed {
            PtrStatus::Unconfirmed
        } else {
            PtrStatus::Ok
        }
    }

    pub fn is_healthy(&self) -> bool {
        matches!(self.status(), PtrStatus::Ok)
    }
}

pub async fn lookup_ptr(resolver: &Resolver, ip: IpAddr) -> Result<Vec<String>> {
    let name = reverse_name(ip);
    let mut names = Vec::new();
    if let Some(lookup) = resolver.lookup(&name, RecordType::PTR).await? {
        for rdata in lookup.iter() {
            if let RData::PTR(ptr) = rdata {
                names.push(trim_root(&ptr.0.to_string()));
            }
        }
    }
    Ok(names)
}

pub async fn check(resolver: &Resolver, ip: IpAddr, expected_host: &str) -> Result<PtrCheck> {
    let names = lookup_ptr(resolver, ip).await?;

    let expected = expected_host.trim_end_matches('.').to_ascii_lowercase();
    let matches_expected = names
        .iter()
        .any(|name| name.eq_ignore_ascii_case(&expected));

    let mut forward_confirmed = false;
    for name in &names {
        if host_ips(resolver, name).await?.contains(&ip) {
            forward_confirmed = true;
            break;
        }
    }

    Ok(PtrCheck {
        names,
        matches_expected,
        forward_confirmed,
    })
}

fn reverse_name(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            format!("{}.{}.{}.{}.in-addr.arpa", o[3], o[2], o[1], o[0])
        }
        IpAddr::V6(v6) => {
            let mut nibbles = Vec::with_capacity(32);
            for byte in v6.octets() {
                nibbles.push(byte >> 4);
                nibbles.push(byte & 0x0f);
            }
            nibbles.reverse();
            let labels = nibbles
                .iter()
                .map(|nibble| format!("{nibble:x}"))
                .collect::<Vec<_>>()
                .join(".");
            format!("{labels}.ip6.arpa")
        }
    }
}

fn trim_root(name: &str) -> String {
    name.strip_suffix('.').unwrap_or(name).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn check_with(names: &[&str], matches: bool, confirmed: bool) -> PtrCheck {
        PtrCheck {
            names: names.iter().map(|n| n.to_string()).collect(),
            matches_expected: matches,
            forward_confirmed: confirmed,
        }
    }

    #[test]
    fn an_ipv4_address_reverses_under_in_addr_arpa() {
        let name = reverse_name(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)));
        assert_eq!(name, "1.2.0.192.in-addr.arpa");
    }

    #[test]
    fn an_ipv6_address_reverses_under_ip6_arpa() {
        let name = reverse_name(IpAddr::V6(Ipv6Addr::LOCALHOST));
        assert!(name.ends_with(".ip6.arpa"));
        let labels = name.strip_suffix(".ip6.arpa").unwrap();
        assert_eq!(labels.split('.').count(), 32);
        assert!(labels.starts_with("1.0.0."));
    }

    #[test]
    fn status_reflects_each_failure_mode() {
        assert_eq!(check_with(&[], false, false).status(), PtrStatus::Missing);
        assert_eq!(
            check_with(&["other.example.com"], false, false).status(),
            PtrStatus::Mismatch
        );
        assert_eq!(
            check_with(&["mail.example.com"], true, false).status(),
            PtrStatus::Unconfirmed
        );
        let healthy = check_with(&["mail.example.com"], true, true);
        assert_eq!(healthy.status(), PtrStatus::Ok);
        assert!(healthy.is_healthy());
    }
}
