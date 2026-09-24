use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use irixmail_core::config::PathsConfig;
use irixmail_core::{BootstrapConfig, IdGenerator};
use irixmail_directory::{Directory, Role};
use irixmail_store::backup::{dir_has_entries, format_utc, read_manifest, unpack};
use irixmail_store::{RocksdbStore, Store};

use crate::cmd_backup::backup_paths;
use crate::setup::prompt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Fresh,
    Restore,
}

pub struct Restored {
    pub config: Option<BootstrapConfig>,
}

pub fn restore_choice(answer: &str) -> Option<Choice> {
    match answer.trim().to_ascii_lowercase().as_str() {
        "" | "fresh" | "f" | "1" => Some(Choice::Fresh),
        "restore" | "r" | "2" => Some(Choice::Restore),
        _ => None,
    }
}

pub fn confirmed(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

pub fn adopt_config(mut archived: BootstrapConfig, paths: &PathsConfig) -> BootstrapConfig {
    archived.paths = paths.clone();
    archived
}

pub fn prefill(existing: Option<BootstrapConfig>, restored: Option<&Restored>) -> BootstrapConfig {
    match (existing, restored.and_then(|entry| entry.config.clone())) {
        (Some(current), _) => current,
        (None, Some(archived)) => archived,
        (None, None) => BootstrapConfig::default(),
    }
}

pub fn needs_admin_step(admin_count: usize) -> bool {
    admin_count == 0
}

pub fn configure(target: &BootstrapConfig, config_file: &Path) -> Result<Option<Restored>> {
    let choice = loop {
        let answer = prompt("Fresh install or restore from a backup archive? [fresh/restore]: ")?;
        match restore_choice(&answer) {
            Some(choice) => break choice,
            None => println!("answer fresh or restore"),
        }
    };
    if choice == Choice::Fresh {
        return Ok(None);
    }
    let archive = loop {
        let answer = prompt("Path to the backup archive (.tar.gz): ")?;
        let path = PathBuf::from(answer.trim());
        if path.is_file() {
            break path;
        }
        println!("no such file: {}", path.display());
    };
    let manifest = read_manifest(&archive).map_err(|error| anyhow!("{error}"))?;
    println!(
        "Archive of {} made {} by irixmail {}",
        manifest.hostname,
        format_utc(manifest.created_at),
        manifest.version
    );
    let paths = backup_paths(target, config_file);
    if dir_has_entries(&paths.db) {
        let answer = prompt(&format!(
            "{} already contains data. Replace it with the archive? [y/N]: ",
            paths.db.display()
        ))?;
        if !confirmed(&answer) {
            anyhow::bail!("restore cancelled; the existing data was left untouched");
        }
    }
    let unpacked = unpack(&archive, &paths).map_err(|error| anyhow!("{error}"))?;
    println!(
        "Data restored into {}.",
        paths.db.parent().unwrap_or(&paths.db).display()
    );
    let config = unpacked
        .config_toml
        .map(|text| BootstrapConfig::parse(&text))
        .transpose()
        .map_err(|error| anyhow!("{error}"))?
        .map(|archived| adopt_config(archived, &target.paths));
    Ok(Some(Restored { config }))
}

pub fn restored_admins(config: &BootstrapConfig) -> Result<(usize, Option<String>)> {
    let store = Arc::new(RocksdbStore::open(&config.paths.db).map_err(|error| anyhow!("{error}"))?);
    let ids = Arc::new(IdGenerator::new(config.server.node_id));
    let directory = Directory::new(Arc::clone(&store) as Arc<dyn Store>, ids, None);
    let admins: Vec<_> = directory
        .accounts()
        .list()
        .map_err(|error| anyhow!("{error}"))?
        .into_iter()
        .filter(|account| account.role == Role::Admin && account.enabled)
        .collect();
    let first = match admins.first() {
        Some(account) => {
            let domain = directory
                .domains()
                .get(account.domain_id)
                .map_err(|error| anyhow!("{error}"))?;
            Some(format!("{}@{}", account.local_part, domain.name))
        }
        None => None,
    };
    Ok((admins.len(), first))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_prompt_defaults_to_a_fresh_install() {
        assert_eq!(restore_choice(""), Some(Choice::Fresh));
        assert_eq!(restore_choice("Restore"), Some(Choice::Restore));
        assert_eq!(restore_choice("2"), Some(Choice::Restore));
        assert_eq!(restore_choice("maybe"), None);
    }

    #[test]
    fn an_archived_config_is_adopted_with_the_host_paths() {
        let mut archived = BootstrapConfig::default();
        archived.server.hostname = "old.example.com".into();
        archived.paths.db = PathBuf::from("/srv/old/db");
        let host = PathsConfig::default();
        let adopted = adopt_config(archived, &host);
        assert_eq!(adopted.server.hostname, "old.example.com");
        assert_eq!(adopted.paths, host);
    }

    #[test]
    fn an_existing_config_wins_over_the_archive() {
        let mut existing = BootstrapConfig::default();
        existing.server.hostname = "new.example.com".into();
        let mut archived = BootstrapConfig::default();
        archived.server.hostname = "old.example.com".into();
        let restored = Restored {
            config: Some(archived),
        };
        assert_eq!(
            prefill(Some(existing), Some(&restored)).server.hostname,
            "new.example.com"
        );
        assert_eq!(
            prefill(None, Some(&restored)).server.hostname,
            "old.example.com"
        );
        assert_eq!(prefill(None, None), BootstrapConfig::default());
    }

    #[test]
    fn the_admin_step_runs_only_when_no_admin_was_restored() {
        assert!(needs_admin_step(0));
        assert!(!needs_admin_step(2));
        assert!(confirmed("Y"));
        assert!(!confirmed(""));
    }
}
