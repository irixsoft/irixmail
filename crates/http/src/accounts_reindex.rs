use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::app::{error_response, AppState};
use crate::validate::parse_id;

pub async fn reindex(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(id) = parse_id(&id) else {
        return error_response(StatusCode::NOT_FOUND, "account not found");
    };
    if state.directory.accounts().get(id).is_err() {
        return error_response(StatusCode::NOT_FOUND, "account not found");
    }
    let store = state.store.clone();
    let blobs = state.blobs.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        irixmail_mail::reindex_account(store.as_ref(), blobs.as_ref(), id as u32)
    })
    .await;
    match outcome {
        Ok(Ok(reindexed)) => {
            (StatusCode::OK, Json(json!({ "reindexed": reindexed }))).into_response()
        }
        _ => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not rebuild the search index",
        ),
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    use irixmail_directory::Role;
    use irixmail_mail::{append_message, AppendRequest, Mailbox, SpecialUse};
    use irixmail_store::{Collection, Flow, FtsIndex, KeyPrefix, Query, Subspace};

    use crate::app::router;
    use crate::tests_support::{admin_token, state, TempDir};

    const MESSAGE: &[u8] = concat!(
        "From: alice@example.com\r\n",
        "Subject: Quarterly invoice\r\n",
        "\r\n",
        "Please review the attached invoice.\r\n",
    )
    .as_bytes();

    #[tokio::test]
    async fn reindex_rebuilds_a_wiped_search_index_over_the_api() {
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
        let inbox = Mailbox::new(1, "Inbox", SpecialUse::Inbox, 1);
        append_message(
            shared.store.as_ref(),
            shared.blobs.as_ref(),
            &shared.notifier,
            &AppendRequest {
                account: &account,
                mailbox: &inbox,
                flags: Vec::new(),
                received_at: 1_700_000_000,
                document_id: 10,
                raw: MESSAGE,
            },
        )
        .unwrap();

        let account_id = account.id as u32;
        let prefix = KeyPrefix::collection(Subspace::Index, account_id, Collection::Email);
        let mut keys = Vec::new();
        shared
            .store
            .iterate(&prefix, &mut |key, _| {
                keys.push(key.to_vec());
                Ok(Flow::Continue)
            })
            .unwrap();
        assert!(!keys.is_empty());
        for key in keys {
            shared.store.delete(&key).unwrap();
        }

        let token = admin_token(&shared);
        let app = router(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/accounts/{}/reindex", account.id))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let hits = FtsIndex::new(shared.store.as_ref())
            .search(
                account_id,
                Collection::Email,
                &Query::term("invoice"),
                &[10],
            )
            .unwrap();
        assert_eq!(hits, vec![10]);
    }

    #[tokio::test]
    async fn reindexing_an_unknown_account_is_not_found() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = admin_token(&shared);
        let app = router(shared);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/accounts/424242/reindex")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
