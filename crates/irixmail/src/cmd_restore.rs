use std::fs;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use irixmail_core::BootstrapConfig;

use crate::cmd_backup::copy_dir;

pub fn run(source: &Path) -> Result<()> {
    let config_file = crate::cmd_run::config_path();
    let backup_config = source.join("config.toml");
    if !config_file.exists() && backup_config.exists() {
        if let Some(parent) = config_file.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::copy(&backup_config, &config_file).context("restoring the configuration")?;
    }

    let config = BootstrapConfig::load(&config_file)
        .with_context(|| format!("loading configuration from {}", config_file.display()))?;

    restore_dir(&source.join("db"), &config.paths.db)?;
    restore_dir(&source.join("blobs"), &config.paths.blobs)?;

    println!("Restored from {}", source.display());
    Ok(())
}

fn restore_dir(source: &Path, destination: &Path) -> Result<()> {
    if !source.exists() {
        return Ok(());
    }
    if destination.exists() {
        fs::remove_dir_all(destination)
            .with_context(|| format!("clearing {}", destination.display()))?;
    }
    copy_dir(source, destination).map_err(|error| anyhow!("{error}"))
}
