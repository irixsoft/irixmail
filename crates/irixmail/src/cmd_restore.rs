use std::path::Path;

use anyhow::{anyhow, Context, Result};
use irixmail_core::BootstrapConfig;
use irixmail_store::backup::{format_utc, read_manifest, unpack};

use crate::cmd_backup::backup_paths;
use crate::setup_restore::adopt_config;
use crate::systemd::{service_active, systemctl, RestartOnDrop};

pub fn run(archive: &Path) -> Result<()> {
    let config_file = crate::cmd_run::config_path();
    let existing = match BootstrapConfig::load(&config_file) {
        Ok(config) => Some(config),
        Err(irixmail_core::Error::NotFound(_)) => None,
        Err(error) => {
            return Err(anyhow!("{error}"))
                .with_context(|| format!("loading configuration from {}", config_file.display()))
        }
    };
    let config = existing.clone().unwrap_or_default();
    let manifest = read_manifest(archive).map_err(|error| anyhow!("{error}"))?;
    println!(
        "Archive of {} made {} by irixmail {}",
        manifest.hostname,
        format_utc(manifest.created_at),
        manifest.version
    );

    let _restart = if service_active() {
        if !crate::ownership::running_as_root() {
            anyhow::bail!(
                "the irixmail service is running; re-run as root so it can be stopped: sudo irixmail restore {}",
                archive.display()
            );
        }
        println!("Stopping the irixmail service while the archive is restored.");
        if !systemctl(&["stop", "irixmail"]) {
            anyhow::bail!("could not stop the irixmail service");
        }
        Some(RestartOnDrop)
    } else {
        None
    };

    let paths = backup_paths(&config, &config_file);
    let unpacked = unpack(archive, &paths).map_err(|error| anyhow!("{error}"))?;
    if existing.is_none() {
        if let Some(text) = unpacked.config_toml {
            let restored = BootstrapConfig::parse(&text).map_err(|error| anyhow!("{error}"))?;
            adopt_config(restored, &config.paths)
                .save(&config_file)
                .map_err(|error| anyhow!("{error}"))?;
            println!("Configuration restored to {}", config_file.display());
        }
    }
    crate::ownership::ensure_service_ownership(&config, &config_file)?;
    println!("Restored {} from {}", manifest.hostname, archive.display());
    Ok(())
}
