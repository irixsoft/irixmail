use irixmail_core::{Error, Result};
use irixmail_store::{
    BatchBuilder, ChangeKind, ChangeLog, ChangeLogEntry, ChangeNotifier, Collection, Flow, Key,
    KeyPrefix, Store, Subspace, WriteOp,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::model::{
    content_etag, AddressBookCollection, CalendarCollection, CalendarEventRecord, ContactCardRecord,
};
use crate::parse::{IcsInfo, VcfInfo};

pub const DEFAULT_CALENDAR_NAME: &str = "calendar";
pub const DEFAULT_ADDRESS_BOOK_NAME: &str = "contacts";
const ID_COUNTER: u32 = 1;
const TOMBSTONE_SUFFIX: &[u8] = b"tomb";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    pub change_id: u64,
    pub parent_id: u32,
    pub name: String,
}

pub struct DavStore<'a> {
    store: &'a dyn Store,
    notifier: &'a ChangeNotifier,
    account_id: u32,
}

impl<'a> DavStore<'a> {
    pub fn new(store: &'a dyn Store, notifier: &'a ChangeNotifier, account_id: u32) -> Self {
        Self {
            store,
            notifier,
            account_id,
        }
    }

    pub fn account_id(&self) -> u32 {
        self.account_id
    }

    fn record_key(&self, collection: Collection, id: u32) -> Vec<u8> {
        Key::new(Subspace::Property, self.account_id, collection, id).encode()
    }

    fn tombstone_key(&self, collection: Collection, id: u32) -> Vec<u8> {
        Key::new(Subspace::Index, self.account_id, collection, id)
            .with_suffix(TOMBSTONE_SUFFIX.to_vec())
            .encode()
    }

    fn allocate_id(&self, collection: Collection) -> Result<u32> {
        let key = Key::new(Subspace::Counter, self.account_id, collection, ID_COUNTER).encode();
        Ok(self.store.add_and_get(&key, 1)? as u32)
    }

    fn load_all<T: DeserializeOwned>(&self, collection: Collection) -> Result<Vec<T>> {
        let prefix = KeyPrefix::collection(Subspace::Property, self.account_id, collection);
        let mut records = Vec::new();
        let mut scan_error = None;
        self.store.iterate(
            &prefix,
            &mut |_key, value| match serde_json::from_slice::<T>(value) {
                Ok(record) => {
                    records.push(record);
                    Ok(Flow::Continue)
                }
                Err(err) => {
                    scan_error = Some(Error::serialize(format!(
                        "could not decode {collection} row: {err}"
                    )));
                    Ok(Flow::Stop)
                }
            },
        )?;
        if let Some(err) = scan_error {
            return Err(err);
        }
        Ok(records)
    }

    fn write_record<T: Serialize>(
        &self,
        collection: Collection,
        id: u32,
        record: &T,
        kind: ChangeKind,
    ) -> Result<()> {
        let value = serde_json::to_vec(record)
            .map_err(|err| Error::serialize(format!("could not encode {collection} row: {err}")))?;
        let mut batch = BatchBuilder::new();
        batch.push(WriteOp::Set {
            key: self.record_key(collection, id),
            value,
        });
        let (change_id, change_op) =
            ChangeLog::new(self.store).record_op(self.account_id, collection, id, kind)?;
        batch.push(change_op);
        self.store.batch(&batch.build())?;
        self.notifier
            .notify_change(self.account_id, collection, change_id);
        Ok(())
    }

    fn delete_record(
        &self,
        collection: Collection,
        id: u32,
        parent_id: u32,
        name: &str,
    ) -> Result<()> {
        let (change_id, change_op) = ChangeLog::new(self.store).record_op(
            self.account_id,
            collection,
            id,
            ChangeKind::Delete,
        )?;
        let tombstone = Tombstone {
            change_id,
            parent_id,
            name: name.to_string(),
        };
        let value = serde_json::to_vec(&tombstone)
            .map_err(|err| Error::serialize(format!("could not encode tombstone: {err}")))?;
        let mut batch = BatchBuilder::new();
        batch.push(WriteOp::Delete {
            key: self.record_key(collection, id),
        });
        batch.push(WriteOp::Set {
            key: self.tombstone_key(collection, id),
            value,
        });
        batch.push(change_op);
        self.store.batch(&batch.build())?;
        self.notifier
            .notify_change(self.account_id, collection, change_id);
        Ok(())
    }

