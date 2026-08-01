use std::fs;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use irixmail_core::BootstrapConfig;
use irixmail_store::RocksdbStore;

pub fn run(destination: &Path) -> Result<()> {
    let config_file = crate::cmd_run::config_path();
    let config = BootstrapConfig::load(&config_file)
        .with_context(|| format!("loading configuration from {}", config_file.display()))?;

    fs::create_dir_all(destination)
        .with_context(|| format!("creating {}", destination.display()))?;

    let store = RocksdbStore::open(&config.paths.db).map_err(|error| anyhow!("{error}"))?;
    store
        .checkpoint(destination.join("db"))
        .map_err(|error| anyhow!("{error}"))?;

    copy_dir(&config.paths.blobs, &destination.join("blobs"))?;
    if config_file.exists() {
        fs::copy(&config_file, destination.join("config.toml"))
            .context("copying the configuration into the backup")?;
    }

    println!("Backup written to {}", destination.display());
    Ok(())
}

pub fn copy_dir(source: &Path, destination: &Path) -> Result<()> {
    if !source.exists() {
        return Ok(());
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
