use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadProperty {
    pub ns: String,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarCollection {
    pub id: u32,
    pub name: String,
    pub display_name: String,
    pub color: Option<String>,
    pub order: u32,
    pub description: Option<String>,
    pub time_zone: Option<String>,
    pub dead_properties: Vec<DeadProperty>,
    pub created: u64,
    pub modified: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarEventRecord {
    pub id: u32,
    pub calendar_id: u32,
    pub name: String,
    pub uid: String,
    pub ics: String,
    pub etag: String,
    pub starts_min: i64,
    pub ends_max: i64,
    pub size: u32,
    pub created: u64,
    pub modified: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressBookCollection {
    pub id: u32,
    pub name: String,
    pub display_name: String,
    pub description: Option<String>,
    pub dead_properties: Vec<DeadProperty>,
    pub created: u64,
    pub modified: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactCardRecord {
    pub id: u32,
    pub book_id: u32,
    pub name: String,
    pub uid: Option<String>,
    pub vcf: String,
    pub etag: String,
    pub full_name: String,
    pub emails: Vec<String>,
    pub size: u32,
    pub created: u64,
    pub modified: u64,
}

pub fn content_etag(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_content_etag_is_stable_and_quoted_free() {
        let first = content_etag(b"BEGIN:VCALENDAR");
        let second = content_etag(b"BEGIN:VCALENDAR");
        let other = content_etag(b"BEGIN:VCARD");
        assert_eq!(first, second);
        assert_ne!(first, other);
        assert!(!first.contains('"'));
        assert!(first.len() >= 32);
    }

    #[test]
    fn calendar_records_round_trip_through_json() {
        let calendar = CalendarCollection {
            id: 1,
            name: "calendar".into(),
            display_name: "Calendar".into(),
            color: Some("#B4842EFF".into()),
            order: 0,
            description: None,
            time_zone: Some("Europe/Stockholm".into()),
            dead_properties: vec![DeadProperty {
                ns: "http://apple.com/ns/ical/".into(),
                name: "calendar-order".into(),
                value: "2".into(),
            }],
            created: 1,
            modified: 2,
        };
        let bytes = serde_json::to_vec(&calendar).unwrap();
        assert_eq!(
            serde_json::from_slice::<CalendarCollection>(&bytes).unwrap(),
            calendar
        );
    }

    #[test]
    fn event_and_card_records_round_trip_through_json() {
        let event = CalendarEventRecord {
            id: 9,
            calendar_id: 1,
            name: "abc.ics".into(),
            uid: "u-1".into(),
            ics: "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n".into(),
            etag: "e".into(),
            starts_min: 100,
            ends_max: 200,
            size: 36,
            created: 1,
            modified: 2,
        };
        let bytes = serde_json::to_vec(&event).unwrap();
        assert_eq!(
            serde_json::from_slice::<CalendarEventRecord>(&bytes).unwrap(),
            event
        );

        let card = ContactCardRecord {
            id: 4,
            book_id: 1,
            name: "abc.vcf".into(),
            uid: None,
            vcf: "BEGIN:VCARD\r\nEND:VCARD\r\n".into(),
            etag: "e".into(),
            full_name: "Saeed".into(),
            emails: vec!["a@b.com".into()],
            size: 24,
            created: 1,
            modified: 2,
        };
        let bytes = serde_json::to_vec(&card).unwrap();
        assert_eq!(
            serde_json::from_slice::<ContactCardRecord>(&bytes).unwrap(),
            card
        );
    }
}
