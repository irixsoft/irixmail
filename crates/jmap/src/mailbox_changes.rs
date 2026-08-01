use serde_json::Value;

use irixmail_store::Collection;

use crate::context::JmapContext;
use crate::reply::collection_changes;
use crate::request::Invocation;

pub fn mailbox_changes(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    collection_changes(
        ctx.store.as_ref(),
        ctx.account_id as u32,
        args,
        "Mailbox/changes",
        call_id,
        Collection::Mailbox,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_context;
    use irixmail_store::{ChangeKind, ChangeLog};
    use serde_json::json;

    #[test]
    fn mailbox_changes_reports_created_and_destroyed_since_a_state() {
        let ctx = test_context();
        let account = ctx.account_id as u32;
        let log = ChangeLog::new(ctx.store.as_ref());
        log.record(account, Collection::Mailbox, 6, ChangeKind::Insert)
            .unwrap();
        log.record(account, Collection::Mailbox, 7, ChangeKind::Insert)
            .unwrap();
        log.record(account, Collection::Mailbox, 6, ChangeKind::Delete)
            .unwrap();

        let response = mailbox_changes(&ctx, &json!({"accountId": "1", "sinceState": "0"}), "c0");
        assert!(response.arguments()["created"]
            .as_array()
            .unwrap()
            .contains(&json!("7")));
        assert!(!response.arguments()["created"]
            .as_array()
            .unwrap()
            .contains(&json!("6")));
        assert_eq!(response.arguments()["newState"], "3");
        assert_eq!(response.arguments()["hasMoreChanges"], false);
    }
}
