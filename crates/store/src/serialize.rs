use irixmail_core::{Error, Result};
use rkyv::api::high::{HighDeserializer, HighSerializer, HighValidator};
use rkyv::bytecheck::CheckBytes;
use rkyv::rancor;
use rkyv::ser::allocator::ArenaHandle;
use rkyv::util::AlignedVec;
use rkyv::{Archive, Deserialize, Portable, Serialize};

type ArchiveError = rancor::Error;

type ArchiveSerializer<'a> = HighSerializer<AlignedVec, ArenaHandle<'a>, ArchiveError>;

pub fn archive<T>(value: &T) -> Result<Vec<u8>>
where
    T: for<'a> Serialize<ArchiveSerializer<'a>>,
{
    rkyv::to_bytes::<ArchiveError>(value)
        .map(|bytes| bytes.to_vec())
        .map_err(|err| Error::serialize(format!("could not archive value: {err}")))
}

pub fn access<T>(bytes: &[u8]) -> Result<&T::Archived>
where
    T: Archive,
    T::Archived: Portable + for<'a> CheckBytes<HighValidator<'a, ArchiveError>>,
{
    rkyv::access::<T::Archived, ArchiveError>(bytes)
        .map_err(|err| Error::serialize(format!("could not access archived value: {err}")))
}

// Caller guarantees bytes are a valid archive of T (skips rkyv validation).
#[allow(clippy::missing_safety_doc)]
pub unsafe fn access_trusted<T>(bytes: &[u8]) -> &T::Archived
where
    T: Archive,
    T::Archived: Portable,
{
    rkyv::access_unchecked::<T::Archived>(bytes)
}

pub fn deserialize<T>(bytes: &[u8]) -> Result<T>
where
    T: Archive,
    T::Archived: Portable
        + for<'a> CheckBytes<HighValidator<'a, ArchiveError>>
        + Deserialize<T, HighDeserializer<ArchiveError>>,
{
    let archived = access::<T>(bytes)?;
    rkyv::deserialize::<T, ArchiveError>(archived)
        .map_err(|err| Error::serialize(format!("could not deserialize archived value: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq)]
    struct MessageMeta {
        subject: String,
        from: String,
        size: u32,
        flags: Vec<String>,
        parts: Vec<PartMeta>,
    }

    #[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq)]
    struct PartMeta {
        content_type: String,
        offset: u32,
        length: u32,
    }

    fn sample() -> MessageMeta {
        MessageMeta {
            subject: "Quarterly figures".to_string(),
            from: "alice@example.com".to_string(),
            size: 4096,
            flags: vec!["\\Seen".to_string(), "\\Flagged".to_string()],
            parts: vec![
                PartMeta {
                    content_type: "text/plain".to_string(),
                    offset: 0,
                    length: 512,
                },
                PartMeta {
                    content_type: "application/pdf".to_string(),
                    offset: 512,
                    length: 3584,
                },
            ],
        }
    }

    #[test]
    fn round_trips_through_deserialize() {
        let original = sample();
        let bytes = archive(&original).expect("archive");
        let restored: MessageMeta = deserialize(&bytes).expect("deserialize");
        assert_eq!(original, restored);
    }

    #[test]
    fn validated_access_reads_fields_in_place() {
        let original = sample();
        let bytes = archive(&original).expect("archive");

        let view = access::<MessageMeta>(&bytes).expect("access");
        assert_eq!(view.subject.as_ref(), "Quarterly figures");
        assert_eq!(view.from.as_ref(), "alice@example.com");
        assert_eq!(view.size.to_native(), 4096);
        assert_eq!(view.flags.len(), 2);
        assert_eq!(view.parts.len(), 2);
        assert_eq!(view.parts[1].content_type.as_ref(), "application/pdf");
        assert_eq!(view.parts[1].offset.to_native(), 512);
    }

    #[test]
    fn trusted_access_matches_validated_access() {
        let original = sample();
        let bytes = archive(&original).expect("archive");

        let trusted = unsafe { access_trusted::<MessageMeta>(&bytes) };
        let checked = access::<MessageMeta>(&bytes).expect("access");
        assert_eq!(trusted.subject.as_ref(), checked.subject.as_ref());
        assert_eq!(trusted.size.to_native(), checked.size.to_native());
    }

    #[test]
    fn empty_collections_survive_the_round_trip() {
        let original = MessageMeta {
            subject: String::new(),
            from: "noreply@example.com".to_string(),
            size: 0,
            flags: Vec::new(),
            parts: Vec::new(),
        };
        let bytes = archive(&original).expect("archive");
        let restored: MessageMeta = deserialize(&bytes).expect("deserialize");
        assert_eq!(original, restored);
        assert!(restored.flags.is_empty());
        assert!(restored.parts.is_empty());
    }

    #[test]
    fn corrupt_bytes_are_rejected_by_validation() {
        let bytes = archive(&sample()).expect("archive");
        let mut damaged = bytes.clone();
        for byte in damaged.iter_mut().take(4) {
            *byte ^= 0xFF;
        }

        match access::<MessageMeta>(&damaged) {
            Ok(_) => panic!("validation accepted corrupt archive bytes"),
            Err(err) => assert!(matches!(err, Error::Serialize(_))),
        }
    }

    #[test]
    fn truncated_bytes_are_rejected() {
        let bytes = archive(&sample()).expect("archive");
        let truncated = &bytes[..bytes.len() / 2];
        assert!(access::<MessageMeta>(truncated).is_err());
        assert!(deserialize::<MessageMeta>(truncated).is_err());
    }

    #[test]
    fn distinct_values_produce_distinct_archives() {
        let one = archive(&sample()).expect("archive");
        let mut other = sample();
        other.subject = "A different subject entirely".to_string();
        let two = archive(&other).expect("archive");
        assert_ne!(one, two);
    }
}
