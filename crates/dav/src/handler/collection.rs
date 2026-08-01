use irixmail_core::Result;

use crate::model::DeadProperty;
use crate::proto::element::{Namespace, PropName};
use crate::proto::request::{parse_mkcol, parse_propertyupdate};
use crate::proto::response::{DavResponse, MultiStatus, PropStat, PropValue};

use super::path::{collection_href, Target};
use super::view::{CollectionView, Ctx};
use super::{DavReply, DavRequest};

pub fn proppatch(ctx: &Ctx<'_>, target: &Target, req: &DavRequest<'_>) -> Result<DavReply> {
    let Target::Collection(family, name) = target else {
        return Ok(DavReply::status(403));
    };
    let Some(mut view) = ctx.collection(*family, name)? else {
        return Ok(DavReply::status(404));
    };
    let Ok(update) = parse_propertyupdate(req.body) else {
        return Ok(DavReply::status(400));
    };
    let mut touched = Vec::new();
    for (prop, value) in &update.set {
        apply_set(&mut view, prop, value);
        touched.push(PropValue::Empty(prop.clone()));
    }
    for prop in &update.remove {
        apply_remove(&mut view, prop);
        touched.push(PropValue::Empty(prop.clone()));
    }
    ctx.save_collection(*family, &view)?;
    let response = DavResponse {
        href: collection_href(*family, ctx.username, &view.name),
        propstats: vec![PropStat {
            status: 200,
            props: touched,
        }],
        status: None,
    };
    Ok(DavReply::xml(
        207,
        MultiStatus {
            responses: vec![response],
            sync_token: None,
        }
        .to_string(),
    ))
}

fn apply_set(view: &mut CollectionView, prop: &PropName, value: &str) {
    if prop.is(&Namespace::Dav, "displayname") {
        view.display_name = value.to_string();
    } else if prop.is(&Namespace::AppleICal, "calendar-color") {
        view.color = Some(value.to_string());
    } else if prop.is(&Namespace::AppleICal, "calendar-order") {
        match value.trim().parse::<u32>() {
            Ok(order) => {
                view.order = order;
                drop_dead(view, prop);
            }
            Err(_) => set_dead(view, prop, value),
        }
    } else if prop.is(&Namespace::CalDav, "calendar-description")
        || prop.is(&Namespace::CardDav, "addressbook-description")
    {
        view.description = Some(value.to_string());
    } else if prop.is(&Namespace::CalDav, "calendar-timezone") {
        view.time_zone = Some(value.to_string());
    } else {
        set_dead(view, prop, value);
    }
}

fn apply_remove(view: &mut CollectionView, prop: &PropName) {
    if prop.is(&Namespace::Dav, "displayname") {
        view.display_name = String::new();
    } else if prop.is(&Namespace::AppleICal, "calendar-color") {
        view.color = None;
    } else if prop.is(&Namespace::AppleICal, "calendar-order") {
        view.order = 0;
    } else if prop.is(&Namespace::CalDav, "calendar-description")
        || prop.is(&Namespace::CardDav, "addressbook-description")
    {
        view.description = None;
    } else if prop.is(&Namespace::CalDav, "calendar-timezone") {
        view.time_zone = None;
    }
    drop_dead(view, prop);
}

fn set_dead(view: &mut CollectionView, prop: &PropName, value: &str) {
    drop_dead(view, prop);
    view.dead.push(DeadProperty {
        ns: prop.ns.uri().to_string(),
        name: prop.name.clone(),
        value: value.to_string(),
    });
}

fn drop_dead(view: &mut CollectionView, prop: &PropName) {
    view.dead
        .retain(|entry| !(entry.ns == prop.ns.uri() && entry.name == prop.name));
}

pub fn mkcol(ctx: &Ctx<'_>, target: &Target, req: &DavRequest<'_>) -> Result<DavReply> {
    let Target::Collection(family, name) = target else {
        return Ok(DavReply::status(403));
    };
    if ctx.collection(*family, name)?.is_some() {
        return Ok(DavReply::status(405));
    }
    let Ok(body) = parse_mkcol(req.body) else {
        return Ok(DavReply::status(400));
    };
    let display_name = body.display_name.unwrap_or_else(|| name.clone());
    ctx.create_collection(*family, name, &display_name, body.color, body.description)?;
    Ok(DavReply::status(201))
}

pub fn delete(ctx: &Ctx<'_>, target: &Target) -> Result<DavReply> {
    match target {
        Target::Object(family, collection, name) => {
            let Some(parent) = ctx.collection(*family, collection)? else {
                return Ok(DavReply::status(404));
            };
            if ctx.delete_object(*family, parent.id, name)? {
                Ok(DavReply::status(204))
            } else {
                Ok(DavReply::status(404))
            }
        }
        Target::Collection(family, name) => {
            let all = ctx.collections(*family)?;
            let Some(view) = all.iter().find(|view| view.name == *name) else {
                return Ok(DavReply::status(404));
            };
            if all.len() <= 1 {
                return Ok(DavReply::status(403));
            }
            ctx.delete_collection(*family, view.id)?;
            Ok(DavReply::status(204))
        }
        _ => Ok(DavReply::status(403)),
    }
}
