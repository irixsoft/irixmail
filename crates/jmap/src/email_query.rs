use std::collections::BTreeSet;

use serde_json::{json, Value};

use irixmail_mail::{Keyword, MessageCacheEntry, MessageStoreCache};
use irixmail_store::{Collection, Field, FtsIndex, Query};

use crate::context::JmapContext;
use crate::reply::{account_id, collection_state};
use crate::request::{method_error, Invocation};
use crate::utc_date;

const DEFAULT_LIMIT: usize = 50;

#[derive(Default)]
struct EmailFilter {
    unmatchable: bool,
    in_mailbox: Option<u32>,
    fts: Vec<Query>,
    has_keywords: Vec<Keyword>,
    not_keywords: Vec<Keyword>,
    before: Option<u64>,
    after: Option<u64>,
    min_size: Option<u64>,
    max_size: Option<u64>,
}

impl EmailFilter {
    fn matches(&self, entry: &MessageCacheEntry) -> bool {
        if self.unmatchable {
            return false;
        }
        if let Some(mailbox) = self.in_mailbox {
            if !entry.in_mailbox(mailbox) {
                return false;
            }
        }
        if !self.has_keywords.iter().all(|k| entry.has_keyword(k)) {
            return false;
        }
        if self.not_keywords.iter().any(|k| entry.has_keyword(k)) {
            return false;
        }
        if let Some(before) = self.before {
            if entry.received_at >= before {
                return false;
            }
        }
        if let Some(after) = self.after {
            if entry.received_at < after {
                return false;
            }
        }
        if let Some(min) = self.min_size {
            if u64::from(entry.size) < min {
                return false;
            }
        }
        if let Some(max) = self.max_size {
            if u64::from(entry.size) >= max {
                return false;
            }
        }
        true
    }
}

enum FilterNode {
    And(Vec<FilterNode>),
    Or(Vec<FilterNode>),
    Not(Vec<FilterNode>),
    Leaf(EmailFilter),
}

fn parse_filter(args: &Value) -> Option<Option<FilterNode>> {
    match args.get("filter").filter(|value| !value.is_null()) {
        None => Some(None),
        Some(value) => parse_node(value).map(Some),
    }
}

fn parse_node(value: &Value) -> Option<FilterNode> {
    let object = value.as_object()?;
    if let Some(operator) = object.get("operator") {
        let conditions = object.get("conditions")?.as_array()?;
        let nodes = conditions
            .iter()
            .map(parse_node)
            .collect::<Option<Vec<_>>>()?;
        return match operator.as_str()? {
            "AND" => Some(FilterNode::And(nodes)),
            "OR" => Some(FilterNode::Or(nodes)),
            "NOT" => Some(FilterNode::Not(nodes)),
            _ => None,
        };
    }
    parse_condition(value).map(FilterNode::Leaf)
}

fn parse_condition(value: &Value) -> Option<EmailFilter> {
    let mut filter = EmailFilter::default();
    for (key, value) in value.as_object()? {
        match key.as_str() {
            "inMailbox" => match value.as_str().and_then(|id| id.parse::<u32>().ok()) {
                Some(id) => filter.in_mailbox = Some(id),
                None => filter.unmatchable = true,
            },
            "text" => filter.fts.push(Query::term(value.as_str()?)),
            "subject" => filter
                .fts
                .push(Query::field(Field::Subject, value.as_str()?)),
            "body" => filter.fts.push(Query::field(Field::Body, value.as_str()?)),
            "from" => filter.fts.push(Query::field(Field::From, value.as_str()?)),
            "to" => filter.fts.push(Query::field(Field::To, value.as_str()?)),
            "cc" => filter.fts.push(Query::field(Field::Cc, value.as_str()?)),
            "bcc" => filter.fts.push(Query::field(Field::Bcc, value.as_str()?)),
            "hasKeyword" => filter
                .has_keywords
                .push(Keyword::from_jmap(value.as_str()?)),
            "notKeyword" => filter
                .not_keywords
                .push(Keyword::from_jmap(value.as_str()?)),
            "hasAttachment" => match value.as_bool()? {
                true => filter.has_keywords.push(Keyword::has_attachment()),
                false => filter.not_keywords.push(Keyword::has_attachment()),
            },
            "before" => filter.before = Some(utc_date::parse(value.as_str()?)?),
            "after" => filter.after = Some(utc_date::parse(value.as_str()?)?),
            "minSize" => filter.min_size = Some(value.as_u64()?),
            "maxSize" => filter.max_size = Some(value.as_u64()?),
            _ => return None,
        }
    }
    Some(filter)
}

