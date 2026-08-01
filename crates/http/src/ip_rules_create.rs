use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use irixmail_directory::IpAction;

use crate::app::{error_response, AppState};
use crate::ip_rules_list::rule_json;
use crate::validate::bad_request;

#[derive(Deserialize)]
pub struct CreateBody {
    pub cidr: String,
    pub action: IpAction,
}

pub async fn create(State(state): State<AppState>, Json(body): Json<CreateBody>) -> Response {
    match state.directory.ip_rules().create(&body.cidr, body.action) {
        Ok(rule) => (StatusCode::CREATED, Json(rule_json(&rule))).into_response(),
        Err(irixmail_core::Error::InvalidInput(reason)) => bad_request(&reason),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not create the IP rule",
        ),
    }
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    use crate::app::router;
    use crate::tests_support::{admin_token, state, TempDir};

    fn request(token: &str, method: &str, uri: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn body_json(response: axum::http::Response<Body>) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn ip_rules_are_created_listed_and_deleted_against_the_store() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let directory = shared.directory.clone();
        let token = admin_token(&shared);
        let app = router(shared);

        let created = app
            .clone()
            .oneshot(request(
                &token,
                "POST",
                "/api/ip-rules",
                r#"{"cidr":"203.0.113.0/24","action":"block"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let created = body_json(created).await;
        assert_eq!(created["cidr"], "203.0.113.0/24");
        assert_eq!(created["action"], "block");
        let id = created["id"].as_str().unwrap().to_string();

        let listed = app
            .clone()
            .oneshot(request(&token, "GET", "/api/ip-rules", ""))
            .await
            .unwrap();
        let listed = body_json(listed).await;
        assert_eq!(listed["rules"][0]["id"], serde_json::json!(id));

        assert_eq!(directory.ip_rules().list().unwrap().len(), 1);

        let deleted = app
            .clone()
            .oneshot(request(
                &token,
                "DELETE",
                &format!("/api/ip-rules/{id}"),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
        assert!(directory.ip_rules().list().unwrap().is_empty());

        let missing = app
            .oneshot(request(
                &token,
                "DELETE",
                &format!("/api/ip-rules/{id}"),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn an_invalid_cidr_is_a_bad_request() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(request(
                &token,
                "POST",
                "/api/ip-rules",
                r#"{"cidr":"not-an-ip","action":"block"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
