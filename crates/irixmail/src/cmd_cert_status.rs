use anyhow::{anyhow, Context, Result};
use irixmail_core::BootstrapConfig;
use irixmail_tls::CertStore;

use crate::setup_cert::certs_dir;

pub fn run() -> Result<()> {
    let config_file = crate::cmd_run::config_path();
    let config = BootstrapConfig::load(&config_file)
        .with_context(|| format!("loading configuration from {}", config_file.display()))?;

    let directory = certs_dir(&config);
    let store = CertStore::new(directory.clone());
    match store
        .load(&config.server.hostname)
        .map_err(|error| anyhow!("{error}"))?
    {
        Some(material) => {
            println!(
                "Certificate present for {} ({} certificate(s) in the chain).",
                config.server.hostname,
                material.chain.len()
            );
            println!("Stored in {}", directory.display());
        }
        None => {
            println!("No certificate found for {}.", config.server.hostname);
            println!("Run `irixmail cert reissue` or `irixmail setup` to obtain one.");
        }
    }
    Ok(())
}
