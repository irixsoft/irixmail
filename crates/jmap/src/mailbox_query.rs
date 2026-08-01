use serde_json::{json, Value};

use irixmail_mail::{load_mailboxes, provision_mailboxes, Mailbox, SpecialUse};

use irixmail_store::Collection;

use crate::context::JmapContext;
use crate::reply::{account_id, collection_state};
use crate::request::{method_error, Invocation};

const DEFAULT_LIMIT: usize = 256;

#[derive(Default)]
struct MailboxFilter {
    name: Option<String>,
    role: Option<Option<SpecialUse>>,
    has_any_role: Option<bool>,
    parent_id: Option<Option<u32>>,
}

impl MailboxFilter {
    fn matches(&self, mailbox: &Mailbox) -> bool {
        if let Some(name) = &self.name {
            if !mailbox.name.to_lowercase().contains(name) {
                return false;
            }
        }
        if let Some(role) = &self.role {
            match role {
                Some(wanted) => {
                    if mailbox.role != *wanted {
                        return false;
                    }
                }
                None => {
                    if mailbox.role != SpecialUse::None {
                        return false;
                    }
                }
            }
        }
        if let Some(wants_role) = self.has_any_role {
            if (mailbox.role != SpecialUse::None) != wants_role {
                return false;
            }
        }
        if let Some(Some(_)) = self.parent_id {
            return false;
        }
        true
    }
}

fn parse_filter(args: &Value) -> Option<MailboxFilter> {
    let mut filter = MailboxFilter::default();
    let Some(conditions) = args.get("filter").filter(|value| !value.is_null()) else {
        return Some(filter);
    };
    for (key, value) in conditions.as_object()? {
        match key.as_str() {
            "name" => filter.name = Some(value.as_str()?.to_lowercase()),
            "role" => {
                filter.role = Some(match value {
                    Value::Null => None,
                    Value::String(role) => Some(role_from_jmap(role)?),
                    _ => return None,
                })
            }
            "hasAnyRole" => filter.has_any_role = Some(value.as_bool()?),
            "parentId" => {
                filter.parent_id = Some(match value {
                    Value::Null => None,
                    Value::String(id) => Some(id.parse::<u32>().ok()?),
                    _ => return None,
                })
            }
            _ => return None,
        }
    }
    Some(filter)
}

