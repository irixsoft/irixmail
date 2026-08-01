use std::collections::VecDeque;
use std::fmt::{self, Write as _};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

pub const DEFAULT_CAPACITY: usize = 2_048;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogSeverity {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogSeverity {
    fn from_level(level: &Level) -> Self {
        match *level {
            Level::ERROR => Self::Error,
            Level::WARN => Self::Warn,
            Level::INFO => Self::Info,
            Level::DEBUG => Self::Debug,
            Level::TRACE => Self::Trace,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

impl fmt::Display for LogSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRecord {
    pub timestamp_millis: u64,
    pub severity: LogSeverity,
    pub source: String,
    pub message: String,
}

#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<Ring>>,
}

struct Ring {
    records: VecDeque<LogRecord>,
    capacity: usize,
}

impl LogBuffer {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            inner: Arc::new(Mutex::new(Ring {
                records: VecDeque::with_capacity(capacity),
                capacity,
            })),
        }
    }

    pub fn push(&self, record: LogRecord) {
        let mut ring = self.inner.lock();
        if ring.records.len() == ring.capacity {
            ring.records.pop_front();
        }
        ring.records.push_back(record);
    }

    pub fn snapshot(&self) -> Vec<LogRecord> {
        let ring = self.inner.lock();
        ring.records.iter().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().records.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.inner.lock().capacity
    }

    pub fn clear(&self) {
        self.inner.lock().records.clear();
    }

    pub fn layer(&self) -> LogBufferLayer {
        LogBufferLayer {
            buffer: self.clone(),
        }
    }
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct LogBufferLayer {
    buffer: LogBuffer,
}

impl<S: Subscriber> Layer<S> for LogBufferLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let record = LogRecord {
            timestamp_millis: now_millis(),
            severity: LogSeverity::from_level(metadata.level()),
            source: metadata.target().to_string(),
            message: visitor.finish(),
        };

        self.buffer.push(record);
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
    fields: String,
}

impl MessageVisitor {
    fn append_field(&mut self, name: &str, value: impl fmt::Debug) {
        if !self.fields.is_empty() {
            self.fields.push(' ');
        }
        let _ = write!(self.fields, "{name}={value:?}");
    }

    fn finish(self) -> String {
        match (self.message, self.fields.is_empty()) {
            (Some(message), true) => message,
            (Some(message), false) => format!("{message} {}", self.fields),
            (None, _) => self.fields,
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        } else {
            self.append_field(field.name(), value);
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.append_field(field.name(), value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::subscriber::with_default;
    use tracing_subscriber::prelude::*;

    fn record(severity: LogSeverity, source: &str, message: &str) -> LogRecord {
        LogRecord {
            timestamp_millis: 0,
            severity,
            source: source.to_string(),
            message: message.to_string(),
        }
    }

    #[test]
    fn severity_maps_from_level_and_renders_its_label() {
        assert_eq!(LogSeverity::from_level(&Level::ERROR), LogSeverity::Error);
        assert_eq!(LogSeverity::from_level(&Level::WARN), LogSeverity::Warn);
        assert_eq!(LogSeverity::from_level(&Level::INFO), LogSeverity::Info);
        assert_eq!(LogSeverity::from_level(&Level::DEBUG), LogSeverity::Debug);
        assert_eq!(LogSeverity::from_level(&Level::TRACE), LogSeverity::Trace);

        assert_eq!(LogSeverity::Error.as_str(), "error");
        assert_eq!(LogSeverity::Trace.to_string(), "trace");
    }

    #[test]
    fn severity_orders_most_to_least_severe() {
        assert!(LogSeverity::Error < LogSeverity::Warn);
        assert!(LogSeverity::Warn < LogSeverity::Info);
        assert!(LogSeverity::Info < LogSeverity::Debug);
        assert!(LogSeverity::Debug < LogSeverity::Trace);
    }

    #[test]
    fn a_new_buffer_is_empty_with_the_default_capacity() {
        let buffer = LogBuffer::new();
        assert!(buffer.is_empty());
        assert_eq!(buffer.len(), 0);
        assert_eq!(buffer.capacity(), DEFAULT_CAPACITY);
        assert!(buffer.snapshot().is_empty());
    }

    #[test]
    fn a_zero_capacity_still_retains_the_most_recent_record() {
        let buffer = LogBuffer::with_capacity(0);
        assert_eq!(buffer.capacity(), 1);

        buffer.push(record(LogSeverity::Info, "core", "first"));
        buffer.push(record(LogSeverity::Info, "core", "second"));

        let snapshot = buffer.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].message, "second");
    }

    #[test]
    fn pushing_beyond_capacity_evicts_the_oldest_first() {
        let buffer = LogBuffer::with_capacity(2);

        buffer.push(record(LogSeverity::Info, "core", "one"));
        buffer.push(record(LogSeverity::Info, "core", "two"));
        buffer.push(record(LogSeverity::Info, "core", "three"));

        let snapshot = buffer.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].message, "two");
        assert_eq!(snapshot[1].message, "three");
    }

    #[test]
    fn the_snapshot_supports_severity_source_and_text_filters() {
        let buffer = LogBuffer::with_capacity(8);
        buffer.push(record(LogSeverity::Error, "smtp", "connection refused"));
        buffer.push(record(LogSeverity::Info, "imap", "client connected"));
        buffer.push(record(LogSeverity::Warn, "smtp", "retry scheduled"));

        let snapshot = buffer.snapshot();

        let errors: Vec<_> = snapshot
            .iter()
            .filter(|r| r.severity == LogSeverity::Error)
            .collect();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "connection refused");

        let from_smtp: Vec<_> = snapshot.iter().filter(|r| r.source == "smtp").collect();
        assert_eq!(from_smtp.len(), 2);

        let matching: Vec<_> = snapshot
            .iter()
            .filter(|r| r.message.contains("connect"))
            .collect();
        assert_eq!(matching.len(), 2);
    }

    #[test]
    fn clearing_discards_every_record() {
        let buffer = LogBuffer::with_capacity(4);
        buffer.push(record(LogSeverity::Info, "core", "kept until cleared"));
        assert_eq!(buffer.len(), 1);

        buffer.clear();
        assert!(buffer.is_empty());
    }

    #[test]
    fn the_layer_captures_events_with_severity_source_and_message() {
        let buffer = LogBuffer::with_capacity(16);
        let subscriber = tracing_subscriber::registry().with(buffer.layer());

        with_default(subscriber, || {
            tracing::info!(target: "irixmail::core", "service started");
            tracing::warn!(target: "irixmail::smtp", code = 421, "greylisted");
        });

        let snapshot = buffer.snapshot();
        assert_eq!(snapshot.len(), 2);

        assert_eq!(snapshot[0].severity, LogSeverity::Info);
        assert_eq!(snapshot[0].source, "irixmail::core");
        assert_eq!(snapshot[0].message, "service started");

        assert_eq!(snapshot[1].severity, LogSeverity::Warn);
        assert_eq!(snapshot[1].source, "irixmail::smtp");
        assert!(snapshot[1].message.contains("greylisted"));
        assert!(snapshot[1].message.contains("code=421"));
    }

    #[test]
    fn a_buffer_clone_shares_the_underlying_storage() {
        let buffer = LogBuffer::with_capacity(4);
        let other = buffer.clone();

        buffer.push(record(LogSeverity::Info, "core", "via the original"));

        assert_eq!(other.len(), 1);
        assert_eq!(other.snapshot()[0].message, "via the original");
    }
}