fn eval_node(
    node: &FilterNode,
    ctx: &JmapContext,
    account: u32,
    cache: Option<&MessageStoreCache>,
    universe: &[u32],
) -> Result<BTreeSet<u32>, &'static str> {
    match node {
        FilterNode::And(nodes) => {
            let mut ids: BTreeSet<u32> = universe.iter().copied().collect();
            for node in nodes {
                let matched = eval_node(node, ctx, account, cache, universe)?;
                ids.retain(|id| matched.contains(id));
            }
            Ok(ids)
        }
        FilterNode::Or(nodes) => {
            let mut ids = BTreeSet::new();
            for node in nodes {
                ids.extend(eval_node(node, ctx, account, cache, universe)?);
            }
            Ok(ids)
        }
        FilterNode::Not(nodes) => {
            let mut excluded = BTreeSet::new();
            for node in nodes {
                excluded.extend(eval_node(node, ctx, account, cache, universe)?);
            }
            Ok(universe
                .iter()
                .copied()
                .filter(|id| !excluded.contains(id))
                .collect())
        }
        FilterNode::Leaf(filter) => {
            let ids: Vec<u32> = cache
                .map(|cache| {
                    cache
                        .entries()
                        .filter(|entry| filter.matches(entry))
                        .map(|entry| entry.document_id)
                        .collect()
                })
                .unwrap_or_default();
            if filter.fts.is_empty() {
                return Ok(ids.into_iter().collect());
            }
            FtsIndex::new(ctx.store.as_ref())
                .search(
                    account,
                    Collection::Email,
                    &Query::all(filter.fts.clone()),
                    &ids,
                )
                .map(|hits| hits.into_iter().collect())
                .map_err(|_| "serverFail")
        }
    }
}

#[derive(Clone, Copy)]
enum SortProperty {
    ReceivedAt,
    SentAt,
    Size,
}

impl SortProperty {
    fn key(self, entry: &MessageCacheEntry) -> u64 {
        match self {
            SortProperty::ReceivedAt => entry.received_at,
            SortProperty::SentAt => entry.sent_at,
            SortProperty::Size => u64::from(entry.size),
        }
    }
}

