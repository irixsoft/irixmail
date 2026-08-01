use irixmail_core::Result;

use crate::proto::response::Prefix;

use super::path::{parse_target, strip_origin, Target};
use super::view::{parse_object, Ctx};
use super::{DavReply, DavRequest, Family};

fn invalid_data(family: Family) -> DavReply {
    match family {
        Family::Cal => DavReply::error(403, Prefix::C, "valid-calendar-data"),
        Family::Card => DavReply::error(403, Prefix::B, "valid-address-data"),
    }
}

fn uid_conflict(family: Family) -> DavReply {
    match family {
        Family::Cal => DavReply::error(409, Prefix::C, "no-uid-conflict"),
        Family::Card => DavReply::error(409, Prefix::B, "no-uid-conflict"),
    }
}

fn unquote(value: &str) -> &str {
    let value = value.trim();
    let value = value.strip_prefix("W/").unwrap_or(value);
    value.trim_matches('"')
}

pub fn get(ctx: &Ctx<'_>, target: &Target, req: &DavRequest<'_>) -> Result<DavReply> {
    let Target::Object(family, collection, name) = target else {
        return Ok(DavReply::status(405));
    };
    let Some(parent) = ctx.collection(*family, collection)? else {
        return Ok(DavReply::status(404));
    };
    let Some(object) = ctx.object(*family, parent.id, name)? else {
        return Ok(DavReply::status(404));
    };
    let body = if req.method == "HEAD" {
        Vec::new()
    } else {
        object.data.into_bytes()
    };
    Ok(DavReply {
        status: 200,
        content_type: Some(family.content_type()),
        etag: Some(object.etag),
        body,
    })
}

pub fn put(ctx: &Ctx<'_>, target: &Target, req: &DavRequest<'_>) -> Result<DavReply> {
    let Target::Object(family, collection, name) = target else {
        return Ok(DavReply::status(405));
    };
    let Some(parent) = ctx.collection(*family, collection)? else {
        return Ok(DavReply::status(404));
    };
    let Ok(text) = std::str::from_utf8(req.body) else {
        return Ok(invalid_data(*family));
    };
    let Some(parsed) = parse_object(*family, text) else {
        return Ok(invalid_data(*family));
    };
    let existing = ctx.object(*family, parent.id, name)?;
    if req.if_none_match.map(unquote) == Some("*") && existing.is_some() {
        return Ok(DavReply::status(412));
    }
    if let Some(condition) = req.if_match.map(unquote) {
        let matched = match &existing {
            Some(object) => condition == "*" || object.etag == condition,
            None => false,
        };
        if !matched {
            return Ok(DavReply::status(412));
        }
    }
    if let Some(uid) = parsed.uid() {
        let clash = ctx
            .objects(*family, parent.id)?
            .into_iter()
            .any(|other| other.name != *name && other.uid.as_deref() == Some(uid));
        if clash {
            return Ok(uid_conflict(*family));
        }
    }
    let (object, created) = ctx.upsert(*family, parent.id, name, text, &parsed)?;
    Ok(DavReply {
        status: if created { 201 } else { 204 },
        content_type: None,
        etag: Some(object.etag),
        body: Vec::new(),
    })
}

pub fn copy_or_move(ctx: &Ctx<'_>, target: &Target, req: &DavRequest<'_>) -> Result<DavReply> {
    let Target::Object(family, collection, name) = target else {
        return Ok(DavReply::status(403));
    };
    let Some(destination) = req.destination else {
        return Ok(DavReply::status(400));
    };
    let Ok(Target::Object(dest_family, dest_collection, dest_name)) =
        parse_target(strip_origin(destination), ctx.username)
    else {
        return Ok(DavReply::status(403));
    };
    if dest_family != *family {
        return Ok(DavReply::status(403));
    }
    let Some(parent) = ctx.collection(*family, collection)? else {
        return Ok(DavReply::status(404));
    };
    let Some(object) = ctx.object(*family, parent.id, name)? else {
        return Ok(DavReply::status(404));
    };
    let Some(dest_parent) = ctx.collection(dest_family, &dest_collection)? else {
        return Ok(DavReply::status(409));
    };
    let overwriting = ctx
        .object(dest_family, dest_parent.id, &dest_name)?
        .is_some();
    if overwriting && !req.overwrite {
        return Ok(DavReply::status(412));
    }
    let Some(parsed) = parse_object(*family, &object.data) else {
        return Ok(invalid_data(*family));
    };
    let (_, created) = ctx.upsert(
        dest_family,
        dest_parent.id,
        &dest_name,
        &object.data,
        &parsed,
    )?;
    if req.method == "MOVE" && !(dest_parent.id == parent.id && dest_name == *name) {
        ctx.delete_object(*family, parent.id, name)?;
    }
    Ok(DavReply::status(if created { 201 } else { 204 }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etag_conditions_lose_their_quotes_and_weak_marker() {
        assert_eq!(unquote("\"abc\""), "abc");
        assert_eq!(unquote("W/\"abc\""), "abc");
        assert_eq!(unquote(" * "), "*");
        assert_eq!(unquote("abc"), "abc");
    }
}
