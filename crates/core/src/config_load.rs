use std::path::Path;

use crate::config::{BootstrapConfig, ProtocolListener};
use crate::error::{Error, Result};

impl BootstrapConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let contents = match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::not_found(format!(
                    "bootstrap configuration file {}",
                    path.display()
                )));
            }
            Err(err) => return Err(Error::from(err)),
        };

        Self::parse(&contents)
    }

    pub fn parse(toml: &str) -> Result<Self> {
        let config: BootstrapConfig =
            toml::from_str(toml).map_err(|err| Error::config(format!("invalid TOML: {err}")))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.server.hostname.trim().is_empty() {
            return Err(Error::config("server hostname must not be empty"));
        }

        if self.listeners.bind.parse::<std::net::IpAddr>().is_err() {
            return Err(Error::config(format!(
                "listener bind address {:?} is not a valid IP address",
                self.listeners.bind
            )));
        }

        let listeners: [(&str, &ProtocolListener); 5] = [
            ("smtp", &self.listeners.smtp),
            ("submission", &self.listeners.submission),
            ("imap", &self.listeners.imap),
            ("pop3", &self.listeners.pop3),
            ("http", &self.listeners.http),
        ];

        let mut any_enabled = false;
        for (name, listener) in listeners {
            if listener.plain == Some(0) || listener.tls == Some(0) {
                return Err(Error::config(format!("listener {name} has a port of zero")));
            }
            if listener.plain.is_some() || listener.tls.is_some() {
                any_enabled = true;
            }
        }

        if !any_enabled {
            return Err(Error::config("at least one listener port must be enabled"));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LogLevel, LogTarget};
    use std::path::PathBuf;

    const FULL_FIXTURE: &str = r#"
        [paths]
        db = "/srv/irixmail/db"
        blobs = "/srv/irixmail/blobs"
        logs = "/srv/irixmail/log"

        [server]
        hostname = "mail.example.com"
        node-id = 2

        [listeners]
        bind = "127.0.0.1"

        [listeners.imap]
        plain = 143
        tls = 993

        [log]
        target = "file"
        level = "warn"
    "#;

    #[test]
    fn a_full_document_parses_and_validates() {
        let config = BootstrapConfig::parse(FULL_FIXTURE).expect("full fixture loads");

        assert_eq!(config.paths.db, PathBuf::from("/srv/irixmail/db"));
        assert_eq!(config.paths.blobs, PathBuf::from("/srv/irixmail/blobs"));
        assert_eq!(config.paths.logs, PathBuf::from("/srv/irixmail/log"));

        assert_eq!(config.server.hostname, "mail.example.com");
        assert_eq!(config.server.node_id, 2);

        assert_eq!(config.listeners.bind, "127.0.0.1");
        assert_eq!(config.listeners.imap.plain, Some(143));
        assert_eq!(config.listeners.imap.tls, Some(993));

        assert_eq!(config.log.target, LogTarget::File);
        assert_eq!(config.log.level, LogLevel::Warn);
    }

    #[test]
    fn an_empty_document_loads_as_the_default() {
        let config = BootstrapConfig::parse("").expect("empty document loads");
        assert_eq!(config, BootstrapConfig::default());
    }

    #[test]
    fn the_default_is_valid() {
        BootstrapConfig::default()
            .validate()
            .expect("the built-in default is a valid configuration");
    }

    #[test]
    fn malformed_toml_is_a_config_error() {
        let err = BootstrapConfig::parse("this is = = not toml").expect_err("malformed TOML fails");
        assert!(matches!(err, Error::Config(_)));
        assert!(err.to_string().contains("invalid TOML"));
    }

    #[test]
    fn a_blank_hostname_is_rejected() {
        let document = r#"
            [server]
            hostname = "   "
        "#;
        let err = BootstrapConfig::parse(document).expect_err("blank hostname fails");
        assert!(matches!(err, Error::Config(_)));
        assert!(err.to_string().contains("hostname"));
    }

    #[test]
    fn a_non_ip_bind_address_is_rejected() {
        let document = r#"
            [listeners]
            bind = "not-an-address"
        "#;
        let err = BootstrapConfig::parse(document).expect_err("bad bind address fails");
        assert!(matches!(err, Error::Config(_)));
        assert!(err.to_string().contains("bind address"));
    }

    #[test]
    fn a_zero_port_is_rejected() {
        let document = r#"
            [listeners.smtp]
            plain = 0
        "#;
        let err = BootstrapConfig::parse(document).expect_err("zero port fails");
        assert!(matches!(err, Error::Config(_)));
        assert!(err.to_string().contains("port of zero"));
    }

    #[test]
    fn a_configuration_with_no_listeners_is_rejected() {
        let document = r#"
            [listeners.smtp]
            [listeners.submission]
            [listeners.imap]
            [listeners.pop3]
            [listeners.http]
        "#;
        let err = BootstrapConfig::parse(document).expect_err("no listeners fails");
        assert!(matches!(err, Error::Config(_)));
        assert!(err.to_string().contains("at least one listener"));
    }

    #[test]
    fn loading_a_missing_file_reports_not_found() {
        let path = std::env::temp_dir().join("irixmail-bootstrap-does-not-exist.toml");
        let err = BootstrapConfig::load(&path).expect_err("missing file fails");
        assert!(matches!(err, Error::NotFound(_)));
    }

    #[test]
    fn loading_reads_parses_and_validates_a_file() {
        let path =
            std::env::temp_dir().join(format!("irixmail-bootstrap-{}.toml", std::process::id()));
        std::fs::write(&path, FULL_FIXTURE).expect("fixture written");

        let config = BootstrapConfig::load(&path).expect("file loads");
        assert_eq!(config.server.hostname, "mail.example.com");

        let _ = std::fs::remove_file(&path);
    }
}
