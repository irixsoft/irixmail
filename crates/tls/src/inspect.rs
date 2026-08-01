use x509_parser::prelude::{FromDer, GeneralName, X509Certificate};

use crate::cert_store::CertMaterial;

pub struct CertSummary {
    pub sans: Vec<String>,
    pub not_after: i64,
    pub issuer: String,
    pub self_signed: bool,
}

pub fn inspect(material: &CertMaterial) -> Option<CertSummary> {
    let leaf = material.chain.first()?;
    let (_, cert) = X509Certificate::from_der(leaf.as_ref()).ok()?;
    let sans = match cert.subject_alternative_name().ok().flatten() {
        Some(ext) => ext
            .value
            .general_names
            .iter()
            .filter_map(|name| match name {
                GeneralName::DNSName(dns) => Some((*dns).to_string()),
                _ => None,
            })
            .collect(),
        None => Vec::new(),
    };
    Some(CertSummary {
        sans,
        not_after: cert.validity().not_after.timestamp(),
        issuer: cert.issuer().to_string(),
        self_signed: cert.issuer().to_string() == cert.subject().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::PrivateKeyDer;

    fn self_signed(hostname: &str) -> CertMaterial {
        let certified = rcgen::generate_simple_self_signed(vec![hostname.to_string()]).unwrap();
        CertMaterial {
            chain: vec![certified.cert.der().clone()],
            key: PrivateKeyDer::Pkcs8(certified.key_pair.serialize_der().into()),
        }
    }

    #[test]
    fn a_self_signed_certificate_reports_its_san_and_expiry() {
        let summary = inspect(&self_signed("mail.irix.example")).expect("a parsed summary");
        assert!(summary.sans.contains(&"mail.irix.example".to_string()));
        assert!(summary.not_after > 0);
        assert!(summary.self_signed);
    }
}
