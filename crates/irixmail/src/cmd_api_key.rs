use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use irixmail_core::{BootstrapConfig, IdGenerator};
use irixmail_directory::{ApiKey, Directory, SecretCipher};
use irixmail_store::{RocksdbStore, Store};

pub fn create(name: &str) -> Result<()> {
    let plaintext = create_with(&load_config()?, name)?;
    println!("API key \"{name}\" created:");
    println!("  {plaintext}");
    println!("Store it now — it will not be shown again.");
    Ok(())
}

pub fn list() -> Result<()> {
    let keys = list_with(&load_config()?)?;
    if keys.is_empty() {
        println!("No API keys.");
        return Ok(());
    }
    for key in keys {
        println!("{}  {}  created-at-ms {}", key.id, key.name, key.created_at);
    }
    Ok(())
}

pub fn revoke(id: &str) -> Result<()> {
    let id: u64 = id.parse().context("the API key id must be a number")?;
    if revoke_with(&load_config()?, id)? {
        println!("API key {id} revoked.");
    } else {
        println!("No API key with id {id}.");
    }
    Ok(())
}

pub fn create_with(config: &BootstrapConfig, name: &str) -> Result<String> {
    let directory = open_directory(config)?;
    let secrets = load_secrets(config)?;
    let (_, plaintext) = directory
        .api_keys()
        .create(name, &secrets)
        .map_err(|error| anyhow!("{error}"))?;
    Ok(plaintext)
}

pub fn list_with(config: &BootstrapConfig) -> Result<Vec<ApiKey>> {
    let directory = open_directory(config)?;
    directory
        .api_keys()
        .list()
        .map_err(|error| anyhow!("{error}"))
}

pub fn revoke_with(config: &BootstrapConfig, id: u64) -> Result<bool> {
    let directory = open_directory(config)?;
    directory
        .api_keys()
        .revoke(id)
        .map_err(|error| anyhow!("{error}"))
}

fn load_config() -> Result<BootstrapConfig> {
    let path = crate::cmd_run::config_path();
    BootstrapConfig::load(&path)
        .with_context(|| format!("loading configuration from {}", path.display()))
}

fn open_directory(config: &BootstrapConfig) -> Result<Directory> {
    let store = Arc::new(RocksdbStore::open(&config.paths.db).map_err(|error| anyhow!("{error}"))?);
    let ids = Arc::new(IdGenerator::new(config.server.node_id));
    Ok(Directory::new(store as Arc<dyn Store>, ids, None))
}

fn load_secrets(config: &BootstrapConfig) -> Result<SecretCipher> {
    SecretCipher::load_or_create(&config.paths.secret_key).map_err(|error| anyhow!("{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_config() -> (BootstrapConfig, std::path::PathBuf) {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let base =
            std::env::temp_dir().join(format!("irixmail-apikey-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let mut config = BootstrapConfig::default();
        config.paths.db = base.join("db");
        config.paths.secret_key = base.join("credential.key");
        (config, base)
    }

    #[test]
    fn create_persists_a_key_that_authenticates() {
        let (config, base) = temp_config();
        let plaintext = create_with(&config, "ci").unwrap();

        let listed = list_with(&config).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "ci");

        let directory = open_directory(&config).unwrap();
        let secrets = load_secrets(&config).unwrap();
        let found = directory.api_keys().verify(&plaintext, &secrets).unwrap();
        assert_eq!(found.map(|key| key.id), Some(listed[0].id));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn revoke_removes_the_key_for_good() {
        let (config, base) = temp_config();
        let plaintext = create_with(&config, "ci").unwrap();
        let id = list_with(&config).unwrap()[0].id;

        assert!(revoke_with(&config, id).unwrap());
        assert!(list_with(&config).unwrap().is_empty());
        assert!(!revoke_with(&config, id).unwrap());

        let directory = open_directory(&config).unwrap();
        let secrets = load_secrets(&config).unwrap();
        assert!(directory
            .api_keys()
            .verify(&plaintext, &secrets)
            .unwrap()
            .is_none());
        let _ = std::fs::remove_dir_all(&base);
    }
}
