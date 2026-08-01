use mail_parser::MessageParser;
use serde_json::{json, Value};

use irixmail_mail::load_raw;

use crate::context::JmapContext;
use crate::reply::account_id;
use crate::request::Invocation;

const PREVIEW_LEN: usize = 255;

pub fn searchsnippet_get(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let account = ctx.account_id as u32;
    let term = args
        .get("filter")
        .and_then(|filter| {
            filter
                .get("text")
                .or_else(|| filter.get("body"))
                .or_else(|| filter.get("subject"))
        })
        .and_then(Value::as_str);

    let mut list = Vec::new();
    let mut not_found = Vec::new();
    if let Some(email_ids) = args.get("emailIds").and_then(Value::as_array) {
        for id in email_ids.iter().filter_map(Value::as_str) {
            match id
                .parse::<u32>()
                .ok()
                .and_then(|doc| snippet(ctx, account, doc, term))
            {
                Some(snippet) => list.push(snippet),
                None => not_found.push(Value::String(id.to_string())),
            }
        }
    }

    Invocation::new(
        "SearchSnippet/get",
        json!({
            "accountId": account_id(args),
            "list": list,
            "notFound": not_found,
        }),
        call_id,
    )
}

fn snippet(ctx: &JmapContext, account: u32, document_id: u32, term: Option<&str>) -> Option<Value> {
    let raw = load_raw(ctx.store.as_ref(), ctx.blobs.as_ref(), account, document_id)
        .ok()
        .flatten()?;
    let message = MessageParser::default().parse(&raw)?;
    let subject = message.subject().unwrap_or_default().to_string();
    let preview: String = message
        .body_text(0)
        .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
        .chars()
        .take(PREVIEW_LEN)
        .collect();

    let (subject, preview) = match term {
        Some(term) if !term.is_empty() => (highlight(&subject, term), highlight(&preview, term)),
        _ => (subject, preview),
    };

    Some(json!({
        "emailId": document_id.to_string(),
        "subject": subject,
        "preview": preview,
    }))
}

// Wrap case-insensitive matches of `term` in <mark>. Skips non-ASCII text where lowercasing
// shifts byte offsets, to avoid slicing on a non-char boundary.
fn highlight(text: &str, term: &str) -> String {
    let lower_text = text.to_lowercase();
    let lower_term = term.to_lowercase();
    if lower_text.len() != text.len() {
        return text.to_string();
    }
    let mut out = String::new();
    let mut last = 0;
    for (start, matched) in lower_text.match_indices(&lower_term) {
        out.push_str(&text[last..start]);
        out.push_str("<mark>");
        out.push_str(&text[start..start + matched.len()]);
        out.push_str("</mark>");
        last = start + matched.len();
    }
    out.push_str(&text[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_context_with_account;
    use irixmail_mail::{
        allocate_document_id, append_message, provision_mailboxes, AppendRequest, INBOX_ID,
    };

    fn seed_email(ctx: &JmapContext, raw: &[u8]) -> u32 {
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
                flags: vec![],
                received_at: 0,
                document_id,
                raw,
            },
        )
        .unwrap();
        document_id
    }

    #[test]
    fn searchsnippet_returns_subject_and_highlighted_preview() {
        let ctx = test_context_with_account();
        let doc = seed_email(
            &ctx,
            b"Subject: Quarterly Report\r\nFrom: a@example.net\r\n\r\nthe invoice total is due soon\r\n",
        );
        let response = searchsnippet_get(
            &ctx,
            &json!({
                "accountId": ctx.account_id.to_string(),
                "filter": { "text": "invoice" },
                "emailIds": [doc.to_string()]
            }),
            "c0",
        );
        let snippet = &response.arguments()["list"][0];
        assert_eq!(snippet["emailId"], doc.to_string());
        assert_eq!(snippet["subject"], "Quarterly Report");
        assert!(snippet["preview"]
            .as_str()
            .unwrap()
            .contains("<mark>invoice</mark>"));
    }

    #[test]
    fn searchsnippet_reports_an_unknown_email_as_not_found() {
        let ctx = test_context_with_account();
        let response = searchsnippet_get(
            &ctx,
            &json!({"accountId": ctx.account_id.to_string(), "emailIds": ["999"]}),
            "c0",
        );
        assert_eq!(response.arguments()["notFound"], json!(["999"]));
        assert!(response.arguments()["list"].as_array().unwrap().is_empty());
    }
}
