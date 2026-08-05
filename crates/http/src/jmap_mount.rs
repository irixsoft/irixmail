use std::collections::HashSet;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::broadcast::error::RecvError;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

use irixmail_jmap as jmap;

use crate::app::{error_response, AppState};
use crate::auth_mw::AuthIdentity;

const SESSION_STATE: &str = "0";

pub fn methods() -> jmap::Router {
    let mut router = jmap::Router::new();
    router
        .register_stateful("Mailbox/get", jmap::mailbox_get)
        .register_stateful("Mailbox/set", jmap::mailbox_set)
        .register_stateful("Mailbox/query", jmap::mailbox_query)
        .register_stateful("Mailbox/changes", jmap::mailbox_changes)
        .register_stateful("Email/get", jmap::email_get)
        .register_stateful("Email/set", jmap::email_set)
        .register_stateful("Email/query", jmap::email_query)
        .register_stateful("Email/changes", jmap::email_changes)
        .register_stateful("Email/queryChanges", jmap::email_querychanges)
        .register_stateful("Mailbox/queryChanges", jmap::mailbox_querychanges)
        .register_stateful("Email/import", jmap::email_import)
        .register_stateful("Email/parse", jmap::email_parse)
        .register_stateful("Email/copy", jmap::email_copy)
        .register_stateful("Thread/get", jmap::thread_get)
        .register_stateful("Identity/get", jmap::identity_get)
        .register_stateful("Identity/set", jmap::identity_set)
        .register_stateful("EmailSubmission/get", jmap::submission_get)
        .register_stateful("EmailSubmission/set", jmap::submission_set)
        .register_stateful("SearchSnippet/get", jmap::searchsnippet_get)
        .register_stateful("VacationResponse/get", jmap::vacation_get)
        .register_stateful("VacationResponse/set", jmap::vacation_set)
        .register_stateful("SieveScript/get", jmap::sievescript_get)
        .register_stateful("SieveScript/set", jmap::sievescript_set)
        .register_stateful("PushSubscription/get", jmap::push_get)
        .register_stateful("PushSubscription/set", jmap::push_set)
        .register_stateful("Calendar/get", jmap::calendar_get)
        .register_stateful("Calendar/set", jmap::calendar_set)
        .register_stateful("Calendar/changes", jmap::calendar_changes)
        .register_stateful("CalendarEvent/get", jmap::calendar_event_get)
        .register_stateful("CalendarEvent/set", jmap::calendar_event_set)
        .register_stateful("CalendarEvent/query", jmap::calendar_event_query)
        .register_stateful("CalendarEvent/changes", jmap::calendar_event_changes)
        .register_stateful("AddressBook/get", jmap::addressbook_get)
        .register_stateful("AddressBook/set", jmap::addressbook_set)
        .register_stateful("AddressBook/changes", jmap::addressbook_changes)
        .register_stateful("ContactCard/get", jmap::contact_get)
        .register_stateful("ContactCard/set", jmap::contact_set)
        .register_stateful("ContactCard/query", jmap::contact_query)
        .register_stateful("ContactCard/changes", jmap::contact_changes);
    router
}

pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/jmap/", post(jmap_api))
        .route("/jmap/session", get(jmap_session))
        .route("/jmap/upload/{accountId}", post(jmap_upload))
        .route("/jmap/upload/{accountId}/", post(jmap_upload))
        .route(
            "/jmap/download/{accountId}/{blobId}/{name}",
            get(jmap_download),
        )
        .route("/jmap/eventsource", get(jmap_eventsource))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth_mw::require_auth,
        ))
        // Reject over-limit bodies in the handlers with JMAP problem shapes, not a bare 413.
        .layer(axum::extract::DefaultBodyLimit::max(
            jmap::MAX_SIZE_UPLOAD + 1024,
        ))
}

fn owns_account(identity: &AuthIdentity, account_id: &str) -> bool {
    identity.is_admin || identity.account_id.to_string() == account_id
}

fn account_key(account_id: &str) -> u32 {
    account_id.parse::<u64>().map_or(0, |id| id as u32)
}

