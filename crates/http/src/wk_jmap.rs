use axum::extract::State;
use axum::{Extension, Json};
use serde_json::Value;

use irixmail_jmap::session_resource;

use crate::app::AppState;
use crate::auth_mw::AuthIdentity;

pub async fn well_known_jmap(
    State(state): State<AppState>,
    identity: Option<Extension<AuthIdentity>>,
) -> Json<Value> {
    let (account_id, username) = match identity {
        Some(Extension(identity)) => (identity.account_id.to_string(), identity.username),
        None => ("0".to_string(), String::new()),
    };
    let webpush_key = irixmail_jmap::webpush::application_server_key(state.store.as_ref()).ok();
    Json(session_resource(
        &account_id,
        &username,
        "0",
        webpush_key.as_deref(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    use crate::tests_support::{state, TempDir};

    #[tokio::test]
    async fn the_well_known_jmap_returns_the_session_resource() {
        let dir = TempDir::new();
        let app = Router::new()
            .route("/.well-known/jmap", get(well_known_jmap))
            .with_state(state(&dir));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/.well-known/jmap")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value["capabilities"].is_object());
    }
}
