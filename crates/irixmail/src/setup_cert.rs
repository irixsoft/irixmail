use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use axum::extract::Path;
use axum::routing::get;
use axum::Router;
use irixmail_core::BootstrapConfig;
use irixmail_tls::acme_account::{load_or_create, production_directory};
use irixmail_tls::{
    issue_with_retry, AcmePersist, CertSource, CertStore, Http01Challenges, IssueRequest,
    RetryPolicy,
};

use crate::setup::prompt;

pub fn certs_dir(config: &BootstrapConfig) -> PathBuf {
    config
        .paths
        .db
        .parent()
        .map(|parent| parent.join("certs"))
        .unwrap_or_else(|| PathBuf::from("/var/lib/irixmail/certs"))
}

pub fn configure(config: &BootstrapConfig, admin_email: &str) -> Result<bool> {
    let answer = prompt("Obtain a Let's Encrypt certificate now? [y/N]: ")?;
    if !matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes") {
        println!(
            "Skipping ACME issuance — a self-signed certificate will be used until you run `irixmail cert reissue`."
        );
        return Ok(false);
    }
    obtain(config, Some(admin_email))
}

pub fn obtain(config: &BootstrapConfig, contact: Option<&str>) -> Result<bool> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building the tokio runtime")?;
    runtime.block_on(issue_certificate(config, contact))
}

async fn issue_certificate(config: &BootstrapConfig, contact: Option<&str>) -> Result<bool> {
    let port = config.listeners.http.plain.unwrap_or(80);
    let expected = irixmail_dns::public_ip::detect_all().await;
    match irixmail_dns::Resolver::from_system() {
        Ok(resolver) => {
            match irixmail_tls::preflight(&resolver, &config.server.hostname, &expected, port).await
            {
                Ok(report) => {
                    if let Some(reason) =
                        preflight_block_reason(&report, &config.server.hostname, port, &expected)
                    {
                        println!("Certificate preflight failed: {reason}");
                        println!(
                            "Re-run `irixmail setup` once this is fixed, or issue the certificate later from the admin panel."
                        );
                        return Ok(false);
                    }
                    if let Some(note) = preflight_note(&report, &config.server.hostname, &expected)
                    {
                        println!("{note}");
                    }
                }
                Err(error) => {
                    println!("Certificate preflight could not run ({error}); attempting issuance anyway.");
                }
            }
        }
        Err(error) => {
            println!("Certificate preflight could not run ({error}); attempting issuance anyway.");
        }
    }

    let challenges = Arc::new(Http01Challenges::new());
    let address = format!("{}:{port}", config.listeners.bind);
    let listener = tokio::net::TcpListener::bind(&address)
        .await
        .with_context(|| format!("binding the ACME challenge listener on {address}"))?;

    let responder = Arc::clone(&challenges);
    let app = Router::new().route(
        "/.well-known/acme-challenge/{token}",
        get(move |Path(token): Path<String>| {
            let responder = Arc::clone(&responder);
            async move {
                responder
                    .get(&token, irixmail_tls::acme_http01::unix_now())
                    .unwrap_or_default()
            }
        }),
    );
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let persist = AcmePersist::new(certs_dir(config));
    let account = load_or_create(&persist, production_directory(), contact)
        .await
        .map_err(|error| anyhow!("{error}"))?;
    let request = IssueRequest {
        account: &account,
        domains: vec![config.server.hostname.clone()],
        http01: &challenges,
    };
    let result = issue_with_retry(&request, &RetryPolicy::default()).await;
    server.abort();

    match result {
        Ok(material) => {
            CertStore::new(certs_dir(config))
                .save(&config.server.hostname, &material, CertSource::Acme)
                .map_err(|error| anyhow!("{error}"))?;
            println!("Certificate issued for {}.", config.server.hostname);
            Ok(true)
        }
        Err(error) => {
            println!("Certificate issuance failed: {error}");
            println!(
                "Re-run `irixmail setup` once DNS has propagated and port {port} is reachable, or issue it later from the admin panel."
            );
            Ok(false)
        }
    }
}

