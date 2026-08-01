use serde_json::{json, Map, Value};

use mail_parser::{Address, Message, MessageParser, MessagePart, MimeHeaders};

use irixmail_mail::{load_data, load_metadata, MessageStoreCache};

use irixmail_store::Collection;

use crate::context::JmapContext;
use crate::reply::{account_id, collection_state, requested_ids};
use crate::request::Invocation;

const PREVIEW_LEN: usize = 200;
const MAX_OBJECTS_IN_GET: usize = 500;

fn all_message_ids(ctx: &JmapContext, account: u32) -> Vec<String> {
    match MessageStoreCache::build(ctx.store.as_ref(), account) {
        Ok(cache) => {
            let mut ids: Vec<u32> = cache.entries().map(|entry| entry.document_id).collect();
            ids.sort_unstable_by(|a, b| b.cmp(a));
            ids.truncate(MAX_OBJECTS_IN_GET);
            ids.into_iter().map(|id| id.to_string()).collect()
        }
        Err(_) => Vec::new(),
    }
}

pub fn email_get(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let account = ctx.account_id as u32;
    let mut list = Vec::new();
    let mut not_found = Vec::new();

    let requested = requested_ids(args).unwrap_or_else(|| all_message_ids(ctx, account));
    for id in requested {
        let document_id = match id.parse::<u32>() {
            Ok(value) => value,
            Err(_) => {
                not_found.push(Value::String(id));
                continue;
            }
        };
        match build_email(ctx, account, document_id) {
            Some(email) => list.push(email),
            None => not_found.push(Value::String(id)),
        }
    }

    Invocation::new(
        "Email/get",
        json!({
            "accountId": account_id(args),
            "state": collection_state(ctx.store.as_ref(), account, Collection::Email),
            "list": list,
            "notFound": not_found,
        }),
        call_id,
    )
}

fn build_email(ctx: &JmapContext, account: u32, document_id: u32) -> Option<Value> {
    let data = load_data(ctx.store.as_ref(), account, document_id)
        .ok()
        .flatten()?;
    let metadata = load_metadata(ctx.store.as_ref(), account, document_id)
        .ok()
        .flatten()?;
    let blob_hash = metadata.blob_hash();
    let raw = ctx.blobs.get_all(&blob_hash).ok().flatten()?;
    let mut object = parse_email(&raw, &blob_hash.to_hex(), data.size as usize)?;

    let mailbox_ids: Map<String, Value> = data
        .mailboxes
        .iter()
        .map(|membership| (membership.mailbox_id.to_string(), Value::Bool(true)))
        .collect();

    let mut keywords = Map::new();
    for keyword in &data.keywords {
        if let Some(name) = keyword.to_jmap() {
            keywords.insert(name.to_string(), Value::Bool(true));
        }
    }

    let received = crate::utc_date::format(data.received_at);
    object.insert("id".to_string(), json!(document_id.to_string()));
    object.insert("threadId".to_string(), json!(data.thread_id.to_string()));
    object.insert("mailboxIds".to_string(), Value::Object(mailbox_ids));
    object.insert("keywords".to_string(), Value::Object(keywords));
    object.insert("receivedAt".to_string(), json!(received));
    Some(Value::Object(object))
}

// Parse raw RFC822 bytes into a JMAP Email object (header/body fields only, no storage
// identity). Shared by Email/get and Email/parse.
pub(crate) fn parse_email(raw: &[u8], blob_id: &str, size: usize) -> Option<Map<String, Value>> {
    let message = MessageParser::default().parse(raw)?;

    let mut body_values = Map::new();
    let mut text_body = Vec::new();
    let mut html_body = Vec::new();
    if let Some(html) = message.body_html(0) {
        body_values.insert(
            "html".to_string(),
            json!({ "value": html.as_ref(), "isTruncated": false }),
        );
        html_body.push(json!({ "partId": "html", "type": "text/html" }));
    }
    if let Some(text) = message.body_text(0) {
        body_values.insert(
            "text".to_string(),
            json!({ "value": text.as_ref(), "isTruncated": false }),
        );
        text_body.push(json!({ "partId": "text", "type": "text/plain" }));
    }

    let attachments: Vec<Value> = message
        .attachments()
        .map(|part| {
            json!({
                "blobId": part_blob_id(blob_id, part),
                "name": part.attachment_name(),
                "type": content_type(part),
                "size": part.len(),
                "disposition": "attachment",
            })
        })
        .collect();

    let date = message.date().map(|date| date.to_rfc3339());

    let mut object = Map::new();
    object.insert("blobId".to_string(), json!(blob_id));
    object.insert("from".to_string(), addresses(message.from()));
    object.insert("to".to_string(), addresses(message.to()));
    object.insert("cc".to_string(), addresses(message.cc()));
    object.insert("bcc".to_string(), addresses(message.bcc()));
    object.insert("replyTo".to_string(), addresses(message.reply_to()));
    object.insert("subject".to_string(), json!(message.subject()));
    object.insert("sentAt".to_string(), json!(date));
    object.insert("preview".to_string(), json!(preview(&message)));
    object.insert("hasAttachment".to_string(), json!(!attachments.is_empty()));
    object.insert("size".to_string(), json!(size));
    object.insert("bodyValues".to_string(), Value::Object(body_values));
    object.insert("textBody".to_string(), Value::Array(text_body));
    object.insert("htmlBody".to_string(), Value::Array(html_body));
    object.insert("attachments".to_string(), Value::Array(attachments));
    Some(object)
}

