use anyhow::{anyhow, Result};
use irixmail_core::BootstrapConfig;
use irixmail_tls::{self_signed, CertSource, CertStore};

use crate::setup_cert::certs_dir;

pub fn configure(config: &BootstrapConfig, cert_issued: bool) -> Result<()> {
    if !cert_issued {
        let material = self_signed::generate(vec![config.server.hostname.clone()])
            .map_err(|error| anyhow!("{error}"))?;
        CertStore::new(certs_dir(config))
            .save(&config.server.hostname, &material, CertSource::SelfSigned)
            .map_err(|error| anyhow!("{error}"))?;
        println!(
            "\nA self-signed certificate is in place — your browser will warn until a real certificate is issued."
        );
    }

    let host = &config.server.hostname;
    let port = config.listeners.http.tls.unwrap_or(443);
    let url = if port == 443 {
        format!("https://{host}/admin/")
    } else {
        format!("https://{host}:{port}/admin/")
    };

    println!("\nSetup complete.");
    println!("Admin panel: {url}");
    println!("Sign in with the admin account you just created.");
    Ok(())
}