fn preflight_block_reason(
    report: &irixmail_tls::PreflightReport,
    hostname: &str,
    port: u16,
    expected: &[std::net::IpAddr],
) -> Option<String> {
    if report.resolved.is_empty() {
        return Some(format!(
            "{hostname} does not resolve to any address; publish its A/AAAA record before requesting a certificate"
        ));
    }
    if !report.challenge_port_bindable {
        return Some(format!(
            "port {port} is not bindable on this machine, so the ACME HTTP-01 challenge cannot be answered; stop whatever is listening on it and retry"
        ));
    }
    if !expected.is_empty() && !report.hostname_matches {
        let resolved = join_ips(&report.resolved);
        let detected = join_ips(expected);
        return Some(format!(
            "{hostname} resolves to {resolved} but this server's detected public address is {detected}; fix the A/AAAA record before requesting a certificate"
        ));
    }
    None
}

fn preflight_note(
    report: &irixmail_tls::PreflightReport,
    hostname: &str,
    expected: &[std::net::IpAddr],
) -> Option<String> {
    if expected.is_empty() || report.resolved.is_empty() || !report.hostname_matches {
        return None;
    }
    if report.resolved.iter().any(|ip| expected.contains(ip)) {
        return None;
    }
    let resolved = join_ips(&report.resolved);
    let detected = join_ips(expected);
    Some(format!(
        "Could not confirm that {hostname} points at this server ({hostname} resolves to {resolved}; detected here: {detected}); continuing with issuance."
    ))
}

fn join_ips(ips: &[std::net::IpAddr]) -> String {
    ips.iter()
        .map(|ip| ip.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use irixmail_tls::PreflightReport;
    use std::net::{IpAddr, Ipv4Addr};

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn report(resolved: Vec<IpAddr>, matches: bool, bindable: bool) -> PreflightReport {
        PreflightReport {
            resolved,
            hostname_matches: matches,
            challenge_port_bindable: bindable,
        }
    }

    #[test]
    fn an_unresolvable_hostname_blocks_issuance() {
        let reason = preflight_block_reason(
            &report(vec![], false, true),
            "mail.test",
            80,
            &[ip(5, 6, 7, 8)],
        )
        .expect("blocked");
        assert!(reason.contains("mail.test"));
        assert!(reason.contains("resolve"));
    }

    #[test]
    fn a_dns_mismatch_blocks_issuance_naming_both_addresses() {
        let reason = preflight_block_reason(
            &report(vec![ip(1, 2, 3, 4)], false, true),
            "mail.test",
            80,
            &[ip(5, 6, 7, 8)],
        )
        .expect("blocked");
        assert!(reason.contains("1.2.3.4"));
        assert!(reason.contains("5.6.7.8"));
    }

    #[test]
    fn an_unbindable_challenge_port_blocks_issuance_naming_the_port() {
        let reason = preflight_block_reason(
            &report(vec![ip(5, 6, 7, 8)], true, false),
            "mail.test",
            80,
            &[ip(5, 6, 7, 8)],
        )
        .expect("blocked");
        assert!(reason.contains("80"));
    }

    #[test]
    fn a_ready_report_does_not_block() {
        let report = report(vec![ip(5, 6, 7, 8)], true, true);
        assert!(preflight_block_reason(&report, "mail.test", 80, &[ip(5, 6, 7, 8)]).is_none());
    }

    #[test]
    fn an_undetectable_public_ip_does_not_block_a_resolvable_host() {
        let report = report(vec![ip(5, 6, 7, 8)], false, true);
        assert!(preflight_block_reason(&report, "mail.test", 80, &[]).is_none());
    }

    #[test]
    fn a_family_disjoint_pass_gets_an_unconfirmed_note() {
        let v6: IpAddr = "2001:db8::7".parse().unwrap();
        let report = report(vec![ip(1, 2, 3, 4)], true, true);
        let note = preflight_note(&report, "mail.test", &[v6]).expect("note");
        assert!(note.contains("1.2.3.4"));
        assert!(note.contains("2001:db8::7"));
    }

    #[test]
    fn an_exact_match_needs_no_note() {
        let report = report(vec![ip(5, 6, 7, 8)], true, true);
        assert!(preflight_note(&report, "mail.test", &[ip(5, 6, 7, 8)]).is_none());
    }
}
