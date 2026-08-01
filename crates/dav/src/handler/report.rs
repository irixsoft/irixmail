use irixmail_core::Result;

use crate::proto::request::{
    parse_report, AddressbookQuery, CalendarQuery, Multiget, Report, TextFilter,
};
use crate::proto::response::{DavResponse, MultiStatus};

use super::path::{object_href, parse_target, strip_origin, Target};
use super::props;
use super::sync;
use super::view::{CollectionView, Ctx, ObjectView};
use super::{DavReply, DavRequest, Family};

pub fn handle(ctx: &Ctx<'_>, target: &Target, req: &DavRequest<'_>) -> Result<DavReply> {
    let Target::Collection(family, name) = target else {
        return Ok(DavReply::status(403));
    };
    let Some(collection) = ctx.collection(*family, name)? else {
        return Ok(DavReply::status(404));
    };
    let Ok(report) = parse_report(req.body) else {
        return Ok(DavReply::status(400));
    };
    match (report, family) {
        (Report::SyncCollection(request), _) => sync::report(ctx, *family, &collection, &request),
        (Report::CalendarQuery(request), Family::Cal) => calendar_query(ctx, &collection, &request),
        (Report::CalendarMultiget(request), Family::Cal) => {
            multiget(ctx, Family::Cal, &collection, &request)
        }
        (Report::AddressbookQuery(request), Family::Card) => {
            addressbook_query(ctx, &collection, &request)
        }
        (Report::AddressbookMultiget(request), Family::Card) => {
            multiget(ctx, Family::Card, &collection, &request)
        }
        _ => Ok(DavReply::status(403)),
    }
}

fn multistatus(responses: Vec<DavResponse>) -> DavReply {
    DavReply::xml(
        207,
        MultiStatus {
            responses,
            sync_token: None,
        }
        .to_string(),
    )
}

fn object_response(
    ctx: &Ctx<'_>,
    family: Family,
    collection: &CollectionView,
    object: &ObjectView,
    request: &crate::proto::request::PropRequest,
) -> DavResponse {
    props::respond(
        object_href(family, ctx.username, &collection.name, &object.name),
        request,
        props::object_props(family, object),
    )
}

fn calendar_query(
    ctx: &Ctx<'_>,
    collection: &CollectionView,
    request: &CalendarQuery,
) -> Result<DavReply> {
    let props = props::with_default_etag(&request.props);
    let responses = ctx
        .objects(Family::Cal, collection.id)?
        .iter()
        .filter(|object| match request.time_range {
            Some((start, end)) => {
                object.starts_min < end.unwrap_or(i64::MAX)
                    && object.ends_max > start.unwrap_or(i64::MIN)
            }
            None => true,
        })
        .map(|object| object_response(ctx, Family::Cal, collection, object, &props))
        .collect();
    Ok(multistatus(responses))
}

fn matches(filter: &TextFilter, object: &ObjectView) -> bool {
    let needle = filter.value.to_lowercase();
    match filter.prop.as_str() {
        "EMAIL" => object
            .emails
            .iter()
            .any(|email| email.to_lowercase().contains(&needle)),
        _ => object.full_name.to_lowercase().contains(&needle),
    }
}

fn addressbook_query(
    ctx: &Ctx<'_>,
    collection: &CollectionView,
    request: &AddressbookQuery,
) -> Result<DavReply> {
    let props = props::with_default_etag(&request.props);
    let mut responses: Vec<DavResponse> = ctx
        .objects(Family::Card, collection.id)?
        .iter()
        .filter(|object| request.filters.iter().all(|filter| matches(filter, object)))
        .map(|object| object_response(ctx, Family::Card, collection, object, &props))
        .collect();
    if let Some(limit) = request.limit {
        responses.truncate(limit);
    }
    Ok(multistatus(responses))
}

fn multiget(
    ctx: &Ctx<'_>,
    family: Family,
    collection: &CollectionView,
    request: &Multiget,
) -> Result<DavReply> {
    let props = props::with_default_etag(&request.props);
    let mut responses = Vec::new();
    for href in &request.hrefs {
        let resolved = match parse_target(strip_origin(href), ctx.username) {
            Ok(Target::Object(target_family, target_collection, name))
                if target_family == family && target_collection == collection.name =>
            {
                ctx.object(family, collection.id, &name)?
            }
            _ => None,
        };
        match resolved {
            Some(object) => {
                responses.push(object_response(ctx, family, collection, &object, &props))
            }
            None => responses.push(DavResponse {
                href: href.clone(),
                propstats: Vec::new(),
                status: Some(404),
            }),
        }
    }
    Ok(multistatus(responses))
}