fn parse_sort(args: &Value) -> Option<Vec<(SortProperty, bool)>> {
    let Some(sort) = args.get("sort").filter(|value| !value.is_null()) else {
        return Some(Vec::new());
    };
    let mut comparators = Vec::new();
    for comparator in sort.as_array()? {
        let property = match comparator.get("property").and_then(Value::as_str)? {
            "receivedAt" => SortProperty::ReceivedAt,
            "sentAt" => SortProperty::SentAt,
            "size" => SortProperty::Size,
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

fn sort_ids(
    ids: &mut [u32],
    cache: Option<&MessageStoreCache>,
    comparators: &[(SortProperty, bool)],
) {
    ids.sort_unstable_by(|a, b| {
        for (property, ascending) in comparators {
            let key = |id: u32| {
                cache
                    .and_then(|cache| cache.get(id))
                    .map(|entry| property.key(entry))
            };
            let ordering = key(*a).cmp(&key(*b));
            let ordering = if *ascending {
                ordering
            } else {
                ordering.reverse()
            };
            if ordering != std::cmp::Ordering::Equal {
                return ordering;
            }
        }
        b.cmp(a)
    });
}

pub(crate) fn query_ids(ctx: &JmapContext, args: &Value) -> Result<Vec<u32>, &'static str> {
    let account = ctx.account_id as u32;
    let filter = parse_filter(args).ok_or("unsupportedFilter")?;
    let comparators = parse_sort(args).ok_or("unsupportedSort")?;

    let cache = MessageStoreCache::build(ctx.store.as_ref(), account).ok();
    let universe: Vec<u32> = cache
        .as_ref()
        .map(|cache| cache.entries().map(|entry| entry.document_id).collect())
        .unwrap_or_default();
    let mut ids: Vec<u32> = match &filter {
        None => universe,
        Some(node) => eval_node(node, ctx, account, cache.as_ref(), &universe)?
            .into_iter()
            .collect(),
    };
    sort_ids(&mut ids, cache.as_ref(), &comparators);
    Ok(ids)
}

pub fn email_query(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let account = ctx.account_id as u32;
    let ids = match query_ids(ctx, args) {
        Ok(ids) => ids,
        Err(kind) => return method_error(kind, call_id),
    };

    let total = ids.len();
    let position = args.get("position").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_LIMIT);

    let page: Vec<String> = ids
        .into_iter()
        .skip(position)
        .take(limit)
        .map(|id| id.to_string())
        .collect();

    Invocation::new(
        "Email/query",
        json!({
            "accountId": account_id(args),
            "queryState": collection_state(ctx.store.as_ref(), account, Collection::Email),
            "canCalculateChanges": true,
            "position": position,
            "ids": page,
            "total": total,
            "limit": limit,
        }),
        call_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{test_context, test_context_with_account};
    use irixmail_mail::{
        allocate_document_id, append_message, provision_mailboxes, AppendRequest, Keyword, INBOX_ID,
    };

    fn seed(ctx: &JmapContext, raw: &[u8], flags: Vec<Keyword>, received_at: u64) -> u32 {
        let account = ctx.account_id as u32;
        let record = ctx.directory.accounts().get(ctx.account_id).unwrap();
        let mailboxes = provision_mailboxes(record.created_at);
        let inbox = mailboxes.iter().find(|m| m.id == INBOX_ID).unwrap();
        let document_id = allocate_document_id(ctx.store.as_ref(), account).unwrap();
        append_message(
            ctx.store.as_ref(),
            ctx.blobs.as_ref(),
            ctx.notifier.as_ref(),
            &AppendRequest {
                account: &record,
                mailbox: inbox,
                flags,
                received_at,
                document_id,
                raw,
            },
        )
        .unwrap();
        document_id
    }

    const INVOICE: &[u8] = concat!(
        "Subject: Quarterly report\r\n",
        "From: Alice Example <alice@example.net>\r\n",
        "To: bob@example.org\r\n",
        "\r\n",
        "please pay the invoice\r\n",
    )
    .as_bytes();

    const STANDUP: &[u8] = concat!(
        "Subject: Meeting notes\r\n",
        "From: carol@example.com\r\n",
        "To: bob@example.org\r\n",
        "\r\n",
        "notes from the standup\r\n",
    )
    .as_bytes();

    fn query(ctx: &JmapContext, filter: Value) -> Invocation {
        email_query(
            ctx,
            &json!({"accountId": ctx.account_id.to_string(), "filter": filter}),
            "c0",
        )
    }

    #[test]
    fn a_text_filter_returns_only_matching_messages() {
        let ctx = test_context_with_account();
        let invoice = seed(&ctx, INVOICE, vec![], 0);
        seed(&ctx, STANDUP, vec![], 0);

        let response = query(&ctx, json!({"text": "invoice"}));
        assert_eq!(response.arguments()["ids"], json!([invoice.to_string()]));
        assert_eq!(response.arguments()["total"], 1);
    }

    #[test]
    fn scoped_field_filters_search_only_that_field() {
        let ctx = test_context_with_account();
        let invoice = seed(&ctx, INVOICE, vec![], 0);
        seed(&ctx, STANDUP, vec![], 0);

        let by_subject = query(&ctx, json!({"subject": "quarterly"}));
        assert_eq!(by_subject.arguments()["ids"], json!([invoice.to_string()]));

        let quarterly_in_body = query(&ctx, json!({"body": "quarterly"}));
        assert_eq!(quarterly_in_body.arguments()["ids"], json!([]));
        assert_eq!(quarterly_in_body.arguments()["total"], 0);

        let by_from = query(&ctx, json!({"from": "alice"}));
        assert_eq!(by_from.arguments()["ids"], json!([invoice.to_string()]));
    }

    #[test]
    fn a_text_filter_intersects_with_the_mailbox_filter() {
        let ctx = test_context_with_account();
        let invoice = seed(&ctx, INVOICE, vec![], 0);

        let in_inbox = query(
            &ctx,
            json!({"inMailbox": INBOX_ID.to_string(), "text": "invoice"}),
        );
        assert_eq!(in_inbox.arguments()["ids"], json!([invoice.to_string()]));

        let elsewhere = query(&ctx, json!({"inMailbox": "99", "text": "invoice"}));
        assert_eq!(elsewhere.arguments()["ids"], json!([]));
        assert_eq!(elsewhere.arguments()["total"], 0);
    }

    #[test]
    fn keyword_filters_narrow_from_the_cache() {
        let ctx = test_context_with_account();
        let seen = seed(&ctx, INVOICE, vec![Keyword::Seen], 0);
        let unseen = seed(&ctx, STANDUP, vec![], 0);

        let with = query(&ctx, json!({"hasKeyword": "$seen"}));
        assert_eq!(with.arguments()["ids"], json!([seen.to_string()]));

        let without = query(&ctx, json!({"notKeyword": "$seen"}));
        assert_eq!(without.arguments()["ids"], json!([unseen.to_string()]));
    }

    #[test]
    fn date_and_size_filters_narrow_from_the_cache() {
        let ctx = test_context_with_account();
        let old = seed(&ctx, INVOICE, vec![], 1_000);
        let recent = seed(&ctx, STANDUP, vec![], 2_000);

        let after = query(&ctx, json!({"after": "1970-01-01T00:20:00Z"}));
        assert_eq!(after.arguments()["ids"], json!([recent.to_string()]));

        let before = query(&ctx, json!({"before": "1970-01-01T00:20:00Z"}));
        assert_eq!(before.arguments()["ids"], json!([old.to_string()]));

        let huge = query(&ctx, json!({"minSize": 1_000_000}));
        assert_eq!(huge.arguments()["ids"], json!([]));
    }

    fn query_sorted(ctx: &JmapContext, sort: Value) -> Invocation {
        email_query(
            ctx,
            &json!({"accountId": ctx.account_id.to_string(), "sort": sort}),
            "c0",
        )
    }

    #[test]
    fn sort_by_received_at_honors_is_ascending() {
        let ctx = test_context_with_account();
        let newer = seed(&ctx, INVOICE, vec![], 2_000);
        let older = seed(&ctx, STANDUP, vec![], 1_000);

        let descending = query_sorted(
            &ctx,
            json!([{"property": "receivedAt", "isAscending": false}]),
        );
        assert_eq!(
            descending.arguments()["ids"],
            json!([newer.to_string(), older.to_string()])
        );

        let ascending = query_sorted(
            &ctx,
            json!([{"property": "receivedAt", "isAscending": true}]),
        );
        assert_eq!(
            ascending.arguments()["ids"],
            json!([older.to_string(), newer.to_string()])
        );
    }

    #[test]
    fn sort_by_size_orders_by_message_size() {
        let ctx = test_context_with_account();
        let long: &[u8] = concat!(
            "Subject: Big one\r\n",
            "From: carol@example.com\r\n",
            "\r\n",
            "this body is deliberately padded with many extra words to be the larger message of the pair\r\n",
        )
        .as_bytes();
        let small = seed(&ctx, INVOICE, vec![], 0);
        let large = seed(&ctx, long, vec![], 0);

        let ascending = query_sorted(&ctx, json!([{"property": "size", "isAscending": true}]));
        assert_eq!(
            ascending.arguments()["ids"],
            json!([small.to_string(), large.to_string()])
        );
    }

    #[test]
    fn sort_by_sent_at_reads_the_date_header() {
        let ctx = test_context_with_account();
        let late: &[u8] = concat!(
            "Subject: Late\r\n",
            "Date: Fri, 01 Jan 2021 00:00:00 +0000\r\n",
            "\r\n",
            "late\r\n",
        )
        .as_bytes();
        let early: &[u8] = concat!(
            "Subject: Early\r\n",
            "Date: Wed, 01 Jan 2020 00:00:00 +0000\r\n",
            "\r\n",
            "early\r\n",
        )
        .as_bytes();
        let sent_late = seed(&ctx, late, vec![], 0);
        let sent_early = seed(&ctx, early, vec![], 0);

        let descending = query_sorted(&ctx, json!([{"property": "sentAt", "isAscending": false}]));
        assert_eq!(
            descending.arguments()["ids"],
            json!([sent_late.to_string(), sent_early.to_string()])
        );
    }

    #[test]
    fn a_later_comparator_breaks_ties() {
        let ctx = test_context_with_account();
        let first = seed(&ctx, INVOICE, vec![], 1_000);
        let second = seed(&ctx, INVOICE, vec![], 2_000);

        let response = query_sorted(
            &ctx,
            json!([
                {"property": "size", "isAscending": true},
                {"property": "receivedAt", "isAscending": true}
            ]),
        );
        assert_eq!(
            response.arguments()["ids"],
            json!([first.to_string(), second.to_string()])
        );
    }

    #[test]
    fn an_unsupported_sort_property_is_rejected() {
        let ctx = test_context_with_account();
        seed(&ctx, INVOICE, vec![], 0);

        let response = query_sorted(&ctx, json!([{"property": "from", "isAscending": true}]));
        assert_eq!(response.name(), "error");
        assert_eq!(response.arguments()["type"], "unsupportedSort");
    }

    const TO_ALICE: &[u8] = concat!(
        "Subject: Re: hello\r\n",
        "From: bob@example.org\r\n",
        "To: alice@example.net\r\n",
        "\r\n",
        "hello alice\r\n",
    )
    .as_bytes();

    const WITH_ATTACHMENT: &[u8] = concat!(
        "Subject: Files\r\n",
        "From: dave@example.com\r\n",
        "To: bob@example.org\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"B\"\r\n",
        "\r\n",
        "--B\r\n",
        "Content-Type: text/plain\r\n",
        "\r\n",
        "see attached\r\n",
        "--B\r\n",
        "Content-Type: application/pdf\r\n",
        "Content-Disposition: attachment; filename=\"x.pdf\"\r\n",
        "\r\n",
        "%PDF-1.4\r\n",
        "--B--\r\n",
    )
    .as_bytes();

    #[test]
    fn an_and_operator_narrows_across_conditions() {
        let ctx = test_context_with_account();
        let seen_invoice = seed(&ctx, INVOICE, vec![Keyword::Seen], 0);
        seed(&ctx, STANDUP, vec![Keyword::Seen], 0);

        let response = query(
            &ctx,
            json!({"operator": "AND", "conditions": [
                {"text": "invoice"},
                {"hasKeyword": "$seen"}
            ]}),
        );
        assert_eq!(
            response.arguments()["ids"],
            json!([seen_invoice.to_string()])
        );
    }

    #[test]
    fn an_or_operator_unions_conditions() {
        let ctx = test_context_with_account();
        let from_alice = seed(&ctx, INVOICE, vec![], 0);
        let to_alice = seed(&ctx, TO_ALICE, vec![], 0);
        seed(&ctx, STANDUP, vec![], 0);

        let response = query(
            &ctx,
            json!({"operator": "OR", "conditions": [
                {"from": "alice@example.net"},
                {"to": "alice@example.net"}
            ]}),
        );
        let mut ids: Vec<String> = response.arguments()["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap().to_string())
            .collect();
        ids.sort();
        let mut expected = vec![from_alice.to_string(), to_alice.to_string()];
        expected.sort();
        assert_eq!(ids, expected);
    }

    #[test]
    fn a_not_operator_excludes_matches() {
        let ctx = test_context_with_account();
        seed(&ctx, INVOICE, vec![], 0);
        let standup = seed(&ctx, STANDUP, vec![], 0);

        let response = query(
            &ctx,
            json!({"operator": "NOT", "conditions": [{"text": "invoice"}]}),
        );
        assert_eq!(response.arguments()["ids"], json!([standup.to_string()]));
    }

    #[test]
    fn operators_nest_inside_conditions() {
        let ctx = test_context_with_account();
        let invoice = seed(&ctx, INVOICE, vec![], 0);
        let standup = seed(&ctx, STANDUP, vec![], 0);
        seed(&ctx, TO_ALICE, vec![], 0);

        let response = query(
            &ctx,
            json!({"operator": "AND", "conditions": [
                {"inMailbox": INBOX_ID.to_string()},
                {"operator": "OR", "conditions": [
                    {"subject": "quarterly"},
                    {"subject": "meeting"}
                ]}
            ]}),
        );
        let mut ids: Vec<String> = response.arguments()["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap().to_string())
            .collect();
        ids.sort();
        let mut expected = vec![invoice.to_string(), standup.to_string()];
        expected.sort();
        assert_eq!(ids, expected);
    }

    #[test]
    fn has_attachment_filters_on_the_stored_keyword() {
        let ctx = test_context_with_account();
        let plain = seed(&ctx, INVOICE, vec![], 0);
        let attached = seed(&ctx, WITH_ATTACHMENT, vec![], 0);

        let with = query(&ctx, json!({"hasAttachment": true}));
        assert_eq!(with.arguments()["ids"], json!([attached.to_string()]));

        let without = query(&ctx, json!({"hasAttachment": false}));
        assert_eq!(without.arguments()["ids"], json!([plain.to_string()]));
    }

    #[test]
    fn an_unknown_operator_is_rejected() {
        let ctx = test_context_with_account();
        seed(&ctx, INVOICE, vec![], 0);

        let response = query(
            &ctx,
            json!({"operator": "XOR", "conditions": [{"text": "x"}]}),
        );
        assert_eq!(response.name(), "error");
        assert_eq!(response.arguments()["type"], "unsupportedFilter");
    }

    #[test]
    fn an_unsupported_filter_condition_is_rejected() {
        let ctx = test_context_with_account();
        seed(&ctx, INVOICE, vec![], 0);

        let response = query(&ctx, json!({"header": ["X-Custom"]}));
        assert_eq!(response.name(), "error");
        assert_eq!(response.arguments()["type"], "unsupportedFilter");
    }

    #[test]
    fn the_query_state_reconciles_with_email_changes() {
        let ctx = test_context_with_account();
        seed(&ctx, INVOICE, vec![], 0);

        let response = email_query(
            &ctx,
            &json!({"accountId": ctx.account_id.to_string()}),
            "c0",
        );
        let state = response.arguments()["queryState"]
            .as_str()
            .unwrap()
            .to_string();

        let quiet = crate::email_changes(
            &ctx,
            &json!({"accountId": ctx.account_id.to_string(), "sinceState": state}),
            "c1",
        );
        assert_eq!(quiet.arguments()["created"], json!([]));
    }

    #[test]
    fn an_empty_account_yields_no_ids() {
        let ctx = test_context();
        let response = email_query(&ctx, &json!({"accountId": "1"}), "c0");
        assert_eq!(response.name(), "Email/query");
        assert_eq!(response.arguments()["total"], 0);
        assert_eq!(response.arguments()["ids"], json!([]));
    }

    #[test]
    fn the_pagination_window_is_echoed() {
        let ctx = test_context();
        let response = email_query(
            &ctx,
            &json!({"accountId": "1", "position": 5, "limit": 20}),
            "c0",
        );
        assert_eq!(response.arguments()["position"], 5);
        assert_eq!(response.arguments()["limit"], 20);
    }
}
