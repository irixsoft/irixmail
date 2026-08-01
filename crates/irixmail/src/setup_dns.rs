use std::net::{IpAddr, ToSocketAddrs};

use anyhow::Result;
use irixmail_core::BootstrapConfig;
use irixmail_dns::{ptr, PtrCheck, PtrStatus, Resolver};

use crate::setup::prompt;

pub fn configure(config: &BootstrapConfig, addresses: &[IpAddr]) -> Result<()> {
    let host = &config.server.hostname;
    println!("\nCreate the following DNS records:");
    if addresses.is_empty() {
        println!("  {host}.  A     <your server's public IPv4>");
    }
    for address in addresses {
        let kind = if address.is_ipv6() { "AAAA" } else { "A" };
        println!("  {host}.  {kind:<5} {address}");
    }
    println!("\nAlso ask your hosting provider to set the reverse DNS (PTR) of the address(es) above to {host}.");
    let _ = prompt("\nPress Enter once the records are in place... ")?;
    verify(host, addresses);
    for line in check_ptr(host, addresses) {
        println!("{line}");
    }
    Ok(())
}

fn check_ptr(host: &str, addresses: &[IpAddr]) -> Vec<String> {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return vec!["Warning: could not check reverse DNS (no async runtime).".to_string()];
    };
    runtime.block_on(async {
        match Resolver::from_system() {
            Ok(resolver) => ptr_report(&resolver, host, addresses).await,
            Err(err) => vec![format!("Warning: could not check reverse DNS: {err}")],
        }
    })
}

async fn ptr_report(resolver: &Resolver, host: &str, addresses: &[IpAddr]) -> Vec<String> {
    let mut lines = Vec::new();
    for address in addresses {
        match ptr::check(resolver, *address, host).await {
            Ok(check) => lines.push(match ptr_warning(&check, *address, host) {
                Some(warning) => warning,
                None => format!("Verified: the PTR for {address} resolves to {host}."),
            }),
            Err(err) => lines.push(format!(
                "Warning: could not look up the PTR for {address}: {err}"
            )),
        }
    }
    lines
}

fn ptr_warning(check: &PtrCheck, address: IpAddr, host: &str) -> Option<String> {
    match check.status() {
        PtrStatus::Ok => None,
        PtrStatus::Missing => Some(format!(
            "Warning: {address} has no PTR record — set its reverse DNS to {host}, or many providers will junk or refuse this server's mail."
        )),
        PtrStatus::Mismatch => Some(format!(
            "Warning: the PTR for {address} points at {} instead of {host} — many providers will junk or refuse this server's mail.",
            check.names.join(", ")
        )),
        PtrStatus::Unconfirmed => Some(format!(
            "Warning: the PTR for {address} names {host}, but {host} does not resolve back to {address} (reverse DNS is not forward-confirmed)."
        )),
    }
}

fn verify(host: &str, expected: &[IpAddr]) {
    let resolved: Vec<IpAddr> = (host, 0u16)
        .to_socket_addrs()
        .map(|iter| iter.map(|addr| addr.ip()).collect())
        .unwrap_or_default();

    if resolved.is_empty() {
        println!("Warning: {host} does not resolve yet — DNS may still be propagating.");
        return;
    }
    if expected.is_empty() || expected.iter().any(|address| resolved.contains(address)) {
        println!("Verified: {host} resolves to this server.");
    } else {
        println!("Warning: {host} resolves to {resolved:?}, which does not match the detected address(es).");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    use irixmail_dns::{PtrCheck, Resolver};

    fn check_with(names: &[&str], matches: bool, confirmed: bool) -> PtrCheck {
        PtrCheck {
            names: names.iter().map(|n| n.to_string()).collect(),
            matches_expected: matches,
            forward_confirmed: confirmed,
        }
    }

    #[tokio::test]
    async fn a_missing_ptr_record_warns() {
        let resolver = Resolver::empty();
        let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        let lines = ptr_report(&resolver, "mail.example.com", &[ip]).await;
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("Warning:"));
        assert!(lines[0].contains("192.0.2.1"));
        assert!(lines[0].contains("PTR"));
        assert!(lines[0].contains("mail.example.com"));
    }

    #[test]
    fn each_ptr_status_maps_to_the_right_message() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        let host = "mail.example.com";

        let missing = ptr_warning(&check_with(&[], false, false), ip, host).unwrap();
        assert!(missing.contains("no PTR record"));

        let mismatch =
            ptr_warning(&check_with(&["other.example.org"], false, false), ip, host).unwrap();
        assert!(mismatch.contains("other.example.org"));
        assert!(mismatch.contains(host));

        let unconfirmed = ptr_warning(&check_with(&[host], true, false), ip, host).unwrap();
        assert!(unconfirmed.contains("resolve back"));

        assert!(ptr_warning(&check_with(&[host], true, true), ip, host).is_none());
    }
}
