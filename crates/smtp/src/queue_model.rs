use irixmail_store::BlobHash;
use serde::{Deserialize, Serialize};

#[derive(
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    PartialEq,
    Eq,
)]
#[rkyv(derive(Debug), compare(PartialEq))]
pub enum RecipientStatus {
    Scheduled,
    Delivered,
    Deferred(String),
    Bounced(String),
}

impl RecipientStatus {
    pub fn is_pending(&self) -> bool {
        matches!(
            self,
            RecipientStatus::Scheduled | RecipientStatus::Deferred(_)
        )
    }

    pub fn is_settled(&self) -> bool {
        !self.is_pending()
    }
}

#[derive(
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
)]
#[rkyv(derive(Debug), compare(PartialEq))]
pub struct RetrySchedule {
    pub attempts: u32,
    pub due: u64,
}

impl RetrySchedule {
    pub fn first(due: u64) -> Self {
        RetrySchedule { attempts: 0, due }
    }

    pub fn is_due(&self, now: u64) -> bool {
        now >= self.due
    }
}

#[derive(
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
)]
#[rkyv(derive(Debug), compare(PartialEq))]
pub struct NotifySchedule {
    pub sent: u32,
    pub due: u64,
}

impl NotifySchedule {
    pub fn first(due: u64) -> Self {
        NotifySchedule { sent: 0, due }
    }

    pub fn is_due(&self, now: u64) -> bool {
        now >= self.due
    }
}

#[derive(
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
)]
#[rkyv(derive(Debug), compare(PartialEq))]
pub enum Expiry {
    At(u64),
    Attempts(u32),
}

impl Expiry {
    pub fn is_expired(&self, attempts: u32, now: u64) -> bool {
        match self {
            Expiry::At(deadline) => now >= *deadline,
            Expiry::Attempts(limit) => attempts >= *limit,
        }
    }
}

#[derive(
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    PartialEq,
    Eq,
)]
#[rkyv(derive(Debug), compare(PartialEq))]
pub struct QueueRecipient {
    pub address: String,
    pub status: RecipientStatus,
    pub retry: RetrySchedule,
    pub notify: NotifySchedule,
    pub expiry: Expiry,
}

impl QueueRecipient {
    pub fn new(address: impl Into<String>, due: u64, expiry: Expiry) -> Self {
        QueueRecipient {
            address: address.into(),
            status: RecipientStatus::Scheduled,
            retry: RetrySchedule::first(due),
            notify: NotifySchedule::first(due),
            expiry,
        }
    }

    pub fn domain(&self) -> &str {
        self.address
            .rsplit_once('@')
            .map(|(_, domain)| domain)
            .unwrap_or("")
    }

    pub fn is_due(&self, now: u64) -> bool {
        self.status.is_pending() && self.retry.is_due(now)
    }

    pub fn has_expired(&self, now: u64) -> bool {
        self.expiry.is_expired(self.retry.attempts, now)
    }
}

#[derive(
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    PartialEq,
    Eq,
)]
#[rkyv(derive(Debug), compare(PartialEq))]
pub struct QueuedMessage {
    pub created: u64,
    pub blob_hash: Vec<u8>,
    pub size: u64,
    pub return_path: String,
    pub recipients: Vec<QueueRecipient>,
}

impl QueuedMessage {
    pub fn new(
        created: u64,
        blob_hash: &BlobHash,
        size: u64,
        return_path: impl Into<String>,
        recipients: Vec<QueueRecipient>,
    ) -> Self {
        QueuedMessage {
            created,
            blob_hash: blob_hash.as_bytes().to_vec(),
            size,
            return_path: return_path.into(),
            recipients,
        }
    }

    pub fn blob_hash(&self) -> BlobHash {
        BlobHash::from_bytes(self.blob_hash.clone())
    }

    pub fn next_due(&self) -> Option<u64> {
        self.recipients
            .iter()
            .filter(|rcpt| rcpt.status.is_pending())
            .map(|rcpt| rcpt.retry.due)
            .min()
    }

    pub fn has_due_recipient(&self, now: u64) -> bool {
        self.recipients.iter().any(|rcpt| rcpt.is_due(now))
    }