    pub fn ensure_defaults(&self, now_millis: u64) -> Result<()> {
        if self.list_calendars()?.is_empty() {
            self.create_calendar(DEFAULT_CALENDAR_NAME, "Calendar", now_millis)?;
        }
        if self.list_address_books()?.is_empty() {
            self.create_address_book(DEFAULT_ADDRESS_BOOK_NAME, "Contacts", now_millis)?;
        }
        Ok(())
    }

    pub fn list_calendars(&self) -> Result<Vec<CalendarCollection>> {
        let mut calendars: Vec<CalendarCollection> = self.load_all(Collection::Calendar)?;
        calendars.sort_by_key(|calendar| calendar.id);
        Ok(calendars)
    }

    pub fn calendar_by_id(&self, id: u32) -> Result<Option<CalendarCollection>> {
        Ok(self
            .list_calendars()?
            .into_iter()
            .find(|calendar| calendar.id == id))
    }

    pub fn calendar_by_name(&self, name: &str) -> Result<Option<CalendarCollection>> {
        Ok(self
            .list_calendars()?
            .into_iter()
            .find(|calendar| calendar.name == name))
    }

    pub fn create_calendar(
        &self,
        name: &str,
        display_name: &str,
        now_millis: u64,
    ) -> Result<CalendarCollection> {
        let id = self.allocate_id(Collection::Calendar)?;
        let calendar = CalendarCollection {
            id,
            name: name.to_string(),
            display_name: display_name.to_string(),
            color: None,
            order: 0,
            description: None,
            time_zone: None,
            dead_properties: Vec::new(),
            created: now_millis,
            modified: now_millis,
        };
        self.write_record(Collection::Calendar, id, &calendar, ChangeKind::Insert)?;
        Ok(calendar)
    }

    pub fn save_calendar(&self, calendar: &CalendarCollection, now_millis: u64) -> Result<()> {
        let mut updated = calendar.clone();
        updated.modified = now_millis;
        self.write_record(
            Collection::Calendar,
            calendar.id,
            &updated,
            ChangeKind::Update,
        )
    }

    pub fn delete_calendar(&self, id: u32) -> Result<bool> {
        let Some(calendar) = self.calendar_by_id(id)? else {
            return Ok(false);
        };
        for event in self.list_events(Some(id))? {
            self.delete_record(Collection::CalendarEvent, event.id, id, &event.name)?;
        }
        self.delete_record(Collection::Calendar, calendar.id, 0, &calendar.name)?;
        Ok(true)
    }

    pub fn list_events(&self, calendar_id: Option<u32>) -> Result<Vec<CalendarEventRecord>> {
        let mut events: Vec<CalendarEventRecord> = self.load_all(Collection::CalendarEvent)?;
        if let Some(calendar_id) = calendar_id {
            events.retain(|event| event.calendar_id == calendar_id);
        }
        events.sort_by_key(|event| event.id);
        Ok(events)
    }

    pub fn event_by_name(
        &self,
        calendar_id: u32,
        name: &str,
    ) -> Result<Option<CalendarEventRecord>> {
        Ok(self
            .list_events(Some(calendar_id))?
            .into_iter()
            .find(|event| event.name == name))
    }

