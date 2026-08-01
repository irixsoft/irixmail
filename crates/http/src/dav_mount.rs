use axum::body::{to_bytes, Body};
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;

use irixmail_dav::handler::{self, DavRequest, DEPTH_INFINITY};

use crate::app::AppState;

const MAX_BODY: usize = 8 * 1024 * 1024;

const DAV_FEATURES: &str = "1, 3, access-control, calendar-access, addressbook, extended-mkcol";

const DAV_ALLOW: &str = concat!(
    "OPTIONS, GET, HEAD, PUT, DELETE, PROPFIND, PROPPATCH, ",
    "MKCOL, MKCALENDAR, REPORT, COPY, MOVE"
);

pub fn routes(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/dav", any(dav_entry))
        .route("/dav/", any(dav_entry))
        .route("/dav/{*rest}", any(dav_entry))
        .layer(DefaultBodyLimit::disable())
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn options_reply() -> Response {
    let mut response = StatusCode::OK.into_response();
    let headers = response.headers_mut();
    headers.insert("DAV", DAV_FEATURES.parse().expect("static dav header"));
    headers.insert(header::ALLOW, DAV_ALLOW.parse().expect("static allow"));
    response
}

fn unauthorized() -> Response {
    let mut response = StatusCode::UNAUTHORIZED.into_response();
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        "Basic realm=\"IRIXMAIL\""
            .parse()
            .expect("static challenge"),
    );
    response
}

