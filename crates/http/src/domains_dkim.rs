use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use irixmail_dns::dkim_record;

use crate::app::{error_response, AppState};
use crate::validate::parse_id;

pub async fn dkim(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(id) = parse_id(&id) else {
        return error_response(StatusCode::NOT_FOUND, "domain not found");
    };
    let domain = match state.directory.domains().get(id) {
        Ok(domain) => domain,
        Err(_) => return error_response(StatusCode::NOT_FOUND, "domain not found"),
    };
    let records = match state.directory.dkim().get(id) {
        Ok(Some(key)) => vec![dkim_record(&domain.name, &key)],
        Ok(None) => Vec::new(),
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not load the DKIM key",
            )
        }
    };
    let key_ids: Vec<String> = if records.is_empty() {
        Vec::new()
    } else {
        vec![domain.id.to_string()]
    };
    Json(json!({
        "domain": domain.name,
        "keyIds": key_ids,
        "records": records,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request};
    use tower::ServiceExt;

    use crate::app::router;
    use crate::tests_support::{admin_token, state, TempDir};

    #[tokio::test]
    async fn the_dkim_keys_are_listed() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        shared
            .directory
            .dkim()
            .get_or_create(domain.id, "default")
            .unwrap();
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/domains/{}/dkim", domain.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value["keyIds"][0].as_str().unwrap(),
            domain.id.to_string(),
            "key ids must be decimal strings so browsers keep every digit"
        );
    }

    #[tokio::test]
    async fn a_non_numeric_domain_id_is_not_found() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/domains/not-a-number/dkim")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
