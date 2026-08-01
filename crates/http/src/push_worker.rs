use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

use irixmail_core::IdGenerator;
use irixmail_jmap::push_store::{
    load_subscriptions, save_subscriptions_quiet, PushSubscriptionRecord,
};
use irixmail_jmap::webpush::{encrypt, load_or_create_vapid, vapid_authorization, VapidKeys};
use irixmail_store::{ChangeNotice, ChangeNotifier, Collection, NewMailNotice, Store};

const DEBOUNCE: Duration = Duration::from_secs(1);
const SEND_ATTEMPTS: usize = 3;
const RETRY_PAUSE: Duration = Duration::from_secs(2);
const MAX_VERIFICATION_SENDS: u32 = 5;
const UNVERIFIED_TTL_SECS: u64 = 15 * 60;

enum SendOutcome {
    Delivered,
    Gone,
    Failed(String),
}

enum VerificationAction {
    Send,
    Skip,
    Destroy,
}

fn verification_action(record: &PushSubscriptionRecord, now: u64) -> VerificationAction {
    if record.verified {
        return VerificationAction::Skip;
    }
    if now.saturating_sub(IdGenerator::timestamp_of(record.id)) > UNVERIFIED_TTL_SECS {
        return VerificationAction::Destroy;
    }
    if record.verification_sends >= MAX_VERIFICATION_SENDS {
        return VerificationAction::Skip;
    }
    VerificationAction::Send
}

pub async fn run_push_worker(
    store: Arc<dyn Store>,
    notifier: Arc<ChangeNotifier>,
    contact: String,
    navigate: String,
) {
    let mut firehose = notifier.subscribe_all();
    let mut mail_feed = notifier.subscribe_new_mail();
    loop {
        let mut pending: HashMap<u32, BTreeMap<&'static str, u64>> = HashMap::new();
        let mut verify: HashSet<u32> = HashSet::new();
        let mut new_mail: HashMap<u32, Vec<NewMailNotice>> = HashMap::new();
        tokio::select! {
            notice = firehose.recv() => match notice {
                Ok(notice) => note(&mut pending, &mut verify, notice),
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return,
            },
            notice = mail_feed.recv() => match notice {
                Ok(notice) => note_mail(&mut new_mail, notice),
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return,
            },
        }
        let deadline = tokio::time::Instant::now() + DEBOUNCE;
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => break,
                notice = firehose.recv() => match notice {
                    Ok(notice) => note(&mut pending, &mut verify, notice),
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                },
                notice = mail_feed.recv() => match notice {
                    Ok(notice) => note_mail(&mut new_mail, notice),
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                },
            }
        }
        if pending.is_empty() && verify.is_empty() {
            continue;
        }
        let store = Arc::clone(&store);
        let contact = contact.clone();
        let navigate = navigate.clone();
        let task = tokio::task::spawn_blocking(move || {
            process(
                store.as_ref(),
                &pending,
                &verify,
                &new_mail,
                &contact,
                &navigate,
            )
        });
        if let Err(error) = task.await {
            tracing::warn!(target: "irixmail::jmap", error = %error, "push delivery task panicked");
        }
    }
}

fn note_mail(new_mail: &mut HashMap<u32, Vec<NewMailNotice>>, notice: NewMailNotice) {
    new_mail.entry(notice.account_id).or_default().push(notice);
}

fn apple_endpoint(url: &str) -> bool {
    url.starts_with("https://web.push.apple.com/")
}

fn note(
    pending: &mut HashMap<u32, BTreeMap<&'static str, u64>>,
    verify: &mut HashSet<u32>,
    notice: ChangeNotice,
) {
    match notice.collection {
        Collection::Email => {
            pending
                .entry(notice.account_id)
                .or_default()
                .insert("Email", notice.change_id);
        }
        Collection::Mailbox => {
            pending
                .entry(notice.account_id)
                .or_default()
                .insert("Mailbox", notice.change_id);
        }
        Collection::Calendar
        | Collection::CalendarEvent
        | Collection::AddressBook
        | Collection::ContactCard => {
            if let Some(name) = irixmail_jmap::eventsource::type_name(notice.collection) {
                pending
                    .entry(notice.account_id)
                    .or_default()
                    .insert(name, notice.change_id);
            }
        }
        Collection::PushSubscription => {
            verify.insert(notice.account_id);
        }
        _ => {}
    }
}

