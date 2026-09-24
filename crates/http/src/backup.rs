use std::convert::Infallible;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use rand::RngCore;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

use irixmail_store::backup::{utc_parts, write_archive};

use crate::app::{error_response, AppState};

const TICKET_TTL: Duration = Duration::from_secs(60);

const CHUNK: usize = 64 * 1024;

const QUEUE: usize = 16;

pub async fn ticket(State(state): State<AppState>) -> Response {
    if state.backup.is_none() {
        return unavailable();
    }
    if state.backup_running.load(Ordering::SeqCst) {
        return busy();
    }
    let token = random_ticket();
    state.backup_tickets.set(token.as_bytes(), b"1", TICKET_TTL);
    (StatusCode::OK, Json(json!({ "ticket": token }))).into_response()
}

#[derive(Deserialize)]
pub struct DownloadQuery {
    #[serde(default)]
    pub ticket: String,
}

pub async fn download(
    State(state): State<AppState>,
    Query(query): Query<DownloadQuery>,
) -> Response {
    if query.ticket.is_empty() || state.backup_tickets.remove(query.ticket.as_bytes()).is_none() {
        return error_response(StatusCode::UNAUTHORIZED, "a valid backup ticket is required");
    }
    let Some(paths) = state.backup.clone() else {
        return unavailable();
    };
    if state.backup_running.swap(true, Ordering::SeqCst) {
        return busy();
    }
    let running = Running(Arc::clone(&state.backup_running));
    let started = unix_now();
    let name = file_name(&state.hostname, started);
    let attachment = format!("attachment; filename=\"{name}\"");
    let store = Arc::clone(&state.store);
    let hostname = state.hostname.clone();
    let (tx, rx) = mpsc::channel::<Vec<u8>>(QUEUE);
    tracing::info!(target: "irixmail::backup", file = %name, "backup download started");
    tokio::task::spawn_blocking(move || {
        let _running = running;
        let writer = ChannelWriter {
            tx,
            buffer: Vec::with_capacity(CHUNK),
        };
        match write_archive(&paths, store.as_ref(), &hostname, crate::VERSION, writer) {
            Ok(_) => tracing::info!(target: "irixmail::backup", file = %name, "backup download finished"),
            Err(error) => tracing::warn!(target: "irixmail::backup", file = %name, error = %error, "backup download failed"),
        }
    });

    let stream = ReceiverStream::new(rx).map(Ok::<Vec<u8>, Infallible>);
    let mut response = Body::from_stream(stream).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/gzip"),
    );
    if let Ok(value) = HeaderValue::from_str(&attachment) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

struct Running(Arc<AtomicBool>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

struct ChannelWriter {
    tx: mpsc::Sender<Vec<u8>>,
    buffer: Vec<u8>,
}

impl ChannelWriter {
    fn send_buffer(&mut self) -> std::io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let chunk = std::mem::replace(&mut self.buffer, Vec::with_capacity(CHUNK));
        self.tx.blocking_send(chunk).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "the download was abandoned")
        })
    }
}

impl Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        if self.buffer.len() >= CHUNK {
            self.send_buffer()?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.send_buffer()
    }
}

impl Drop for ChannelWriter {
    fn drop(&mut self) {
        let _ = self.send_buffer();
    }
}

fn unavailable() -> Response {
    error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "backup is unavailable in this configuration",
    )
}

fn busy() -> Response {
    error_response(StatusCode::CONFLICT, "a backup is already running")
}

fn random_ticket() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn file_name(hostname: &str, secs: u64) -> String {
    let host: String = hostname
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' })
        .collect();
    let (year, month, day, hour, minute) = utc_parts(secs);
    format!("irixmail-backup-{host}-{year:04}{month:02}{day:02}-{hour:02}{minute:02}.tar.gz")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use tower::ServiceExt;

    use irixmail_store::BackupPaths;

    use crate::app::router;
    use crate::tests_support::{admin_token, state, TempDir};

    async fn request_ticket(shared: &AppState, token: Option<&str>) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder().method("POST").uri("/api/backup/ticket");
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let response = router(shared.clone())
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
    }

    async fn get(shared: &AppState, uri: &str) -> axum::http::Response<Body> {
        router(shared.clone())
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    fn with_paths(shared: &mut AppState, dir: &TempDir) {
        shared.backup = Some(Arc::new(BackupPaths {
            config_file: dir.path.join("config.toml"),
            db: dir.path.join("db"),
            blobs: dir.path.join("blobs"),
            secret_key: dir.path.join("credential.key"),
            certs: dir.path.join("certs"),
        }));
    }

    #[tokio::test]
    async fn a_ticket_needs_an_admin_and_backup_paths() {
        let dir = TempDir::new();
        let mut shared = state(&dir);
        let (status, _) = request_ticket(&shared, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _) = request_ticket(&shared, Some(&admin_token(&shared))).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        with_paths(&mut shared, &dir);
        let (status, value) = request_ticket(&shared, Some(&admin_token(&shared))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["ticket"].as_str().map(str::len), Some(64));
    }

    #[tokio::test]
    async fn a_download_ticket_is_required_and_single_use() {
        let dir = TempDir::new();
        let shared = state(&dir);
        assert_eq!(get(&shared, "/api/backup").await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            get(&shared, "/api/backup?ticket=nope").await.status(),
            StatusCode::UNAUTHORIZED
        );
        shared.backup_tickets.set(b"t1", b"1", TICKET_TTL);
        assert_eq!(
            get(&shared, "/api/backup?ticket=t1").await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            get(&shared, "/api/backup?ticket=t1").await.status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn only_one_backup_runs_at_a_time() {
        let dir = TempDir::new();
        let mut shared = state(&dir);
        with_paths(&mut shared, &dir);
        shared.backup_running.store(true, Ordering::SeqCst);
        let (status, _) = request_ticket(&shared, Some(&admin_token(&shared))).await;
        assert_eq!(status, StatusCode::CONFLICT);
        shared.backup_tickets.set(b"t1", b"1", TICKET_TTL);
        assert_eq!(
            get(&shared, "/api/backup?ticket=t1").await.status(),
            StatusCode::CONFLICT
        );
    }

    #[tokio::test]
    async fn the_download_streams_a_gzip_archive() {
        let dir = TempDir::new();
        let mut shared = state(&dir);
        with_paths(&mut shared, &dir);
        shared.backup_tickets.set(b"t1", b"1", TICKET_TTL);
        let response = get(&shared, "/api/backup?ticket=t1").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/gzip"
        );
        let disposition = response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(disposition.starts_with("attachment; filename=\"irixmail-backup-mail.example.com-"));
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(bytes.len() > 2 && bytes[0] == 0x1f && bytes[1] == 0x8b);
        assert!(!shared.backup_running.load(Ordering::SeqCst));
    }

    #[test]
    fn file_names_carry_the_host_and_the_utc_minute() {
        assert_eq!(
            file_name("mail.example.com", 1_700_000_000),
            "irixmail-backup-mail.example.com-20231114-2213.tar.gz"
        );
        assert_eq!(
            file_name("weird host/name", 0),
            "irixmail-backup-weird_host_name-19700101-0000.tar.gz"
        );
    }
}
