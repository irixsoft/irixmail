use axum::Json;
use serde_json::{json, Value};

pub async fn live() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::app::router;
    use crate::tests_support::{state, TempDir};

    #[tokio::test]
    async fn liveness_is_public_and_ok() {
        let dir = TempDir::new();
        let app = router(state(&dir));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/healthz/live")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
