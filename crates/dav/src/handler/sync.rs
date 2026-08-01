use irixmail_core::Result;
use irixmail_store::{ChangeKind, ChangeLogEntry};
use std::collections::BTreeMap;

use crate::proto::request::SyncCollection;
use crate::proto::response::{DavResponse, MultiStatus};

use super::path::object_href;
use super::props;
use super::view::{CollectionView, Ctx};
use super::{DavReply, Family};

pub const SYNC_TOKEN_PREFIX: &str = "urn:irixmail:davsync:";

pub fn sync_token(change_id: u64) -> String {
    format!("{SYNC_TOKEN_PREFIX}{change_id:x}")
}

pub fn parse_sync_token(token: Option<&str>) -> Option<u64> {
    let value = token?.trim().strip_prefix(SYNC_TOKEN_PREFIX)?;
    u64::from_str_radix(value, 16).ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coalesced {
    pub change_id: u64,
    pub document_id: u32,
    pub deleted: bool,
}

pub fn coalesce(changes: &[ChangeLogEntry]) -> Vec<Coalesced> {
    let mut seen: BTreeMap<u32, (u64, bool, bool)> = BTreeMap::new();
    for entry in changes {
        let slot = seen.entry(entry.document_id).or_insert((0, false, false));
        slot.0 = slot.0.max(entry.change_id);
        match entry.kind {
            ChangeKind::Insert => {
                slot.1 = true;
                slot.2 = false;
            }
            ChangeKind::Update => slot.2 = false,
            ChangeKind::Delete => slot.2 = true,
        }
    }
    let mut rows: Vec<Coalesced> = seen
        .into_iter()
        .filter(|(_, (_, inserted, deleted))| !(*inserted && *deleted))
        .map(|(document_id, (change_id, _, deleted))| Coalesced {
            change_id,
            document_id,
            deleted,
        })
        .collect();
    rows.sort_by_key(|row| row.change_id);
    rows
}

pub fn report(
    ctx: &Ctx<'_>,
    family: Family,
    collection: &CollectionView,
    request: &SyncCollection,
) -> Result<DavReply> {
    let objects = family.object_collection();
    let props = props::with_default_etag(&request.props);
    let user = ctx.username;
    let object_response = |object: &super::view::ObjectView| {
        props::respond(
            object_href(family, user, &collection.name, &object.name),
            &props,
            props::object_props(family, object),
        )
    };
    let (responses, token) = match parse_sync_token(request.sync_token.as_deref()) {
        None => {
            let responses = ctx
                .objects(family, collection.id)?
                .iter()
                .map(object_response)
                .collect();
            (responses, ctx.dav.state(objects)?)
        }
        Some(since) => {
            let changes = ctx.dav.changes_since(objects, since)?;
            let mut included: Vec<(u64, DavResponse)> = Vec::new();
            for row in coalesce(&changes) {
                if row.deleted {
                    let Some(tomb) = ctx.dav.tombstone(objects, row.document_id)? else {
                        continue;
                    };
                    if tomb.parent_id != collection.id {
                        continue;
                    }
                    included.push((
                        row.change_id,
                        DavResponse {
                            href: object_href(family, user, &collection.name, &tomb.name),
                            propstats: Vec::new(),
                            status: Some(404),
                        },
                    ));
                } else if let Some(object) = ctx.object_by_id(family, row.document_id)? {
                    if object.parent == collection.id {
                        included.push((row.change_id, object_response(&object)));
                    }
                }
            }
            let mut token = match changes.iter().map(|change| change.change_id).max() {
                Some(latest) => latest,
                None => ctx.dav.state(objects)?,
            };
            if let Some(limit) = request.limit {
                if included.len() > limit {
                    included.truncate(limit);
                    token = included.last().map_or(since, |(id, _)| *id);
                }
            }
            (
                included.into_iter().map(|(_, response)| response).collect(),
                token,
            )
        }
    };
    Ok(DavReply::xml(
        207,
        MultiStatus {
            responses,
            sync_token: Some(sync_token(token)),
        }
        .to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(change_id: u64, document_id: u32, kind: ChangeKind) -> ChangeLogEntry {
        ChangeLogEntry {
            change_id,
            document_id,
            kind,
        }
    }

    #[test]
    fn a_sync_token_round_trips_through_lowercase_hex() {
        assert_eq!(sync_token(255), "urn:irixmail:davsync:ff");
        assert_eq!(parse_sync_token(Some("urn:irixmail:davsync:ff")), Some(255));
        assert_eq!(parse_sync_token(Some("urn:irixmail:davsync:0")), Some(0));
    }

    #[test]
    fn a_missing_or_junk_sync_token_means_an_initial_sync() {
        assert_eq!(parse_sync_token(None), None);
        assert_eq!(parse_sync_token(Some("")), None);
        assert_eq!(parse_sync_token(Some("garbage")), None);
        assert_eq!(parse_sync_token(Some("urn:irixmail:davsync:zz")), None);
        assert_eq!(parse_sync_token(Some("urn:irixmail:davsync:")), None);
    }

    #[test]
    fn changes_coalesce_to_one_row_per_document_in_change_order() {
        let changes = vec![
            entry(1, 10, ChangeKind::Insert),
            entry(2, 11, ChangeKind::Insert),
            entry(3, 10, ChangeKind::Update),
            entry(4, 12, ChangeKind::Update),
            entry(5, 11, ChangeKind::Delete),
            entry(6, 13, ChangeKind::Delete),
        ];
        let rows = coalesce(&changes);
        assert_eq!(
            rows,
            vec![
                Coalesced {
                    change_id: 3,
                    document_id: 10,
                    deleted: false
                },
                Coalesced {
                    change_id: 4,
                    document_id: 12,
                    deleted: false
                },
                Coalesced {
                    change_id: 6,
                    document_id: 13,
                    deleted: true
                },
            ]
        );
    }

    #[test]
    fn a_document_created_and_deleted_inside_the_window_disappears() {
        let changes = vec![
            entry(1, 10, ChangeKind::Insert),
            entry(2, 10, ChangeKind::Update),
            entry(3, 10, ChangeKind::Delete),
        ];
        assert!(coalesce(&changes).is_empty());
    }
}
