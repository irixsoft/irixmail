use rustls::pki_types::PrivateKeyDer;

use irixmail_core::{Error, Result};

use crate::cert_store::CertMaterial;

pub fn generate(hostnames: Vec<String>) -> Result<CertMaterial> {
    let certified = rcgen::generate_simple_self_signed(hostnames).map_err(|err| {
        Error::internal(format!(
            "could not generate a self-signed certificate: {err}"
        ))
    })?;
    Ok(CertMaterial {
        chain: vec![certified.cert.der().clone()],
        key: PrivateKeyDer::Pkcs8(certified.key_pair.serialize_der().into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_self_signed_certificate_is_produced_for_the_hostnames() {
        let material = generate(vec!["mail.example.com".to_string()]).unwrap();
        assert_eq!(material.chain.len(), 1);
        assert!(!material.chain[0].as_ref().is_empty());
        assert!(!material.key.secret_der().is_empty());
    }
}
