use std::fs::{self, File};
use std::io::BufWriter;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use irixmail_core::BootstrapConfig;
use irixmail_store::backup::write_archive;
use irixmail_store::{BackupPaths, RocksdbStore};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn backup_paths(config: &BootstrapConfig, config_file: &Path) -> BackupPaths {
    BackupPaths {
        config_file: config_file.to_path_buf(),
        db: config.paths.db.clone(),
        blobs: config.paths.blobs.clone(),
        secret_key: config.paths.secret_key.clone(),
        certs: crate::setup_cert::certs_dir(config),
    }
}

pub fn run(destination: &Path) -> Result<()> {
    let config_file = crate::cmd_run::config_path();
    let config = BootstrapConfig::load(&config_file)
        .with_context(|| format!("loading configuration from {}", config_file.display()))?;
    if let Some(parent) = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let store = RocksdbStore::open(&config.paths.db).map_err(|error| {
        anyhow!("{error}").context(
            "opening the store; while the irixmail service runs, download the backup from the admin panel instead",
        )
    })?;
    let file =
        File::create(destination).with_context(|| format!("creating {}", destination.display()))?;
    let paths = backup_paths(&config, &config_file);
    let manifest = write_archive(
        &paths,
        &store,
        &config.server.hostname,
        VERSION,
        BufWriter::new(file),
    )
    .map_err(|error| anyhow!("{error}"))?;
    println!(
        "Backup of {} written to {}",
        manifest.hostname,
        destination.display()
    );
    Ok(())
}
