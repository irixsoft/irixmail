use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::Mutex;

use tracing_subscriber::filter::{EnvFilter, LevelFilter};
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;

use crate::config::{LogConfig, LogLevel, LogTarget};
use crate::error::{Error, Result};
use crate::log_buffer::{LogBuffer, LogBufferLayer};

const LOG_FILE_NAME: &str = "irixmail.log";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TelemetryGuard {
    target: LogTarget,
    level: LogLevel,
}

impl TelemetryGuard {
    pub fn target(&self) -> LogTarget {
        self.target
    }

    pub fn level(&self) -> LogLevel {
        self.level
    }
}

pub fn init_logging(
    config: &LogConfig,
    log_dir: impl AsRef<Path>,
    log_buffer: &LogBuffer,
) -> Result<TelemetryGuard> {
    let filter = filter_for(config.level);
    let buffer_layer = log_buffer.layer();

    match config.target {
        LogTarget::Journald | LogTarget::Stderr => {
            let fmt_layer = fmt::layer().with_ansi(false).with_writer(std::io::stderr);
            install(filter, fmt_layer, buffer_layer)?;
        }
        LogTarget::File => {
            let file = open_log_file(log_dir.as_ref())?;
            let fmt_layer = fmt::layer().with_ansi(false).with_writer(Mutex::new(file));
            install(filter, fmt_layer, buffer_layer)?;
        }
    }

    Ok(TelemetryGuard {
        target: config.target,
        level: config.level,
    })
}

const QUIET_TARGETS: &str = "rustls=error";

fn filter_for(level: LogLevel) -> EnvFilter {
    EnvFilter::builder()
        .with_default_directive(level_filter_for(level).into())
        .parse_lossy(filter_source(level, std::env::var("RUST_LOG").ok()))
}

fn filter_source(level: LogLevel, env: Option<String>) -> String {
    match env {
        Some(value) if !value.trim().is_empty() => value,
        _ => format!("{},{QUIET_TARGETS}", level_filter_for(level)),
    }
}

fn level_filter_for(level: LogLevel) -> LevelFilter {
    match level {
        LogLevel::Error => LevelFilter::ERROR,
        LogLevel::Warn => LevelFilter::WARN,
        LogLevel::Info => LevelFilter::INFO,
        LogLevel::Debug => LevelFilter::DEBUG,
        LogLevel::Trace => LevelFilter::TRACE,
    }
}

fn open_log_file(log_dir: &Path) -> Result<File> {
    std::fs::create_dir_all(log_dir)?;
    let path = log_dir.join(LOG_FILE_NAME);
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    Ok(file)
}

fn install<L>(filter: EnvFilter, fmt_layer: L, buffer_layer: LogBufferLayer) -> Result<()>
where
    L: tracing_subscriber::Layer<tracing_subscriber::Registry> + Send + Sync + 'static,
{
    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(buffer_layer)
        .with(filter)
        .try_init()
        .map_err(|err| Error::internal(format!("could not install the logging subscriber: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_level_maps_to_its_filter() {
        assert_eq!(level_filter_for(LogLevel::Error), LevelFilter::ERROR);
        assert_eq!(level_filter_for(LogLevel::Warn), LevelFilter::WARN);
        assert_eq!(level_filter_for(LogLevel::Info), LevelFilter::INFO);
        assert_eq!(level_filter_for(LogLevel::Debug), LevelFilter::DEBUG);
        assert_eq!(level_filter_for(LogLevel::Trace), LevelFilter::TRACE);
    }

    #[test]
    fn the_default_directives_silence_rustls_handshake_noise() {
        assert_eq!(filter_source(LogLevel::Info, None), "info,rustls=error");
    }

    #[test]
    fn an_explicit_rust_log_wins_over_the_defaults() {
        assert_eq!(filter_source(LogLevel::Warn, Some("debug".into())), "debug");
    }

    #[test]
    fn a_blank_rust_log_falls_back_to_the_defaults() {
        assert_eq!(
            filter_source(LogLevel::Warn, Some("  ".into())),
            "warn,rustls=error"
        );
    }

    #[test]
    fn the_default_filter_carries_the_configured_level() {
        if std::env::var_os("RUST_LOG").is_none() {
            let filter = filter_for(LogLevel::Warn);
            assert_eq!(filter.max_level_hint(), Some(LevelFilter::WARN));
        }
    }

    #[test]
    fn opening_the_log_file_creates_the_directory_and_appends() {
        use std::io::Write;

        let dir = std::env::temp_dir().join(format!(
            "irixmail-telemetry-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        let mut first = open_log_file(&dir).expect("first open creates the directory and file");
        writeln!(first, "one").expect("first line writes");
        drop(first);

        let mut second = open_log_file(&dir).expect("second open reuses the existing file");
        writeln!(second, "two").expect("second line writes");
        drop(second);

        let contents = std::fs::read_to_string(dir.join(LOG_FILE_NAME)).expect("log file reads");
        assert_eq!(contents, "one\ntwo\n");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_guard_reports_the_installed_target_and_level() {
        let guard = TelemetryGuard {
            target: LogTarget::File,
            level: LogLevel::Debug,
        };
        assert_eq!(guard.target(), LogTarget::File);
        assert_eq!(guard.level(), LogLevel::Debug);
    }

    #[test]
    fn the_log_buffer_layer_is_fed_alongside_the_formatting_layer() {
        use tracing::subscriber::with_default;

        let buffer = LogBuffer::with_capacity(16);
        let fmt_layer = fmt::layer().with_ansi(false).with_writer(std::io::sink);
        let subscriber = tracing_subscriber::registry()
            .with(buffer.layer())
            .with(fmt_layer);

        with_default(subscriber, || {
            tracing::info!(target: "irixmail::core", "viewer is fed");
        });

        let snapshot = buffer.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].source, "irixmail::core");
        assert_eq!(snapshot[0].message, "viewer is fed");
    }
}
