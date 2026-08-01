use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use irixmail_directory::app_password;

use crate::app::{error_response, AppState};
use crate::validate::parse_id;

#[derive(Deserialize)]
pub struct CreateBody {
    #[serde(default)]
    pub name: String,
}

pub async fn create(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CreateBody>,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return error_response(StatusCode::NOT_FOUND, "account not found");
    };
    if state.directory.accounts().get(id).is_err() {
        return error_response(StatusCode::NOT_FOUND, "account not found");
    }
    let name = if body.name.trim().is_empty() {
        "app password".to_string()
    } else {
        body.name
    };
    let Ok(generated) = app_password::generate(rand::random::<u64>(), &name, now_millis()) else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not generate app password",
        );
    };
    if state
        .directory
        .credentials()
        .add_app_password(id, generated.record.clone())
        .is_err()
    {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not store the app password",
        );
    }
    (
        StatusCode::CREATED,
        Json(json!({
            "id": generated.record.id.to_string(),
            "name": generated.record.name,
            "password": generated.plaintext,
        })),
    )
        .into_response()
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request};
    use tower::ServiceExt;

    use irixmail_directory::Role;

    use crate::app::router;
    use crate::tests_support::{admin_token, state, TempDir};

    #[tokio::test]
    async fn an_app_password_is_revealed_once() {
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
        let app = router(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/accounts/{}/app-passwords", account.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"iPhone"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["name"], "iPhone");
        let id = value["id"].as_str().unwrap();
        assert!(id.parse::<u64>().is_ok());
        let plaintext = value["password"].as_str().unwrap().to_string();
        assert!(!plaintext.is_empty());

        let stored = shared
            .directory
            .credentials()
            .list_app_passwords(account.id)
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].name, "iPhone");
        assert!(irixmail_directory::app_password::verify(&plaintext, &stored[0]).unwrap());
    }
}
