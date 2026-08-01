use std::sync::Arc;

use crate::config::BootstrapConfig;
use crate::id::IdGenerator;
use crate::log_buffer::LogBuffer;
use crate::registry::Registry;
use crate::storage::Storage;

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub bootstrap: BootstrapConfig,
}

impl RuntimeConfig {
    pub fn new(bootstrap: BootstrapConfig) -> Self {
        Self { bootstrap }
    }
}

impl From<BootstrapConfig> for RuntimeConfig {
    fn from(bootstrap: BootstrapConfig) -> Self {
        Self::new(bootstrap)
    }
}

pub struct ServerState {
    config: Arc<RuntimeConfig>,
    ids: Arc<IdGenerator>,
    logs: LogBuffer,
    storage: Storage,
    registry: Registry,
}

#[derive(Clone)]
pub struct Server(Arc<ServerState>);

impl Server {
    pub fn new(config: RuntimeConfig, ids: IdGenerator, logs: LogBuffer, storage: Storage) -> Self {
        Self(Arc::new(ServerState {
            config: Arc::new(config),
            ids: Arc::new(ids),
            logs,
            storage,
            registry: Registry::new(),
        }))
    }

    pub fn config(&self) -> Arc<RuntimeConfig> {
        Arc::clone(&self.0.config)
    }

    pub fn ids(&self) -> &IdGenerator {
        &self.0.ids
    }

    pub fn ids_handle(&self) -> Arc<IdGenerator> {
        Arc::clone(&self.0.ids)
    }

    pub fn logs(&self) -> &LogBuffer {
        &self.0.logs
    }

    pub fn storage(&self) -> &Storage {
        &self.0.storage
    }

    pub fn registry(&self) -> &Registry {
        &self.0.registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LogLevel;

    struct FakeBackend;

    fn hostname(name: &str) -> BootstrapConfig {
        let mut config = BootstrapConfig::default();
        config.server.hostname = name.to_string();
        config
    }

    fn storage() -> Storage {
        Storage::new(Arc::new(FakeBackend), Arc::new(FakeBackend))
    }

    fn server(config: BootstrapConfig) -> Server {
        Server::new(
            RuntimeConfig::new(config),
            IdGenerator::new(0),
            LogBuffer::new(),
            storage(),
        )
    }

    #[test]
    fn a_handle_reads_the_initial_configuration() {
        let server = server(hostname("mail.example.com"));
        assert_eq!(
            server.config().bootstrap.server.hostname,
            "mail.example.com"
        );
    }

    #[test]
    fn cloned_handles_share_state() {
        let server = server(hostname("host.example.com"));
        let other = server.clone();

        let first = server.ids().generate();
        let second = other.ids().generate();

        assert!(second > first);
        assert_eq!(other.config().bootstrap.server.hostname, "host.example.com");
        assert!(server.logs().is_empty());
        assert!(other.registry().is_empty());
        assert!(server.storage().store::<FakeBackend>().is_ok());
        assert!(other.storage().blob_store::<FakeBackend>().is_ok());
    }

    #[test]
    fn the_shared_id_handle_is_the_one_the_server_holds() {
        let server = server(hostname("ids.example.com"));
        let handle = server.ids_handle();

        let from_handle = handle.generate();
        let from_server = server.ids().generate();

        assert!(from_server > from_handle);
    }

    #[test]
    fn a_runtime_config_can_be_built_from_a_bootstrap_config() {
        let config: RuntimeConfig = hostname("conv.example.com").into();
        assert_eq!(config.bootstrap.server.hostname, "conv.example.com");
        assert_eq!(config.bootstrap.log.level, LogLevel::Info);
    }
}
