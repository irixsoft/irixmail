use irixmail_core::Result;
use irixmail_store::Collection;

use crate::model::{
    AddressBookCollection, CalendarCollection, CalendarEventRecord, ContactCardRecord, DeadProperty,
};
use crate::parse::{parse_ics, parse_vcf, IcsInfo, VcfInfo};
use crate::storage::DavStore;

use super::Family;

impl Family {
    pub fn collection(&self) -> Collection {
        match self {
            Self::Cal => Collection::Calendar,
            Self::Card => Collection::AddressBook,
        }
    }

    pub fn object_collection(&self) -> Collection {
        match self {
            Self::Cal => Collection::CalendarEvent,
            Self::Card => Collection::ContactCard,
        }
    }

    pub fn content_type(&self) -> &'static str {
        match self {
            Self::Cal => "text/calendar; charset=utf-8",
            Self::Card => "text/vcard; charset=utf-8",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionView {
    pub id: u32,
    pub name: String,
    pub display_name: String,
    pub color: Option<String>,
    pub order: u32,
    pub description: Option<String>,
    pub time_zone: Option<String>,
    pub dead: Vec<DeadProperty>,
    pub created: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectView {
    pub id: u32,
    pub parent: u32,
    pub name: String,
    pub uid: Option<String>,
    pub data: String,
    pub etag: String,
    pub size: u32,
    pub starts_min: i64,
    pub ends_max: i64,
    pub full_name: String,
    pub emails: Vec<String>,
}

pub enum Parsed {
    Ics(IcsInfo),
    Vcf(VcfInfo),
}

impl Parsed {
    pub fn uid(&self) -> Option<&str> {
        match self {
            Self::Ics(info) => Some(info.uid.as_str()),
            Self::Vcf(info) => info.uid.as_deref(),
        }
    }
}

pub fn parse_object(family: Family, raw: &str) -> Option<Parsed> {
    match family {
        Family::Cal => parse_ics(raw).ok().map(|(_, info)| Parsed::Ics(info)),
        Family::Card => parse_vcf(raw).ok().map(|(_, info)| Parsed::Vcf(info)),
    }
}

fn calendar_view(record: &CalendarCollection) -> CollectionView {
    CollectionView {
        id: record.id,
        name: record.name.clone(),
        display_name: record.display_name.clone(),
        color: record.color.clone(),
        order: record.order,
        description: record.description.clone(),
        time_zone: record.time_zone.clone(),
        dead: record.dead_properties.clone(),
        created: record.created,
    }
}

fn book_view(record: &AddressBookCollection) -> CollectionView {
    CollectionView {
        id: record.id,
        name: record.name.clone(),
        display_name: record.display_name.clone(),
        color: None,
        order: 0,
        description: record.description.clone(),
        time_zone: None,
        dead: record.dead_properties.clone(),
        created: record.created,
    }
}

fn event_view(record: &CalendarEventRecord) -> ObjectView {
    ObjectView {
        id: record.id,
        parent: record.calendar_id,
        name: record.name.clone(),
        uid: Some(record.uid.clone()),
        data: record.ics.clone(),
        etag: record.etag.clone(),
        size: record.size,
        starts_min: record.starts_min,
        ends_max: record.ends_max,
        full_name: String::new(),
        emails: Vec::new(),
    }
}

fn card_view(record: &ContactCardRecord) -> ObjectView {
    ObjectView {
        id: record.id,
        parent: record.book_id,
        name: record.name.clone(),
        uid: record.uid.clone(),
        data: record.vcf.clone(),
        etag: record.etag.clone(),
        size: record.size,
        starts_min: 0,
        ends_max: 0,
        full_name: record.full_name.clone(),
        emails: record.emails.clone(),
    }
}

pub struct Ctx<'a> {
    pub dav: DavStore<'a>,
    pub username: &'a str,
    pub now: u64,
}

impl Ctx<'_> {
    pub fn collections(&self, family: Family) -> Result<Vec<CollectionView>> {
        match family {
            Family::Cal => Ok(self
                .dav
                .list_calendars()?
                .iter()
                .map(calendar_view)
                .collect()),
            Family::Card => Ok(self
                .dav
                .list_address_books()?
                .iter()
                .map(book_view)
                .collect()),
        }
    }

    pub fn collection(&self, family: Family, name: &str) -> Result<Option<CollectionView>> {
        Ok(self
            .collections(family)?
            .into_iter()
            .find(|view| view.name == name))
    }

    pub fn objects(&self, family: Family, parent: u32) -> Result<Vec<ObjectView>> {
        match family {
            Family::Cal => Ok(self
                .dav
                .list_events(Some(parent))?
                .iter()
                .map(event_view)
                .collect()),
            Family::Card => Ok(self
                .dav
                .list_cards(Some(parent))?
                .iter()
                .map(card_view)
                .collect()),
        }
    }

    pub fn object(&self, family: Family, parent: u32, name: &str) -> Result<Option<ObjectView>> {
        Ok(self
            .objects(family, parent)?
            .into_iter()
            .find(|view| view.name == name))
    }

    pub fn object_by_id(&self, family: Family, id: u32) -> Result<Option<ObjectView>> {
        match family {
            Family::Cal => Ok(self
                .dav
                .list_events(None)?
                .iter()
                .find(|record| record.id == id)
                .map(event_view)),
            Family::Card => Ok(self
                .dav
                .list_cards(None)?
                .iter()
                .find(|record| record.id == id)
                .map(card_view)),
        }
    }

    pub fn create_collection(
        &self,
        family: Family,
        name: &str,
        display_name: &str,
        color: Option<String>,
        description: Option<String>,
    ) -> Result<()> {
        match family {
            Family::Cal => {
                let mut calendar = self.dav.create_calendar(name, display_name, self.now)?;
                if color.is_some() || description.is_some() {
                    calendar.color = color;
                    calendar.description = description;
                    self.dav.save_calendar(&calendar, self.now)?;
                }
            }
            Family::Card => {
                let mut book = self.dav.create_address_book(name, display_name, self.now)?;
                if description.is_some() {
                    book.description = description;
                    self.dav.save_address_book(&book, self.now)?;
                }
            }
        }
        Ok(())
    }

    pub fn save_collection(&self, family: Family, view: &CollectionView) -> Result<()> {
        match family {
            Family::Cal => self.dav.save_calendar(
                &CalendarCollection {
                    id: view.id,
                    name: view.name.clone(),
                    display_name: view.display_name.clone(),
                    color: view.color.clone(),
                    order: view.order,
                    description: view.description.clone(),
                    time_zone: view.time_zone.clone(),
                    dead_properties: view.dead.clone(),
                    created: view.created,
                    modified: self.now,
                },
                self.now,
            ),
            Family::Card => self.dav.save_address_book(
                &AddressBookCollection {
                    id: view.id,
                    name: view.name.clone(),
                    display_name: view.display_name.clone(),
                    description: view.description.clone(),
                    dead_properties: view.dead.clone(),
                    created: view.created,
                    modified: self.now,
                },
                self.now,
            ),
        }
    }

    pub fn delete_collection(&self, family: Family, id: u32) -> Result<bool> {
        match family {
            Family::Cal => self.dav.delete_calendar(id),
            Family::Card => self.dav.delete_address_book(id),
        }
    }

    pub fn upsert(
        &self,
        family: Family,
        parent: u32,
        name: &str,
        raw: &str,
        parsed: &Parsed,
    ) -> Result<(ObjectView, bool)> {
        match (family, parsed) {
            (Family::Cal, Parsed::Ics(info)) => {
                let (record, created) = self.dav.upsert_event(parent, name, raw, info, self.now)?;
                Ok((event_view(&record), created))
            }
            (Family::Card, Parsed::Vcf(info)) => {
                let (record, created) = self.dav.upsert_card(parent, name, raw, info, self.now)?;
                Ok((card_view(&record), created))
            }
            _ => Err(irixmail_core::Error::invalid_input(
                "object does not match its collection",
            )),
        }
    }

    pub fn delete_object(&self, family: Family, parent: u32, name: &str) -> Result<bool> {
        match family {
            Family::Cal => self.dav.delete_event(parent, name),
            Family::Card => self.dav.delete_card(parent, name),
        }
    }
}
