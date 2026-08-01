use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::app::{error_response, AppState};
use crate::validate::parse_id;

pub async fn delete(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(id) = parse_id(&id) else {
        return error_response(StatusCode::NOT_FOUND, "account not found");
    };
    if state.directory.accounts().get(id).is_err() {
        return error_response(StatusCode::NOT_FOUND, "account not found");
    }
    if state.directory.credentials().remove_all(id).is_err() {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not delete account",
        );
    }
    match state.directory.accounts().delete(id) {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not delete account",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt;

    use irixmail_directory::Role;

    use crate::app::router;
    use crate::tests_support::{admin_token, state, TempDir};

    #[tokio::test]
    async fn an_existing_account_is_deleted() {
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
        shared
            .directory
            .credentials()
            .set_primary_password(account.id, "$argon2id$primary")
            .unwrap();
        let token = admin_token(&shared);
        let app = router(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/accounts/{}", account.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(shared.directory.accounts().get(account.id).is_err());
        assert!(shared
            .directory
            .credentials()
            .list(account.id)
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn a_failed_credential_removal_is_not_swallowed() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        use irixmail_core::{IdGenerator, LogBuffer, Result};
        use irixmail_directory::Directory;
        use irixmail_store::{
            BlobStore, ChangeNotifier, Flow, FsBlobStore, KeyPrefix, RocksdbStore, Store, WriteOp,
        };

        const TAG_CREDENTIAL: u8 = 0x22;

        struct CredentialFailStore {
            inner: RocksdbStore,
            armed: AtomicBool,
        }

        impl Store for CredentialFailStore {
            fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
                self.inner.get(key)
            }
            fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
                self.inner.put(key, value)
            }
            fn delete(&self, key: &[u8]) -> Result<()> {
                self.inner.delete(key)
            }
            fn iterate(
                &self,
                prefix: &KeyPrefix,
                visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
            ) -> Result<()> {
                self.inner.iterate(prefix, visit)
            }
            fn batch(&self, ops: &[WriteOp]) -> Result<()> {
                if self.armed.load(Ordering::SeqCst) {
                    let touches_credentials = ops.iter().any(|op| {
                        let key = match op {
                            WriteOp::Set { key, .. }
                            | WriteOp::Delete { key }
                            | WriteOp::Add { key, .. } => key,
                        };
                        key.len() >= 2
                            && key[0] == irixmail_store::Subspace::Registry.as_byte()
                            && key[1] == TAG_CREDENTIAL
                    });
                    if touches_credentials {
                        return Err(irixmail_core::Error::store("injected credential failure"));
                    }
                }
                self.inner.batch(ops)
            }
            fn add_and_get(&self, key: &[u8], by: i64) -> Result<i64> {
                self.inner.add_and_get(key, by)
            }
            fn counter(&self, key: &[u8]) -> Result<i64> {
                self.inner.counter(key)
            }
        }

        let dir = TempDir::new();
        let flaky = Arc::new(CredentialFailStore {
            inner: RocksdbStore::open(dir.path.join("db")).unwrap(),
            armed: AtomicBool::new(false),
        });
        let store: Arc<dyn Store> = flaky.clone();
        let directory = Directory::new(Arc::clone(&store), Arc::new(IdGenerator::new(0)), None);
        let blobs: Arc<dyn BlobStore> =
            Arc::new(FsBlobStore::open(dir.path.join("blobs")).unwrap());
        let shared = crate::app::AppState::new(
            directory,
            LogBuffer::new(),
            store,
            blobs,
            Arc::new(ChangeNotifier::new()),
            "mail.example.com",
            irixmail_dns::Resolver::empty(),
            crate::tests_support::test_cipher(),
        );

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
        shared
            .directory
            .credentials()
            .set_primary_password(account.id, "$argon2id$primary")
            .unwrap();
        let token = admin_token(&shared);
        flaky.armed.store(true, Ordering::SeqCst);

        let app = router(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/accounts/{}", account.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            shared.directory.accounts().get(account.id).is_ok(),
            "the account must survive so the delete can be retried"
        );
        assert!(!shared
            .directory
            .credentials()
            .list(account.id)
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn deleting_an_unknown_account_is_not_found() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/accounts/999")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
