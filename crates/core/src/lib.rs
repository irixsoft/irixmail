pub mod config;
pub mod config_load;
pub mod config_write;
pub mod error;
pub mod id;
pub mod log_buffer;
pub mod registry;
pub mod server;
pub mod shutdown;
pub mod storage;
pub mod telemetry;

pub use config::{
    BootstrapConfig, ListenersConfig, LogConfig, LogLevel, LogTarget, PathsConfig,
    ProtocolListener, RelayConfig, ServerConfig,
};
pub use error::{Error, Result};
pub use id::{DocumentIdAllocator, IdGenerator};
pub use log_buffer::{LogBuffer, LogBufferLayer, LogRecord, LogSeverity, DEFAULT_CAPACITY};
pub use registry::{Registry, ServiceFuture, ServiceKind};
pub use server::{RuntimeConfig, Server, ServerState};
pub use shutdown::{Shutdown, ShutdownCause, ShutdownSignal};
pub use storage::Storage;
pub use telemetry::{init_logging, TelemetryGuard};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
