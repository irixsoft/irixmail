use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use irixmail_directory::{password, Role};

use crate::accounts_list::account_json;
use crate::app::{error_response, AppState};
use crate::validate::{bad_request, parse_id};

#[derive(Deserialize)]
pub struct CreateBody {
    #[serde(rename = "localPart")]
    pub local_part: String,
    #[serde(rename = "domainId")]
    pub domain_id: String,
    #[serde(rename = "displayName", default)]
    pub display_name: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub password: String,
}

pub async fn create(State(state): State<AppState>, Json(body): Json<CreateBody>) -> Response {
    if body.local_part.trim().is_empty() {
        return bad_request("a local part is required");
    }
    let Some(domain_id) = parse_id(&body.domain_id) else {
        return bad_request("unknown domain");
    };
    if state.directory.domains().get(domain_id).is_err() {
        return bad_request("unknown domain");
    }
    let role = match body.role.as_deref() {
        Some(value) if value.eq_ignore_ascii_case("admin") => Role::Admin,
        _ => Role::User,
    };
    let account = match state.directory.accounts().create_with_extra_ops(
        &body.local_part,
        domain_id,
        &body.display_name,
        role,
        |id, created_at| irixmail_mail::provision_ops(id as u32, created_at),
    ) {
        Ok(account) => account,
        Err(_) => {
            return error_response(StatusCode::CONFLICT, "account already exists or is invalid")
        }
    };
    if !body.password.trim().is_empty() {
        let Ok(hash) = password::hash(&body.password) else {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not hash the password",
            );
        };
        if state
            .directory
            .credentials()
            .set_primary_password(account.id, hash)
            .is_err()
        {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not store the password",
            );
        }
    }
    (
        StatusCode::CREATED,
        Json(json!({ "account": account_json(&account) })),
    )
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
    async fn an_account_is_created_in_an_existing_domain() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let token = admin_token(&shared);
        let app = router(shared);
        let body = format!(
            r#"{{"localPart":"alice","domainId":"{}","displayName":"Alice"}}"#,
            domain.id
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/accounts")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let id = value["account"]["id"].as_str().unwrap();
        assert!(id.parse::<u64>().unwrap() > (1u64 << 53));
        assert_eq!(
            value["account"]["domain_id"].as_str().unwrap(),
            domain.id.to_string()
        );
    }

    #[tokio::test]
    async fn a_non_numeric_domain_id_is_rejected() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/accounts")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"localPart":"alice","domainId":"nope"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_supplied_password_is_stored_as_a_credential() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let token = admin_token(&shared);
        let app = router(shared.clone());
        let body = format!(
            r#"{{"localPart":"alice","domainId":"{}","password":"correct horse"}}"#,
            domain.id
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/accounts")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let account = shared
            .directory
            .accounts()
            .get_by_address("alice", domain.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            shared
                .directory
                .credentials()
                .list(account.id)
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn creating_an_account_provisions_its_five_mailbox_rows() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let token = admin_token(&shared);
        let app = router(shared.clone());
        let body = format!(r#"{{"localPart":"alice","domainId":"{}"}}"#, domain.id);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/accounts")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let account = shared
            .directory
            .accounts()
            .get_by_address("alice", domain.id)
            .unwrap()
            .unwrap();
        let mailboxes =
            irixmail_mail::load_mailboxes(shared.store.as_ref(), account.id as u32).unwrap();
        let names: Vec<&str> = mailboxes.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["Inbox", "Sent", "Drafts", "Trash", "Spam"]);
    }

    #[tokio::test]
    async fn an_unknown_domain_is_rejected() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/accounts")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"localPart":"alice","domainId":"999"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
