use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::app::{error_response, AppState};
use crate::validate::parse_id;

pub async fn delete(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(id) = parse_id(&id) else {
        return error_response(StatusCode::NOT_FOUND, "domain not found");
    };
    if state.directory.domains().get(id).is_err() {
        return error_response(StatusCode::NOT_FOUND, "domain not found");
    }
    match state.directory.domains().delete(id) {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Err(irixmail_core::Error::InvalidInput(_)) => {
            error_response(StatusCode::CONFLICT, "domain still has accounts")
        }
        Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "could not delete domain"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt;

    use crate::app::router;
    use crate::tests_support::{admin_token, state, TempDir};

    #[tokio::test]
    async fn an_existing_domain_is_deleted() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let token = admin_token(&shared);
        let app = router(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/domains/{}", domain.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(shared.directory.domains().get(domain.id).is_err());
    }

    #[tokio::test]
    async fn deleting_a_domain_with_accounts_is_a_conflict() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        shared
            .directory
            .accounts()
            .create("alice", domain.id, "Alice", irixmail_directory::Role::User)
            .unwrap();
        let token = admin_token(&shared);
        let app = router(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/domains/{}", domain.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert!(shared.directory.domains().get(domain.id).is_ok());
    }

    #[tokio::test]
    async fn deleting_an_unknown_domain_is_not_found() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/domains/999")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
