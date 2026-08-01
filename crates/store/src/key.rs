use std::fmt;

const ACCOUNT_ID_LEN: usize = std::mem::size_of::<u32>();
const DOCUMENT_ID_LEN: usize = std::mem::size_of::<u32>();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Subspace {
    Property = b'p',
    Index = b'i',
    ChangeLog = b'c',
    BlobRef = b'b',
    Queue = b'q',
    Registry = b'r',
    Counter = b'n',
}

impl Subspace {
    pub fn as_byte(self) -> u8 {
        self as u8
    }

    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            b'p' => Some(Subspace::Property),
            b'i' => Some(Subspace::Index),
            b'c' => Some(Subspace::ChangeLog),
            b'b' => Some(Subspace::BlobRef),
            b'q' => Some(Subspace::Queue),
            b'r' => Some(Subspace::Registry),
            b'n' => Some(Subspace::Counter),
            _ => None,
        }
    }
}

impl fmt::Display for Subspace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Subspace::Property => "property",
            Subspace::Index => "index",
            Subspace::ChangeLog => "changelog",
            Subspace::BlobRef => "blobref",
            Subspace::Queue => "queue",
            Subspace::Registry => "registry",
            Subspace::Counter => "counter",
        };
        f.write_str(name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Collection {
    Mailbox = 0,
    Email = 1,
    Thread = 2,
    Identity = 3,
    EmailSubmission = 4,
    SieveScript = 5,
    EmailVanished = 6,
    PushSubscription = 7,
    Calendar = 8,
    CalendarEvent = 9,
    AddressBook = 10,
    ContactCard = 11,
}

impl Collection {
    pub fn as_byte(self) -> u8 {
        self as u8
    }

    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Collection::Mailbox),
            1 => Some(Collection::Email),
            2 => Some(Collection::Thread),
            3 => Some(Collection::Identity),
            4 => Some(Collection::EmailSubmission),
            5 => Some(Collection::SieveScript),
            6 => Some(Collection::EmailVanished),
            7 => Some(Collection::PushSubscription),
            8 => Some(Collection::Calendar),
            9 => Some(Collection::CalendarEvent),
            10 => Some(Collection::AddressBook),
            11 => Some(Collection::ContactCard),
            _ => None,
        }
    }
}

impl fmt::Display for Collection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Collection::Mailbox => "mailbox",
            Collection::Email => "email",
            Collection::Thread => "thread",
            Collection::Identity => "identity",
            Collection::EmailSubmission => "email-submission",
            Collection::SieveScript => "sieve-script",
            Collection::EmailVanished => "email-vanished",
            Collection::PushSubscription => "push-subscription",
            Collection::Calendar => "calendar",
            Collection::CalendarEvent => "calendar-event",
            Collection::AddressBook => "address-book",
            Collection::ContactCard => "contact-card",
        };
        f.write_str(name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    subspace: Subspace,
    account_id: u32,
    collection: Collection,
    document_id: u32,
    suffix: Vec<u8>,
}

impl Key {
    pub fn new(
        subspace: Subspace,
        account_id: u32,
        collection: Collection,
        document_id: u32,
    ) -> Self {
        Self {
            subspace,
            account_id,
            collection,
            document_id,
            suffix: Vec::new(),
        }
    }

    pub fn with_suffix(mut self, suffix: impl Into<Vec<u8>>) -> Self {
        self.suffix = suffix.into();
        self
    }

    pub fn subspace(&self) -> Subspace {
        self.subspace
    }

    pub fn account_id(&self) -> u32 {
        self.account_id
    }

    pub fn collection(&self) -> Collection {
        self.collection
    }

    pub fn document_id(&self) -> u32 {
        self.document_id
    }

