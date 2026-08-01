use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};

use irixmail_dns::{mtasts_policy, MtaStsMode};

use crate::app::AppState;

const MAX_AGE: u32 = 604_800;

pub async fn mta_sts(State(state): State<AppState>) -> Response {
    let policy = mtasts_policy(&state.hostname, MtaStsMode::Enforce, MAX_AGE);
    ([(header::CONTENT_TYPE, "text/plain")], policy).into_response()
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
    async fn the_policy_names_the_mail_host() {
        let dir = TempDir::new();
        let app = Router::new()
            .route("/.well-known/mta-sts.txt", get(mta_sts))
            .with_state(state(&dir));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/.well-known/mta-sts.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("version: STSv1"));
        assert!(text.contains("mx: mail.example.com"));
    }
}
