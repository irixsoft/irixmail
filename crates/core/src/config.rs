use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const DEFAULT_DB_PATH: &str = "/var/lib/irixmail/db";
const DEFAULT_BLOB_PATH: &str = "/var/lib/irixmail/blobs";
const DEFAULT_LOG_PATH: &str = "/var/log/irixmail";
const DEFAULT_SECRET_KEY_PATH: &str = "/var/lib/irixmail/credential.key";
const DEFAULT_HOSTNAME: &str = "localhost";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct BootstrapConfig {
    pub paths: PathsConfig,
    pub server: ServerConfig,
    pub listeners: ListenersConfig,
    pub log: LogConfig,
    pub relay: Option<RelayConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct RelayConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub implicit_tls: bool,
    pub require_tls: bool,
    pub accept_invalid_certs: bool,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 587,
            username: None,
            password: None,
            implicit_tls: false,
            require_tls: false,
            accept_invalid_certs: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct PathsConfig {
    pub db: PathBuf,
    pub blobs: PathBuf,
    pub logs: PathBuf,
    pub secret_key: PathBuf,
}

impl Default for PathsConfig {
    fn default() -> Self {
        Self {
            db: PathBuf::from(DEFAULT_DB_PATH),
            blobs: PathBuf::from(DEFAULT_BLOB_PATH),
            logs: PathBuf::from(DEFAULT_LOG_PATH),
            secret_key: PathBuf::from(DEFAULT_SECRET_KEY_PATH),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ServerConfig {
    pub hostname: String,
    pub node_id: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            hostname: DEFAULT_HOSTNAME.to_string(),
            node_id: 0,
        }
    }
}

const DEFAULT_BIND_ADDRESS: &str = "0.0.0.0";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ProtocolListener {
    pub plain: Option<u16>,
    pub tls: Option<u16>,
}

impl ProtocolListener {
    fn plain(port: u16) -> Self {
        Self {
            plain: Some(port),
            tls: None,
        }
    }

    fn both(plain: u16, tls: u16) -> Self {
        Self {
            plain: Some(plain),
            tls: Some(tls),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ListenersConfig {
    pub bind: String,
    pub smtp: ProtocolListener,
    pub submission: ProtocolListener,
    pub imap: ProtocolListener,
    pub pop3: ProtocolListener,
    pub managesieve: ProtocolListener,
    pub http: ProtocolListener,
}

impl Default for ListenersConfig {
    fn default() -> Self {
        Self {
            bind: DEFAULT_BIND_ADDRESS.to_string(),
            smtp: ProtocolListener::plain(25),
            submission: ProtocolListener::both(587, 465),
            imap: ProtocolListener::both(143, 993),
            pop3: ProtocolListener::both(110, 995),
            managesieve: ProtocolListener::plain(4190),
            http: ProtocolListener::both(80, 443),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogTarget {
    #[default]
    Journald,
    File,
    Stderr,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct LogConfig {
    pub target: LogTarget,
    pub level: LogLevel,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_documented_layout_and_ports() {
        let config = BootstrapConfig::default();

        assert_eq!(config.paths.db, PathBuf::from("/var/lib/irixmail/db"));
        assert_eq!(config.paths.blobs, PathBuf::from("/var/lib/irixmail/blobs"));
        assert_eq!(config.paths.logs, PathBuf::from("/var/log/irixmail"));
        assert_eq!(
            config.paths.secret_key,
            PathBuf::from("/var/lib/irixmail/credential.key")
        );

        assert_eq!(config.server.hostname, "localhost");
        assert_eq!(config.server.node_id, 0);

        assert_eq!(config.listeners.bind, "0.0.0.0");
        assert_eq!(config.listeners.smtp, ProtocolListener::plain(25));
        assert_eq!(
            config.listeners.submission,
            ProtocolListener::both(587, 465)
        );
        assert_eq!(config.listeners.imap, ProtocolListener::both(143, 993));
        assert_eq!(config.listeners.pop3, ProtocolListener::both(110, 995));
        assert_eq!(config.listeners.managesieve, ProtocolListener::plain(4190));
        assert_eq!(config.listeners.http, ProtocolListener::both(80, 443));

        assert_eq!(config.log.target, LogTarget::Journald);
        assert_eq!(config.log.level, LogLevel::Info);
    }

    #[test]
    fn an_empty_document_deserializes_to_the_default() {
        let parsed: BootstrapConfig = toml::from_str("").expect("empty document parses");
        assert_eq!(parsed, BootstrapConfig::default());
    }

    #[test]
    fn the_default_round_trips_through_toml() {
        let original = BootstrapConfig::default();
        let serialized = toml::to_string(&original).expect("default serializes");
        let parsed: BootstrapConfig =
            toml::from_str(&serialized).expect("serialized default parses");
        assert_eq!(parsed, original);
    }

    #[test]
    fn a_partial_document_overrides_only_the_named_fields() {
        let document = r#"
            [server]
            hostname = "mail.example.com"
            node-id = 3

            [log]
            target = "stderr"
            level = "debug"
        "#;

        let parsed: BootstrapConfig = toml::from_str(document).expect("partial document parses");

        assert_eq!(parsed.server.hostname, "mail.example.com");
        assert_eq!(parsed.server.node_id, 3);
        assert_eq!(parsed.log.target, LogTarget::Stderr);
        assert_eq!(parsed.log.level, LogLevel::Debug);

        assert_eq!(parsed.paths, PathsConfig::default());
        assert_eq!(parsed.listeners, ListenersConfig::default());
    }

    #[test]
    fn a_relay_section_parses_with_auth_and_defaults() {
        let document = r#"
            [relay]
            host = "smart.example"
            username = "mailer"
            password = "hunter2"
        "#;

        let parsed: BootstrapConfig = toml::from_str(document).expect("relay document parses");
        let relay = parsed.relay.expect("a relay is configured");
        assert_eq!(relay.host, "smart.example");
        assert_eq!(relay.port, 587);
        assert_eq!(relay.username.as_deref(), Some("mailer"));
        assert_eq!(relay.password.as_deref(), Some("hunter2"));
        assert!(!relay.implicit_tls);
        assert!(!relay.require_tls);
        assert!(!relay.accept_invalid_certs);
    }

    #[test]
    fn a_relay_can_require_tls() {
        let document = r#"
            [relay]
            host = "smart.example"
            require-tls = true
        "#;

        let parsed: BootstrapConfig = toml::from_str(document).expect("relay document parses");
        assert!(parsed.relay.expect("a relay is configured").require_tls);
    }

    #[test]
    fn no_relay_section_means_direct_mx_delivery() {
        let parsed: BootstrapConfig = toml::from_str("").expect("empty document parses");
        assert_eq!(parsed.relay, None);
    }

    #[test]
    fn a_listener_port_can_be_disabled_by_omission() {
        let document = r#"
            [listeners.imap]
            tls = 993
        "#;

        let parsed: BootstrapConfig = toml::from_str(document).expect("listener document parses");
        assert_eq!(parsed.listeners.imap.plain, None);
        assert_eq!(parsed.listeners.imap.tls, Some(993));
    }
}
