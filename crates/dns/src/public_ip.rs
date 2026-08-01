use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, UdpSocket};
use std::time::Duration;

use hickory_resolver::config::{NameServerConfigGroup, ResolverConfig, ResolverOpts};
use hickory_resolver::proto::rr::{RData, RecordType};

use crate::resolver::Resolver;

const SELF_LOOKUP_HOST: &str = "myip.opendns.com";
const OPENDNS_V4: [IpAddr; 2] = [
    IpAddr::V4(Ipv4Addr::new(208, 67, 222, 222)),
    IpAddr::V4(Ipv4Addr::new(208, 67, 220, 220)),
];
const OPENDNS_V6: [IpAddr; 2] = [
    IpAddr::V6(Ipv6Addr::new(0x2620, 0x119, 0x35, 0, 0, 0, 0, 0x35)),
    IpAddr::V6(Ipv6Addr::new(0x2620, 0x119, 0x53, 0, 0, 0, 0, 0x53)),
];

fn outbound(bind: &str, target: &str) -> Option<IpAddr> {
    let socket = UdpSocket::bind(bind).ok()?;
    socket.connect(target).ok()?;
    socket.local_addr().ok().map(|addr| addr.ip())
}

pub fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => {
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast())
        }
        IpAddr::V6(v6) => {
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local())
        }
    }
}

pub fn detect() -> Vec<IpAddr> {
    [
        outbound("0.0.0.0:0", "8.8.8.8:80"),
        outbound("[::]:0", "[2001:4860:4860::8888]:80"),
    ]
    .into_iter()
    .flatten()
    .filter(|address| is_public(*address))
    .collect()
}

pub async fn detect_all() -> Vec<IpAddr> {
    let mut addresses = detect();
    if !addresses.iter().any(|address| address.is_ipv4()) {
        let found = self_lookup(&pinned_resolver(&OPENDNS_V4), RecordType::A).await;
        merge_public_unique(&mut addresses, found);
    }
    if !addresses.iter().any(|address| address.is_ipv6()) {
        let found = self_lookup(&pinned_resolver(&OPENDNS_V6), RecordType::AAAA).await;
        merge_public_unique(&mut addresses, found);
    }
    addresses
}

pub fn first_v4(addresses: &[IpAddr]) -> Option<Ipv4Addr> {
    addresses.iter().find_map(|address| match address {
        IpAddr::V4(v4) => Some(*v4),
        IpAddr::V6(_) => None,
    })
}

pub fn first_v6(addresses: &[IpAddr]) -> Option<Ipv6Addr> {
    addresses.iter().find_map(|address| match address {
        IpAddr::V4(_) => None,
        IpAddr::V6(v6) => Some(*v6),
    })
}

fn pinned_resolver(servers: &[IpAddr]) -> Resolver {
    let group = NameServerConfigGroup::from_ips_clear(servers, 53, true);
    let config = ResolverConfig::from_parts(None, Vec::new(), group);
    let mut options = ResolverOpts::default();
    options.attempts = 1;
    options.timeout = Duration::from_secs(3);
    Resolver::from_config(config, options)
}

async fn self_lookup(resolver: &Resolver, record_type: RecordType) -> Vec<IpAddr> {
    match resolver.lookup(SELF_LOOKUP_HOST, record_type).await {
        Ok(Some(lookup)) => lookup.iter().filter_map(record_address).collect(),
        _ => Vec::new(),
    }
}

fn record_address(rdata: &RData) -> Option<IpAddr> {
    match rdata {
        RData::A(a) => Some(IpAddr::V4(a.0)),
        RData::AAAA(aaaa) => Some(IpAddr::V6(aaaa.0)),
        _ => None,
    }
}

fn merge_public_unique(base: &mut Vec<IpAddr>, extra: Vec<IpAddr>) {
    for address in extra {
        if is_public(address) && !base.contains(&address) {
            base.push(address);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_outbound_socket_reports_the_local_address_for_a_route() {
        assert_eq!(
            outbound("127.0.0.1:0", "127.0.0.1:9"),
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
        );
    }

    #[test]
    fn private_and_special_addresses_are_not_public() {
        for address in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.1.5",
            "172.16.0.1",
            "169.254.0.1",
            "0.0.0.0",
            "::1",
            "::",
            "fd00::1",
            "fe80::1",
        ] {
            assert!(
                !is_public(address.parse().unwrap()),
                "{address} must not count as public"
            );
        }
    }

    #[test]
    fn globally_routable_addresses_are_public() {
        for address in ["198.51.100.7", "8.8.8.8", "2001:db8::7", "2600::1"] {
            assert!(
                is_public(address.parse().unwrap()),
                "{address} must count as public"
            );
        }
    }

    #[test]
    fn the_first_address_of_each_family_is_selected() {
        let addresses: Vec<IpAddr> = ["2001:db8::7", "198.51.100.7", "203.0.113.9"]
            .iter()
            .map(|address| address.parse().unwrap())
            .collect();
        assert_eq!(first_v4(&addresses), Some(Ipv4Addr::new(198, 51, 100, 7)));
        assert_eq!(first_v6(&addresses), Some("2001:db8::7".parse().unwrap()));
        assert_eq!(first_v4(&[]), None);
        assert_eq!(first_v6(&[]), None);
    }

    #[test]
    fn address_records_yield_addresses_and_other_records_do_not() {
        use hickory_resolver::proto::rr::rdata::{A, AAAA, TXT};
        let a = RData::A(A::new(198, 51, 100, 7));
        let aaaa = RData::AAAA(AAAA::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 7));
        let txt = RData::TXT(TXT::new(vec!["nope".into()]));
        assert_eq!(record_address(&a), Some("198.51.100.7".parse().unwrap()));
        assert_eq!(record_address(&aaaa), Some("2001:db8::7".parse().unwrap()));
        assert_eq!(record_address(&txt), None);
    }

    #[test]
    fn merging_keeps_only_new_public_addresses() {
        let existing: IpAddr = "198.51.100.7".parse().unwrap();
        let fresh: IpAddr = "203.0.113.9".parse().unwrap();
        let mut base = vec![existing];
        merge_public_unique(
            &mut base,
            vec![existing, "10.0.0.1".parse().unwrap(), fresh],
        );
        assert_eq!(base, vec![existing, fresh]);
    }
}
