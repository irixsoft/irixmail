use std::net::{IpAddr, Ipv4Addr};

use hickory_resolver::lookup::Lookup;
use hickory_resolver::proto::rr::{RData, RecordType};

use irixmail_core::Result;

use crate::resolver::Resolver;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsblListing {
    pub codes: Vec<Ipv4Addr>,
    pub reason: Option<String>,
}

pub async fn check_ip(resolver: &Resolver, ip: IpAddr, zone: &str) -> Result<Option<DnsblListing>> {
    let name = query_name(ip, zone);

    let codes: Vec<Ipv4Addr> = match resolver.lookup(&name, RecordType::A).await? {
        Some(lookup) => lookup.iter().filter_map(listing_code).collect(),
        None => Vec::new(),
    };
    if codes.is_empty() {
        return Ok(None);
    }

    let reason = match resolver.lookup(&name, RecordType::TXT).await? {
        Some(lookup) => reason_from_lookup(&lookup),
        None => None,
    };
    Ok(Some(DnsblListing { codes, reason }))
}

fn query_name(ip: IpAddr, zone: &str) -> String {
    let reversed = match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            format!("{}.{}.{}.{}", o[3], o[2], o[1], o[0])
        }
        IpAddr::V6(v6) => {
            let mut nibbles = Vec::with_capacity(32);
            for byte in v6.octets() {
                nibbles.push(byte >> 4);
                nibbles.push(byte & 0x0f);
            }
            nibbles.reverse();
            nibbles
                .iter()
                .map(|nibble| format!("{nibble:x}"))
                .collect::<Vec<_>>()
                .join(".")
        }
    };
    format!("{reversed}.{zone}")
}

fn listing_code(rdata: &RData) -> Option<Ipv4Addr> {
    match rdata {
        RData::A(a) if a.0.octets()[0] == 127 => Some(a.0),
        _ => None,
    }
}

fn reason_from_lookup(lookup: &Lookup) -> Option<String> {
    let parts: Vec<String> = lookup
        .iter()
        .filter_map(|rdata| match rdata {
            RData::TXT(txt) => {
                let joined: String = txt
                    .txt_data()
                    .iter()
                    .map(|chunk| String::from_utf8_lossy(chunk))
                    .collect();
                (!joined.is_empty()).then_some(joined)
            }
            _ => None,
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv6Addr;

    #[test]
    fn an_ipv4_address_is_reversed_before_the_zone() {
        let name = query_name(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)), "zen.spamhaus.org");
        assert_eq!(name, "4.3.2.1.zen.spamhaus.org");
    }

    #[test]
    fn an_ipv6_address_is_reversed_nibble_by_nibble() {
        let name = query_name(IpAddr::V6(Ipv6Addr::LOCALHOST), "example.bl");
        let (nibbles, zone) = name
            .rsplit_once(".example.bl")
            .map(|(n, _)| (n, "example.bl"))
            .unwrap();
        assert_eq!(zone, "example.bl");
        assert_eq!(nibbles.split('.').count(), 32);
        assert!(nibbles.starts_with("1.0.0."));
    }

    #[test]
    fn only_loopback_codes_count_as_a_listing() {
        let listed = RData::A(hickory_resolver::proto::rr::rdata::A::new(127, 0, 0, 2));
        let other = RData::A(hickory_resolver::proto::rr::rdata::A::new(192, 0, 2, 1));
        assert_eq!(listing_code(&listed), Some(Ipv4Addr::new(127, 0, 0, 2)));
        assert_eq!(listing_code(&other), None);
    }
}
