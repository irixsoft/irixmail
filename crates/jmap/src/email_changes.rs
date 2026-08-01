use serde_json::Value;

use irixmail_store::Collection;

use crate::context::JmapContext;
use crate::reply::collection_changes;
use crate::request::Invocation;

pub fn email_changes(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    collection_changes(
        ctx.store.as_ref(),
        ctx.account_id as u32,
        args,
        "Email/changes",
        call_id,
        Collection::Email,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_context;
    use irixmail_store::{ChangeKind, ChangeLog};
    use serde_json::json;

    #[test]
    fn max_changes_paginates_and_signals_has_more_changes() {
        let ctx = test_context();
        let account = ctx.account_id as u32;
        let log = ChangeLog::new(ctx.store.as_ref());
        for document in 10..15u32 {
            log.record(account, Collection::Email, document, ChangeKind::Insert)
                .unwrap();
        }

        let first = email_changes(
            &ctx,
            &json!({"accountId": "1", "sinceState": "0", "maxChanges": 2}),
            "c0",
        );
        let args = first.arguments();
        let created = args["created"].as_array().unwrap();
        assert_eq!(created, &vec![json!("10"), json!("11")]);
        assert_eq!(args["hasMoreChanges"], true);
        assert_eq!(args["newState"], "2");

        let second = email_changes(
            &ctx,
            &json!({"accountId": "1", "sinceState": "2", "maxChanges": 100}),
            "c1",
        );
        let args = second.arguments();
        let created = args["created"].as_array().unwrap();
        assert_eq!(created, &vec![json!("12"), json!("13"), json!("14")]);
        assert_eq!(args["hasMoreChanges"], false);
        assert_eq!(args["newState"], "5");
    }

    #[test]
    fn a_since_state_older_than_the_pruned_log_cannot_calculate_changes() {
        let ctx = test_context();
        let account = ctx.account_id as u32;
        let log = ChangeLog::new(ctx.store.as_ref());
        for document in 0..10u32 {
            log.record(account, Collection::Email, document, ChangeKind::Insert)
                .unwrap();
        }
        log.prune(account, Collection::Email, 3).unwrap();

        let stale = email_changes(&ctx, &json!({"accountId": "1", "sinceState": "2"}), "c0");
        assert_eq!(stale.name(), "error");
        assert_eq!(stale.arguments()["type"], "cannotCalculateChanges");

        let fresh = email_changes(&ctx, &json!({"accountId": "1", "sinceState": "8"}), "c1");
        assert_eq!(fresh.name(), "Email/changes");
    }

    #[test]
    fn email_changes_collapses_the_change_log_since_a_state() {
        let ctx = test_context();
        let account = ctx.account_id as u32;
        let log = ChangeLog::new(ctx.store.as_ref());
        log.record(account, Collection::Email, 10, ChangeKind::Insert)
            .unwrap();
        log.record(account, Collection::Email, 11, ChangeKind::Insert)
            .unwrap();
        log.record(account, Collection::Email, 10, ChangeKind::Update)
            .unwrap();
        log.record(account, Collection::Email, 11, ChangeKind::Delete)
            .unwrap();

        let full = email_changes(&ctx, &json!({"accountId": "1", "sinceState": "0"}), "c0");
        let created = full.arguments()["created"].as_array().unwrap();
        assert!(
            created.contains(&json!("10")),
            "doc 10 created: {:?}",
            full.arguments()
        );
        assert!(
            !created.contains(&json!("11")),
            "doc 11 created-then-deleted cancels"
        );
        assert!(full.arguments()["destroyed"].as_array().unwrap().is_empty());
        assert_eq!(full.arguments()["newState"], "4");

        let delta = email_changes(&ctx, &json!({"accountId": "1", "sinceState": "2"}), "c1");
        assert!(delta.arguments()["updated"]
            .as_array()
            .unwrap()
            .contains(&json!("10")));
        assert!(delta.arguments()["destroyed"]
            .as_array()
            .unwrap()
            .contains(&json!("11")));
        assert_eq!(delta.arguments()["oldState"], "2");
    }
}
