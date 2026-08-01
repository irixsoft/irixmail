use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::app::{error_response, AppState};
use crate::validate::{bad_request, is_valid_email, parse_id};

#[derive(Deserialize)]
pub struct AliasBody {
    pub alias: String,
}

pub async fn create(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<AliasBody>,
) -> Response {
    if !is_valid_email(&body.alias) {
        return bad_request("invalid alias address");
    }
    let Some(id) = parse_id(&id) else {
        return error_response(StatusCode::NOT_FOUND, "account not found");
    };
    let mut account = match state.directory.accounts().get(id) {
        Ok(account) => account,
        Err(_) => return error_response(StatusCode::NOT_FOUND, "account not found"),
    };
    let alias = body.alias.to_ascii_lowercase();
    if !account
        .aliases
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&alias))
    {
        account.aliases.push(alias);
        match state.directory.accounts().update(account.clone()) {
            Ok(()) => {}
            Err(irixmail_core::Error::InvalidInput(_)) => {
                return error_response(StatusCode::CONFLICT, "alias already in use");
            }
            Err(_) => {
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "could not add alias");
            }
        }
    }
    (
        StatusCode::CREATED,
        Json(json!({ "aliases": account.aliases })),
    )
        .into_response()
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
    async fn a_valid_alias_is_added() {
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
                    .method("POST")
                    .uri(format!("/api/accounts/{}/aliases", account.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"alias":"a.adams@example.com"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn an_alias_owned_by_another_account_is_a_conflict() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let alice = shared
            .directory
            .accounts()
            .create("alice", domain.id, "Alice", Role::User)
            .unwrap();
        let bob = shared
            .directory
            .accounts()
            .create("bob", domain.id, "Bob", Role::User)
            .unwrap();
        let token = admin_token(&shared);
        let app = router(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/accounts/{}/aliases", bob.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"alias":"alice@example.com"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            shared
                .directory
                .addresses()
                .resolve("alice@example.com")
                .unwrap()
                .unwrap()
                .account_id(),
            Some(alice.id)
        );
    }

    #[tokio::test]
    async fn an_invalid_alias_is_rejected() {
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
                    .method("POST")
                    .uri(format!("/api/accounts/{}/aliases", account.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"alias":"nope"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