    pub fn suffix(&self) -> &[u8] {
        &self.suffix
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf =
            Vec::with_capacity(1 + ACCOUNT_ID_LEN + 1 + DOCUMENT_ID_LEN + self.suffix.len());
        buf.push(self.subspace.as_byte());
        buf.extend_from_slice(&self.account_id.to_be_bytes());
        buf.push(self.collection.as_byte());
        buf.extend_from_slice(&self.document_id.to_be_bytes());
        buf.extend_from_slice(&self.suffix);
        buf
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPrefix {
    subspace: Subspace,
    account_id: Option<u32>,
    collection: Option<Collection>,
}

impl KeyPrefix {
    pub fn subspace(subspace: Subspace) -> Self {
        Self {
            subspace,
            account_id: None,
            collection: None,
        }
    }

    pub fn account(subspace: Subspace, account_id: u32) -> Self {
        Self {
            subspace,
            account_id: Some(account_id),
            collection: None,
        }
    }

    pub fn collection(subspace: Subspace, account_id: u32, collection: Collection) -> Self {
        Self {
            subspace,
            account_id: Some(account_id),
            collection: Some(collection),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + ACCOUNT_ID_LEN + 1);
        buf.push(self.subspace.as_byte());
        if let Some(account_id) = self.account_id {
            buf.extend_from_slice(&account_id.to_be_bytes());
            if let Some(collection) = self.collection {
                buf.push(collection.as_byte());
            }
        }
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subspace_byte_without_a_column_family_is_not_decodable() {
        assert_eq!(Subspace::from_byte(b'm'), None);
    }

    #[test]
    fn subspace_bytes_round_trip() {
        for subspace in [
            Subspace::Property,
            Subspace::Index,
            Subspace::ChangeLog,
            Subspace::BlobRef,
            Subspace::Queue,
            Subspace::Registry,
            Subspace::Counter,
        ] {
            assert_eq!(Subspace::from_byte(subspace.as_byte()), Some(subspace));
        }
        assert_eq!(Subspace::from_byte(b'?'), None);
    }

    #[test]
    fn subspace_bytes_are_distinct() {
        use std::collections::HashSet;
        let bytes: HashSet<u8> = [
            Subspace::Property,
            Subspace::Index,
            Subspace::ChangeLog,
            Subspace::BlobRef,
            Subspace::Queue,
            Subspace::Registry,
            Subspace::Counter,
        ]
        .iter()
        .map(|s| s.as_byte())
        .collect();
        assert_eq!(bytes.len(), 7);
    }

    #[test]
    fn collection_bytes_round_trip() {
        for collection in [
            Collection::Mailbox,
            Collection::Email,
            Collection::Thread,
            Collection::Identity,
            Collection::EmailSubmission,
            Collection::SieveScript,
            Collection::Calendar,
            Collection::CalendarEvent,
            Collection::AddressBook,
            Collection::ContactCard,
        ] {
            assert_eq!(
                Collection::from_byte(collection.as_byte()),
                Some(collection)
            );
        }
        assert_eq!(Collection::from_byte(99), None);
    }

    #[test]
    fn dav_collections_extend_the_byte_space_without_collisions() {
        assert_eq!(Collection::Calendar.as_byte(), 8);
        assert_eq!(Collection::CalendarEvent.as_byte(), 9);
        assert_eq!(Collection::AddressBook.as_byte(), 10);
        assert_eq!(Collection::ContactCard.as_byte(), 11);
        assert_eq!(Collection::Calendar.to_string(), "calendar");
        assert_eq!(Collection::CalendarEvent.to_string(), "calendar-event");
        assert_eq!(Collection::AddressBook.to_string(), "address-book");
        assert_eq!(Collection::ContactCard.to_string(), "contact-card");
    }

    #[test]
    fn encoded_key_has_the_documented_layout() {
        let key = Key::new(
            Subspace::Property,
            0x0102_0304,
            Collection::Email,
            0x0A0B_0C0D,
        );
        let bytes = key.encode();

        assert_eq!(bytes.len(), 1 + 4 + 1 + 4);
        assert_eq!(bytes[0], b'p');
        assert_eq!(&bytes[1..5], &[0x01, 0x02, 0x03, 0x04]);
        assert_eq!(bytes[5], Collection::Email.as_byte());
        assert_eq!(&bytes[6..10], &[0x0A, 0x0B, 0x0C, 0x0D]);
    }

    #[test]
    fn suffix_is_appended_after_the_fixed_components() {
        let key =
            Key::new(Subspace::Property, 7, Collection::Mailbox, 3).with_suffix(vec![0xFE, 0xFF]);
        let bytes = key.encode();

        assert_eq!(bytes.len(), 1 + 4 + 1 + 4 + 2);
        assert_eq!(&bytes[10..], &[0xFE, 0xFF]);
        assert_eq!(key.suffix(), &[0xFE, 0xFF]);
    }

    #[test]
    fn big_endian_ids_sort_in_numeric_order() {
        let lo = Key::new(Subspace::Property, 1, Collection::Email, 1).encode();
        let hi = Key::new(Subspace::Property, 1, Collection::Email, 256).encode();
        assert!(lo < hi);

        let small_account = Key::new(Subspace::Property, 1, Collection::Email, 999).encode();
        let large_account = Key::new(Subspace::Property, 2, Collection::Email, 0).encode();
        assert!(small_account < large_account);
    }

    #[test]
    fn subspace_orders_ahead_of_everything_else() {
        let property = Key::new(Subspace::Index, u32::MAX, Collection::Email, u32::MAX).encode();
        let registry = Key::new(Subspace::Registry, 0, Collection::Email, 0).encode();
        assert!(property[0] < registry[0]);
        assert!(property < registry);
    }

    #[test]
    fn prefixes_bound_their_matching_keys() {
        let prefix = KeyPrefix::subspace(Subspace::Property).encode();
        let key = Key::new(Subspace::Property, 5, Collection::Email, 9).encode();
        assert!(key.starts_with(&prefix));

        let prefix = KeyPrefix::account(Subspace::Property, 5).encode();
        assert!(key.starts_with(&prefix));
        let other_account = Key::new(Subspace::Property, 6, Collection::Email, 9).encode();
        assert!(!other_account.starts_with(&prefix));

        let prefix = KeyPrefix::collection(Subspace::Property, 5, Collection::Email).encode();
        assert!(key.starts_with(&prefix));
        let other_collection = Key::new(Subspace::Property, 5, Collection::Mailbox, 9).encode();
        assert!(!other_collection.starts_with(&prefix));
    }

    #[test]
    fn account_prefix_matches_every_collection_in_order() {
        let prefix = KeyPrefix::account(Subspace::Property, 10).encode();
        for collection in [
            Collection::Mailbox,
            Collection::Email,
            Collection::SieveScript,
        ] {
            let key = Key::new(Subspace::Property, 10, collection, 1).encode();
            assert!(key.starts_with(&prefix));
            assert!(key.as_slice() >= prefix.as_slice());
        }
    }

    #[test]
    fn subspace_and_collection_render_human_labels() {
        assert_eq!(Subspace::Counter.to_string(), "counter");
        assert_eq!(Collection::EmailSubmission.to_string(), "email-submission");
    }
}