    pub fn is_complete(&self) -> bool {
        self.recipients.iter().all(|rcpt| rcpt.status.is_settled())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message() -> QueuedMessage {
        let hash = BlobHash::from_bytes(vec![1, 2, 3, 4]);
        QueuedMessage::new(
            1_000,
            &hash,
            512,
            "sender@example.com",
            vec![
                QueueRecipient::new("a@one.example", 1_000, Expiry::At(5_000)),
                QueueRecipient::new("b@two.example", 2_000, Expiry::Attempts(5)),
            ],
        )
    }

    #[test]
    fn a_scheduled_recipient_is_pending_and_a_delivered_one_is_settled() {
        assert!(RecipientStatus::Scheduled.is_pending());
        assert!(RecipientStatus::Deferred("4xx".into()).is_pending());
        assert!(RecipientStatus::Delivered.is_settled());
        assert!(RecipientStatus::Bounced("5xx".into()).is_settled());
    }

    #[test]
    fn a_retry_is_due_once_its_instant_has_passed() {
        let retry = RetrySchedule::first(1_000);
        assert_eq!(retry.attempts, 0);
        assert!(!retry.is_due(999));
        assert!(retry.is_due(1_000));
        assert!(retry.is_due(1_001));
    }

    #[test]
    fn a_warning_is_due_once_its_instant_has_passed() {
        let notify = NotifySchedule::first(2_000);
        assert_eq!(notify.sent, 0);
        assert!(!notify.is_due(1_999));
        assert!(notify.is_due(2_000));
    }

    #[test]
    fn a_time_expiry_lapses_at_its_deadline_regardless_of_attempts() {
        let expiry = Expiry::At(5_000);
        assert!(!expiry.is_expired(99, 4_999));
        assert!(expiry.is_expired(0, 5_000));
    }

    #[test]
    fn an_attempt_expiry_lapses_once_the_cap_is_reached() {
        let expiry = Expiry::Attempts(3);
        assert!(!expiry.is_expired(2, u64::MAX));
        assert!(expiry.is_expired(3, 0));
        assert!(expiry.is_expired(4, 0));
    }

    #[test]
    fn a_recipient_yields_the_domain_after_its_at_sign() {
        let rcpt = QueueRecipient::new("user@mail.example.org", 0, Expiry::Attempts(1));
        assert_eq!(rcpt.domain(), "mail.example.org");
        let bare = QueueRecipient::new("malformed", 0, Expiry::Attempts(1));
        assert_eq!(bare.domain(), "");
    }

    #[test]
    fn a_recipient_is_due_only_while_pending_and_past_its_retry() {
        let mut rcpt = QueueRecipient::new("a@one.example", 1_000, Expiry::Attempts(5));
        assert!(!rcpt.is_due(999));
        assert!(rcpt.is_due(1_000));
        rcpt.status = RecipientStatus::Delivered;
        assert!(!rcpt.is_due(1_000));
    }

    #[test]
    fn next_due_is_the_earliest_pending_recipient_instant() {
        let mut msg = message();
        assert_eq!(msg.next_due(), Some(1_000));
        msg.recipients[0].status = RecipientStatus::Delivered;
        assert_eq!(msg.next_due(), Some(2_000));
        msg.recipients[1].status = RecipientStatus::Bounced("5xx".into());
        assert_eq!(msg.next_due(), None);
    }

    #[test]
    fn a_message_has_a_due_recipient_once_any_pending_one_comes_due() {
        let msg = message();
        assert!(!msg.has_due_recipient(999));
        assert!(msg.has_due_recipient(1_000));
    }

    #[test]
    fn a_message_is_complete_only_when_every_recipient_is_settled() {
        let mut msg = message();
        assert!(!msg.is_complete());
        msg.recipients[0].status = RecipientStatus::Delivered;
        assert!(!msg.is_complete());
        msg.recipients[1].status = RecipientStatus::Bounced("5xx".into());
        assert!(msg.is_complete());
    }

    #[test]
    fn the_blob_hash_round_trips_through_the_stored_bytes() {
        let hash = BlobHash::from_bytes(vec![9, 8, 7, 6, 5]);
        let msg = QueuedMessage::new(0, &hash, 1, "s@example.com", Vec::new());
        assert_eq!(msg.blob_hash(), hash);
    }

    #[test]
    fn the_record_round_trips_through_an_archive() {
        let original = message();
        let bytes = irixmail_store::serialize::archive(&original).expect("archive");
        let restored: QueuedMessage =
            irixmail_store::serialize::deserialize(&bytes).expect("deserialize");
        assert_eq!(original, restored);
    }
}
