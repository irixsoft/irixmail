use std::net::IpAddr;

use irixmail_core::Result;
use irixmail_dns::{host_ips, Resolver};

pub struct PreflightReport {
    pub resolved: Vec<IpAddr>,
    pub hostname_matches: bool,
    pub challenge_port_bindable: bool,
}

impl PreflightReport {
    pub fn is_ready(&self) -> bool {
        self.hostname_matches && self.challenge_port_bindable
    }
}

pub async fn preflight(
    resolver: &Resolver,
    hostname: &str,
    expected_ips: &[IpAddr],
    challenge_port: u16,
) -> Result<PreflightReport> {
    let resolved = host_ips(resolver, hostname).await?;
    Ok(PreflightReport {
        hostname_matches: matches_expected(&resolved, expected_ips),
        challenge_port_bindable: port_is_bindable(challenge_port),
        resolved,
    })
}

fn matches_expected(resolved: &[IpAddr], expected: &[IpAddr]) -> bool {
    if resolved.is_empty() || expected.is_empty() {
        return false;
    }
    [true, false]
        .into_iter()
        .all(|v4| family_agrees(resolved, expected, v4))
}

fn family_agrees(resolved: &[IpAddr], expected: &[IpAddr], v4: bool) -> bool {
    let expected: Vec<&IpAddr> = expected.iter().filter(|ip| ip.is_ipv4() == v4).collect();
    let mut resolved = resolved.iter().filter(|ip| ip.is_ipv4() == v4).peekable();
    if expected.is_empty() || resolved.peek().is_none() {
        return true;
    }
    resolved.any(|ip| expected.contains(&ip))
}

pub fn port_is_bindable(port: u16) -> bool {
    std::net::TcpListener::bind(("0.0.0.0", port)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    #[test]
    fn the_hostname_matches_only_when_a_resolved_ip_is_expected() {
        assert!(matches_expected(&[ip(1, 2, 3, 4)], &[ip(1, 2, 3, 4)]));
        assert!(!matches_expected(&[ip(1, 2, 3, 4)], &[ip(5, 6, 7, 8)]));
        assert!(!matches_expected(&[], &[ip(1, 2, 3, 4)]));
        assert!(!matches_expected(&[ip(1, 2, 3, 4)], &[]));
    }

    #[test]
    fn family_disjoint_detection_does_not_contradict_dns() {
        let v6: IpAddr = "2001:db8::7".parse().unwrap();
        assert!(matches_expected(&[ip(1, 2, 3, 4)], &[v6]));
        assert!(matches_expected(&[v6], &[ip(1, 2, 3, 4)]));
    }

    #[test]
    fn a_mismatch_within_a_shared_family_still_fails() {
        let v6a: IpAddr = "2001:db8::7".parse().unwrap();
        let v6b: IpAddr = "2001:db8::8".parse().unwrap();
        assert!(!matches_expected(&[ip(1, 2, 3, 4), v6a], &[v6b]));
        assert!(matches_expected(&[ip(1, 2, 3, 4), v6a], &[v6a]));
    }

    #[test]
    fn an_ephemeral_port_is_bindable() {
        assert!(port_is_bindable(0));
    }

    #[test]
    fn a_port_already_bound_is_not_bindable() {
        let held = std::net::TcpListener::bind(("0.0.0.0", 0)).unwrap();
        let port = held.local_addr().unwrap().port();
        assert!(!port_is_bindable(port));
        drop(held);
    }
}
