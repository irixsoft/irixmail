use irixmail_core::Result;

use crate::proto::request::parse_propfind;
use crate::proto::response::MultiStatus;

use super::path::{collection_href, home_href, object_href, principal_href, Target};
use super::props;
use super::view::Ctx;
use super::{DavReply, DavRequest};

pub fn handle(ctx: &Ctx<'_>, target: &Target, req: &DavRequest<'_>) -> Result<DavReply> {
    let Ok(propfind) = parse_propfind(req.body) else {
        return Ok(DavReply::status(400));
    };
    let depth = req.depth.unwrap_or(1);
    if depth > 1 {
        return Ok(DavReply::status(403));
    }
    let request = &propfind.props;
    let user = ctx.username;
    let mut responses = Vec::new();
    match target {
        Target::Root => responses.push(props::respond(
            "/dav/".to_string(),
            request,
            props::service_root_props(user),
        )),
        Target::Service(family) => responses.push(props::respond(
            format!("/dav/{}/", family.segment()),
            request,
            props::service_root_props(user),
        )),
        Target::Principal => responses.push(props::respond(
            principal_href(user),
            request,
            props::principal_props(user),
        )),
        Target::Home(family) => {
            responses.push(props::respond(
                home_href(*family, user),
                request,
                props::home_props(*family, user),
            ));
            if depth >= 1 {
                let state = ctx.dav.state(family.object_collection())?;
                for view in ctx.collections(*family)? {
                    responses.push(props::respond(
                        collection_href(*family, user, &view.name),
                        request,
                        props::collection_props(*family, user, &view, state),
                    ));
                }
            }
        }
        Target::Collection(family, name) => {
            let Some(view) = ctx.collection(*family, name)? else {
                return Ok(DavReply::status(404));
            };
            let state = ctx.dav.state(family.object_collection())?;
            responses.push(props::respond(
                collection_href(*family, user, &view.name),
                request,
                props::collection_props(*family, user, &view, state),
            ));
            if depth >= 1 {
                for object in ctx.objects(*family, view.id)? {
                    responses.push(props::respond(
                        object_href(*family, user, &view.name, &object.name),
                        request,
                        props::object_props(*family, &object),
                    ));
                }
            }
        }
        Target::Object(family, collection, name) => {
            let Some(parent) = ctx.collection(*family, collection)? else {
                return Ok(DavReply::status(404));
            };
            let Some(object) = ctx.object(*family, parent.id, name)? else {
                return Ok(DavReply::status(404));
            };
            responses.push(props::respond(
                object_href(*family, user, &parent.name, &object.name),
                request,
                props::object_props(*family, &object),
            ));
        }
    }
    Ok(DavReply::xml(
        207,
        MultiStatus {
            responses,
            sync_token: None,
        }
        .to_string(),
    ))
}
