use serde::{Deserialize, Serialize};

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
    Hash,
)]
#[rkyv(derive(Debug), compare(PartialEq))]
pub struct MailboxUid {
    pub mailbox_id: u32,
    pub uid: u32,
}

impl MailboxUid {
    pub fn new(mailbox_id: u32, uid: u32) -> Self {
        MailboxUid { mailbox_id, uid }
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
    Hash,
)]
#[rkyv(derive(Debug), compare(PartialEq))]
pub enum Keyword {
    Seen,
    Draft,
    Flagged,
    Answered,
    Deleted,
    Recent,
    Junk,
    NotJunk,
    Forwarded,
    Custom(String),
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
#[rkyv(derive(Debug))]
pub struct MessageData {
    pub mailboxes: Vec<MailboxUid>,
    pub keywords: Vec<Keyword>,
    pub thread_id: u32,
    pub size: u32,
    pub received_at: u64,
    pub sent_at: u64,
}

impl MessageData {
    pub fn new(thread_id: u32, size: u32) -> Self {
        MessageData {
            mailboxes: Vec::new(),
            keywords: Vec::new(),
            thread_id,
            size,
            received_at: 0,
            sent_at: 0,
        }
    }

    pub fn add_mailbox(&mut self, mailbox_id: u32, uid: u32) -> bool {
        if self.mailboxes.iter().any(|m| m.mailbox_id == mailbox_id) {
            false
        } else {
            self.mailboxes.push(MailboxUid::new(mailbox_id, uid));
            true
        }
    }

    pub fn remove_mailbox(&mut self, mailbox_id: u32) -> bool {
        let before = self.mailboxes.len();
        self.mailboxes.retain(|m| m.mailbox_id != mailbox_id);
        self.mailboxes.len() != before
    }

    pub fn in_mailbox(&self, mailbox_id: u32) -> bool {
        self.mailboxes.iter().any(|m| m.mailbox_id == mailbox_id)
    }

    pub fn uid_in(&self, mailbox_id: u32) -> Option<u32> {
        self.mailboxes
            .iter()
            .find(|m| m.mailbox_id == mailbox_id)
            .map(|m| m.uid)
    }

    pub fn add_keyword(&mut self, keyword: Keyword) -> bool {
        if self.keywords.contains(&keyword) {
            false
        } else {
            self.keywords.push(keyword);
            true
        }
    }

    pub fn remove_keyword(&mut self, keyword: &Keyword) -> bool {
        let before = self.keywords.len();
        self.keywords.retain(|k| k != keyword);
        self.keywords.len() != before
    }

    pub fn has_keyword(&self, keyword: &Keyword) -> bool {
        self.keywords.contains(keyword)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_record_starts_empty_with_thread_and_size() {
        let data = MessageData::new(42, 4096);
        assert_eq!(data.thread_id, 42);
        assert_eq!(data.size, 4096);
        assert!(data.mailboxes.is_empty());
        assert!(data.keywords.is_empty());
    }

    #[test]
    fn filing_into_a_mailbox_records_its_uid() {
        let mut data = MessageData::new(1, 10);
        assert!(data.add_mailbox(7, 100));
        assert!(data.in_mailbox(7));
        assert_eq!(data.uid_in(7), Some(100));
        assert_eq!(data.uid_in(8), None);
    }

    #[test]
    fn refiling_into_the_same_mailbox_keeps_the_original_uid() {
        let mut data = MessageData::new(1, 10);
        assert!(data.add_mailbox(7, 100));
        assert!(!data.add_mailbox(7, 999));
        assert_eq!(data.uid_in(7), Some(100));
        assert_eq!(data.mailboxes.len(), 1);
    }

    #[test]
    fn a_message_can_belong_to_several_mailboxes_at_once() {
        let mut data = MessageData::new(1, 10);
        data.add_mailbox(1, 5);
        data.add_mailbox(2, 9);
        data.add_mailbox(3, 1);
        assert_eq!(data.mailboxes.len(), 3);
        assert_eq!(data.uid_in(1), Some(5));
        assert_eq!(data.uid_in(2), Some(9));
        assert_eq!(data.uid_in(3), Some(1));
    }

    #[test]
    fn removing_a_mailbox_reports_whether_it_was_present() {
        let mut data = MessageData::new(1, 10);
        data.add_mailbox(7, 100);
        assert!(data.remove_mailbox(7));
        assert!(!data.in_mailbox(7));
        assert!(!data.remove_mailbox(7));
    }

    #[test]
    fn keywords_are_set_cleared_and_queried_without_duplicates() {
        let mut data = MessageData::new(1, 10);
        assert!(data.add_keyword(Keyword::Seen));
        assert!(!data.add_keyword(Keyword::Seen));
        assert!(data.has_keyword(&Keyword::Seen));
        assert!(data.add_keyword(Keyword::Custom("project-x".to_string())));
        assert_eq!(data.keywords.len(), 2);

        assert!(data.remove_keyword(&Keyword::Seen));
        assert!(!data.has_keyword(&Keyword::Seen));
        assert!(!data.remove_keyword(&Keyword::Flagged));
        assert!(data.has_keyword(&Keyword::Custom("project-x".to_string())));
    }

    #[test]
    fn record_round_trips_through_the_archive() {
        let mut original = MessageData::new(99, 8192);
        original.add_mailbox(1, 3);
        original.add_mailbox(4, 7);
        original.add_keyword(Keyword::Seen);
        original.add_keyword(Keyword::Custom("invoices".to_string()));
        original.received_at = 1_700_000_000;
        original.sent_at = 1_580_515_200;

        let bytes = irixmail_store::serialize::archive(&original).expect("archive");
        let restored: MessageData =
            irixmail_store::serialize::deserialize(&bytes).expect("deserialize");
        assert_eq!(original, restored);
        assert_eq!(restored.received_at, 1_700_000_000);
        assert_eq!(restored.sent_at, 1_580_515_200);
    }

    #[test]
    fn archived_view_reads_membership_and_flags_in_place() {
        let mut original = MessageData::new(5, 256);
        original.add_mailbox(2, 11);
        original.add_keyword(Keyword::Flagged);

        let bytes = irixmail_store::serialize::archive(&original).expect("archive");
        let view = irixmail_store::serialize::access::<MessageData>(&bytes).expect("access");
        assert_eq!(view.thread_id.to_native(), 5);
        assert_eq!(view.size.to_native(), 256);
        assert_eq!(view.mailboxes.len(), 1);
        assert_eq!(view.mailboxes[0].mailbox_id.to_native(), 2);
        assert_eq!(view.mailboxes[0].uid.to_native(), 11);
        assert_eq!(view.keywords.len(), 1);
    }
}
