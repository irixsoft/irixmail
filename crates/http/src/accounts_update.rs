use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use irixmail_directory::Role;

use crate::accounts_list::account_json;
use crate::app::{error_response, AppState};
use crate::validate::parse_id;

#[derive(Deserialize)]
pub struct UpdateBody {
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    pub enabled: Option<bool>,
    pub role: Option<String>,
    #[serde(rename = "quotaBytes")]
    pub quota_bytes: Option<u64>,
    #[serde(rename = "quotaMessages")]
    pub quota_messages: Option<u64>,
    pub signature: Option<String>,
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateBody>,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return error_response(StatusCode::NOT_FOUND, "account not found");
    };
    let mut account = match state.directory.accounts().get(id) {
        Ok(account) => account,
        Err(_) => return error_response(StatusCode::NOT_FOUND, "account not found"),
    };
    if let Some(value) = body.display_name {
        account.display_name = value;
    }
    if let Some(value) = body.enabled {
        account.enabled = value;
    }
    if let Some(value) = body.role {
        account.role = if value.eq_ignore_ascii_case("admin") {
            Role::Admin
        } else {
            Role::User
        };
    }
    if let Some(value) = body.quota_bytes {
        account.quota_bytes = value;
    }
    if let Some(value) = body.quota_messages {
        account.quota_messages = value;
    }
    if let Some(value) = body.signature {
        account.signature = value;
    }
    match state.directory.accounts().update(account.clone()) {
        Ok(()) => Json(json!({ "account": account_json(&account) })).into_response(),
        Err(irixmail_core::Error::InvalidInput(_)) => {
            error_response(StatusCode::CONFLICT, "address already in use")
        }
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not update account",
        ),
    }
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
    async fn the_display_name_and_quota_are_updated() {
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
                    .method("PUT")
                    .uri(format!("/api/accounts/{}", account.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"displayName":"Alice A","quotaBytes":1024}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["account"]["display_name"], "Alice A");
        assert_eq!(value["account"]["quota_bytes"], 1024);
        assert_eq!(
            value["account"]["id"].as_str().unwrap(),
            account.id.to_string()
        );
    }
}