fn addresses(address: Option<&Address<'_>>) -> Value {
    match address {
        Some(address) => Value::Array(
            address
                .iter()
                .map(|addr| {
                    json!({
                        "name": addr.name.as_deref(),
                        "email": addr.address.as_deref().unwrap_or(""),
                    })
                })
                .collect(),
        ),
        None => Value::Null,
    }
}

fn part_blob_id(blob_id: &str, part: &MessagePart<'_>) -> String {
    crate::blob_download::section_blob_id(
        blob_id,
        part.offset_body,
        part.offset_end.saturating_sub(part.offset_body),
        part.encoding as u8,
    )
}

fn content_type(part: &MessagePart<'_>) -> String {
    match part.content_type() {
        Some(ct) => match &ct.c_subtype {
            Some(subtype) => format!("{}/{}", ct.c_type, subtype),
            None => ct.c_type.to_string(),
        },
        None => "application/octet-stream".to_string(),
    }
}

fn preview(message: &Message<'_>) -> String {
    message
        .body_text(0)
        .map(|text| {
            text.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(PREVIEW_LEN)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{test_context, test_context_with_account};
    use irixmail_mail::{
        allocate_document_id, append_message, provision_mailboxes, AppendRequest, INBOX_ID,
    };

    fn seed_email(ctx: &JmapContext, raw: &[u8]) -> u32 {
        seed_email_at(ctx, raw, 0)
    }

    fn seed_email_at(ctx: &JmapContext, raw: &[u8], received_at: u64) -> u32 {
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
                received_at,
                document_id,
                raw,
            },
        )
        .unwrap();
        document_id
    }

    fn get_one(ctx: &JmapContext, doc: u32) -> Value {
        let response = email_get(
            &ctx.clone(),
            &json!({"accountId": ctx.account_id.to_string(), "ids": [doc.to_string()]}),
            "c0",
        );
        response.arguments()["list"][0].clone()
    }

    #[test]
    fn an_explicit_id_list_over_the_get_limit_is_request_too_large() {
        use crate::request::{Request, Router};

        let ctx = test_context();
        let mut router = Router::new();
        router.register_stateful("Email/get", email_get);
        let ids: Vec<String> = (0..=500).map(|id| id.to_string()).collect();
        let request = Request {
            using: Vec::new(),
            method_calls: vec![crate::request::Invocation::new(
                "Email/get",
                json!({"accountId": "1", "ids": ids}),
                "c0",
            )],
        };
        let response = router.process(&ctx, &request, "s");
        assert_eq!(response.method_responses[0].name(), "error");
        assert_eq!(
            response.method_responses[0].arguments()["type"],
            "requestTooLarge"
        );
    }

    #[test]
    fn received_at_is_the_delivery_timestamp_not_the_date_header() {
        let ctx = test_context_with_account();
        let raw: &[u8] = concat!(
            "Subject: Dated\r\n",
            "From: a@example.net\r\n",
            "Date: Sat, 01 Feb 2020 00:00:00 +0000\r\n",
            "\r\n",
            "body\r\n",
        )
        .as_bytes();
        let doc = seed_email_at(&ctx, raw, 1_700_000_000);

        let email = get_one(&ctx, doc);
        assert_eq!(email["receivedAt"], "2023-11-14T22:13:20Z");
    }

    #[test]
    fn received_at_is_present_when_the_date_header_is_absent() {
        let ctx = test_context_with_account();
        let raw: &[u8] = b"Subject: Undated\r\nFrom: a@example.net\r\n\r\nbody\r\n";
        let doc = seed_email_at(&ctx, raw, 1_700_000_000);

        let email = get_one(&ctx, doc);
        assert_eq!(email["receivedAt"], "2023-11-14T22:13:20Z");
        assert_eq!(email["sentAt"], Value::Null);
    }

    #[test]
    fn the_email_blob_id_downloads_the_raw_message() {
        let ctx = test_context_with_account();
        let raw: &[u8] = b"Subject: Raw\r\nFrom: a@example.net\r\n\r\nthe raw body\r\n";
        let doc = seed_email(&ctx, raw);

        let response = email_get(
            &ctx,
            &json!({"accountId": ctx.account_id.to_string(), "ids": [doc.to_string()]}),
            "c0",
        );

        let blob_id = response.arguments()["list"][0]["blobId"]
            .as_str()
            .unwrap()
            .to_string();
        let fetched = crate::fetch_blob(ctx.blobs.as_ref(), &blob_id).unwrap();
        assert_eq!(fetched.as_deref(), Some(raw));
    }

    #[test]
    fn an_attachment_blob_id_downloads_only_that_part() {
        let ctx = test_context_with_account();
        let attachment_bytes: &[u8] = b"%PDF-1.4 fake report bytes";
        let encoded = {
            const TABLE: &[u8] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut out = String::new();
            for chunk in attachment_bytes.chunks(3) {
                let b = [
                    chunk[0],
                    *chunk.get(1).unwrap_or(&0),
                    *chunk.get(2).unwrap_or(&0),
                ];
                let n = u32::from_be_bytes([0, b[0], b[1], b[2]]);
                for i in 0..4 {
                    let keep = i < chunk.len() + 1;
                    out.push(if keep {
                        TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char
                    } else {
                        '='
                    });
                }
            }
            out
        };
        let raw = format!(
            "From: a@example.net\r\nSubject: With attachment\r\n\
             MIME-Version: 1.0\r\n\
             Content-Type: multipart/mixed; boundary=\"XX\"\r\n\r\n\
             --XX\r\nContent-Type: text/plain\r\n\r\nsee attached\r\n\
             --XX\r\nContent-Type: application/pdf; name=\"report.pdf\"\r\n\
             Content-Disposition: attachment; filename=\"report.pdf\"\r\n\
             Content-Transfer-Encoding: base64\r\n\r\n{encoded}\r\n\
             --XX--\r\n"
        );
        let doc = seed_email(&ctx, raw.as_bytes());

        let response = email_get(
            &ctx,
            &json!({"accountId": ctx.account_id.to_string(), "ids": [doc.to_string()]}),
            "c0",
        );

        let email = &response.arguments()["list"][0];
        let message_blob_id = email["blobId"].as_str().unwrap();
        let attachment = &email["attachments"][0];
        let part_blob_id = attachment["blobId"].as_str().unwrap();
        assert_ne!(part_blob_id, message_blob_id);

        let fetched = crate::fetch_blob(ctx.blobs.as_ref(), part_blob_id)
            .unwrap()
            .expect("attachment blob downloads");
        assert_eq!(fetched, attachment_bytes);

        let whole = crate::fetch_blob(ctx.blobs.as_ref(), message_blob_id)
            .unwrap()
            .expect("message blob still downloads");
        assert_eq!(whole, raw.as_bytes());
    }

    #[test]
    fn null_ids_fetch_all_the_account_messages() {
        let ctx = test_context_with_account();
        let first = seed_email(
            &ctx,
            b"Subject: One\r\nFrom: a@example.net\r\n\r\nfirst\r\n",
        );
        let second = seed_email(
            &ctx,
            b"Subject: Two\r\nFrom: b@example.net\r\n\r\nsecond\r\n",
        );

        let response = email_get(
            &ctx,
            &json!({"accountId": ctx.account_id.to_string(), "ids": null}),
            "c0",
        );

        let list = response.arguments()["list"].as_array().unwrap().clone();
        let mut ids: Vec<String> = list
            .iter()
            .map(|email| email["id"].as_str().unwrap().to_string())
            .collect();
        ids.sort();
        let mut wanted = vec![first.to_string(), second.to_string()];
        wanted.sort();
        assert_eq!(ids, wanted);
        assert_eq!(response.arguments()["notFound"], json!([]));
    }

    #[test]
    fn the_state_reconciles_with_email_changes() {
        let ctx = test_context_with_account();
        seed_email(
            &ctx,
            b"Subject: One\r\nFrom: a@example.net\r\n\r\nfirst\r\n",
        );

        let response = email_get(
            &ctx,
            &json!({"accountId": ctx.account_id.to_string(), "ids": null}),
            "c0",
        );
        let state = response.arguments()["state"].as_str().unwrap().to_string();

        let quiet = crate::email_changes(
            &ctx,
            &json!({"accountId": ctx.account_id.to_string(), "sinceState": state}),
            "c1",
        );
        assert_eq!(quiet.arguments()["created"], json!([]));
        assert_eq!(quiet.arguments()["updated"], json!([]));
        assert_eq!(quiet.arguments()["destroyed"], json!([]));

        let second = seed_email(
            &ctx,
            b"Subject: Two\r\nFrom: b@example.net\r\n\r\nsecond\r\n",
        );
        let delta = crate::email_changes(
            &ctx,
            &json!({"accountId": ctx.account_id.to_string(), "sinceState": state}),
            "c2",
        );
        let created = delta.arguments()["created"].as_array().unwrap().clone();
        assert!(
            created.contains(&json!(second.to_string())),
            "created: {created:?}"
        );
    }

    #[test]
    fn an_unknown_id_is_reported_not_found() {
        let ctx = test_context();
        let response = email_get(&ctx, &json!({"accountId": "1", "ids": ["404"]}), "c0");
        assert_eq!(response.name(), "Email/get");
        assert_eq!(response.arguments()["list"], json!([]));
        assert_eq!(response.arguments()["notFound"], json!(["404"]));
    }

    #[test]
    fn a_non_numeric_id_is_not_found() {
        let ctx = test_context();
        let response = email_get(&ctx, &json!({"accountId": "1", "ids": ["abc"]}), "c0");
        assert_eq!(response.arguments()["notFound"], json!(["abc"]));
    }
}
