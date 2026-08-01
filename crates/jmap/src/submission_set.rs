use std::collections::HashMap;

use mail_parser::{Address, Message, MessageParser};
use serde_json::{json, Map, Value};

use irixmail_core::{Error, Result};
use irixmail_mail::load_raw;
use irixmail_store::{Collection, Key, Subspace};

use crate::context::JmapContext;
use crate::email_set::apply_patch;
use crate::reply::{account_id, STATE};
use crate::request::Invocation;

pub fn submission_set(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let account = ctx.account_id as u32;
    let mut created = Map::new();
    let mut not_created = Map::new();
    let mut updated = Map::new();
    let mut not_updated = Map::new();
    let mut creation_to_email: HashMap<String, u32> = HashMap::new();

    if let Some(create) = args.get("create").and_then(Value::as_object) {
        for (creation_id, submission) in create {
            match submit(ctx, account, submission) {
                Ok(sent) => match persist_submission(ctx, account, &sent) {
                    Ok(()) => {
                        created.insert(
                            creation_id.clone(),
                            json!({ "id": sent.submission_id.to_string() }),
                        );
                        creation_to_email.insert(creation_id.clone(), sent.email_id);
                    }
                    Err(_) => {
                        not_created.insert(creation_id.clone(), set_error("serverFail"));
                    }
                },
                Err(kind) => {
                    not_created.insert(creation_id.clone(), set_error(kind));
                }
            }
        }
    }

    if let Some(updates) = args.get("onSuccessUpdateEmail").and_then(Value::as_object) {
        for (key, patch) in updates {
            let creation_id = key.strip_prefix('#').unwrap_or(key);
            if let Some(document_id) = creation_to_email.get(creation_id) {
                let email_id = document_id.to_string();
                match apply_patch(ctx, account, *document_id, patch) {
                    Ok(true) => {
                        updated.insert(email_id, Value::Null);
                    }
                    Ok(false) => {
                        not_updated.insert(email_id, set_error("notFound"));
                    }
                    Err(_) => {
                        not_updated.insert(email_id, set_error("serverFail"));
                    }
                }
            }
        }
    }

    Invocation::new(
        "EmailSubmission/set",
        json!({
            "accountId": account_id(args),
            "oldState": STATE,
            "newState": STATE,
            "created": created,
            "updated": updated,
            "destroyed": [],
            "notCreated": not_created,
            "notUpdated": not_updated,
            "notDestroyed": {},
        }),
        call_id,
    )
}

struct Sent {
    submission_id: u32,
    email_id: u32,
}

fn submit(
    ctx: &JmapContext,
    account: u32,
    submission: &Value,
) -> std::result::Result<Sent, &'static str> {
    let submitter = ctx.submitter.as_ref().ok_or("serverFail")?;
    let email_id = submission
        .get("emailId")
        .and_then(Value::as_str)
        .and_then(|id| id.parse::<u32>().ok())
        .ok_or("invalidProperties")?;
    let raw = load_raw(ctx.store.as_ref(), ctx.blobs.as_ref(), account, email_id)
        .map_err(|_| "serverFail")?
        .ok_or("invalidEmail")?;
    let parsed = MessageParser::default().parse(&raw).ok_or("invalidEmail")?;

    let owned = account_addresses(ctx).map_err(|_| "serverFail")?;
    let return_path = match envelope_mail_from(submission) {
        Some(address) => {
            if !owned.iter().any(|own| own.eq_ignore_ascii_case(&address)) {
                return Err("forbiddenMailFrom");
            }
            address
        }
        None => owned.first().cloned().ok_or("forbiddenMailFrom")?,
    };

    let mut recipients = envelope_rcpt_to(submission);
    if recipients.is_empty() {
        recipients = collect_recipients(&parsed);
    }
    let recipients = dedup_recipients(recipients);
    if recipients.is_empty() {
        return Err("noRecipients");
    }

    let outgoing = strip_bcc(&raw, &parsed);
    submitter(&outgoing, &return_path, &recipients).map_err(|_| "serverFail")?;

    let submission_id = allocate_submission_id(ctx, account).map_err(|_| "serverFail")?;
    Ok(Sent {
        submission_id,
        email_id,
    })
}

fn account_addresses(ctx: &JmapContext) -> Result<Vec<String>> {
    let account = ctx.directory.accounts().get(ctx.account_id)?;
    let domain = ctx.directory.domains().get(account.domain_id)?;
    let mut addresses = vec![format!("{}@{}", account.local_part, domain.name)];
    addresses.extend(
        account
            .aliases
            .iter()
            .filter(|alias| !alias.trim().is_empty())
            .cloned(),
    );
    Ok(addresses)
}