async fn jmap_api(
    State(state): State<AppState>,
    Extension(identity): Extension<AuthIdentity>,
    body: Bytes,
) -> Response {
    if body.len() > jmap::MAX_SIZE_REQUEST {
        tracing::warn!(
            target: "irixmail::jmap",
            account = identity.account_id,
            bytes = body.len(),
            "jmap request rejected: over the size limit"
        );
        return jmap_problem(jmap::limit_problem("maxSizeRequest"));
    }
    let value: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => {
            tracing::warn!(
                target: "irixmail::jmap",
                account = identity.account_id,
                "jmap request rejected: not json"
            );
            return jmap_problem(jmap::problem("notJSON"));
        }
    };
    let request: jmap::Request = match serde_json::from_value(value) {
        Ok(request) => request,
        Err(_) => {
            tracing::warn!(
                target: "irixmail::jmap",
                account = identity.account_id,
                "jmap request rejected: not a request envelope"
            );
            return jmap_problem(jmap::problem("notRequest"));
        }
    };
    if let Some(capability) = jmap::unknown_capability(&request.using) {
        tracing::warn!(
            target: "irixmail::jmap",
            account = identity.account_id,
            capability = %capability,
            "jmap request rejected: unknown capability"
        );
        return jmap_problem(jmap::unknown_capability_problem(capability));
    }
    if request.method_calls.len() > jmap::MAX_CALLS_IN_REQUEST {
        tracing::warn!(
            target: "irixmail::jmap",
            account = identity.account_id,
            calls = request.method_calls.len(),
            "jmap request rejected: too many calls"
        );
        return jmap_problem(jmap::limit_problem("maxCallsInRequest"));
    }
    let methods_called: Vec<&str> = request
        .method_calls
        .iter()
        .map(|call| call.name())
        .collect();
    let started = std::time::Instant::now();
    let ctx = jmap::JmapContext::from_parts(
        Arc::clone(&state.store),
        Arc::clone(&state.blobs),
        Arc::clone(&state.notifier),
        state.directory.clone(),
        identity.account_id,
        state.submitter.clone(),
    );
    let response = methods().process(&ctx, &request, SESSION_STATE);
    let errors = response
        .method_responses
        .iter()
        .filter(|invocation| invocation.name() == "error")
        .count();
    tracing::info!(
        target: "irixmail::jmap",
        account = identity.account_id,
        user = %identity.username,
        methods = %methods_called.join(","),
        calls = methods_called.len(),
        errors,
        ms = started.elapsed().as_millis() as u64,
        "jmap request"
    );
    Json(response).into_response()
}

fn jmap_problem(body: Value) -> Response {
    (StatusCode::BAD_REQUEST, Json(body)).into_response()
}

struct EventSourceClosed {
    account_id: u64,
}

impl Drop for EventSourceClosed {
    fn drop(&mut self) {
        tracing::info!(
            target: "irixmail::jmap",
            account = self.account_id,
            "eventsource closed"
        );
    }
}

async fn jmap_session(
    State(state): State<AppState>,
    Extension(identity): Extension<AuthIdentity>,
) -> Json<Value> {
    let webpush_key = jmap::webpush::application_server_key(state.store.as_ref()).ok();
    Json(jmap::session_resource(
        &identity.account_id.to_string(),
        &identity.username,
        SESSION_STATE,
        webpush_key.as_deref(),
    ))
}

async fn jmap_upload(
    State(state): State<AppState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(account_id): Path<String>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Response {
    if !owns_account(&identity, &account_id) {
        return error_response(StatusCode::FORBIDDEN, "account mismatch");
    }
    if body.len() > jmap::MAX_SIZE_UPLOAD {
        return jmap_problem(jmap::limit_problem("maxSizeUpload"));
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    match jmap::store_upload(
        state.store.as_ref(),
        state.blobs.as_ref(),
        identity.account_id as u32,
        &body,
        irixmail_tls::acme_http01::unix_now(),
    ) {
        Ok(blob_id) => Json(jmap::upload_response(
            &account_id,
            &blob_id,
            &content_type,
            body.len(),
        ))
        .into_response(),
        Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "blob store error"),
    }
}

#[derive(Deserialize)]
struct DownloadParams {
    #[serde(default)]
    accept: Option<String>,
}

async fn jmap_download(
    State(state): State<AppState>,
    Extension(identity): Extension<AuthIdentity>,
    Path((account_id, blob_id, name)): Path<(String, String, String)>,
    Query(params): Query<DownloadParams>,
) -> Response {
    if !owns_account(&identity, &account_id) {
        return error_response(StatusCode::FORBIDDEN, "account mismatch");
    }
    let account = account_key(&account_id);
    let referenced = jmap::blob_hash_of(&blob_id).is_some_and(|hash| {
        let store = state.store.as_ref();
        let now = irixmail_tls::acme_http01::unix_now();
        irixmail_mail::account_references_blob(store, account, &hash).unwrap_or(false)
            || irixmail_mail::has_live_reservation(store, account, &hash, now).unwrap_or(false)
    });
    if !referenced {
        return error_response(StatusCode::NOT_FOUND, "blob not found");
    }
    let content_type = params
        .accept
        .as_deref()
        .and_then(|accept| HeaderValue::from_str(accept).ok())
        .unwrap_or_else(|| HeaderValue::from_static("application/octet-stream"));
    match jmap::fetch_blob(state.blobs.as_ref(), &blob_id) {
        Ok(Some(bytes)) => {
            let mut response = bytes.into_response();
            response
                .headers_mut()
                .insert(header::CONTENT_TYPE, content_type);
            if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")) {
                response
                    .headers_mut()
                    .insert(header::CONTENT_DISPOSITION, value);
            }
            response
        }
        Ok(None) => error_response(StatusCode::NOT_FOUND, "blob not found"),
        Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "blob store error"),
    }
}

