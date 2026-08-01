mod collection;
mod object;
mod path;
mod propfind;
mod props;
mod report;
mod sync;
mod view;

pub use path::DEPTH_INFINITY;

use std::time::{SystemTime, UNIX_EPOCH};

use irixmail_core::Result;
use irixmail_store::{ChangeNotifier, Store};

use crate::proto::response::{error_body, Prefix};
use crate::storage::DavStore;

use view::Ctx;

pub const XML_CONTENT_TYPE: &str = "application/xml; charset=utf-8";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Cal,
    Card,
}

impl Family {
    pub fn segment(&self) -> &'static str {
        match self {
            Self::Cal => "cal",
            Self::Card => "card",
        }
    }
}

pub struct DavRequest<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub depth: Option<u8>,
    pub if_match: Option<&'a str>,
    pub if_none_match: Option<&'a str>,
    pub destination: Option<&'a str>,
    pub overwrite: bool,
    pub body: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DavReply {
    pub status: u16,
    pub content_type: Option<&'static str>,
    pub etag: Option<String>,
    pub body: Vec<u8>,
}

impl DavReply {
    pub fn status(status: u16) -> Self {
        Self {
            status,
            content_type: None,
            etag: None,
            body: Vec::new(),
        }
    }

    pub fn xml(status: u16, body: String) -> Self {
        Self {
            status,
            content_type: Some(XML_CONTENT_TYPE),
            etag: None,
            body: body.into_bytes(),
        }
    }

    pub fn error(status: u16, prefix: Prefix, condition: &str) -> Self {
        Self::xml(status, error_body(prefix, condition))
    }
}

pub fn handle(
    store: &dyn Store,
    notifier: &ChangeNotifier,
    account_id: u32,
    username: &str,
    req: &DavRequest<'_>,
) -> DavReply {
    match run(store, notifier, account_id, username, req) {
        Ok(reply) => reply,
        Err(error) => {
            tracing::warn!(target: "irixmail::dav", account = account_id, path = req.path, %error, "dav request failed");
            DavReply::status(500)
        }
    }
}

fn run(
    store: &dyn Store,
    notifier: &ChangeNotifier,
    account_id: u32,
    username: &str,
    req: &DavRequest<'_>,
) -> Result<DavReply> {
    if req.method == "OPTIONS" {
        return Ok(DavReply::status(200));
    }
    let now = now_millis();
    let dav = DavStore::new(store, notifier, account_id);
    dav.ensure_defaults(now)?;
    let ctx = Ctx { dav, username, now };
    let target = match path::parse_target(req.path, username) {
        Ok(target) => target,
        Err(status) => return Ok(DavReply::status(status)),
    };
    match req.method {
        "PROPFIND" => propfind::handle(&ctx, &target, req),
        "PROPPATCH" => collection::proppatch(&ctx, &target, req),
        "MKCALENDAR" | "MKCOL" => collection::mkcol(&ctx, &target, req),
        "GET" | "HEAD" => object::get(&ctx, &target, req),
        "PUT" => object::put(&ctx, &target, req),
        "DELETE" => collection::delete(&ctx, &target),
        "COPY" | "MOVE" => object::copy_or_move(&ctx, &target, req),
        "REPORT" => report::handle(&ctx, &target, req),
        _ => Ok(DavReply::status(405)),
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::MemStore;

    fn request<'a>(method: &'a str, path: &'a str) -> DavRequest<'a> {
        DavRequest {
            method,
            path,
            depth: None,
            if_match: None,
            if_none_match: None,
            destination: None,
            overwrite: true,
            body: b"",
        }
    }

    fn reply(req: &DavRequest<'_>) -> DavReply {
        let store = MemStore::default();
        let notifier = ChangeNotifier::new();
        handle(&store, &notifier, 7, "saeed@irixsoft.com", req)
    }

    #[test]
    fn an_unknown_path_is_not_found_and_a_foreign_subtree_is_forbidden() {
        assert_eq!(reply(&request("PROPFIND", "/nope/")).status, 404);
        assert_eq!(
            reply(&request("PROPFIND", "/dav/cal/other@irixsoft.com/")).status,
            403
        );
    }

    #[test]
    fn an_unsupported_method_is_rejected_and_options_is_allowed() {
        assert_eq!(reply(&request("PATCH", "/dav/cal/")).status, 405);
        assert_eq!(reply(&request("OPTIONS", "/dav/")).status, 200);
    }

    #[test]
    fn a_propfind_with_infinite_depth_is_forbidden() {
        let mut req = request("PROPFIND", "/dav/cal/saeed@irixsoft.com/");
        req.depth = Some(DEPTH_INFINITY);
        assert_eq!(reply(&req).status, 403);
    }
}