fn envelope_mail_from(submission: &Value) -> Option<String> {
    submission
        .get("envelope")
        .and_then(|envelope| envelope.get("mailFrom"))
        .and_then(|from| from.get("email"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn envelope_rcpt_to(submission: &Value) -> Vec<String> {
    submission
        .get("envelope")
        .and_then(|envelope| envelope.get("rcptTo"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("email").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn collect_recipients(message: &Message<'_>) -> Vec<String> {
    let mut recipients = Vec::new();
    for address in [message.to(), message.cc(), message.bcc()]
        .into_iter()
        .flatten()
    {
        append_addresses(address, &mut recipients);
    }
    recipients
}

fn append_addresses(address: &Address<'_>, out: &mut Vec<String>) {
    for addr in address.iter() {
        if let Some(email) = addr.address.as_deref() {
            out.push(email.to_string());
        }
    }
}

fn dedup_recipients(recipients: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    recipients
        .into_iter()
        .filter(|address| seen.insert(address.to_ascii_lowercase()))
        .collect()
}

fn strip_bcc(raw: &[u8], parsed: &Message<'_>) -> Vec<u8> {
    let mut ranges: Vec<(usize, usize)> = parsed
        .headers()
        .iter()
        .filter(|header| header.name == mail_parser::HeaderName::Bcc)
        .map(|header| (header.offset_field as usize, header.offset_end as usize))
        .collect();
    if ranges.is_empty() {
        return raw.to_vec();
    }
    ranges.sort_unstable();
    let mut out = Vec::with_capacity(raw.len());
    let mut cursor = 0;
    for (start, end) in ranges {
        out.extend_from_slice(&raw[cursor..start.min(raw.len())]);
        cursor = end.min(raw.len()).max(cursor);
    }
    out.extend_from_slice(&raw[cursor..]);
    out
}

fn set_error(kind: &str) -> Value {
    json!({ "type": kind })
}

pub(crate) fn submission_key(account: u32, submission_id: u32) -> Vec<u8> {
    Key::new(
        Subspace::Property,
        account,
        Collection::EmailSubmission,
        submission_id,
    )
    .encode()
}

fn allocate_submission_id(ctx: &JmapContext, account: u32) -> Result<u32> {
    let key = Key::new(Subspace::Counter, account, Collection::EmailSubmission, 0).encode();
    Ok(ctx.store.add_and_get(&key, 1)? as u32)
}

// A sent message can no longer be recalled, so the record is written with undoStatus "final".
fn persist_submission(ctx: &JmapContext, account: u32, sent: &Sent) -> Result<()> {
    let record = json!({
        "id": sent.submission_id.to_string(),
        "emailId": sent.email_id.to_string(),
        "undoStatus": "final",
    });
    let bytes = serde_json::to_vec(&record)
        .map_err(|err| Error::serialize(format!("could not encode the submission: {err}")))?;
    ctx.store
        .put(&submission_key(account, sent.submission_id), &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_context;

    #[test]
    fn a_submission_without_a_submitter_is_not_created() {
        let ctx = test_context();
        let response = submission_set(
            &ctx,
            &json!({"accountId": "1", "create": {"send": {"emailId": "1"}}}),
            "c0",
        );
        assert_eq!(response.name(), "EmailSubmission/set");
        assert!(response.arguments()["notCreated"]["send"].is_object());
    }

    fn seeded_submission_context_with(raw: &[u8]) -> (JmapContext, u32) {
        use crate::context::test_context_with_account;
        use irixmail_mail::{
            allocate_document_id, append_message, provision_mailboxes, AppendRequest, INBOX_ID,
        };
        use std::sync::Arc;

        let mut ctx = test_context_with_account();
        ctx.submitter = Some(Arc::new(|_: &[u8], _: &str, _: &[String]| Ok(())));
        let account = ctx.account_id as u32;
        let record = ctx.directory.accounts().get(ctx.account_id).unwrap();
        let mailboxes = provision_mailboxes(record.created_at);
        let inbox = mailboxes.iter().find(|m| m.id == INBOX_ID).unwrap();
        let email = allocate_document_id(ctx.store.as_ref(), account).unwrap();
        append_message(
            ctx.store.as_ref(),
            ctx.blobs.as_ref(),
            ctx.notifier.as_ref(),
            &AppendRequest {
                account: &record,
                mailbox: inbox,
                flags: vec![],
                received_at: 0,
                document_id: email,
                raw,
            },
        )
        .unwrap();
        (ctx, email)
    }

    fn seeded_submission_context() -> (JmapContext, u32) {
        seeded_submission_context_with(
            b"Subject: Send\r\nFrom: alice@example.com\r\nTo: bob@example.net\r\n\r\nbody\r\n",
        )
    }

    type Calls = std::sync::Arc<std::sync::Mutex<Vec<(Vec<u8>, String, Vec<String>)>>>;

    fn capture_submitter(ctx: &mut JmapContext) -> Calls {
        use std::sync::Arc;

        let calls: Calls = Calls::default();
        let sink = Arc::clone(&calls);
        ctx.submitter = Some(Arc::new(move |raw: &[u8], from: &str, rcpts: &[String]| {
            sink.lock()
                .unwrap()
                .push((raw.to_vec(), from.to_string(), rcpts.to_vec()));
            Ok(())
        }));
        calls
    }

    fn with_flaky_store(
        ctx: &JmapContext,
    ) -> (
        JmapContext,
        std::sync::Arc<crate::context::test_flaky::FlakyStore>,
    ) {
        use std::sync::Arc;

        let flaky = crate::context::test_flaky::FlakyStore::wrap(Arc::clone(&ctx.store));
        let store: Arc<dyn irixmail_store::Store> = flaky.clone();
        let wrapped = JmapContext::from_parts(
            store,
            Arc::clone(&ctx.blobs),
            Arc::clone(&ctx.notifier),
            ctx.directory.clone(),
            ctx.account_id,
            ctx.submitter.clone(),
        );
        (wrapped, flaky)
    }

    #[test]
    fn a_successful_submission_is_persisted() {
        let (ctx, email) = seeded_submission_context();
        let account = ctx.account_id as u32;

        let response = submission_set(
            &ctx,
            &json!({"accountId": account.to_string(), "create": {"send": {"emailId": email.to_string()}}}),
            "c0",
        );
        assert!(response.arguments()["created"]["send"].is_object());
        let id: u32 = response.arguments()["created"]["send"]["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!(ctx
            .store
            .get(&submission_key(account, id))
            .unwrap()
            .is_some());
    }

    #[test]
    fn a_submission_whose_record_fails_to_persist_is_not_reported_created() {
        use std::sync::atomic::Ordering;

        let (base, email) = seeded_submission_context();
        let account = base.account_id as u32;
        let (ctx, flaky) = with_flaky_store(&base);
        flaky.fail_puts.store(true, Ordering::SeqCst);

        let response = submission_set(
            &ctx,
            &json!({"accountId": account.to_string(), "create": {"send": {"emailId": email.to_string()}}}),
            "c0",
        );

        assert!(
            response.arguments()["created"]
                .as_object()
                .unwrap()
                .is_empty(),
            "a lost record must not be reported created: {:?}",
            response.arguments()
        );
        assert_eq!(
            response.arguments()["notCreated"]["send"]["type"],
            "serverFail"
        );

        let prefix = irixmail_store::KeyPrefix::collection(
            Subspace::Property,
            account,
            Collection::EmailSubmission,
        );
        let mut stored = 0;
        base.store
            .iterate(&prefix, &mut |_key, _value| {
                stored += 1;
                Ok(irixmail_store::Flow::Continue)
            })
            .unwrap();
        assert_eq!(stored, 0, "no half-written submission record may remain");
    }

    #[test]
    fn a_successful_on_success_update_email_patch_is_acknowledged() {
        let (ctx, email) = seeded_submission_context();
        let account = ctx.account_id as u32;

        let response = submission_set(
            &ctx,
            &json!({
                "accountId": account.to_string(),
                "create": {"send": {"emailId": email.to_string()}},
                "onSuccessUpdateEmail": {"#send": {"keywords/$seen": true}}
            }),
            "c0",
        );

        assert!(response.arguments()["created"]["send"].is_object());
        assert!(
            response.arguments()["updated"]
                .as_object()
                .unwrap()
                .contains_key(&email.to_string()),
            "the applied patch must be acknowledged: {:?}",
            response.arguments()
        );
        let data = irixmail_mail::load_data(ctx.store.as_ref(), account, email)
            .unwrap()
            .unwrap();
        assert!(data.keywords.iter().any(|k| k.to_jmap() == Some("$seen")));
    }

    #[test]
    fn the_bcc_header_is_stripped_from_the_queued_message_but_kept_in_the_envelope() {
        let (mut ctx, email) = seeded_submission_context_with(
            b"Subject: Send\r\nFrom: alice@example.com\r\nTo: bob@example.net\r\nBcc: carol@example.net\r\n\r\nbody\r\n",
        );
        let calls = capture_submitter(&mut ctx);
        let account = ctx.account_id as u32;

        let response = submission_set(
            &ctx,
            &json!({"accountId": account.to_string(), "create": {"send": {"emailId": email.to_string()}}}),
            "c0",
        );
        assert!(response.arguments()["created"]["send"].is_object());

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let (raw, _, recipients) = &calls[0];
        let text = String::from_utf8_lossy(raw);
        assert!(
            !text.to_ascii_lowercase().contains("bcc:"),
            "the queued bytes disclose the blind recipients: {text}"
        );
        assert!(text.contains("To: bob@example.net"));
        assert!(text.contains("body"));
        assert!(recipients.iter().any(|r| r == "carol@example.net"));
        assert!(recipients.iter().any(|r| r == "bob@example.net"));
    }

    #[test]
    fn an_envelope_rcpt_to_overrides_the_header_recipients() {
        let (mut ctx, email) = seeded_submission_context();
        let calls = capture_submitter(&mut ctx);
        let account = ctx.account_id as u32;

        let response = submission_set(
            &ctx,
            &json!({
                "accountId": account.to_string(),
                "create": {"send": {
                    "emailId": email.to_string(),
                    "envelope": {
                        "mailFrom": {"email": "alice@example.com"},
                        "rcptTo": [{"email": "dave@example.org"}]
                    }
                }}
            }),
            "c0",
        );
        assert!(response.arguments()["created"]["send"].is_object());

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let (_, from, recipients) = &calls[0];
        assert_eq!(from, "alice@example.com");
        assert_eq!(recipients, &vec!["dave@example.org".to_string()]);
    }

    #[test]
    fn a_mail_from_the_account_does_not_own_is_refused() {
        let (mut ctx, email) = seeded_submission_context();
        let calls = capture_submitter(&mut ctx);
        let account = ctx.account_id as u32;

        let response = submission_set(
            &ctx,
            &json!({
                "accountId": account.to_string(),
                "create": {"send": {
                    "emailId": email.to_string(),
                    "envelope": {"mailFrom": {"email": "mallory@evil.example"}}
                }}
            }),
            "c0",
        );

        assert_eq!(
            response.arguments()["notCreated"]["send"]["type"],
            "forbiddenMailFrom",
            "got: {:?}",
            response.arguments()
        );
        assert!(calls.lock().unwrap().is_empty(), "nothing must be sent");
    }

    #[test]
    fn a_recipient_listed_twice_is_delivered_once() {
        let (mut ctx, email) = seeded_submission_context_with(
            b"Subject: Send\r\nFrom: alice@example.com\r\nTo: bob@example.net\r\nCc: BOB@example.net\r\n\r\nbody\r\n",
        );
        let calls = capture_submitter(&mut ctx);
        let account = ctx.account_id as u32;

        let response = submission_set(
            &ctx,
            &json!({"accountId": account.to_string(), "create": {"send": {"emailId": email.to_string()}}}),
            "c0",
        );
        assert!(response.arguments()["created"]["send"].is_object());

        let calls = calls.lock().unwrap();
        assert_eq!(
            calls[0].2.len(),
            1,
            "duplicates must collapse: {:?}",
            calls[0].2
        );
    }

    #[test]
    fn resending_the_same_email_yields_two_distinct_submissions() {
        use irixmail_store::{Collection, Flow, KeyPrefix, Subspace};

        let (ctx, email) = seeded_submission_context();
        let account = ctx.account_id as u32;

        let first = submission_set(
            &ctx,
            &json!({"accountId": account.to_string(), "create": {"s1": {"emailId": email.to_string()}}}),
            "c0",
        );
        let second = submission_set(
            &ctx,
            &json!({"accountId": account.to_string(), "create": {"s2": {"emailId": email.to_string()}}}),
            "c1",
        );

        let first_id = first.arguments()["created"]["s1"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let second_id = second.arguments()["created"]["s2"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_ne!(first_id, second_id, "a resend must not collide");

        let prefix =
            KeyPrefix::collection(Subspace::Property, account, Collection::EmailSubmission);
        let mut stored = 0;
        ctx.store
            .iterate(&prefix, &mut |_key, _value| {
                stored += 1;
                Ok(Flow::Continue)
            })
            .unwrap();
        assert_eq!(stored, 2, "both submission records must survive");
    }

    #[test]
    fn a_failed_on_success_update_email_patch_is_reported() {
        use std::sync::atomic::Ordering;

        let (base, email) = seeded_submission_context();
        let account = base.account_id as u32;
        let (ctx, flaky) = with_flaky_store(&base);
        flaky.fail_batches.store(true, Ordering::SeqCst);

        let response = submission_set(
            &ctx,
            &json!({
                "accountId": account.to_string(),
                "create": {"send": {"emailId": email.to_string()}},
                "onSuccessUpdateEmail": {"#send": {"keywords/$seen": true}}
            }),
            "c0",
        );

        assert!(response.arguments()["created"]["send"].is_object());
        assert_eq!(
            response.arguments()["notUpdated"][email.to_string()]["type"],
            "serverFail",
            "a dropped patch must be reported: {:?}",
            response.arguments()
        );
    }
}