#[derive(Deserialize)]
struct EventSourceParams {
    #[serde(default)]
    types: Option<String>,
    #[serde(default)]
    closeafter: Option<String>,
    #[serde(default)]
    ping: Option<String>,
}

async fn jmap_eventsource(
    State(state): State<AppState>,
    Extension(identity): Extension<AuthIdentity>,
    Query(params): Query<EventSourceParams>,
) -> Response {
    let account_id = identity.account_id;
    let account_label = account_id.to_string();
    let selected = selected_types(params.types.as_deref());
    let close_after_state = params.closeafter.as_deref() == Some("state");
    let ping_secs = params
        .ping
        .as_deref()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|secs| *secs > 0);

    tracing::info!(
        target: "irixmail::jmap",
        account = account_id,
        user = %identity.username,
        "eventsource connected"
    );
    let mut subscription = state.notifier.subscribe(account_id as u32);
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(32);
    tokio::spawn(async move {
        let _closed = EventSourceClosed { account_id };
        let mut ping = match ping_secs {
            Some(secs) => {
                if tx.send(jmap::ping_event(secs)).await.is_err() {
                    return;
                }
                let period = Duration::from_secs(secs);
                Some(tokio::time::interval_at(
                    tokio::time::Instant::now() + period,
                    period,
                ))
            }
            None => None,
        };
        loop {
            tokio::select! {
                biased;
                _ = async { ping.as_mut().expect("gated on ping.is_some()").tick().await },
                    if ping.is_some() =>
                {
                    let frame = jmap::ping_event(ping_secs.unwrap_or(0));
                    if tx.send(frame).await.is_err() {
                        return;
                    }
                }
                result = subscription.recv() => match result {
                    Ok(notice) => {
                        let Some(type_name) = jmap::type_name(notice.collection) else {
                            continue;
                        };
                        if let Some(selected) = &selected {
                            if !selected.contains(type_name) {
                                continue;
                            }
                        }
                        let change_state = notice.change_id.to_string();
                        let payload =
                            jmap::state_change_single(&account_label, type_name, &change_state);
                        let frame = jmap::sse_event("state", Some(&change_state), &payload);
                        if tx.send(frame).await.is_err() || close_after_state {
                            return;
                        }
                    }
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                },
                _ = tx.closed() => return,
            }
        }
    });

    let stream = ReceiverStream::new(rx).map(Ok::<String, Infallible>);
    let mut response = Body::from_stream(stream).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn selected_types(raw: Option<&str>) -> Option<HashSet<String>> {
    let raw = raw?.trim();
    if raw.is_empty() || raw == "*" {
        return None;
    }
    Some(
        raw.split(',')
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    use irixmail_store::Collection;

    use crate::app::router;
    use crate::tests_support::{admin_token, state, TempDir};

    async fn json_body(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn a_snowflake_account_id_string_maps_to_its_document_key() {
        let id: u64 = 340_000_000_000_000_123;
        assert_eq!(account_key(&id.to_string()), id as u32);
        assert_eq!(account_key("nonsense"), 0);
    }

    #[test]
    fn the_method_router_knows_the_mail_methods() {
        let methods = methods();
        assert!(methods.handles("Mailbox/get"));
        assert!(methods.handles("Email/query"));
        assert!(methods.handles("VacationResponse/set"));
        assert!(methods.handles("Calendar/get"));
        assert!(methods.handles("Calendar/set"));
        assert!(methods.handles("Calendar/changes"));
        assert!(methods.handles("CalendarEvent/get"));
        assert!(methods.handles("CalendarEvent/set"));
        assert!(methods.handles("CalendarEvent/query"));
        assert!(methods.handles("CalendarEvent/changes"));
        assert!(methods.handles("AddressBook/get"));
        assert!(methods.handles("AddressBook/set"));
        assert!(methods.handles("AddressBook/changes"));
        assert!(methods.handles("ContactCard/get"));
        assert!(methods.handles("ContactCard/set"));
        assert!(methods.handles("ContactCard/query"));
        assert!(methods.handles("ContactCard/changes"));
        assert!(!methods.handles("Nonsense/get"));
    }

    #[tokio::test]
    async fn the_jmap_api_requires_authentication() {
        let dir = TempDir::new();
        let app = router(state(&dir));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/jmap/")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"using":[],"methodCalls":[]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn an_app_password_over_basic_auth_still_reaches_the_data_plane() {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine as _;
        use irixmail_directory::{app_password, Role};

        let dir = TempDir::new();
        let shared = state(&dir);
        let domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let account = shared
            .directory
            .accounts()
            .create("alice", domain.id, "Alice", Role::User)
            .unwrap();
        let minted = app_password::generate(1, "phone", 0).unwrap();
        shared
            .directory
            .credentials()
            .add_app_password(account.id, minted.record)
            .unwrap();
        let encoded = STANDARD.encode(format!("alice@example.com:{}", minted.plaintext));
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/jmap/session")
                    .header(header::AUTHORIZATION, format!("Basic {encoded}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn the_advertised_trailing_slash_upload_url_is_routed() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/jmap/upload/999999/")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from("hello"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert!(body["blobId"].is_string());
    }

    #[tokio::test]
    async fn a_processed_request_is_logged_with_its_methods() {
        use tracing_subscriber::layer::SubscriberExt;
        static LOGS: std::sync::OnceLock<irixmail_core::LogBuffer> = std::sync::OnceLock::new();
        let logs = LOGS
            .get_or_init(|| {
                let buffer = irixmail_core::LogBuffer::new();
                let _ = tracing::subscriber::set_global_default(
                    tracing_subscriber::registry().with(buffer.layer()),
                );
                buffer
            })
            .clone();

        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let body = r#"{"using":["urn:ietf:params:jmap:mail"],"methodCalls":[["Mailbox/get",{"accountId":"1","ids":null},"c0"]]}"#;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/jmap/")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let text: String = logs
            .snapshot()
            .into_iter()
            .filter(|record| record.source == "irixmail::jmap")
            .map(|record| record.message)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("jmap request"), "{text}");
        assert!(text.contains("Mailbox/get"), "{text}");
    }

    #[tokio::test]
    async fn the_api_endpoint_processes_a_method_call() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let body = r#"{"using":["urn:ietf:params:jmap:mail"],"methodCalls":[["Mailbox/get",{"accountId":"1","ids":null},"c0"]]}"#;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/jmap/")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value = json_body(response).await;
        assert_eq!(value["methodResponses"][0][0], "Mailbox/get");
    }

    #[tokio::test]
    async fn too_many_method_calls_are_rejected() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let calls: Vec<String> = (0..jmap::MAX_CALLS_IN_REQUEST + 1)
            .map(|index| format!(r#"["Core/echo",{{}},"c{index}"]"#))
            .collect();
        let body = format!(r#"{{"using":[],"methodCalls":[{}]}}"#, calls.join(","));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/jmap/")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = json_body(response).await;
        assert_eq!(value["type"], "urn:ietf:params:jmap:error:limit");
        assert_eq!(value["limit"], "maxCallsInRequest");
    }

    #[tokio::test]
    async fn an_unknown_capability_is_rejected_at_the_request_level() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let body = r#"{"using":["urn:ietf:params:jmap:core","urn:bogus"],"methodCalls":[]}"#;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/jmap/")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = json_body(response).await;
        assert_eq!(
            value["type"],
            "urn:ietf:params:jmap:error:unknownCapability"
        );
    }

    #[tokio::test]
    async fn an_oversized_request_is_rejected_with_the_max_size_request_limit() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let padding = "a".repeat(10_000_001);
        let body = format!(r#"{{"using":[],"methodCalls":[],"padding":"{padding}"}}"#);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/jmap/")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = json_body(response).await;
        assert_eq!(value["type"], "urn:ietf:params:jmap:error:limit");
        assert_eq!(value["limit"], "maxSizeRequest");
    }

    #[tokio::test]
    async fn an_oversized_upload_is_rejected_with_the_max_size_upload_limit() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = routes(shared.clone()).with_state(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/jmap/upload/1")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header(header::CONTENT_TYPE, "application/octet-stream")
                    .body(Body::from(vec![0u8; 50_000_001]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = json_body(response).await;
        assert_eq!(value["type"], "urn:ietf:params:jmap:error:limit");
        assert_eq!(value["limit"], "maxSizeUpload");
    }

    #[tokio::test]
    async fn a_download_honors_the_accept_type_parameter() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = routes(shared.clone()).with_state(shared.clone());
        let upload = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/jmap/upload/1")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from("typed payload"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let blob_id = json_body(upload).await["blobId"]
            .as_str()
            .unwrap()
            .to_string();

        let app = routes(shared.clone()).with_state(shared);
        let download = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/jmap/download/1/{blob_id}/note.txt?accept=text/plain"
                    ))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(download.status(), StatusCode::OK);
        assert_eq!(
            download.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/plain"
        );
    }

    #[tokio::test]
    async fn an_unparseable_body_is_a_jmap_not_json_error() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/jmap/")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("this is not json"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = json_body(response).await;
        assert_eq!(value["type"], "urn:ietf:params:jmap:error:notJSON");
    }

    #[tokio::test]
    async fn a_structurally_wrong_body_is_a_jmap_not_request_error() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/jmap/")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"unexpected": true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = json_body(response).await;
        assert_eq!(value["type"], "urn:ietf:params:jmap:error:notRequest");
    }

    #[tokio::test]
    async fn the_session_endpoint_returns_capabilities() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = routes(shared.clone()).with_state(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/jmap/session")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value = json_body(response).await;
        assert!(value["capabilities"].is_object());
    }

    #[tokio::test]
    async fn the_eventsource_advertises_the_event_stream_content_type() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = routes(shared.clone()).with_state(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/jmap/eventsource")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(content_type.starts_with("text/event-stream"));
    }

    #[tokio::test]
    async fn the_eventsource_streams_a_change_notice() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = routes(shared.clone()).with_state(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/jmap/eventsource?types=*&closeafter=state&ping=0")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        shared.notifier.notify_change(1, Collection::Email, 42);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("event: state"));
        assert!(text.contains("id: 42"));
        assert!(text.contains("\"Email\":\"42\""));
    }

    #[tokio::test]
    async fn the_eventsource_honors_the_types_filter() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = routes(shared.clone()).with_state(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/jmap/eventsource?types=Mailbox&closeafter=state&ping=0")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        shared.notifier.notify_change(1, Collection::Email, 7);
        shared.notifier.notify_change(1, Collection::Mailbox, 9);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("\"Mailbox\":\"9\""));
        assert!(!text.contains("Email"));
    }

    #[tokio::test]
    async fn the_ping_interval_emits_rfc8620_ping_events() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = routes(shared.clone()).with_state(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/jmap/eventsource?types=*&closeafter=state&ping=1")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        shared.notifier.notify_change(1, Collection::Email, 3);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(
            text.contains("event: ping"),
            "ping must be a named SSE event: {text:?}"
        );
        assert!(
            text.contains("\"interval\":1"),
            "ping data must carry the interval: {text:?}"
        );
        assert!(
            text.contains("event: state"),
            "the state event still closes the stream: {text:?}"
        );
    }

    #[tokio::test]
    async fn an_unreferenced_blob_is_not_downloadable() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let foreign = shared.blobs.put(b"someone else's message bytes").unwrap();
        let app = routes(shared.clone()).with_state(shared);

        let download = app
            .oneshot(
                Request::builder()
                    .uri(format!("/jmap/download/1/{}/leak.txt", foreign.to_hex()))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(download.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_blob_round_trips_through_upload_and_download() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = routes(shared.clone()).with_state(shared.clone());
        let upload = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/jmap/upload/1")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from("payload"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(upload.status(), StatusCode::OK);
        let blob_id = json_body(upload).await["blobId"]
            .as_str()
            .unwrap()
            .to_string();

        let app = routes(shared.clone()).with_state(shared);
        let download = app
            .oneshot(
                Request::builder()
                    .uri(format!("/jmap/download/1/{blob_id}/file.txt"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(download.status(), StatusCode::OK);
        let bytes = to_bytes(download.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], b"payload");
    }
}
