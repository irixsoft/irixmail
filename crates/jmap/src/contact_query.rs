use serde_json::{json, Value};

use irixmail_store::Collection;

use crate::context::JmapContext;
use crate::reply::{account_id, collection_state, dav_store, now_millis};
use crate::request::Invocation;

pub fn contact_query(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let store = dav_store(ctx);
    let _ = store.ensure_defaults(now_millis());

    let filter = args.get("filter").filter(|value| !value.is_null());
    let text = filter
        .and_then(|filter| filter.get("text"))
        .and_then(Value::as_str)
        .map(str::to_lowercase)
        .filter(|text| !text.is_empty());
    let book = filter
        .and_then(|filter| filter.get("inAddressBook"))
        .and_then(Value::as_str)
        .and_then(|id| id.parse::<u32>().ok());

    let mut matched: Vec<(String, u32)> = store
        .list_cards(None)
        .unwrap_or_default()
        .into_iter()
        .filter(|record| book.map(|id| record.book_id == id).unwrap_or(true))
        .filter(|record| match &text {
            Some(needle) => {
                record.full_name.to_lowercase().contains(needle)
                    || record
                        .emails
                        .iter()
                        .any(|email| email.to_lowercase().contains(needle))
                    || record
                        .uid
                        .as_deref()
                        .map(|uid| uid.to_lowercase().contains(needle))
                        .unwrap_or(false)
            }
            None => true,
        })
        .map(|record| (record.full_name.to_lowercase(), record.id))
        .collect();
    matched.sort();

    let ids: Vec<Value> = matched
        .iter()
        .map(|(_, id)| Value::String(id.to_string()))
        .collect();

    Invocation::new(
        "ContactCard/query",
        json!({
            "accountId": account_id(args),
            "queryState": collection_state(ctx.store.as_ref(), ctx.account_id as u32, Collection::ContactCard),
            "canCalculateChanges": false,
            "position": 0,
            "ids": ids,
            "total": matched.len(),
        }),
        call_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contact_get::addressbook_get;
    use crate::contact_set::contact_set;
    use crate::context::test_context;

    fn default_book(ctx: &JmapContext) -> String {
        addressbook_get(ctx, &json!({"accountId": "1"}), "c0").arguments()["list"][0]["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn create(ctx: &JmapContext, object: Value) -> String {
        contact_set(
            ctx,
            &json!({"accountId": "1", "create": {"a": object}}),
            "c0",
        )
        .arguments()["created"]["a"]["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn cards_come_back_sorted_by_full_name() {
        let ctx = test_context();
        let book = default_book(&ctx);
        let zoe = create(&ctx, json!({"addressBookId": book, "fullName": "Zoe Zulu"}));
        let ada = create(
            &ctx,
            json!({"addressBookId": book, "fullName": "ada lovelace"}),
        );
        let response = contact_query(&ctx, &json!({"accountId": "1"}), "c1");
        assert_eq!(response.arguments()["ids"], json!([ada, zoe]));
        assert_eq!(response.arguments()["total"], 2);
    }

    #[test]
    fn a_text_filter_matches_names_and_emails_case_insensitively() {
        let ctx = test_context();
        let book = default_book(&ctx);
        let ada = create(
            &ctx,
            json!({
                "addressBookId": book,
                "fullName": "Ada Lovelace",
                "emails": [{"value": "ada@example.com", "label": ""}],
            }),
        );
        create(&ctx, json!({"addressBookId": book, "fullName": "Zoe Zulu"}));

        let by_name = contact_query(
            &ctx,
            &json!({"accountId": "1", "filter": {"text": "LOVELACE"}}),
            "c1",
        );
        assert_eq!(by_name.arguments()["ids"], json!([ada]));

        let by_email = contact_query(
            &ctx,
            &json!({"accountId": "1", "filter": {"text": "ada@Example"}}),
            "c2",
        );
        assert_eq!(by_email.arguments()["ids"], json!([ada]));

        let no_match = contact_query(
            &ctx,
            &json!({"accountId": "1", "filter": {"text": "nobody"}}),
            "c3",
        );
        assert_eq!(no_match.arguments()["ids"], json!([]));
    }

    #[test]
    fn an_address_book_filter_excludes_other_books() {
        let ctx = test_context();
        let book = default_book(&ctx);
        create(&ctx, json!({"addressBookId": book, "fullName": "Ada"}));
        let response = contact_query(
            &ctx,
            &json!({"accountId": "1", "filter": {"inAddressBook": "999"}}),
            "c1",
        );
        assert_eq!(response.arguments()["ids"], json!([]));
        assert_eq!(response.arguments()["total"], 0);
    }
}