    pub fn upsert_event(
        &self,
        calendar_id: u32,
        name: &str,
        ics: &str,
        info: &IcsInfo,
        now_millis: u64,
    ) -> Result<(CalendarEventRecord, bool)> {
        let existing = self.event_by_name(calendar_id, name)?;
        let created = existing.is_none();
        let (id, created_at, kind) = match &existing {
            Some(event) => (event.id, event.created, ChangeKind::Update),
            None => (
                self.allocate_id(Collection::CalendarEvent)?,
                now_millis,
                ChangeKind::Insert,
            ),
        };
        let record = CalendarEventRecord {
            id,
            calendar_id,
            name: name.to_string(),
            uid: info.uid.clone(),
            ics: ics.to_string(),
            etag: content_etag(ics.as_bytes()),
            starts_min: info.starts_min,
            ends_max: info.ends_max,
            size: ics.len() as u32,
            created: created_at,
            modified: now_millis,
        };
        self.write_record(Collection::CalendarEvent, id, &record, kind)?;
        Ok((record, created))
    }

    pub fn delete_event(&self, calendar_id: u32, name: &str) -> Result<bool> {
        match self.event_by_name(calendar_id, name)? {
            Some(event) => {
                self.delete_record(Collection::CalendarEvent, event.id, calendar_id, name)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn list_address_books(&self) -> Result<Vec<AddressBookCollection>> {
        let mut books: Vec<AddressBookCollection> = self.load_all(Collection::AddressBook)?;
        books.sort_by_key(|book| book.id);
        Ok(books)
    }

    pub fn address_book_by_id(&self, id: u32) -> Result<Option<AddressBookCollection>> {
        Ok(self
            .list_address_books()?
            .into_iter()
            .find(|book| book.id == id))
    }

    pub fn address_book_by_name(&self, name: &str) -> Result<Option<AddressBookCollection>> {
        Ok(self
            .list_address_books()?
            .into_iter()
            .find(|book| book.name == name))
    }

    pub fn create_address_book(
        &self,
        name: &str,
        display_name: &str,
        now_millis: u64,
    ) -> Result<AddressBookCollection> {
        let id = self.allocate_id(Collection::AddressBook)?;
        let book = AddressBookCollection {
            id,
            name: name.to_string(),
            display_name: display_name.to_string(),
            description: None,
            dead_properties: Vec::new(),
            created: now_millis,
            modified: now_millis,
        };
        self.write_record(Collection::AddressBook, id, &book, ChangeKind::Insert)?;
        Ok(book)
    }

    pub fn save_address_book(&self, book: &AddressBookCollection, now_millis: u64) -> Result<()> {
        let mut updated = book.clone();
        updated.modified = now_millis;
        self.write_record(
            Collection::AddressBook,
            book.id,
            &updated,
            ChangeKind::Update,
        )
    }

    pub fn delete_address_book(&self, id: u32) -> Result<bool> {
        let Some(book) = self.address_book_by_id(id)? else {
            return Ok(false);
        };
        for card in self.list_cards(Some(id))? {
            self.delete_record(Collection::ContactCard, card.id, id, &card.name)?;
        }
        self.delete_record(Collection::AddressBook, book.id, 0, &book.name)?;
        Ok(true)
    }

    pub fn list_cards(&self, book_id: Option<u32>) -> Result<Vec<ContactCardRecord>> {
        let mut cards: Vec<ContactCardRecord> = self.load_all(Collection::ContactCard)?;
        if let Some(book_id) = book_id {
            cards.retain(|card| card.book_id == book_id);
        }
        cards.sort_by_key(|card| card.id);
        Ok(cards)
    }

    pub fn card_by_name(&self, book_id: u32, name: &str) -> Result<Option<ContactCardRecord>> {
        Ok(self
            .list_cards(Some(book_id))?
            .into_iter()
            .find(|card| card.name == name))
    }

    pub fn upsert_card(
        &self,
        book_id: u32,
        name: &str,
        vcf: &str,
        info: &VcfInfo,
        now_millis: u64,
    ) -> Result<(ContactCardRecord, bool)> {
        let existing = self.card_by_name(book_id, name)?;
        let created = existing.is_none();
        let (id, created_at, kind) = match &existing {
            Some(card) => (card.id, card.created, ChangeKind::Update),
            None => (
                self.allocate_id(Collection::ContactCard)?,
                now_millis,
                ChangeKind::Insert,
            ),
        };
        let record = ContactCardRecord {
            id,
            book_id,
            name: name.to_string(),
            uid: info.uid.clone(),
            vcf: vcf.to_string(),
            etag: content_etag(vcf.as_bytes()),
            full_name: info.full_name.clone(),
            emails: info.emails.clone(),
            size: vcf.len() as u32,
            created: created_at,
            modified: now_millis,
        };
        self.write_record(Collection::ContactCard, id, &record, kind)?;
        Ok((record, created))
    }

    pub fn delete_card(&self, book_id: u32, name: &str) -> Result<bool> {
        match self.card_by_name(book_id, name)? {
            Some(card) => {
                self.delete_record(Collection::ContactCard, card.id, book_id, name)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn tombstone(&self, collection: Collection, document_id: u32) -> Result<Option<Tombstone>> {
        match self
            .store
            .get(&self.tombstone_key(collection, document_id))?
        {
            Some(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|err| Error::serialize(format!("could not decode tombstone: {err}"))),
            None => Ok(None),
        }
    }

    pub fn state(&self, collection: Collection) -> Result<u64> {
        ChangeLog::new(self.store).latest_change_id(self.account_id, collection)
    }

    pub fn changes_since(&self, collection: Collection, since: u64) -> Result<Vec<ChangeLogEntry>> {
        ChangeLog::new(self.store).changes_since(self.account_id, collection, since)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::parse::{parse_ics, parse_vcf};
    use irixmail_store::{ChangeKind, ChangeLog, Flow, KeyPrefix, WriteOp};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub(crate) struct MemStore {
        map: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
    }

    impl MemStore {
        fn read_counter(map: &BTreeMap<Vec<u8>, Vec<u8>>, key: &[u8]) -> i64 {
            map.get(key)
                .map(|bytes| {
                    let mut array = [0u8; 8];
                    array.copy_from_slice(bytes);
                    i64::from_le_bytes(array)
                })
                .unwrap_or(0)
        }
    }

    impl Store for MemStore {
        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
            Ok(self.map.lock().unwrap().get(key).cloned())
        }

        fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
            self.map
                .lock()
                .unwrap()
                .insert(key.to_vec(), value.to_vec());
            Ok(())
        }

        fn delete(&self, key: &[u8]) -> Result<()> {
            self.map.lock().unwrap().remove(key);
            Ok(())
        }

        fn iterate(
            &self,
            prefix: &KeyPrefix,
            visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
        ) -> Result<()> {
            let bound = prefix.encode();
            let map = self.map.lock().unwrap();
            for (key, value) in map.iter() {
                if !key.starts_with(&bound) {
                    continue;
                }
                if visit(key, value)? == Flow::Stop {
                    break;
                }
            }
            Ok(())
        }

        fn batch(&self, ops: &[WriteOp]) -> Result<()> {
            let mut map = self.map.lock().unwrap();
            for op in ops {
                match op {
                    WriteOp::Set { key, value } => {
                        map.insert(key.clone(), value.clone());
                    }
                    WriteOp::Delete { key } => {
                        map.remove(key);
                    }
                    WriteOp::Add { key, by } => {
                        let next = Self::read_counter(&map, key) + by;
                        map.insert(key.clone(), next.to_le_bytes().to_vec());
                    }
                }
            }
            Ok(())
        }

        fn add_and_get(&self, key: &[u8], by: i64) -> Result<i64> {
            let mut map = self.map.lock().unwrap();
            let next = Self::read_counter(&map, key) + by;
            map.insert(key.to_vec(), next.to_le_bytes().to_vec());
            Ok(next)
        }

        fn counter(&self, key: &[u8]) -> Result<i64> {
            let map = self.map.lock().unwrap();
            Ok(Self::read_counter(&map, key))
        }
    }

    const ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\nBEGIN:VEVENT\r\nUID:one@example.com\r\nDTSTAMP:20260101T000000Z\r\nDTSTART:20260210T100000Z\r\nDTEND:20260210T110000Z\r\nSUMMARY:Meeting\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    const ICS_LATER: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\nBEGIN:VEVENT\r\nUID:one@example.com\r\nDTSTAMP:20260101T000000Z\r\nDTSTART:20260211T100000Z\r\nDTEND:20260211T110000Z\r\nSUMMARY:Moved\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    const VCF: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:card-1\r\nFN:Saeed Sakib\r\nEMAIL:saeed@example.com\r\nEND:VCARD\r\n";

    fn setup() -> (MemStore, ChangeNotifier) {
        (MemStore::default(), ChangeNotifier::new())
    }

    #[test]
    fn defaults_provision_one_calendar_and_one_address_book_idempotently() {
        let (store, notifier) = setup();
        let dav = DavStore::new(&store, &notifier, 7);
        dav.ensure_defaults(1_000).unwrap();
        dav.ensure_defaults(2_000).unwrap();

        let calendars = dav.list_calendars().unwrap();
        assert_eq!(calendars.len(), 1);
        assert_eq!(calendars[0].name, "calendar");
        assert_eq!(calendars[0].display_name, "Calendar");

        let books = dav.list_address_books().unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].name, "contacts");
        assert_eq!(books[0].display_name, "Contacts");
    }

    #[test]
    fn a_created_calendar_is_listed_and_found_by_name() {
        let (store, notifier) = setup();
        let dav = DavStore::new(&store, &notifier, 7);
        dav.ensure_defaults(1_000).unwrap();
        let created = dav.create_calendar("work", "Work", 2_000).unwrap();
        assert!(created.id > 0);

        let found = dav.calendar_by_name("work").unwrap().unwrap();
        assert_eq!(found.display_name, "Work");
        assert_eq!(dav.list_calendars().unwrap().len(), 2);
        assert!(dav.calendar_by_name("missing").unwrap().is_none());
    }

    #[test]
    fn saving_a_calendar_updates_it_and_records_the_change() {
        let (store, notifier) = setup();
        let dav = DavStore::new(&store, &notifier, 7);
        let mut calendar = dav.create_calendar("work", "Work", 1_000).unwrap();
        calendar.display_name = "Werk".into();
        calendar.color = Some("#112233FF".into());
        dav.save_calendar(&calendar, 2_000).unwrap();

        let found = dav.calendar_by_name("work").unwrap().unwrap();
        assert_eq!(found.display_name, "Werk");
        assert_eq!(found.color.as_deref(), Some("#112233FF"));
        assert_eq!(found.modified, 2_000);

        let changes = ChangeLog::new(&store)
            .changes_since(7, Collection::Calendar, 0)
            .unwrap();
        assert!(changes.iter().any(|entry| entry.kind == ChangeKind::Update));
    }

    #[test]
    fn events_upsert_by_name_and_keep_their_creation_time() {
        let (store, notifier) = setup();
        let dav = DavStore::new(&store, &notifier, 7);
        let calendar = dav.create_calendar("work", "Work", 1_000).unwrap();

        let (_ical, info) = parse_ics(ICS).unwrap();
        let (event, created) = dav
            .upsert_event(calendar.id, "abc.ics", ICS, &info, 1_500)
            .unwrap();
        assert!(created);
        assert_eq!(event.uid, "one@example.com");
        assert_eq!(event.created, 1_500);

        let (_ical, info) = parse_ics(ICS_LATER).unwrap();
        let (updated, created) = dav
            .upsert_event(calendar.id, "abc.ics", ICS_LATER, &info, 2_500)
            .unwrap();
        assert!(!created);
        assert_eq!(updated.id, event.id);
        assert_eq!(updated.created, 1_500);
        assert_eq!(updated.modified, 2_500);
        assert_ne!(updated.etag, event.etag);

        let listed = dav.list_events(Some(calendar.id)).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].ics, ICS_LATER);
        assert_eq!(
            dav.event_by_name(calendar.id, "abc.ics")
                .unwrap()
                .unwrap()
                .id,
            event.id
        );
    }

    #[test]
    fn deleting_an_event_leaves_a_tombstone_for_sync() {
        let (store, notifier) = setup();
        let dav = DavStore::new(&store, &notifier, 7);
        let calendar = dav.create_calendar("work", "Work", 1_000).unwrap();
        let (_ical, info) = parse_ics(ICS).unwrap();
        let (event, _) = dav
            .upsert_event(calendar.id, "abc.ics", ICS, &info, 1_500)
            .unwrap();

        assert!(dav.delete_event(calendar.id, "abc.ics").unwrap());
        assert!(!dav.delete_event(calendar.id, "abc.ics").unwrap());
        assert!(dav.event_by_name(calendar.id, "abc.ics").unwrap().is_none());

        let tomb = dav
            .tombstone(Collection::CalendarEvent, event.id)
            .unwrap()
            .unwrap();
        assert_eq!(tomb.parent_id, calendar.id);
        assert_eq!(tomb.name, "abc.ics");

        let changes = ChangeLog::new(&store)
            .changes_since(7, Collection::CalendarEvent, 0)
            .unwrap();
        assert!(changes.iter().any(|entry| entry.kind == ChangeKind::Delete));
    }

    #[test]
    fn deleting_a_calendar_removes_its_events_with_tombstones() {
        let (store, notifier) = setup();
        let dav = DavStore::new(&store, &notifier, 7);
        let keep = dav.create_calendar("keep", "Keep", 1_000).unwrap();
        let gone = dav.create_calendar("gone", "Gone", 1_000).unwrap();
        let (_ical, info) = parse_ics(ICS).unwrap();
        dav.upsert_event(keep.id, "keep.ics", ICS, &info, 1_500)
            .unwrap();
        let (gone_event, _) = dav
            .upsert_event(gone.id, "gone.ics", ICS, &info, 1_500)
            .unwrap();

        assert!(dav.delete_calendar(gone.id).unwrap());
        assert!(!dav.delete_calendar(gone.id).unwrap());

        assert_eq!(dav.list_calendars().unwrap().len(), 1);
        assert_eq!(dav.list_events(None).unwrap().len(), 1);
        assert!(dav
            .tombstone(Collection::CalendarEvent, gone_event.id)
            .unwrap()
            .is_some());
    }

    #[test]
    fn cards_upsert_and_delete_like_events() {
        let (store, notifier) = setup();
        let dav = DavStore::new(&store, &notifier, 7);
        dav.ensure_defaults(1_000).unwrap();
        let book = dav.list_address_books().unwrap().remove(0);

        let (_card, info) = parse_vcf(VCF).unwrap();
        let (card, created) = dav
            .upsert_card(book.id, "abc.vcf", VCF, &info, 1_500)
            .unwrap();
        assert!(created);
        assert_eq!(card.full_name, "Saeed Sakib");
        assert_eq!(card.emails, vec!["saeed@example.com"]);
        assert_eq!(
            dav.card_by_name(book.id, "abc.vcf").unwrap().unwrap().id,
            card.id
        );

        assert!(dav.delete_card(book.id, "abc.vcf").unwrap());
        assert!(dav
            .tombstone(Collection::ContactCard, card.id)
            .unwrap()
            .is_some());
    }

    #[test]
    fn collection_state_tracks_the_latest_change() {
        let (store, notifier) = setup();
        let dav = DavStore::new(&store, &notifier, 7);
        assert_eq!(dav.state(Collection::CalendarEvent).unwrap(), 0);
        let calendar = dav.create_calendar("work", "Work", 1_000).unwrap();
        let (_ical, info) = parse_ics(ICS).unwrap();
        dav.upsert_event(calendar.id, "a.ics", ICS, &info, 1_500)
            .unwrap();
        assert!(dav.state(Collection::CalendarEvent).unwrap() > 0);
        assert!(dav.state(Collection::Calendar).unwrap() > 0);
    }
}