fn role_from_jmap(name: &str) -> Option<SpecialUse> {
    match name {
        "inbox" => Some(SpecialUse::Inbox),
        "sent" => Some(SpecialUse::Sent),
        "drafts" => Some(SpecialUse::Drafts),
        "trash" => Some(SpecialUse::Trash),
        "junk" => Some(SpecialUse::Junk),
        "archive" => Some(SpecialUse::Archive),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum SortProperty {
    Name,
    SortOrder,
}

fn parse_sort(args: &Value) -> Option<Vec<(SortProperty, bool)>> {
    let Some(sort) = args.get("sort").filter(|value| !value.is_null()) else {
        return Some(Vec::new());
    };
    let mut comparators = Vec::new();
    for comparator in sort.as_array()? {
        let property = match comparator.get("property").and_then(Value::as_str)? {
            "name" => SortProperty::Name,
            "sortOrder" => SortProperty::SortOrder,
            _ => return None,
        };
        let ascending = comparator
            .get("isAscending")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        comparators.push((property, ascending));
    }
    Some(comparators)
}

fn sort_rows(rows: &mut [(usize, Mailbox)], comparators: &[(SortProperty, bool)]) {
    rows.sort_by(|(order_a, a), (order_b, b)| {
        for (property, ascending) in comparators {
            let ordering = match property {
                SortProperty::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortProperty::SortOrder => order_a.cmp(order_b),
            };
            let ordering = if *ascending {
                ordering
            } else {
                ordering.reverse()
            };
            if ordering != std::cmp::Ordering::Equal {
                return ordering;
            }
        }
        order_a.cmp(order_b)
    });
}

pub(crate) fn query_ids(ctx: &JmapContext, args: &Value) -> Result<Vec<u32>, &'static str> {
    let filter = parse_filter(args).ok_or("unsupportedFilter")?;
    let comparators = parse_sort(args).ok_or("unsupportedSort")?;

    let mailboxes = match load_mailboxes(ctx.store.as_ref(), ctx.account_id as u32) {
        Ok(rows) if !rows.is_empty() => rows,
        _ => provision_mailboxes(0),
    };
    let mut rows: Vec<(usize, Mailbox)> = mailboxes
        .into_iter()
        .enumerate()
        .filter(|(_, mailbox)| filter.matches(mailbox))
        .collect();
    sort_rows(&mut rows, &comparators);
    Ok(rows.into_iter().map(|(_, mailbox)| mailbox.id).collect())
}

pub fn mailbox_query(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let rows = match query_ids(ctx, args) {
        Ok(rows) => rows,
        Err(kind) => return method_error(kind, call_id),
    };

    let total = rows.len();
    let position = args.get("position").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_LIMIT);
    let ids: Vec<Value> = rows
        .into_iter()
        .skip(position)
        .take(limit)
        .map(|id| Value::String(id.to_string()))
        .collect();

    Invocation::new(
        "Mailbox/query",
        json!({
            "accountId": account_id(args),
            "queryState": collection_state(ctx.store.as_ref(), ctx.account_id as u32, Collection::Mailbox),
            "canCalculateChanges": true,
            "position": position,
            "ids": ids,
            "total": total,
            "limit": limit,
        }),
        call_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_context;

    fn seeded_ctx() -> JmapContext {
        use irixmail_mail::{mailbox_ops, Mailbox, SpecialUse};

        let ctx = test_context();
        let mut mailboxes = provision_mailboxes(1_700_000_000_000);
        mailboxes.push(Mailbox::new(6, "Receipts", SpecialUse::None, 1));
        mailboxes.push(Mailbox::new(7, "Archive", SpecialUse::Archive, 1));
        ctx.store
            .batch(&mailbox_ops(ctx.account_id as u32, &mailboxes))
            .unwrap();
        ctx
    }

    fn ids_of(response: &Invocation) -> Vec<String> {
        response.arguments()["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn a_name_filter_matches_case_insensitive_substrings() {
        let ctx = seeded_ctx();
        let response = mailbox_query(
            &ctx,
            &json!({"accountId": "1", "filter": {"name": "cEip"}}),
            "c0",
        );
        assert_eq!(ids_of(&response), vec!["6".to_string()]);
        assert_eq!(response.arguments()["total"], 1);
    }

    #[test]
    fn a_role_filter_selects_that_special_use() {
        let ctx = seeded_ctx();
        let archive = mailbox_query(
            &ctx,
            &json!({"accountId": "1", "filter": {"role": "archive"}}),
            "c0",
        );
        assert_eq!(ids_of(&archive), vec!["7".to_string()]);

        let none = mailbox_query(
            &ctx,
            &json!({"accountId": "1", "filter": {"role": null}}),
            "c1",
        );
        assert_eq!(ids_of(&none), vec!["6".to_string()]);
    }

    #[test]
    fn has_any_role_splits_system_from_user_mailboxes() {
        let ctx = seeded_ctx();
        let user = mailbox_query(
            &ctx,
            &json!({"accountId": "1", "filter": {"hasAnyRole": false}}),
            "c0",
        );
        assert_eq!(ids_of(&user), vec!["6".to_string()]);

        let system = mailbox_query(
            &ctx,
            &json!({"accountId": "1", "filter": {"hasAnyRole": true}}),
            "c1",
        );
        assert_eq!(system.arguments()["total"], 6);
    }

    #[test]
    fn a_null_parent_id_filter_matches_the_flat_hierarchy() {
        let ctx = seeded_ctx();
        let response = mailbox_query(
            &ctx,
            &json!({"accountId": "1", "filter": {"parentId": null}}),
            "c0",
        );
        assert_eq!(response.arguments()["total"], 7);
    }

    #[test]
    fn an_unsupported_filter_condition_is_rejected() {
        let ctx = seeded_ctx();
        let response = mailbox_query(
            &ctx,
            &json!({"accountId": "1", "filter": {"isSubscribed": true}}),
            "c0",
        );
        assert_eq!(response.name(), "error");
        assert_eq!(response.arguments()["type"], "unsupportedFilter");
    }

    #[test]
    fn sort_by_name_honors_is_ascending() {
        let ctx = seeded_ctx();
        let ascending = mailbox_query(
            &ctx,
            &json!({"accountId": "1", "sort": [{"property": "name", "isAscending": true}]}),
            "c0",
        );
        assert_eq!(
            ids_of(&ascending).first(),
            Some(&"7".to_string()),
            "Archive first"
        );

        let descending = mailbox_query(
            &ctx,
            &json!({"accountId": "1", "sort": [{"property": "name", "isAscending": false}]}),
            "c1",
        );
        assert_eq!(
            ids_of(&descending).first(),
            Some(&"4".to_string()),
            "Trash first"
        );
    }

    #[test]
    fn an_unsupported_sort_property_is_rejected() {
        let ctx = seeded_ctx();
        let response = mailbox_query(
            &ctx,
            &json!({"accountId": "1", "sort": [{"property": "parentId"}]}),
            "c0",
        );
        assert_eq!(response.name(), "error");
        assert_eq!(response.arguments()["type"], "unsupportedSort");
    }

    #[test]
    fn position_and_limit_window_the_results() {
        let ctx = seeded_ctx();
        let response = mailbox_query(
            &ctx,
            &json!({"accountId": "1", "position": 2, "limit": 3}),
            "c0",
        );
        assert_eq!(
            ids_of(&response),
            vec!["3".to_string(), "4".to_string(), "5".to_string()]
        );
        assert_eq!(response.arguments()["total"], 7);
        assert_eq!(response.arguments()["position"], 2);
        assert_eq!(response.arguments()["limit"], 3);
    }

    #[test]
    fn query_lists_the_default_mailboxes_when_none_are_persisted() {
        let ctx = test_context();
        let response = mailbox_query(&ctx, &json!({"accountId": "1"}), "c0");
        let ids = response.arguments()["ids"].as_array().unwrap();
        assert_eq!(ids.len(), 5);
        assert_eq!(response.arguments()["total"], 5);
        assert!(ids.contains(&json!("1")));
    }

    #[test]
    fn query_lists_persisted_mailbox_ids() {
        use irixmail_mail::{mailbox_ops, Mailbox, SpecialUse};

        let ctx = test_context();
        let mut mailboxes = provision_mailboxes(1_700_000_000_000);
        mailboxes.push(Mailbox::new(
            6,
            "Archive",
            SpecialUse::Archive,
            1_700_000_000,
        ));
        ctx.store
            .batch(&mailbox_ops(ctx.account_id as u32, &mailboxes))
            .unwrap();

        let response = mailbox_query(&ctx, &json!({"accountId": "1"}), "c0");
        let ids = response.arguments()["ids"].as_array().unwrap();
        assert_eq!(ids.len(), 6);
        assert!(ids.contains(&json!("6")));
    }
}
