use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use irixmail_directory::Account;

use crate::app::AppState;

pub async fn list(State(state): State<AppState>) -> Json<Value> {
    let accounts = state.directory.accounts().list().unwrap_or_default();
    let accounts: Vec<Value> = accounts.iter().map(account_json).collect();
    Json(json!({ "accounts": accounts }))
}

pub fn account_json(account: &Account) -> Value {
    let mut value = serde_json::to_value(account).unwrap_or_else(|_| json!({}));
    if let Some(fields) = value.as_object_mut() {
        fields.insert("id".into(), json!(account.id.to_string()));
        fields.insert("domain_id".into(), json!(account.domain_id.to_string()));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    use irixmail_directory::Role;

    use crate::app::router;
    use crate::tests_support::{admin_token, state, TempDir};

    #[tokio::test]
    async fn an_admin_lists_accounts() {
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
                    .uri("/api/accounts")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["accounts"][0]["local_part"], "alice");
        assert!(account.id > (1u64 << 53));
        let id = value["accounts"][0]["id"].as_str().unwrap();
        assert_eq!(id, account.id.to_string());
        assert_eq!(id.parse::<u64>().unwrap(), account.id);
        assert_eq!(
            value["accounts"][0]["domain_id"].as_str().unwrap(),
            domain.id.to_string()
        );
    }
}
