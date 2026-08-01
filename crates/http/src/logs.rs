use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::AppState;

#[derive(Deserialize)]
pub struct LogQuery {
    pub severity: Option<String>,
    pub source: Option<String>,
    pub search: Option<String>,
    pub since: Option<u64>,
}

pub async fn logs(State(state): State<AppState>, Query(query): Query<LogQuery>) -> Json<Value> {
    let records: Vec<Value> = state
        .logs
        .snapshot()
        .into_iter()
        .filter(|record| {
            query
                .severity
                .as_ref()
                .is_none_or(|wanted| record.severity.as_str().eq_ignore_ascii_case(wanted))
        })
        .filter(|record| {
            query
                .source
                .as_ref()
                .is_none_or(|wanted| record.source.contains(wanted.as_str()))
        })
        .filter(|record| {
            query
                .search
                .as_ref()
                .is_none_or(|wanted| record.message.contains(wanted.as_str()))
        })
        .filter(|record| {
            query
                .since
                .is_none_or(|since| record.timestamp_millis >= since)
        })
        .map(|record| {
            json!({
                "timestamp": record.timestamp_millis,
                "severity": record.severity.as_str(),
                "source": record.source,
                "message": record.message,
            })
        })
        .collect();
    Json(json!({ "logs": records }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    use irixmail_core::{LogRecord, LogSeverity};

    use crate::app::router;
    use crate::tests_support::{admin_token, state, TempDir};

    #[tokio::test]
    async fn the_log_records_are_returned_and_filtered() {
        let dir = TempDir::new();
        let shared = state(&dir);
        shared.logs.push(LogRecord {
            timestamp_millis: 10,
            severity: LogSeverity::Info,
            source: "smtp".into(),
            message: "accepted mail".into(),
        });
        shared.logs.push(LogRecord {
            timestamp_millis: 20,
            severity: LogSeverity::Error,
            source: "imap".into(),
            message: "login failed".into(),
        });
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/logs?severity=error")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["logs"].as_array().unwrap().len(), 1);
        assert_eq!(value["logs"][0]["message"], "login failed");
    }
}
