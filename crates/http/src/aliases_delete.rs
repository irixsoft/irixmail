use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::app::{error_response, AppState};
use crate::validate::parse_id;

pub async fn delete(
    State(state): State<AppState>,
    Path((id, alias)): Path<(String, String)>,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return error_response(StatusCode::NOT_FOUND, "account not found");
    };
    let mut account = match state.directory.accounts().get(id) {
        Ok(account) => account,
        Err(_) => return error_response(StatusCode::NOT_FOUND, "account not found"),
    };
    let before = account.aliases.len();
    account
        .aliases
        .retain(|existing| !existing.eq_ignore_ascii_case(&alias));
    if account.aliases.len() == before {
        return error_response(StatusCode::NOT_FOUND, "alias not found");
    }
    if state.directory.accounts().update(account.clone()).is_err() {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "could not remove alias");
    }
    (StatusCode::OK, Json(json!({ "aliases": account.aliases }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt;

    use irixmail_directory::Role;

    use crate::app::router;
    use crate::tests_support::{admin_token, state, TempDir};

    #[tokio::test]
    async fn an_existing_alias_is_removed() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let mut account = shared
            .directory
            .accounts()
            .create("alice", domain.id, "Alice", Role::User)
            .unwrap();
        account.aliases.push("a.adams@example.com".into());
        shared.directory.accounts().update(account.clone()).unwrap();
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!(
                        "/api/accounts/{}/aliases/a.adams@example.com",
                        account.id
                    ))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn an_unknown_alias_is_not_found() {
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
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!(
                        "/api/accounts/{}/aliases/ghost@example.com",
                        account.id
                    ))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