fn process(
    store: &dyn Store,
    pending: &HashMap<u32, BTreeMap<&'static str, u64>>,
    verify: &HashSet<u32>,
    new_mail: &HashMap<u32, Vec<NewMailNotice>>,
    contact: &str,
    navigate: &str,
) {
    let vapid = match load_or_create_vapid(store) {
        Ok(vapid) => vapid,
        Err(error) => {
            tracing::warn!(target: "irixmail::jmap", error = %error, "vapid key unavailable");
            return;
        }
    };
    let now = now_seconds();
    for &account_id in verify {
        let Ok(mut subscriptions) = load_subscriptions(store, account_id, now) else {
            continue;
        };
        let mut dropped: Vec<u64> = Vec::new();
        let mut dirty = false;
        for subscription in subscriptions.iter_mut() {
            match verification_action(subscription, now) {
                VerificationAction::Skip => continue,
                VerificationAction::Destroy => {
                    tracing::info!(
                        target: "irixmail::jmap",
                        account = account_id,
                        subscription = subscription.id,
                        "dropping stale unverified push subscription"
                    );
                    dropped.push(subscription.id);
                    continue;
                }
                VerificationAction::Send => {}
            }
            subscription.verification_sends += 1;
            dirty = true;
            let payload = json!({
                "@type": "PushVerification",
                "pushSubscriptionId": subscription.id.to_string(),
                "verificationCode": subscription.verification_code,
            })
            .to_string();
            match send(subscription, payload.as_bytes(), &vapid, contact, now) {
                SendOutcome::Delivered => tracing::info!(
                    target: "irixmail::jmap",
                    account = account_id,
                    subscription = subscription.id,
                    "push verification sent"
                ),
                SendOutcome::Gone => {
                    tracing::warn!(
                        target: "irixmail::jmap",
                        account = account_id,
                        subscription = subscription.id,
                        endpoint = %subscription.url,
                        "push verification endpoint gone, dropping subscription"
                    );
                    dropped.push(subscription.id);
                }
                SendOutcome::Failed(error) => tracing::warn!(
                    target: "irixmail::jmap",
                    account = account_id,
                    subscription = subscription.id,
                    endpoint = %subscription.url,
                    error = %error,
                    "push verification failed"
                ),
            }
        }
        if dirty || !dropped.is_empty() {
            let remaining: Vec<PushSubscriptionRecord> = subscriptions
                .into_iter()
                .filter(|record| !dropped.contains(&record.id))
                .collect();
            if let Err(error) = save_subscriptions_quiet(store, account_id, &remaining) {
                tracing::warn!(
                    target: "irixmail::jmap",
                    account = account_id,
                    error = %error,
                    "saving push verification state failed"
                );
            }
        }
    }
    for (&account_id, types) in pending {
        let Ok(subscriptions) = load_subscriptions(store, account_id, now) else {
            continue;
        };
        let arrived = new_mail
            .get(&account_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut gone: Vec<u64> = Vec::new();
        for subscription in subscriptions.iter().filter(|record| record.verified) {
            let changed: serde_json::Map<String, serde_json::Value> = types
                .iter()
                .filter(|(type_name, _)| subscription.wants(type_name))
                .map(|(type_name, change_id)| (type_name.to_string(), json!(change_id.to_string())))
                .collect();
            if changed.is_empty() {
                continue;
            }
            let payload = state_change_payload(account_id, changed, arrived, navigate);
            // Apple shows a notification for every push, so mail-silent waves stay off that path.
            if payload.get("notification").is_none() && apple_endpoint(&subscription.url) {
                continue;
            }
            let payload = payload.to_string();
            match send(subscription, payload.as_bytes(), &vapid, contact, now) {
                SendOutcome::Delivered => tracing::info!(
                    target: "irixmail::jmap",
                    account = account_id,
                    subscription = subscription.id,
                    "push delivered"
                ),
                SendOutcome::Gone => {
                    tracing::info!(
                        target: "irixmail::jmap",
                        account = account_id,
                        subscription = subscription.id,
                        "push endpoint gone, dropping subscription"
                    );
                    gone.push(subscription.id);
                }
                SendOutcome::Failed(error) => tracing::warn!(
                    target: "irixmail::jmap",
                    account = account_id,
                    subscription = subscription.id,
                    error = %error,
                    "push delivery failed"
                ),
            }
        }
        if !gone.is_empty() {
            let remaining: Vec<PushSubscriptionRecord> = subscriptions
                .into_iter()
                .filter(|record| !gone.contains(&record.id))
                .collect();
            if let Err(error) = save_subscriptions_quiet(store, account_id, &remaining) {
                tracing::warn!(
                    target: "irixmail::jmap",
                    account = account_id,
                    error = %error,
                    "pruning dead push subscriptions failed"
                );
            }
        }
    }
}

fn state_change_payload(
    account_id: u32,
    changed: serde_json::Map<String, serde_json::Value>,
    new_mail: &[NewMailNotice],
    navigate: &str,
) -> serde_json::Value {
    let has_mail = changed.contains_key("Email");
    let mut payload = json!({
        "@type": "StateChange",
        "changed": { account_id.to_string(): changed },
    });
    let Some(first) = new_mail.first().filter(|_| has_mail) else {
        return payload;
    };
    let sender = if first.sender.is_empty() {
        "New mail"
    } else {
        &first.sender
    };
    let subject = if first.subject.is_empty() {
        "You have new mail."
    } else {
        &first.subject
    };
    let (title, body, target) = if new_mail.len() == 1 {
        let target = format!("{navigate}{}/{}", first.mailbox_id, first.document_id);
        (sender.to_string(), subject.to_string(), target)
    } else {
        (
            format!("{} new messages", new_mail.len()),
            format!("{sender}: {subject}"),
            navigate.to_string(),
        )
    };
    // web_push 8030 makes Safari 18.4+ render this declaratively without waking the SW
    payload["web_push"] = json!(8030);
    payload["notification"] = json!({
        "title": title,
        "body": body,
        "tag": "irixmail-new-mail",
        "navigate": target,
    });
    payload
}

fn send(
    subscription: &PushSubscriptionRecord,
    payload: &[u8],
    vapid: &VapidKeys,
    contact: &str,
    now: u64,
) -> SendOutcome {
    let (body, encrypted) = match &subscription.keys {
        Some(keys) => {
            let (Some(p256dh), Some(auth)) = (decode_key(&keys.p256dh), decode_key(&keys.auth))
            else {
                return SendOutcome::Failed("subscription keys are undecodable".to_string());
            };
            match encrypt(&p256dh, &auth, payload) {
                Ok(body) => (body, true),
                Err(error) => return SendOutcome::Failed(error.to_string()),
            }
        }
        None => (payload.to_vec(), false),
    };
    let authorization = match vapid_authorization(vapid, &subscription.url, contact, now) {
        Ok(header) => header,
        Err(error) => return SendOutcome::Failed(error.to_string()),
    };
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    let mut last_error = String::new();
    for attempt in 0..SEND_ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(RETRY_PAUSE);
        }
        let mut request = agent
            .post(&subscription.url)
            .header("TTL", "86400")
            .header("Urgency", "normal")
            .header("Authorization", &authorization);
        if encrypted {
            request = request
                .header("Content-Encoding", "aes128gcm")
                .header("Content-Type", "application/octet-stream");
        } else {
            request = request.header("Content-Type", "application/json");
        }
        match request.send(&body[..]) {
            Ok(_) => return SendOutcome::Delivered,
            Err(ureq::Error::StatusCode(404 | 410)) => return SendOutcome::Gone,
            Err(error) => last_error = error.to_string(),
        }
    }
    SendOutcome::Failed(last_error)
}

