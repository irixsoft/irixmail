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
    Copy,
    PartialEq,
    Eq,
)]
#[rkyv(derive(Debug), compare(PartialEq))]
pub struct ByteRange {
    pub start: u32,
    pub end: u32,
}

impl ByteRange {
    pub fn new(start: u32, end: u32) -> Self {
        ByteRange { start, end }
    }

    pub fn len(&self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    pub fn as_range(&self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
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
    Hash,
)]
#[rkyv(derive(Debug), compare(PartialEq))]
pub enum PartEncoding {
    None,
    Base64,
    QuotedPrintable,
    Unknown,
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
pub enum PartBody {
    Text,
    Html,
    Binary,
    InlineBinary,
    Message(u32),
    Multipart(Vec<u32>),
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
pub struct PartHeader {
    pub name: HeaderName,
    pub value: ByteRange,
}

impl PartHeader {
    pub fn new(name: HeaderName, value: ByteRange) -> Self {
        PartHeader { name, value }
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
pub enum HeaderName {
    Subject,
    From,
    To,
    Cc,
    Bcc,
    ReplyTo,
    Sender,
    Date,
    MessageId,
    InReplyTo,
    References,
    ReturnPath,
    MimeVersion,
    ContentType,
    ContentTransferEncoding,
    ContentDisposition,
    ContentId,
    ContentDescription,
    ContentLanguage,
    ContentLocation,
    ListId,
    ListUnsubscribe,
    Other(String),
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
pub struct MessagePart {
    pub headers: Vec<PartHeader>,
    pub header: ByteRange,
    pub body: ByteRange,
    pub encoding: PartEncoding,
    pub kind: PartBody,
}

impl MessagePart {
    pub fn is_container(&self) -> bool {
        matches!(self.kind, PartBody::Multipart(_) | PartBody::Message(_))
    }

    pub fn child_indices(&self) -> Vec<u32> {
        match &self.kind {
            PartBody::Multipart(children) => children.clone(),
            PartBody::Message(root) => vec![*root],
            _ => Vec::new(),
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
#[rkyv(derive(Debug))]
pub struct MessageMetadata {
    pub blob_hash: Vec<u8>,
    pub parts: Vec<MessagePart>,
    pub raw_headers: Vec<u8>,
    pub preview: String,
}

impl MessageMetadata {
    pub fn new(blob_hash: BlobHash, parts: Vec<MessagePart>) -> Self {
        MessageMetadata {
            blob_hash: blob_hash.into_bytes(),
            parts,
            raw_headers: Vec::new(),
            preview: String::new(),
        }
    }

    pub fn blob_hash(&self) -> BlobHash {
        BlobHash::from_bytes(self.blob_hash.clone())
    }

    pub fn root(&self) -> Option<&MessagePart> {
        self.parts.first()
    }

    pub fn part(&self, index: u32) -> Option<&MessagePart> {
        self.parts.get(index as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(kind: PartBody, header: ByteRange, body: ByteRange) -> MessagePart {
        MessagePart {
            headers: Vec::new(),
            header,
            body,
            encoding: PartEncoding::None,
            kind,
        }
    }

    #[test]
    fn byte_range_reports_length_and_emptiness() {
        let range = ByteRange::new(10, 30);
        assert_eq!(range.len(), 20);
        assert!(!range.is_empty());
        assert_eq!(range.as_range(), 10..30);

        let empty = ByteRange::new(40, 40);
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);

        let inverted = ByteRange::new(50, 20);
        assert!(inverted.is_empty());
        assert_eq!(inverted.len(), 0);
    }

    #[test]
    fn leaf_parts_are_not_containers_and_have_no_children() {
        let part = leaf(
            PartBody::Text,
            ByteRange::new(0, 20),
            ByteRange::new(20, 80),
        );
        assert!(!part.is_container());
        assert!(part.child_indices().is_empty());
    }

    #[test]
    fn container_parts_report_their_children() {
        let multipart = leaf(
            PartBody::Multipart(vec![1, 2]),
            ByteRange::new(0, 30),
            ByteRange::new(0, 0),
        );
        assert!(multipart.is_container());
        assert_eq!(multipart.child_indices(), vec![1, 2]);

        let embedded = leaf(
            PartBody::Message(3),
            ByteRange::new(0, 40),
            ByteRange::new(40, 200),
        );
        assert!(embedded.is_container());
        assert_eq!(embedded.child_indices(), vec![3]);
    }

    #[test]
    fn root_and_part_resolve_indices_into_the_flat_tree() {
        let hash = BlobHash::from_bytes(vec![0xaa, 0xbb, 0xcc]);
        let parts = vec![
            leaf(
                PartBody::Multipart(vec![1, 2]),
                ByteRange::new(0, 30),
                ByteRange::new(0, 0),
            ),
            leaf(
                PartBody::Text,
                ByteRange::new(30, 60),
                ByteRange::new(60, 120),
            ),
            leaf(
                PartBody::Html,
                ByteRange::new(120, 150),
                ByteRange::new(150, 300),
            ),
        ];
        let metadata = MessageMetadata::new(hash, parts);

        let root = metadata.root().expect("root part");
        assert!(root.is_container());
        assert_eq!(metadata.part(1).map(|p| &p.kind), Some(&PartBody::Text));
        assert_eq!(metadata.part(2).map(|p| &p.kind), Some(&PartBody::Html));
        assert!(metadata.part(3).is_none());
    }

    #[test]
    fn blob_hash_round_trips_through_the_record() {
        let hash = BlobHash::from_bytes(vec![1, 2, 3, 4, 5]);
        let metadata = MessageMetadata::new(hash.clone(), Vec::new());
        assert_eq!(metadata.blob_hash(), hash);
        assert!(metadata.root().is_none());
    }

    #[test]
    fn record_round_trips_through_the_archive() {
        let hash = BlobHash::from_bytes(vec![0x11, 0x22, 0x33, 0x44]);
        let mut root = leaf(
            PartBody::Multipart(vec![1]),
            ByteRange::new(0, 50),
            ByteRange::new(0, 0),
        );
        root.headers
            .push(PartHeader::new(HeaderName::Subject, ByteRange::new(9, 30)));
        root.headers.push(PartHeader::new(
            HeaderName::Other("X-Custom".to_string()),
            ByteRange::new(40, 48),
        ));

        let mut body = leaf(
            PartBody::Text,
            ByteRange::new(50, 80),
            ByteRange::new(80, 200),
        );
        body.encoding = PartEncoding::QuotedPrintable;

        let mut original = MessageMetadata::new(hash, vec![root, body]);
        original.raw_headers = b"Subject: Hi\r\nFrom: a@example.com\r\n".to_vec();
        original.preview = "A short preview of the body text".to_string();

        let bytes = irixmail_store::serialize::archive(&original).expect("archive");
        let restored: MessageMetadata =
            irixmail_store::serialize::deserialize(&bytes).expect("deserialize");
        assert_eq!(original, restored);
    }

    #[test]
    fn archived_view_reads_offsets_and_structure_in_place() {
        let hash = BlobHash::from_bytes(vec![0xde, 0xad, 0xbe, 0xef]);
        let mut text = leaf(
            PartBody::Text,
            ByteRange::new(0, 40),
            ByteRange::new(40, 160),
        );
        text.encoding = PartEncoding::Base64;
        text.headers.push(PartHeader::new(
            HeaderName::ContentType,
            ByteRange::new(14, 38),
        ));

        let mut original = MessageMetadata::new(hash, vec![text]);
        original.preview = "preview".to_string();

        let bytes = irixmail_store::serialize::archive(&original).expect("archive");
        let view = irixmail_store::serialize::access::<MessageMetadata>(&bytes).expect("access");

        assert_eq!(view.parts.len(), 1);
        let part = &view.parts[0];
        assert_eq!(part.body.start.to_native(), 40);
        assert_eq!(part.body.end.to_native(), 160);
        assert_eq!(part.headers.len(), 1);
        assert_eq!(part.headers[0].value.start.to_native(), 14);
        assert_eq!(view.preview.as_ref(), "preview");
        assert_eq!(view.blob_hash.len(), 4);
    }

    #[test]
    fn empty_part_tree_survives_the_round_trip() {
        let original = MessageMetadata::new(BlobHash::from_bytes(Vec::new()), Vec::new());
        let bytes = irixmail_store::serialize::archive(&original).expect("archive");
        let restored: MessageMetadata =
            irixmail_store::serialize::deserialize(&bytes).expect("deserialize");
        assert_eq!(original, restored);
        assert!(restored.parts.is_empty());
        assert!(restored.raw_headers.is_empty());
        assert!(restored.preview.is_empty());
    }
}
