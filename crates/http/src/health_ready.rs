use std::sync::atomic::Ordering;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::app::AppState;

pub async fn ready(State(state): State<AppState>) -> Response {
    let started = state.ready.load(Ordering::Relaxed);
    let store_reachable = state
        .store
        .get(&irixmail_store::schema_version_key())
        .is_ok();
    if started && store_reachable {
        Json(json!({ "status": "ok" })).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "status": "unavailable" })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::app::router;
    use crate::tests_support::{state, TempDir};

    #[tokio::test]
    async fn readiness_is_public_and_ok() {
        let dir = TempDir::new();
        let app = router(state(&dir));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/healthz/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn readiness_is_refused_before_the_server_has_started() {
        let dir = TempDir::new();
        let shared = state(&dir);
        shared
            .ready
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/healthz/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn readiness_is_refused_when_the_store_probe_fails() {
        use std::sync::Arc;

        use irixmail_core::Result;
        use irixmail_store::{Flow, KeyPrefix, Store, WriteOp};

        struct FailingStore;

        impl Store for FailingStore {
            fn get(&self, _key: &[u8]) -> Result<Option<Vec<u8>>> {
                Err(irixmail_core::Error::store("the store is unreachable"))
            }
            fn put(&self, _key: &[u8], _value: &[u8]) -> Result<()> {
                Err(irixmail_core::Error::store("the store is unreachable"))
            }
            fn delete(&self, _key: &[u8]) -> Result<()> {
                Err(irixmail_core::Error::store("the store is unreachable"))
            }
            fn iterate(
                &self,
                _prefix: &KeyPrefix,
                _visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
            ) -> Result<()> {
                Err(irixmail_core::Error::store("the store is unreachable"))
            }
            fn batch(&self, _ops: &[WriteOp]) -> Result<()> {
                Err(irixmail_core::Error::store("the store is unreachable"))
            }
            fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
                Err(irixmail_core::Error::store("the store is unreachable"))
            }
            fn counter(&self, _key: &[u8]) -> Result<i64> {
                Err(irixmail_core::Error::store("the store is unreachable"))
            }
        }

        let dir = TempDir::new();
        let mut shared = state(&dir);
        shared.store = Arc::new(FailingStore);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/healthz/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
