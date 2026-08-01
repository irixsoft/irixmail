use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use irixmail_tls::{import_pem, CertSource};

use crate::app::{error_response, AppState};
use crate::validate::bad_request;

#[derive(Deserialize)]
pub struct UploadBody {
    pub certificate: String,
    #[serde(rename = "privateKey")]
    pub private_key: String,
}

pub async fn upload(State(state): State<AppState>, Json(body): Json<UploadBody>) -> Response {
    let handles = match state.tls.as_ref() {
        Some(handles) => handles,
        None => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "TLS management is not available",
            )
        }
    };
    let (Some(provider), Some(cert_store)) = (&handles.provider, &handles.cert_store) else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "TLS management is not available",
        );
    };
    let material = match import_pem(provider.clone(), &body.certificate, &body.private_key) {
        Ok(material) => material,
        Err(err) => return bad_request(&err.to_string()),
    };
    if let Err(err) = cert_store.save(&state.hostname, &material, CertSource::Uploaded) {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, &err.to_string());
    }
    if let Some(resolver) = &handles.sni_resolver {
        if let Err(err) = resolver.set(&material) {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, &err.to_string());
        }
    }
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "status": "custom" })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt;

    use irixmail_tls::rustls::crypto::aws_lc_rs;
    use irixmail_tls::{CertStore, SniResolver};

    use crate::app::{router, AppState, TlsHandles};
    use crate::tests_support::{admin_token, state, TempDir};

    fn post(token: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/tls/upload")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn pem_pair(hostname: &str) -> (String, String) {
        let certified = rcgen::generate_simple_self_signed(vec![hostname.to_string()]).unwrap();
        (certified.cert.pem(), certified.key_pair.serialize_pem())
    }

    fn upload_body(cert: &str, key: &str) -> String {
        serde_json::json!({ "certificate": cert, "privateKey": key }).to_string()
    }

    fn wired(dir: &TempDir) -> (AppState, Arc<CertStore>, Arc<SniResolver>) {
        let mut shared = state(dir);
        let provider = Arc::new(aws_lc_rs::default_provider());
        let cert_store = Arc::new(CertStore::new(dir.path.join("certs")));
        let resolver = Arc::new(SniResolver::new(provider.clone()));
        shared.tls = Some(TlsHandles {
            cert_store: Some(cert_store.clone()),
            provider: Some(provider),
            sni_resolver: Some(resolver.clone()),
            ..TlsHandles::default()
        });
        (shared, cert_store, resolver)
    }

    #[tokio::test]
    async fn a_matching_pair_is_saved_and_hot_reloaded() {
        let dir = TempDir::new();
        let (shared, cert_store, resolver) = wired(&dir);
        let hostname = shared.hostname.clone();
        let (cert, key) = pem_pair(&hostname);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(post(&token, &upload_body(&cert, &key)))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(cert_store.load(&hostname).unwrap().is_some());
        assert!(resolver.has_certificate());
    }

    #[tokio::test]
    async fn a_mismatched_key_is_rejected_and_nothing_is_saved() {
        let dir = TempDir::new();
        let (shared, cert_store, _resolver) = wired(&dir);
        let hostname = shared.hostname.clone();
        let (cert, _) = pem_pair(&hostname);
        let (_, other_key) = pem_pair(&hostname);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(post(&token, &upload_body(&cert, &other_key)))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(cert_store.load(&hostname).unwrap().is_none());
    }

    #[tokio::test]
    async fn upload_without_tls_management_is_unavailable() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let (cert, key) = pem_pair("mail.example.com");
        let response = app
            .oneshot(post(&token, &upload_body(&cert, &key)))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