fn decode_key(value: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(value)
        .or_else(|_| URL_SAFE.decode(value))
        .ok()
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(account_id: u32, collection: Collection, change_id: u64) -> ChangeNotice {
        ChangeNotice::new(account_id, collection, change_id)
    }

    #[test]
    fn notes_coalesce_the_latest_change_per_type() {
        let mut pending = HashMap::new();
        let mut verify = HashSet::new();
        note(&mut pending, &mut verify, notice(1, Collection::Email, 5));
        note(&mut pending, &mut verify, notice(1, Collection::Email, 9));
        note(&mut pending, &mut verify, notice(1, Collection::Mailbox, 7));
        note(&mut pending, &mut verify, notice(2, Collection::Thread, 3));
        note(
            &mut pending,
            &mut verify,
            notice(3, Collection::PushSubscription, 1),
        );

        assert_eq!(pending[&1]["Email"], 9);
        assert_eq!(pending[&1]["Mailbox"], 7);
        assert!(!pending.contains_key(&2));
        assert!(verify.contains(&3));
    }

    fn sub_record(id: u64) -> PushSubscriptionRecord {
        PushSubscriptionRecord {
            id,
            device_client_id: "device".to_string(),
            url: "https://push.example.com/sub".to_string(),
            keys: None,
            verification_code: "code".to_string(),
            verified: false,
            expires: u64::MAX,
            types: Vec::new(),
            verification_sends: 0,
        }
    }

    fn snowflake_at(secs: u64) -> u64 {
        ((secs - 1_704_067_200) * 1_000) << 22
    }

    #[test]
    fn fresh_unverified_subscriptions_are_sent_verification() {
        let now = 1_800_000_000;
        let record = sub_record(snowflake_at(now - 60));
        assert!(matches!(
            verification_action(&record, now),
            VerificationAction::Send
        ));
    }

    #[test]
    fn verification_sends_are_capped() {
        let now = 1_800_000_000;
        let mut record = sub_record(snowflake_at(now - 60));
        record.verification_sends = MAX_VERIFICATION_SENDS;
        assert!(matches!(
            verification_action(&record, now),
            VerificationAction::Skip
        ));
    }

    #[test]
    fn stale_unverified_subscriptions_are_destroyed() {
        let now = 1_800_000_000;
        let record = sub_record(snowflake_at(now - 20 * 60));
        assert!(matches!(
            verification_action(&record, now),
            VerificationAction::Destroy
        ));
    }

    #[test]
    fn legacy_small_ids_are_treated_as_stale() {
        let record = sub_record(1);
        assert!(matches!(
            verification_action(&record, 1_800_000_000),
            VerificationAction::Destroy
        ));
    }

    #[test]
    fn verified_subscriptions_need_no_verification() {
        let now = 1_800_000_000;
        let mut record = sub_record(snowflake_at(now - 30 * 24 * 3600));
        record.verified = true;
        assert!(matches!(
            verification_action(&record, now),
            VerificationAction::Skip
        ));
    }

    fn mail_notice(document_id: u32, sender: &str, subject: &str) -> NewMailNotice {
        NewMailNotice {
            account_id: 1,
            document_id,
            mailbox_id: 1,
            sender: sender.to_string(),
            subject: subject.to_string(),
        }
    }

    #[test]
    fn mail_state_changes_without_new_mail_stay_silent() {
        let mut changed = serde_json::Map::new();
        changed.insert("Email".to_string(), json!("9"));
        let payload = state_change_payload(1, changed, &[], "https://mail.example.com/webmail/");

        assert_eq!(payload["@type"], "StateChange");
        assert_eq!(payload["changed"]["1"]["Email"], "9");
        assert!(payload.get("web_push").is_none());
        assert!(payload.get("notification").is_none());
    }

    #[test]
    fn a_single_new_mail_carries_sender_subject_and_a_deep_link() {
        let mut changed = serde_json::Map::new();
        changed.insert("Email".to_string(), json!("9"));
        let payload = state_change_payload(
            1,
            changed,
            &[mail_notice(42, "Ana Lang", "Hello")],
            "https://mail.example.com/webmail/",
        );

        assert_eq!(payload["@type"], "StateChange");
        assert_eq!(payload["web_push"], 8030);
        assert_eq!(payload["notification"]["title"], "Ana Lang");
        assert_eq!(payload["notification"]["body"], "Hello");
        assert_eq!(payload["notification"]["tag"], "irixmail-new-mail");
        assert_eq!(
            payload["notification"]["navigate"],
            "https://mail.example.com/webmail/1/42"
        );
    }

    #[test]
    fn multiple_new_mails_coalesce_into_a_count() {
        let mut changed = serde_json::Map::new();
        changed.insert("Email".to_string(), json!("9"));
        let payload = state_change_payload(
            1,
            changed,
            &[
                mail_notice(42, "Ana Lang", "Hello"),
                mail_notice(43, "Bob", "Re: Hello"),
            ],
            "https://mail.example.com/webmail/",
        );

        assert_eq!(payload["notification"]["title"], "2 new messages");
        assert_eq!(payload["notification"]["body"], "Ana Lang: Hello");
        assert_eq!(payload["notification"]["tag"], "irixmail-new-mail");
        assert_eq!(
            payload["notification"]["navigate"],
            "https://mail.example.com/webmail/"
        );
    }

    #[test]
    fn a_nameless_or_subjectless_mail_falls_back_to_generic_text() {
        let mut changed = serde_json::Map::new();
        changed.insert("Email".to_string(), json!("9"));
        let payload = state_change_payload(
            1,
            changed,
            &[mail_notice(42, "", "")],
            "https://mail.example.com/webmail/",
        );

        assert_eq!(payload["notification"]["title"], "New mail");
        assert_eq!(payload["notification"]["body"], "You have new mail.");
    }

    #[test]
    fn non_mail_state_changes_stay_silent() {
        let mut changed = serde_json::Map::new();
        changed.insert("CalendarEvent".to_string(), json!("4"));
        let payload = state_change_payload(
            1,
            changed,
            &[mail_notice(42, "Ana Lang", "Hello")],
            "https://mail.example.com/webmail/",
        );

        assert_eq!(payload["@type"], "StateChange");
        assert!(payload.get("web_push").is_none());
        assert!(payload.get("notification").is_none());
    }

    #[test]
    fn apple_endpoints_are_recognized() {
        assert!(apple_endpoint("https://web.push.apple.com/QOln2CGWyI"));
        assert!(!apple_endpoint("https://fcm.googleapis.com/fcm/send/x"));
    }

    #[test]
    fn dav_changes_are_noted_for_push() {
        let mut pending = HashMap::new();
        let mut verify = HashSet::new();
        note(
            &mut pending,
            &mut verify,
            notice(1, Collection::Calendar, 2),
        );
        note(
            &mut pending,
            &mut verify,
            notice(1, Collection::CalendarEvent, 4),
        );
        note(
            &mut pending,
            &mut verify,
            notice(1, Collection::AddressBook, 6),
        );
        note(
            &mut pending,
            &mut verify,
            notice(1, Collection::ContactCard, 8),
        );

        assert_eq!(pending[&1]["Calendar"], 2);
        assert_eq!(pending[&1]["CalendarEvent"], 4);
        assert_eq!(pending[&1]["AddressBook"], 6);
        assert_eq!(pending[&1]["ContactCard"], 8);
        assert!(verify.is_empty());
    }
}
