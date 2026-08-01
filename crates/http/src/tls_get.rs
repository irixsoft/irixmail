use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use irixmail_tls::{inspect, CertSource};

use crate::app::AppState;

pub async fn get(State(state): State<AppState>) -> Json<Value> {
    let store = state.tls.as_ref().and_then(|tls| tls.cert_store.as_ref());
    let summary = store
        .and_then(|store| store.load(&state.hostname).ok().flatten())
        .and_then(|material| inspect(&material));
    let source = store.and_then(|store| store.source(&state.hostname));
    match summary {
        Some(summary) => Json(json!({
            "status": if summary.self_signed {
                "self-signed"
            } else if source == Some(CertSource::Acme) {
                "acme"
            } else {
                "custom"
            },
            "issuer": summary.issuer,
            "sans": summary.sans,
            "expiresAt": summary.not_after * 1000,
        })),
        None => Json(json!({
            "status": "none",
            "issuer": Value::Null,
            "sans": [],
            "expiresAt": Value::Null,
        })),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    use irixmail_tls::CertStore;

    use crate::app::{router, TlsHandles};
    use crate::tests_support::{admin_token, state, TempDir};

    #[tokio::test]
    async fn the_tls_status_is_returned() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/tls")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn the_tls_status_reports_the_stored_certificate_sans_and_expiry() {
        let dir = TempDir::new();
        let mut shared = state(&dir);
        let store = CertStore::new(dir.path.join("certs"));
        let material = irixmail_tls::self_signed::generate(vec![shared.hostname.clone()]).unwrap();
        store
            .save(
                &shared.hostname,
                &material,
                irixmail_tls::CertSource::SelfSigned,
            )
            .unwrap();
        shared.tls = Some(TlsHandles {
            cert_store: Some(Arc::new(store)),
            ..TlsHandles::default()
        });
        let token = admin_token(&shared);
        let hostname = shared.hostname.clone();
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/tls")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["status"], "self-signed");
        let sans: Vec<String> = value["sans"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry.as_str().unwrap().to_string())
            .collect();
        assert!(sans.contains(&hostname));
        let not_after = irixmail_tls::inspect(&material).unwrap().not_after;
        assert_eq!(value["expiresAt"], not_after * 1000);
    }

    fn ca_signed(hostname: &str) -> irixmail_tls::CertMaterial {
        use rustls::pki_types::PrivateKeyDer;

        let ca_key = rcgen::KeyPair::generate().unwrap();
        let mut ca_params = rcgen::CertificateParams::new(Vec::new()).unwrap();
        ca_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "IRIX Test CA");
        let ca_cert = ca_params.self_signed(&ca_key).unwrap();
        let leaf_key = rcgen::KeyPair::generate().unwrap();
        let leaf = rcgen::CertificateParams::new(vec![hostname.to_string()])
            .unwrap()
            .signed_by(&leaf_key, &ca_cert, &ca_key)
            .unwrap();
        irixmail_tls::CertMaterial {
            chain: vec![leaf.der().clone()],
            key: PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
        }
    }

    async fn status_of(shared: crate::app::AppState) -> serde_json::Value {
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/tls")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn an_acme_certificate_reports_the_acme_status() {
        let dir = TempDir::new();
        let mut shared = state(&dir);
        let store = CertStore::new(dir.path.join("certs"));
        let material = ca_signed(&shared.hostname);
        store
            .save(&shared.hostname, &material, irixmail_tls::CertSource::Acme)
            .unwrap();
        shared.tls = Some(TlsHandles {
            cert_store: Some(Arc::new(store)),
            ..TlsHandles::default()
        });

        let value = status_of(shared).await;
        assert_eq!(value["status"], "acme");
        assert!(value["issuer"].as_str().unwrap().contains("IRIX Test CA"));
    }

    #[tokio::test]
    async fn a_certificate_without_provenance_reports_custom() {
        let dir = TempDir::new();
        let mut shared = state(&dir);
        let store = CertStore::new(dir.path.join("certs"));
        let material = ca_signed(&shared.hostname);
        store
            .save(&shared.hostname, &material, irixmail_tls::CertSource::Acme)
            .unwrap();
        std::fs::remove_file(
            dir.path
                .join("certs")
                .join(format!("{}.source", shared.hostname)),
        )
        .unwrap();
        shared.tls = Some(TlsHandles {
            cert_store: Some(Arc::new(store)),
            ..TlsHandles::default()
        });

        let value = status_of(shared).await;
        assert_eq!(value["status"], "custom");
    }
}
