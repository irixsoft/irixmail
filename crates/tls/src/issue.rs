use std::time::Duration;

use instant_acme::{AuthorizationStatus, ChallengeType, Identifier, NewOrder, Order, OrderStatus};
use rcgen::{CertificateParams, DistinguishedName, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::time::sleep;

use irixmail_core::{Error, Result};

use crate::acme_account::AcmeAccount;
use crate::acme_http01::Http01Challenges;
use crate::cert_store::CertMaterial;

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_POLLS: usize = 30;

pub struct IssueRequest<'a> {
    pub account: &'a AcmeAccount,
    pub domains: Vec<String>,
    pub http01: &'a Http01Challenges,
}

pub async fn issue(request: &IssueRequest<'_>) -> Result<CertMaterial> {
    let mut tokens = Vec::new();
    let result = issue_order(request, &mut tokens).await;
    cleanup(request.http01, &tokens);
    result
}

async fn issue_order(request: &IssueRequest<'_>, tokens: &mut Vec<String>) -> Result<CertMaterial> {
    if request.domains.is_empty() {
        return Err(Error::invalid_input(
            "issuance requires at least one domain",
        ));
    }
    let identifiers: Vec<Identifier> = request
        .domains
        .iter()
        .cloned()
        .map(Identifier::Dns)
        .collect();
    let account = request.account.account();

    let mut order = account
        .new_order(&NewOrder {
            identifiers: &identifiers,
        })
        .await
        .map_err(|err| Error::internal(format!("could not create the ACME order: {err}")))?;

    let authorizations = order
        .authorizations()
        .await
        .map_err(|err| Error::internal(format!("could not fetch ACME authorizations: {err}")))?;

    for authz in &authorizations {
        if authz.status == AuthorizationStatus::Valid {
            continue;
        }
        let challenge = authz
            .challenges
            .iter()
            .find(|challenge| challenge.r#type == ChallengeType::Http01)
            .ok_or_else(|| Error::internal("the CA did not offer an HTTP-01 challenge"))?;

        let key_auth = order.key_authorization(challenge);
        request.http01.insert(
            challenge.token.clone(),
            key_auth.as_str().to_string(),
            crate::acme_http01::unix_now(),
        );
        tokens.push(challenge.token.clone());

        order
            .set_challenge_ready(&challenge.url)
            .await
            .map_err(|err| {
                Error::internal(format!("could not signal challenge readiness: {err}"))
            })?;
    }

    let status = wait_for(&mut order, |status| {
        matches!(status, OrderStatus::Ready | OrderStatus::Invalid)
    })
    .await?;
    if status == OrderStatus::Invalid {
        return Err(Error::internal("the ACME order was rejected"));
    }

    let key_pair = KeyPair::generate()
        .map_err(|err| Error::internal(format!("could not generate the certificate key: {err}")))?;
    let csr = csr_params(request.domains.clone())?
        .serialize_request(&key_pair)
        .map_err(|err| Error::internal(format!("could not serialize the CSR: {err}")))?;
    order
        .finalize(csr.der().as_ref())
        .await
        .map_err(|err| Error::internal(format!("could not finalize the ACME order: {err}")))?;

    let chain_pem = fetch_certificate(&mut order).await?;

    Ok(CertMaterial {
        chain: parse_chain(&chain_pem)?,
        key: PrivateKeyDer::Pkcs8(key_pair.serialize_der().into()),
    })
}

fn csr_params(domains: Vec<String>) -> Result<CertificateParams> {
    let mut params = CertificateParams::new(domains)
        .map_err(|err| Error::internal(format!("could not build the CSR: {err}")))?;
    params.distinguished_name = DistinguishedName::new();
    Ok(params)
}

async fn wait_for(order: &mut Order, ready: impl Fn(OrderStatus) -> bool) -> Result<OrderStatus> {
    for _ in 0..MAX_POLLS {
        let status = order
            .refresh()
            .await
            .map_err(|err| Error::internal(format!("could not refresh the ACME order: {err}")))?
            .status;
        if ready(status) {
            return Ok(status);
        }
        sleep(POLL_INTERVAL).await;
    }
    Err(Error::internal(
        "timed out waiting for the ACME order to advance",
    ))
}

async fn fetch_certificate(order: &mut Order) -> Result<String> {
    for _ in 0..MAX_POLLS {
        if let Some(pem) = order.certificate().await.map_err(|err| {
            Error::internal(format!("could not fetch the issued certificate: {err}"))
        })? {
            return Ok(pem);
        }
        sleep(POLL_INTERVAL).await;
    }
    Err(Error::internal(
        "timed out waiting for the issued certificate",
    ))
}

fn cleanup(http01: &Http01Challenges, tokens: &[String]) {
    for token in tokens {
        http01.remove(token);
    }
}

fn parse_chain(pem: &str) -> Result<Vec<CertificateDer<'static>>> {
    let mut reader = std::io::BufReader::new(pem.as_bytes());
    let chain = rustls_pemfile::certs(&mut reader).collect::<std::result::Result<Vec<_>, _>>()?;
    if chain.is_empty() {
        return Err(Error::internal("the issued certificate chain was empty"));
    }
    Ok(chain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pem_chain_parses_into_certificates() {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let chain = parse_chain(&certified.cert.pem()).unwrap();
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn an_empty_chain_is_rejected() {
        assert!(parse_chain("").is_err());
    }

    #[test]
    fn the_csr_has_an_empty_subject_and_the_domains_as_sans() {
        let params = csr_params(vec![
            "mail.example.com".to_string(),
            "example.com".to_string(),
        ])
        .unwrap();
        assert_eq!(params.distinguished_name.iter().count(), 0);
        let sans: Vec<&str> = params
            .subject_alt_names
            .iter()
            .map(|san| match san {
                rcgen::SanType::DnsName(name) => name.as_str(),
                other => panic!("unexpected SAN: {other:?}"),
            })
            .collect();
        assert_eq!(sans, ["mail.example.com", "example.com"]);
    }
}
