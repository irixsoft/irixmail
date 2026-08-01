use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use irixmail_core::{BootstrapConfig, IdGenerator};
use irixmail_directory::{password, Directory};
use irixmail_store::{RocksdbStore, Store};

pub fn run(email: &str) -> Result<()> {
    let path = crate::cmd_run::config_path();
    let config = BootstrapConfig::load(&path)
        .with_context(|| format!("loading configuration from {}", path.display()))?;

    let (local, domain_name) = email
        .rsplit_once('@')
        .ok_or_else(|| anyhow!("the email must include a domain"))?;

    let store = Arc::new(RocksdbStore::open(&config.paths.db).map_err(|error| anyhow!("{error}"))?);
    let ids = Arc::new(IdGenerator::new(config.server.node_id));
    let directory = Directory::new(Arc::clone(&store) as Arc<dyn Store>, ids, None);

    let domain = directory
        .domains()
        .get_by_name(domain_name)
        .map_err(|error| anyhow!("{error}"))?
        .ok_or_else(|| anyhow!("no such domain: {domain_name}"))?;
    let account = directory
        .accounts()
        .get_by_address(local, domain.id)
        .map_err(|error| anyhow!("{error}"))?
        .ok_or_else(|| anyhow!("no such account: {email}"))?;

    let secret = rpassword::prompt_password("New password: ").context("reading the password")?;
    if secret.is_empty() {
        return Err(anyhow!("the password cannot be empty"));
    }
    let hash = password::hash(&secret).map_err(|error| anyhow!("{error}"))?;
    directory
        .credentials()
        .set_primary_password(account.id, hash)
        .map_err(|error| anyhow!("{error}"))?;

    println!("Password reset for {email}.");
    Ok(())
}
