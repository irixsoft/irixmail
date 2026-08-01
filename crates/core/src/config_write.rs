use std::path::Path;

use crate::config::BootstrapConfig;
use crate::error::{Error, Result};

#[cfg(unix)]
const OWNER_ONLY_MODE: u32 = 0o600;

impl BootstrapConfig {
    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(|err| {
            Error::serialize(format!("could not encode bootstrap configuration: {err}"))
        })
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let contents = self.to_toml()?;

        let directory = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty());
        let temp_path = match directory {
            Some(parent) => parent.join(Self::temp_file_name(path)),
            None => Path::new(&Self::temp_file_name(path)).to_path_buf(),
        };

        // Tighten permissions on the temp file before the rename so the destination
        // is never momentarily world-readable.
        std::fs::write(&temp_path, contents.as_bytes())?;

        if let Err(err) = Self::restrict_permissions(&temp_path) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(err);
        }

        if let Err(err) = std::fs::rename(&temp_path, path) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(Error::from(err));
        }

        Ok(())
    }

    fn temp_file_name(path: &Path) -> String {
        let stem = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config.toml");
        format!(".{stem}.tmp")
    }

    #[cfg(unix)]
    fn restrict_permissions(path: &Path) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let permissions = std::fs::Permissions::from_mode(OWNER_ONLY_MODE);
        std::fs::set_permissions(path, permissions)?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn restrict_permissions(_path: &Path) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LogLevel, LogTarget};

    fn temp_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "irixmail-config-write-{}-{}-{:?}.toml",
            label,
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn rendered_toml_round_trips_through_the_loader() {
        let mut config = BootstrapConfig::default();
        config.server.hostname = "mail.example.com".to_string();
        config.server.node_id = 5;
        config.log.target = LogTarget::File;
        config.log.level = LogLevel::Warn;

        let rendered = config.to_toml().expect("configuration renders to TOML");
        let parsed = BootstrapConfig::parse(&rendered).expect("rendered TOML loads back");
        assert_eq!(parsed, config);
    }

    #[test]
    fn saving_then_loading_returns_an_equal_configuration() {
        let path = temp_path("round-trip");

        let mut config = BootstrapConfig::default();
        config.server.hostname = "mail.example.org".to_string();

        config.save(&path).expect("configuration saves");
        let loaded = BootstrapConfig::load(&path).expect("saved configuration loads");
        assert_eq!(loaded, config);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn saving_overwrites_an_existing_file() {
        let path = temp_path("overwrite");

        BootstrapConfig::default()
            .save(&path)
            .expect("first configuration saves");

        let mut replacement = BootstrapConfig::default();
        replacement.server.hostname = "replaced.example.com".to_string();
        replacement
            .save(&path)
            .expect("replacement configuration saves");

        let loaded = BootstrapConfig::load(&path).expect("replacement loads");
        assert_eq!(loaded.server.hostname, "replaced.example.com");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn saving_leaves_no_temporary_file_behind() {
        let path = temp_path("no-temp");

        BootstrapConfig::default()
            .save(&path)
            .expect("configuration saves");

        let temp = path
            .parent()
            .expect("temporary path has a parent")
            .join(BootstrapConfig::temp_file_name(&path));
        assert!(!temp.exists(), "temporary file should be renamed away");

        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn a_saved_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let path = temp_path("perms");
        BootstrapConfig::default()
            .save(&path)
            .expect("configuration saves");

        let mode = std::fs::metadata(&path)
            .expect("saved file has metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, OWNER_ONLY_MODE);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn saving_into_a_missing_directory_is_an_io_error() {
        let path = std::env::temp_dir()
            .join("irixmail-config-write-missing-dir")
            .join("config.toml");

        let err = BootstrapConfig::default()
            .save(&path)
            .expect_err("saving into a missing directory fails");
        assert!(matches!(err, Error::Io(_)));
    }
}
