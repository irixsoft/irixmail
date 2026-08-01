use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use irixmail_core::{BootstrapConfig, IdGenerator};
use irixmail_directory::{password, Directory, Role};
use irixmail_store::{RocksdbStore, Store};

use crate::setup::prompt;

const ADMIN_DISPLAY_NAME: &str = "Administrator";

pub fn configure(config: &BootstrapConfig) -> Result<String> {
    let email = prompt("Admin email (e.g. you@example.com): ")?;
    let (local, domain_name) = email
        .rsplit_once('@')
        .ok_or_else(|| anyhow!("the admin email must include a domain"))?;

    let store = Arc::new(RocksdbStore::open(&config.paths.db).map_err(|error| anyhow!("{error}"))?);
    let ids = Arc::new(IdGenerator::new(config.server.node_id));
    let directory = Directory::new(Arc::clone(&store) as Arc<dyn Store>, ids, None);

    let domain = match directory
        .domains()
        .get_by_name(domain_name)
        .map_err(|error| anyhow!("{error}"))?
    {
        Some(existing) => existing,
        None => directory
            .domains()
            .create(domain_name, Vec::new())
            .map_err(|error| anyhow!("{error}"))?,
    };
    if directory
        .accounts()
        .get_by_address(local, domain.id)
        .map_err(|error| anyhow!("{error}"))?
        .is_some()
    {
        println!(
            "Admin account {local}@{domain_name} already exists — keeping it (use `irixmail admin reset-password` to change the password)."
        );
        return Ok(email);
    }

    let secret = rpassword::prompt_password("Admin password: ").context("reading the password")?;
    if secret.is_empty() {
        return Err(anyhow!("the password cannot be empty"));
    }
    let account = directory
        .accounts()
        .create(local, domain.id, ADMIN_DISPLAY_NAME, Role::Admin)
        .map_err(|error| anyhow!("{error}"))?;
    let hash = password::hash(&secret).map_err(|error| anyhow!("{error}"))?;
    directory
        .credentials()
        .set_primary_password(account.id, hash)
        .map_err(|error| anyhow!("{error}"))?;

    println!("Admin account {local}@{domain_name} created.");
    Ok(email)
}
