use irixmail_core::{Error, Result};
use irixmail_store::BlobHash;
use mail_parser::{Encoding, Header, HeaderName as ParsedHeaderName, MessageParser, PartType};

use crate::metadata::{
    ByteRange, HeaderName, MessageMetadata, MessagePart, PartBody, PartEncoding, PartHeader,
};

const PREVIEW_LEN: usize = 256;

pub fn ingest(blob_hash: BlobHash, raw: &[u8]) -> Result<MessageMetadata> {
    let parsed = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| Error::invalid_input("message has no parseable headers"))?;

    let preview = parsed
        .body_preview(PREVIEW_LEN)
        .map(|text| text.into_owned())
        .unwrap_or_default();

    let root = parsed.root_part();
    let header_block = raw
        .get(root.offset_header as usize..root.offset_body as usize)
        .unwrap_or_default()
        .to_vec();

    let parts = flatten(&parsed);

    let mut metadata = MessageMetadata::new(blob_hash, parts);
    metadata.raw_headers = header_block;
    metadata.preview = preview;
    Ok(metadata)
}

fn flatten(message: &mail_parser::Message<'_>) -> Vec<MessagePart> {
    let mut flat: Vec<MessagePart> = Vec::with_capacity(message.parts.len());
    let mut queue: std::collections::VecDeque<(&mail_parser::Message<'_>, u32)> =
        std::collections::VecDeque::new();
    queue.push_back((message, 0));

    let mut next_base = message.parts.len() as u32;

    while let Some((current, base)) = queue.pop_front() {
        for part in &current.parts {
            let kind = match &part.body {
                PartType::Text(_) => PartBody::Text,
                PartType::Html(_) => PartBody::Html,
                PartType::Binary(_) => PartBody::Binary,
                PartType::InlineBinary(_) => PartBody::InlineBinary,
                PartType::Message(sub) => {
                    let root_index = next_base;
                    next_base += sub.parts.len() as u32;
                    queue.push_back((sub, root_index));
                    PartBody::Message(root_index)
                }
                PartType::Multipart(children) => {
                    PartBody::Multipart(children.iter().map(|&c| c + base).collect())
                }
            };

            flat.push(MessagePart {
                headers: part.headers.iter().map(convert_header).collect(),
                header: ByteRange::new(part.offset_header, part.offset_body),
                body: ByteRange::new(part.offset_body, part.offset_end),
                encoding: convert_encoding(part.encoding),
                kind,
            });
        }
    }

    flat
}

fn convert_header(header: &Header<'_>) -> PartHeader {
    PartHeader::new(
        convert_header_name(&header.name),
        ByteRange::new(header.offset_start, header.offset_end),
    )
}

fn convert_encoding(encoding: Encoding) -> PartEncoding {
    match encoding {
        Encoding::None => PartEncoding::None,
        Encoding::Base64 => PartEncoding::Base64,
        Encoding::QuotedPrintable => PartEncoding::QuotedPrintable,
    }
}

fn convert_header_name(name: &ParsedHeaderName<'_>) -> HeaderName {
    match name {
        ParsedHeaderName::Subject => HeaderName::Subject,
        ParsedHeaderName::From => HeaderName::From,
        ParsedHeaderName::To => HeaderName::To,
        ParsedHeaderName::Cc => HeaderName::Cc,
        ParsedHeaderName::Bcc => HeaderName::Bcc,
        ParsedHeaderName::ReplyTo => HeaderName::ReplyTo,
        ParsedHeaderName::Sender => HeaderName::Sender,
        ParsedHeaderName::Date => HeaderName::Date,
        ParsedHeaderName::MessageId => HeaderName::MessageId,
        ParsedHeaderName::InReplyTo => HeaderName::InReplyTo,
        ParsedHeaderName::References => HeaderName::References,
        ParsedHeaderName::ReturnPath => HeaderName::ReturnPath,
        ParsedHeaderName::MimeVersion => HeaderName::MimeVersion,
        ParsedHeaderName::ContentType => HeaderName::ContentType,
        ParsedHeaderName::ContentTransferEncoding => HeaderName::ContentTransferEncoding,
        ParsedHeaderName::ContentDisposition => HeaderName::ContentDisposition,
        ParsedHeaderName::ContentId => HeaderName::ContentId,
        ParsedHeaderName::ContentDescription => HeaderName::ContentDescription,
        ParsedHeaderName::ContentLanguage => HeaderName::ContentLanguage,
        ParsedHeaderName::ContentLocation => HeaderName::ContentLocation,
        ParsedHeaderName::ListId => HeaderName::ListId,
        ParsedHeaderName::ListUnsubscribe => HeaderName::ListUnsubscribe,
        other => HeaderName::Other(other.as_str().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash() -> BlobHash {
        BlobHash::from_bytes(vec![0x01, 0x02, 0x03, 0x04])
    }

    fn value(raw: &[u8], range: ByteRange) -> &[u8] {
        &raw[range.as_range()]
    }

    #[test]
    fn rejects_a_message_with_no_headers() {
        let result = ingest(hash(), b"");
        assert!(matches!(result, Err(Error::InvalidInput(_))));
    }

    #[test]
    fn parses_a_plain_message_into_a_single_root_part() {
        let raw = b"Subject: Hello\r\nFrom: alice@example.com\r\n\r\nThis is the body.\r\n";
        let metadata = ingest(hash(), raw).expect("ingest");

        assert_eq!(metadata.blob_hash(), hash());

        assert_eq!(metadata.parts.len(), 1);
        let root = metadata.root().expect("root part");
        assert_eq!(root.kind, PartBody::Text);
        assert!(!root.is_container());

        let subject = root
            .headers
            .iter()
            .find(|h| h.name == HeaderName::Subject)
            .expect("subject header");
        assert_eq!(value(raw, subject.value), b" Hello\r\n");

        let from = root
            .headers
            .iter()
            .find(|h| h.name == HeaderName::From)
            .expect("from header");
        assert_eq!(value(raw, from.value), b" alice@example.com\r\n");

        assert_eq!(value(raw, root.body), b"This is the body.\r\n");
    }

    #[test]
    fn captures_the_top_level_header_block_and_preview() {
        let raw = b"Subject: Greetings\r\nFrom: bob@example.com\r\n\r\nHello there, world.\r\n";
        let metadata = ingest(hash(), raw).expect("ingest");

        assert_eq!(
            metadata.raw_headers,
            b"Subject: Greetings\r\nFrom: bob@example.com\r\n\r\n"
        );
        assert!(metadata.preview.contains("Hello there, world."));
    }

    #[test]
    fn records_the_transfer_encoding_of_a_part() {
        let raw = concat!(
            "Subject: Encoded\r\n",
            "Content-Type: text/plain\r\n",
            "Content-Transfer-Encoding: base64\r\n",
            "\r\n",
            "SGVsbG8gd29ybGQ=\r\n",
        )
        .as_bytes();
        let metadata = ingest(hash(), raw).expect("ingest");

        let root = metadata.root().expect("root part");
        assert_eq!(root.encoding, PartEncoding::Base64);
    }

    #[test]
    fn flattens_a_multipart_into_a_container_with_children() {
        let raw = concat!(
            "Subject: Multipart\r\n",
            "Content-Type: multipart/alternative; boundary=\"sep\"\r\n",
            "\r\n",
            "--sep\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "plain body\r\n",
            "--sep\r\n",
            "Content-Type: text/html\r\n",
            "\r\n",
            "<p>html body</p>\r\n",
            "--sep--\r\n",
        )
        .as_bytes();
        let metadata = ingest(hash(), raw).expect("ingest");

        assert_eq!(metadata.parts.len(), 3);
        let root = metadata.root().expect("root part");
        assert!(root.is_container());
        let children = root.child_indices();
        assert_eq!(children.len(), 2);

        let kinds: Vec<_> = children
            .iter()
            .map(|&i| metadata.part(i).expect("child part").kind.clone())
            .collect();
        assert!(kinds.contains(&PartBody::Text));
        assert!(kinds.contains(&PartBody::Html));
    }

    #[test]
    fn flattens_a_nested_message_onto_the_same_part_list() {
        let raw = concat!(
            "Subject: Outer\r\n",
            "Content-Type: multipart/mixed; boundary=\"out\"\r\n",
            "\r\n",
            "--out\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "see attached\r\n",
            "--out\r\n",
            "Content-Type: message/rfc822\r\n",
            "\r\n",
            "Subject: Inner\r\n",
            "From: inner@example.com\r\n",
            "\r\n",
            "inner body\r\n",
            "--out--\r\n",
        )
        .as_bytes();
        let metadata = ingest(hash(), raw).expect("ingest");

        for part in &metadata.parts {
            for child in part.child_indices() {
                assert!(
                    metadata.part(child).is_some(),
                    "child index {child} is out of range",
                );
            }
        }

        let embedded = metadata
            .parts
            .iter()
            .find(|p| matches!(p.kind, PartBody::Message(_)))
            .expect("embedded message part");
        let inner_root = match embedded.kind {
            PartBody::Message(index) => metadata.part(index).expect("inner root"),
            _ => unreachable!(),
        };
        assert!(inner_root
            .headers
            .iter()
            .any(|h| h.name == HeaderName::Subject));
    }

    #[test]
    fn record_round_trips_through_the_archive() {
        let raw = b"Subject: Roundtrip\r\nFrom: r@example.com\r\n\r\nbody text here\r\n";
        let original = ingest(hash(), raw).expect("ingest");

        let bytes = irixmail_store::serialize::archive(&original).expect("archive");
        let restored: MessageMetadata =
            irixmail_store::serialize::deserialize(&bytes).expect("deserialize");
        assert_eq!(original, restored);
    }

    #[test]
    fn maps_an_unrecognised_header_to_other_verbatim() {
        let raw = b"Subject: Custom\r\nX-Irixmail-Tag: special\r\n\r\nbody\r\n";
        let metadata = ingest(hash(), raw).expect("ingest");
        let root = metadata.root().expect("root part");
        assert!(root.headers.iter().any(|h| matches!(
            &h.name,
            HeaderName::Other(name) if name.eq_ignore_ascii_case("X-Irixmail-Tag")
        )));
    }
}