async fn dav_entry(State(state): State<AppState>, request: Request) -> Response {
    if request.method() == axum::http::Method::OPTIONS {
        return options_reply();
    }
    let Some(identity) = crate::auth_mw::authenticate_request(&state, &request).await else {
        return unauthorized();
    };
    let method = request.method().as_str().to_ascii_uppercase();
    let path = request.uri().path().to_string();
    let (parts, body) = request.into_parts();
    let body = match to_bytes(body, MAX_BODY).await {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let headers = &parts.headers;
    let depth = header_str(headers, "depth").map(|value| match value.trim() {
        "0" => 0,
        "1" => 1,
        _ => DEPTH_INFINITY,
    });
    let overwrite = header_str(headers, "overwrite")
        .is_none_or(|value| !value.trim().eq_ignore_ascii_case("F"));
    let request = DavRequest {
        method: &method,
        path: &path,
        depth,
        if_match: header_str(headers, "if-match"),
        if_none_match: header_str(headers, "if-none-match"),
        destination: header_str(headers, "destination"),
        overwrite,
        body: &body,
    };
    let reply = handler::handle(
        state.store.as_ref(),
        &state.notifier,
        identity.account_id as u32,
        &identity.username,
        &request,
    );
    let mut response = Response::builder()
        .status(reply.status)
        .header("DAV", DAV_FEATURES);
    if let Some(content_type) = reply.content_type {
        response = response.header(header::CONTENT_TYPE, content_type);
    }
    if let Some(etag) = &reply.etag {
        response = response.header(header::ETAG, format!("\"{etag}\""));
    }
    response
        .body(Body::from(reply.body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request as HttpRequest, StatusCode};
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    use tower::ServiceExt;

    use irixmail_directory::{password, Role};

    use crate::app::{router, AppState};
    use crate::tests_support::{state, TempDir};

    const ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//IRIXMAIL//EN\r\nBEGIN:VEVENT\r\nUID:one@example.com\r\nDTSTAMP:20260101T000000Z\r\nDTSTART:20260210T100000Z\r\nDTEND:20260210T110000Z\r\nSUMMARY:Meeting\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    const ICS_UPDATED: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//IRIXMAIL//EN\r\nBEGIN:VEVENT\r\nUID:one@example.com\r\nDTSTAMP:20260101T000000Z\r\nDTSTART:20260210T120000Z\r\nDTEND:20260210T130000Z\r\nSUMMARY:Moved\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    const VCF: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:card-1\r\nFN:Saeed Sakib\r\nEMAIL:saeed@example.com\r\nEND:VCARD\r\n";

    fn setup(dir: &TempDir) -> (AppState, String) {
        let shared = state(dir);
        let domain = shared
            .directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let account = shared
            .directory
            .accounts()
            .create("alice", domain.id, "Alice", Role::User)
            .unwrap();
        shared
            .directory
            .credentials()
            .set_primary_password(account.id, password::hash("hunter2").unwrap())
            .unwrap();
        let auth = format!("Basic {}", STANDARD.encode("alice@example.com:hunter2"));
        (shared, auth)
    }

    fn build(method: &str, uri: &str, auth: Option<&str>, body: &str) -> HttpRequest<Body> {
        let mut request = HttpRequest::builder().method(method).uri(uri);
        if let Some(auth) = auth {
            request = request.header(header::AUTHORIZATION, auth);
        }
        request.body(Body::from(body.to_string())).unwrap()
    }

    async fn send(shared: &AppState, request: HttpRequest<Body>) -> (StatusCode, String, String) {
        let response = router(shared.clone()).oneshot(request).await.unwrap();
        let status = response.status();
        let etag = response
            .headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, etag, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn call(
        shared: &AppState,
        auth: &str,
        method: &str,
        uri: &str,
        body: &str,
    ) -> (StatusCode, String, String) {
        send(shared, build(method, uri, Some(auth), body)).await
    }

    fn between(body: &str, tag: &str) -> String {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        let start = body.find(&open).expect("open tag") + open.len();
        let end = body[start..].find(&close).expect("close tag") + start;
        body[start..end].to_string()
    }

    #[tokio::test]
    async fn an_unauthenticated_dav_request_asks_for_basic_credentials() {
        let dir = TempDir::new();
        let (shared, _) = setup(&dir);
        let response = router(shared)
            .oneshot(build("PROPFIND", "/dav/cal/", None, ""))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let challenge = response
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(challenge.starts_with("Basic"), "{challenge}");
    }

    #[tokio::test]
    async fn options_needs_no_credentials_and_advertises_the_dav_features() {
        let dir = TempDir::new();
        let (shared, _) = setup(&dir);
        let response = router(shared)
            .oneshot(build("OPTIONS", "/dav/cal/", None, ""))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let features = response.headers().get("dav").unwrap().to_str().unwrap();
        assert!(features.contains("calendar-access"), "{features}");
        assert!(features.contains("addressbook"), "{features}");
        let allow = response
            .headers()
            .get(header::ALLOW)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(allow.contains("PROPFIND"), "{allow}");
        assert!(allow.contains("MKCALENDAR"), "{allow}");
        assert!(allow.contains("REPORT"), "{allow}");
    }

    #[tokio::test]
    async fn the_well_known_urls_redirect_to_the_dav_service_roots() {
        let dir = TempDir::new();
        let (shared, _) = setup(&dir);
        for (well_known, location) in [
            ("/.well-known/caldav", "/dav/cal/"),
            ("/.well-known/carddav", "/dav/card/"),
        ] {
            let response = router(shared.clone())
                .oneshot(build("PROPFIND", well_known, None, ""))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
            assert_eq!(
                response.headers().get(header::LOCATION).unwrap(),
                location.parse::<axum::http::HeaderValue>().unwrap()
            );
        }
    }

    #[tokio::test]
    async fn a_service_root_propfind_points_at_the_current_user_principal() {
        let dir = TempDir::new();
        let (shared, auth) = setup(&dir);
        let (status, _, body) = call(&shared, &auth, "PROPFIND", "/dav/cal/", "").await;
        assert_eq!(status, StatusCode::MULTI_STATUS);
        assert!(
            body.contains(
                "<D:current-user-principal><D:href>/dav/principal/alice@example.com/</D:href>"
            ),
            "{body}"
        );
    }

    #[tokio::test]
    async fn a_principal_propfind_lists_both_home_sets() {
        let dir = TempDir::new();
        let (shared, auth) = setup(&dir);
        let (status, _, body) = call(
            &shared,
            &auth,
            "PROPFIND",
            "/dav/principal/alice@example.com/",
            "",
        )
        .await;
        assert_eq!(status, StatusCode::MULTI_STATUS);
        assert!(
            body.contains("<C:calendar-home-set><D:href>/dav/cal/alice@example.com/</D:href>"),
            "{body}"
        );
        assert!(
            body.contains("<B:addressbook-home-set><D:href>/dav/card/alice@example.com/</D:href>"),
            "{body}"
        );
        assert!(body.contains("<D:principal/>"), "{body}");
    }

    #[tokio::test]
    async fn a_calendar_home_propfind_lists_the_default_calendar() {
        let dir = TempDir::new();
        let (shared, auth) = setup(&dir);
        let request = HttpRequest::builder()
            .method("PROPFIND")
            .uri("/dav/cal/alice@example.com/")
            .header(header::AUTHORIZATION, &auth)
            .header("depth", "1")
            .body(Body::empty())
            .unwrap();
        let (status, _, body) = send(&shared, request).await;
        assert_eq!(status, StatusCode::MULTI_STATUS);
        assert!(
            body.contains("<D:href>/dav/cal/alice@example.com/calendar/</D:href>"),
            "{body}"
        );
        assert!(body.contains("<C:calendar/>"), "{body}");
    }

    #[tokio::test]
    async fn a_calendar_collection_is_created_by_mkcalendar_and_refuses_a_duplicate() {
        let dir = TempDir::new();
        let (shared, auth) = setup(&dir);
        let body = concat!(
            r#"<?xml version="1.0" encoding="utf-8"?>"#,
            r#"<C:mkcalendar xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">"#,
            "<D:set><D:prop><D:displayname>Work</D:displayname></D:prop></D:set>",
            "</C:mkcalendar>"
        );
        let (status, _, _) = call(
            &shared,
            &auth,
            "MKCALENDAR",
            "/dav/cal/alice@example.com/work/",
            body,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let (again, _, _) = call(
            &shared,
            &auth,
            "MKCALENDAR",
            "/dav/cal/alice@example.com/work/",
            body,
        )
        .await;
        assert_eq!(again, StatusCode::METHOD_NOT_ALLOWED);

        let (_, _, listing) = call(
            &shared,
            &auth,
            "PROPFIND",
            "/dav/cal/alice@example.com/work/",
            "",
        )
        .await;
        assert!(
            listing.contains("<D:displayname>Work</D:displayname>"),
            "{listing}"
        );
    }

    #[tokio::test]
    async fn an_event_round_trips_through_put_and_get_with_its_etag() {
        let dir = TempDir::new();
        let (shared, auth) = setup(&dir);
        let path = "/dav/cal/alice@example.com/calendar/one.ics";
        let (status, etag, _) = call(&shared, &auth, "PUT", path, ICS).await;
        assert_eq!(status, StatusCode::CREATED);
        assert!(etag.starts_with('"') && etag.ends_with('"'), "{etag}");

        let (status, get_etag, body) = call(&shared, &auth, "GET", path, "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(get_etag, etag);
        assert_eq!(body, ICS);

        let (status, new_etag, _) = call(&shared, &auth, "PUT", path, ICS_UPDATED).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_ne!(new_etag, etag);
    }

    #[tokio::test]
    async fn stale_conditional_headers_fail_the_precondition() {
        let dir = TempDir::new();
        let (shared, auth) = setup(&dir);
        let path = "/dav/cal/alice@example.com/calendar/one.ics";
        let (status, _, _) = call(&shared, &auth, "PUT", path, ICS).await;
        assert_eq!(status, StatusCode::CREATED);

        let stale = HttpRequest::builder()
            .method("PUT")
            .uri(path)
            .header(header::AUTHORIZATION, &auth)
            .header(header::IF_MATCH, "\"deadbeef\"")
            .body(Body::from(ICS_UPDATED))
            .unwrap();
        let (status, _, _) = send(&shared, stale).await;
        assert_eq!(status, StatusCode::PRECONDITION_FAILED);

        let exists = HttpRequest::builder()
            .method("PUT")
            .uri(path)
            .header(header::AUTHORIZATION, &auth)
            .header(header::IF_NONE_MATCH, "*")
            .body(Body::from(ICS_UPDATED))
            .unwrap();
        let (status, _, _) = send(&shared, exists).await;
        assert_eq!(status, StatusCode::PRECONDITION_FAILED);
    }

    #[tokio::test]
    async fn invalid_calendar_data_and_duplicate_uids_are_rejected() {
        let dir = TempDir::new();
        let (shared, auth) = setup(&dir);
        let (status, _, body) = call(
            &shared,
            &auth,
            "PUT",
            "/dav/cal/alice@example.com/calendar/junk.ics",
            "not a calendar",
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.contains("valid-calendar-data"), "{body}");

        let (status, _, _) = call(
            &shared,
            &auth,
            "PUT",
            "/dav/cal/alice@example.com/calendar/one.ics",
            ICS,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let (status, _, body) = call(
            &shared,
            &auth,
            "PUT",
            "/dav/cal/alice@example.com/calendar/two.ics",
            ICS,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(body.contains("no-uid-conflict"), "{body}");
    }

    #[tokio::test]
    async fn a_depth_one_collection_propfind_lists_the_event_with_a_quoted_etag() {
        let dir = TempDir::new();
        let (shared, auth) = setup(&dir);
        let (_, etag, _) = call(
            &shared,
            &auth,
            "PUT",
            "/dav/cal/alice@example.com/calendar/one.ics",
            ICS,
        )
        .await;
        let request = HttpRequest::builder()
            .method("PROPFIND")
            .uri("/dav/cal/alice@example.com/calendar/")
            .header(header::AUTHORIZATION, &auth)
            .header("depth", "1")
            .body(Body::empty())
            .unwrap();
        let (status, _, body) = send(&shared, request).await;
        assert_eq!(status, StatusCode::MULTI_STATUS);
        assert!(
            body.contains("<D:href>/dav/cal/alice@example.com/calendar/one.ics</D:href>"),
            "{body}"
        );
        let quoted = etag.replace('"', "&quot;");
        assert!(
            body.contains(&format!("<D:getetag>{quoted}</D:getetag>")),
            "{body}"
        );
    }

    #[tokio::test]
    async fn a_proppatch_stores_the_calendar_colour() {
        let dir = TempDir::new();
        let (shared, auth) = setup(&dir);
        let body = concat!(
            r#"<?xml version="1.0" encoding="utf-8"?>"#,
            r#"<D:propertyupdate xmlns:D="DAV:" xmlns:IC="http://apple.com/ns/ical/">"#,
            "<D:set><D:prop><IC:calendar-color>#FF0000FF</IC:calendar-color></D:prop></D:set>",
            "</D:propertyupdate>"
        );
        let (status, _, _) = call(
            &shared,
            &auth,
            "PROPPATCH",
            "/dav/cal/alice@example.com/calendar/",
            body,
        )
        .await;
        assert_eq!(status, StatusCode::MULTI_STATUS);

        let (_, _, listing) = call(
            &shared,
            &auth,
            "PROPFIND",
            "/dav/cal/alice@example.com/calendar/",
            "",
        )
        .await;
        assert!(
            listing.contains("<IC:calendar-color>#FF0000FF</IC:calendar-color>"),
            "{listing}"
        );
    }

    #[tokio::test]
    async fn a_calendar_query_filters_on_the_time_range() {
        let dir = TempDir::new();
        let (shared, auth) = setup(&dir);
        call(
            &shared,
            &auth,
            "PUT",
            "/dav/cal/alice@example.com/calendar/one.ics",
            ICS,
        )
        .await;
        let report = |start: &str, end: &str| {
            format!(
                concat!(
                    r#"<?xml version="1.0" encoding="utf-8"?>"#,
                    r#"<C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">"#,
                    "<D:prop><D:getetag/><C:calendar-data/></D:prop>",
                    r#"<C:filter><C:comp-filter name="VCALENDAR"><C:comp-filter name="VEVENT">"#,
                    r#"<C:time-range start="{start}" end="{end}"/>"#,
                    "</C:comp-filter></C:comp-filter></C:filter></C:calendar-query>"
                ),
                start = start,
                end = end
            )
        };

        let (status, _, hit) = call(
            &shared,
            &auth,
            "REPORT",
            "/dav/cal/alice@example.com/calendar/",
            &report("20260210T000000Z", "20260211T000000Z"),
        )
        .await;
        assert_eq!(status, StatusCode::MULTI_STATUS);
        assert!(hit.contains("one.ics"), "{hit}");
        assert!(hit.contains("<C:calendar-data>BEGIN:VCALENDAR"), "{hit}");

        let (_, _, miss) = call(
            &shared,
            &auth,
            "REPORT",
            "/dav/cal/alice@example.com/calendar/",
            &report("20260301T000000Z", "20260302T000000Z"),
        )
        .await;
        assert!(!miss.contains("one.ics"), "{miss}");
    }

    #[tokio::test]
    async fn a_calendar_multiget_reports_missing_hrefs_as_not_found() {
        let dir = TempDir::new();
        let (shared, auth) = setup(&dir);
        call(
            &shared,
            &auth,
            "PUT",
            "/dav/cal/alice@example.com/calendar/one.ics",
            ICS,
        )
        .await;
        let body = concat!(
            r#"<?xml version="1.0" encoding="utf-8"?>"#,
            r#"<C:calendar-multiget xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">"#,
            "<D:prop><D:getetag/></D:prop>",
            "<D:href>/dav/cal/alice@example.com/calendar/one.ics</D:href>",
            "<D:href>/dav/cal/alice@example.com/calendar/bogus.ics</D:href>",
            "</C:calendar-multiget>"
        );
        let (status, _, response) = call(
            &shared,
            &auth,
            "REPORT",
            "/dav/cal/alice@example.com/calendar/",
            body,
        )
        .await;
        assert_eq!(status, StatusCode::MULTI_STATUS);
        assert!(response.contains("one.ics"), "{response}");
        assert!(
            response.contains(concat!(
                "<D:response><D:href>/dav/cal/alice@example.com/calendar/bogus.ics</D:href>",
                "<D:status>HTTP/1.1 404 Not Found</D:status></D:response>"
            )),
            "{response}"
        );
    }

    #[tokio::test]
    async fn a_sync_collection_reports_an_initial_state_and_then_a_tombstone() {
        let dir = TempDir::new();
        let (shared, auth) = setup(&dir);
        let path = "/dav/cal/alice@example.com/calendar/one.ics";
        call(&shared, &auth, "PUT", path, ICS).await;
        let body = |token: &str| {
            format!(
                concat!(
                    r#"<?xml version="1.0" encoding="utf-8"?>"#,
                    r#"<D:sync-collection xmlns:D="DAV:">"#,
                    "<D:sync-token>{token}</D:sync-token><D:sync-level>1</D:sync-level>",
                    "<D:prop><D:getetag/></D:prop></D:sync-collection>"
                ),
                token = token
            )
        };

        let (status, _, initial) = call(
            &shared,
            &auth,
            "REPORT",
            "/dav/cal/alice@example.com/calendar/",
            &body(""),
        )
        .await;
        assert_eq!(status, StatusCode::MULTI_STATUS);
        assert_eq!(initial.matches("<D:response>").count(), 1);
        assert!(initial.contains("one.ics"), "{initial}");
        let token = between(&initial, "D:sync-token");
        assert!(token.starts_with("urn:irixmail:davsync:"), "{token}");

        let (status, _, _) = call(&shared, &auth, "DELETE", path, "").await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, _, changed) = call(
            &shared,
            &auth,
            "REPORT",
            "/dav/cal/alice@example.com/calendar/",
            &body(&token),
        )
        .await;
        assert_eq!(status, StatusCode::MULTI_STATUS);
        assert_eq!(changed.matches("<D:response>").count(), 1);
        assert!(
            changed.contains(concat!(
                "<D:response><D:href>/dav/cal/alice@example.com/calendar/one.ics</D:href>",
                "<D:status>HTTP/1.1 404 Not Found</D:status></D:response>"
            )),
            "{changed}"
        );
        assert_ne!(between(&changed, "D:sync-token"), token);
    }

    #[tokio::test]
    async fn the_last_calendar_of_an_account_cannot_be_deleted() {
        let dir = TempDir::new();
        let (shared, auth) = setup(&dir);
        let (status, _, _) = call(
            &shared,
            &auth,
            "MKCALENDAR",
            "/dav/cal/alice@example.com/work/",
            "",
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _, _) = call(
            &shared,
            &auth,
            "DELETE",
            "/dav/cal/alice@example.com/work/",
            "",
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _, _) = call(
            &shared,
            &auth,
            "DELETE",
            "/dav/cal/alice@example.com/calendar/",
            "",
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn a_card_is_stored_and_found_by_an_addressbook_query() {
        let dir = TempDir::new();
        let (shared, auth) = setup(&dir);
        let path = "/dav/card/alice@example.com/contacts/one.vcf";
        let (status, _, _) = call(&shared, &auth, "PUT", path, VCF).await;
        assert_eq!(status, StatusCode::CREATED);

        let query = |needle: &str| {
            format!(
                concat!(
                    r#"<?xml version="1.0" encoding="utf-8"?>"#,
                    r#"<B:addressbook-query xmlns:D="DAV:" xmlns:B="urn:ietf:params:xml:ns:carddav">"#,
                    "<D:prop><D:getetag/><B:address-data/></D:prop>",
                    r#"<B:filter><B:prop-filter name="FN">"#,
                    r#"<B:text-match match-type="contains">{needle}</B:text-match>"#,
                    "</B:prop-filter></B:filter></B:addressbook-query>"
                ),
                needle = needle
            )
        };

        let (status, _, hit) = call(
            &shared,
            &auth,
            "REPORT",
            "/dav/card/alice@example.com/contacts/",
            &query("saeed"),
        )
        .await;
        assert_eq!(status, StatusCode::MULTI_STATUS);
        assert!(hit.contains("one.vcf"), "{hit}");
        assert!(hit.contains("<B:address-data>BEGIN:VCARD"), "{hit}");

        let (_, _, miss) = call(
            &shared,
            &auth,
            "REPORT",
            "/dav/card/alice@example.com/contacts/",
            &query("nobody"),
        )
        .await;
        assert!(!miss.contains("one.vcf"), "{miss}");
    }

    #[tokio::test]
    async fn an_addressbook_multiget_and_sync_collection_answer_for_cards() {
        let dir = TempDir::new();
        let (shared, auth) = setup(&dir);
        call(
            &shared,
            &auth,
            "PUT",
            "/dav/card/alice@example.com/contacts/one.vcf",
            VCF,
        )
        .await;

        let multiget = concat!(
            r#"<?xml version="1.0" encoding="utf-8"?>"#,
            r#"<B:addressbook-multiget xmlns:D="DAV:" xmlns:B="urn:ietf:params:xml:ns:carddav">"#,
            "<D:prop><D:getetag/></D:prop>",
            "<D:href>/dav/card/alice@example.com/contacts/one.vcf</D:href>",
            "</B:addressbook-multiget>"
        );
        let (status, _, body) = call(
            &shared,
            &auth,
            "REPORT",
            "/dav/card/alice@example.com/contacts/",
            multiget,
        )
        .await;
        assert_eq!(status, StatusCode::MULTI_STATUS);
        assert!(body.contains("one.vcf"), "{body}");

        let sync = concat!(
            r#"<?xml version="1.0" encoding="utf-8"?>"#,
            r#"<D:sync-collection xmlns:D="DAV:"><D:sync-token/>"#,
            "<D:prop><D:getetag/></D:prop></D:sync-collection>"
        );
        let (status, _, body) = call(
            &shared,
            &auth,
            "REPORT",
            "/dav/card/alice@example.com/contacts/",
            sync,
        )
        .await;
        assert_eq!(status, StatusCode::MULTI_STATUS);
        assert_eq!(body.matches("<D:response>").count(), 1);
        assert!(body.contains("urn:irixmail:davsync:"), "{body}");
    }

    #[tokio::test]
    async fn an_event_is_copied_and_then_moved_to_another_calendar() {
        let dir = TempDir::new();
        let (shared, auth) = setup(&dir);
        let source = "/dav/cal/alice@example.com/calendar/one.ics";
        call(&shared, &auth, "PUT", source, ICS).await;
        call(
            &shared,
            &auth,
            "MKCALENDAR",
            "/dav/cal/alice@example.com/work/",
            "",
        )
        .await;

        let transfer = |method: &'static str, destination: &'static str, overwrite: bool| {
            let mut request = HttpRequest::builder()
                .method(method)
                .uri(source)
                .header(header::AUTHORIZATION, &auth)
                .header("destination", destination);
            if !overwrite {
                request = request.header("overwrite", "F");
            }
            request.body(Body::empty()).unwrap()
        };

        let (status, _, _) = send(
            &shared,
            transfer("COPY", "/dav/cal/alice@example.com/work/copy.ics", true),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _, copied) = call(
            &shared,
            &auth,
            "GET",
            "/dav/cal/alice@example.com/work/copy.ics",
            "",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(copied, ICS);

        let (status, _, _) = send(
            &shared,
            transfer("MOVE", "/dav/cal/alice@example.com/work/copy.ics", false),
        )
        .await;
        assert_eq!(status, StatusCode::PRECONDITION_FAILED);

        let (status, _, _) = send(
            &shared,
            transfer("MOVE", "/dav/cal/alice@example.com/work/moved.ics", true),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _, _) = call(&shared, &auth, "GET", source, "").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn an_object_propfind_returns_its_calendar_data_and_a_collection_get_is_refused() {
        let dir = TempDir::new();
        let (shared, auth) = setup(&dir);
        let path = "/dav/cal/alice@example.com/calendar/one.ics";
        call(&shared, &auth, "PUT", path, ICS).await;

        let body = concat!(
            r#"<?xml version="1.0" encoding="utf-8"?>"#,
            r#"<D:propfind xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">"#,
            "<D:prop><D:getetag/><C:calendar-data/></D:prop></D:propfind>"
        );
        let (status, _, response) = call(&shared, &auth, "PROPFIND", path, body).await;
        assert_eq!(status, StatusCode::MULTI_STATUS);
        assert!(
            response.contains("<C:calendar-data>BEGIN:VCALENDAR"),
            "{response}"
        );

        let (status, _, _) = call(
            &shared,
            &auth,
            "GET",
            "/dav/cal/alice@example.com/calendar/",
            "",
        )
        .await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn another_accounts_dav_subtree_is_forbidden() {
        let dir = TempDir::new();
        let (shared, _) = setup(&dir);
        let domain = shared
            .directory
            .domains()
            .get_by_name("example.com")
            .unwrap()
            .unwrap();
        let bob = shared
            .directory
            .accounts()
            .create("bob", domain.id, "Bob", Role::User)
            .unwrap();
        shared
            .directory
            .credentials()
            .set_primary_password(bob.id, password::hash("hunter3").unwrap())
            .unwrap();
        let auth = format!("Basic {}", STANDARD.encode("bob@example.com:hunter3"));

        let (status, _, _) = call(
            &shared,
            &auth,
            "PROPFIND",
            "/dav/cal/alice@example.com/calendar/",
            "",
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _, _) =
            call(&shared, &auth, "PROPFIND", "/dav/cal/bob@example.com/", "").await;
        assert_eq!(status, StatusCode::MULTI_STATUS);
    }
}
