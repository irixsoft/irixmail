use crate::proto::element::{Namespace, PropName};
use std::borrow::Cow;
use std::fmt::{self, Display, Write};

const XML_DECL: &str = r#"<?xml version="1.0" encoding="utf-8"?>"#;

const ROOT_NAMESPACES: &str = concat!(
    r#"xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav" "#,
    r#"xmlns:B="urn:ietf:params:xml:ns:carddav" "#,
    r#"xmlns:CS="http://calendarserver.org/ns/" "#,
    r#"xmlns:IC="http://apple.com/ns/ical/""#
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prefix {
    D,
    C,
    B,
}

impl Prefix {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::D => "D",
            Self::C => "C",
            Self::B => "B",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceType {
    Collection,
    Calendar,
    AddressBook,
    Principal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropValue {
    Text(PropName, String),
    Empty(PropName),
    ResourceTypes(Vec<ResourceType>),
    Href(PropName, String),
    HrefSet(PropName, Vec<String>),
    SupportedReports(Vec<(Prefix, &'static str)>),
    SupportedCalendarComponents(Vec<String>),
    CalendarData(String),
    AddressData(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropStat {
    pub status: u16,
    pub props: Vec<PropValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DavResponse {
    pub href: String,
    pub propstats: Vec<PropStat>,
    pub status: Option<u16>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MultiStatus {
    pub responses: Vec<DavResponse>,
    pub sync_token: Option<String>,
}

impl Display for MultiStatus {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "{XML_DECL}<D:multistatus {ROOT_NAMESPACES}>")?;
        for response in &self.responses {
            response.fmt(out)?;
        }
        if let Some(token) = &self.sync_token {
            write!(out, "<D:sync-token>{}</D:sync-token>", xml_escape(token))?;
        }
        write!(out, "</D:multistatus>")
    }
}

impl Display for DavResponse {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "<D:response><D:href>{}</D:href>",
            xml_escape(&self.href)
        )?;
        match self.status {
            Some(status) => write!(out, "<D:status>{}</D:status>", status_line(status))?,
            None => {
                for propstat in &self.propstats {
                    propstat.fmt(out)?;
                }
            }
        }
        write!(out, "</D:response>")
    }
}

impl Display for PropStat {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "<D:propstat><D:prop>")?;
        for prop in &self.props {
            prop.fmt(out)?;
        }
        write!(
            out,
            "</D:prop><D:status>{}</D:status></D:propstat>",
            status_line(self.status)
        )
    }
}

impl Display for PropValue {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(name, value) => {
                open(out, name)?;
                write!(out, "{}", xml_escape(value))?;
                close(out, name)
            }
            Self::Empty(name) => empty(out, name),
            Self::ResourceTypes(kinds) => {
                let name = PropName::new(Namespace::Dav, "resourcetype");
                open(out, &name)?;
                for kind in kinds {
                    write!(out, "{}", kind.tag())?;
                }
                close(out, &name)
            }
            Self::Href(name, href) => {
                open(out, name)?;
                write!(out, "<D:href>{}</D:href>", xml_escape(href))?;
                close(out, name)
            }
            Self::HrefSet(name, hrefs) => {
                open(out, name)?;
                for href in hrefs {
                    write!(out, "<D:href>{}</D:href>", xml_escape(href))?;
                }
                close(out, name)
            }
            Self::SupportedReports(reports) => {
                write!(out, "<D:supported-report-set>")?;
                for (prefix, report) in reports {
                    write!(
                        out,
                        "<D:supported-report><D:report><{prefix}:{report}/></D:report></D:supported-report>",
                        prefix = prefix.as_str()
                    )?;
                }
                write!(out, "</D:supported-report-set>")
            }
            Self::SupportedCalendarComponents(comps) => {
                write!(out, "<C:supported-calendar-component-set>")?;
                for comp in comps {
                    write!(out, "<C:comp name=\"{}\"/>", xml_escape(comp))?;
                }
                write!(out, "</C:supported-calendar-component-set>")
            }
            Self::CalendarData(data) => {
                write!(
                    out,
                    "<C:calendar-data>{}</C:calendar-data>",
                    xml_escape(data)
                )
            }
            Self::AddressData(data) => {
                write!(out, "<B:address-data>{}</B:address-data>", xml_escape(data))
            }
        }
    }
}

impl ResourceType {
    fn tag(&self) -> &'static str {
        match self {
            Self::Collection => "<D:collection/>",
            Self::Calendar => "<C:calendar/>",
            Self::AddressBook => "<B:addressbook/>",
            Self::Principal => "<D:principal/>",
        }
    }
}

pub fn error_body(prefix: Prefix, condition: &str) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "{XML_DECL}<D:error xmlns:D=\"DAV:\" xmlns:C=\"urn:ietf:params:xml:ns:caldav\" \
         xmlns:B=\"urn:ietf:params:xml:ns:carddav\"><{prefix}:{condition}/></D:error>",
        prefix = prefix.as_str()
    );
    out
}

pub fn xml_escape(text: &str) -> Cow<'_, str> {
    if !text.contains(['&', '<', '>', '"']) {
        return Cow::Borrowed(text);
    }
    let mut escaped = String::with_capacity(text.len() + 16);
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            other => escaped.push(other),
        }
    }
    Cow::Owned(escaped)
}

fn status_line(status: u16) -> String {
    let reason = match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        412 => "Precondition Failed",
        423 => "Locked",
        424 => "Failed Dependency",
        507 => "Insufficient Storage",
        _ => "Status",
    };
    format!("HTTP/1.1 {status} {reason}")
}

fn open(out: &mut fmt::Formatter<'_>, name: &PropName) -> fmt::Result {
    match prefix_of(&name.ns) {
        Some(prefix) => write!(out, "<{prefix}:{}>", name.name),
        None => write!(
            out,
            "<x:{} xmlns:x=\"{}\">",
            name.name,
            xml_escape(name.ns.uri())
        ),
    }
}

fn close(out: &mut fmt::Formatter<'_>, name: &PropName) -> fmt::Result {
    match prefix_of(&name.ns) {
        Some(prefix) => write!(out, "</{prefix}:{}>", name.name),
        None => write!(out, "</x:{}>", name.name),
    }
}

fn empty(out: &mut fmt::Formatter<'_>, name: &PropName) -> fmt::Result {
    match prefix_of(&name.ns) {
        Some(prefix) => write!(out, "<{prefix}:{}/>", name.name),
        None => write!(
            out,
            "<x:{} xmlns:x=\"{}\"/>",
            name.name,
            xml_escape(name.ns.uri())
        ),
    }
}

fn prefix_of(ns: &Namespace) -> Option<&'static str> {
    match ns {
        Namespace::Dav => Some("D"),
        Namespace::CalDav => Some("C"),
        Namespace::CardDav => Some("B"),
        Namespace::CalendarServer => Some("CS"),
        Namespace::AppleICal => Some("IC"),
        Namespace::Other(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::element::{Namespace, PropName};

    const DECL: &str = r#"<?xml version="1.0" encoding="utf-8"?>"#;

    const ROOT_NS: &str = concat!(
        r#"xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav" "#,
        r#"xmlns:B="urn:ietf:params:xml:ns:carddav" "#,
        r#"xmlns:CS="http://calendarserver.org/ns/" "#,
        r#"xmlns:IC="http://apple.com/ns/ical/""#
    );

    fn dav(name: &str) -> PropName {
        PropName::new(Namespace::Dav, name)
    }

    #[test]
    fn a_multistatus_with_a_propstat_and_a_tombstone_renders() {
        let multistatus = MultiStatus {
            responses: vec![
                DavResponse {
                    href: "/dav/cal/u/work/".to_string(),
                    propstats: vec![PropStat {
                        status: 200,
                        props: vec![
                            PropValue::Text(dav("displayname"), "Work".to_string()),
                            PropValue::ResourceTypes(vec![
                                ResourceType::Collection,
                                ResourceType::Calendar,
                            ]),
                            PropValue::Href(
                                dav("current-user-principal"),
                                "/dav/principal/u/".to_string(),
                            ),
                        ],
                    }],
                    status: None,
                },
                DavResponse {
                    href: "/dav/cal/u/work/gone.ics".to_string(),
                    propstats: Vec::new(),
                    status: Some(404),
                },
            ],
            sync_token: None,
        };
        let expected = format!(
            concat!(
                "{decl}<D:multistatus {ns}>",
                "<D:response><D:href>/dav/cal/u/work/</D:href>",
                "<D:propstat><D:prop>",
                "<D:displayname>Work</D:displayname>",
                "<D:resourcetype><D:collection/><C:calendar/></D:resourcetype>",
                "<D:current-user-principal><D:href>/dav/principal/u/</D:href>",
                "</D:current-user-principal>",
                "</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat>",
                "</D:response>",
                "<D:response><D:href>/dav/cal/u/work/gone.ics</D:href>",
                "<D:status>HTTP/1.1 404 Not Found</D:status></D:response>",
                "</D:multistatus>"
            ),
            decl = DECL,
            ns = ROOT_NS
        );
        assert_eq!(multistatus.to_string(), expected);
    }

    #[test]
    fn a_multistatus_carries_its_sync_token() {
        let multistatus = MultiStatus {
            responses: Vec::new(),
            sync_token: Some("http://irixmail/ns/sync/42".to_string()),
        };
        let expected = format!(
            "{DECL}<D:multistatus {ROOT_NS}>\
             <D:sync-token>http://irixmail/ns/sync/42</D:sync-token></D:multistatus>"
        );
        assert_eq!(multistatus.to_string(), expected);
    }

    #[test]
    fn special_characters_are_xml_escaped() {
        assert_eq!(xml_escape("a<b&c>\"d\""), "a&lt;b&amp;c&gt;&quot;d&quot;");
        assert_eq!(xml_escape("plain"), "plain");
    }

    #[test]
    fn calendar_data_is_escaped_inside_its_element() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:a < b & c\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let rendered = MultiStatus {
            responses: vec![DavResponse {
                href: "/dav/cal/u/work/one.ics".to_string(),
                propstats: vec![PropStat {
                    status: 200,
                    props: vec![PropValue::CalendarData(ics.to_string())],
                }],
                status: None,
            }],
            sync_token: None,
        }
        .to_string();
        assert!(rendered.contains(
            "<C:calendar-data>BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\n\
             SUMMARY:a &lt; b &amp; c\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n</C:calendar-data>"
        ));
    }

    #[test]
    fn address_data_renders_in_the_carddav_namespace() {
        let rendered = props_to_string(vec![PropValue::AddressData(
            "BEGIN:VCARD\r\nFN:A & B\r\nEND:VCARD\r\n".to_string(),
        )]);
        assert!(rendered.contains(
            "<B:address-data>BEGIN:VCARD\r\nFN:A &amp; B\r\nEND:VCARD\r\n</B:address-data>"
        ));
    }

    #[test]
    fn supported_reports_and_components_render_with_the_right_prefixes() {
        let rendered = props_to_string(vec![
            PropValue::SupportedReports(vec![
                (Prefix::D, "sync-collection"),
                (Prefix::C, "calendar-multiget"),
                (Prefix::B, "addressbook-query"),
            ]),
            PropValue::SupportedCalendarComponents(vec!["VEVENT".to_string(), "VTODO".to_string()]),
        ]);
        assert!(rendered.contains(concat!(
            "<D:supported-report-set>",
            "<D:supported-report><D:report><D:sync-collection/></D:report></D:supported-report>",
            "<D:supported-report><D:report><C:calendar-multiget/></D:report></D:supported-report>",
            "<D:supported-report><D:report><B:addressbook-query/></D:report></D:supported-report>",
            "</D:supported-report-set>"
        )));
        assert!(rendered.contains(concat!(
            r#"<C:supported-calendar-component-set><C:comp name="VEVENT"/>"#,
            r#"<C:comp name="VTODO"/></C:supported-calendar-component-set>"#
        )));
    }

    #[test]
    fn href_sets_and_empty_props_render() {
        let rendered = props_to_string(vec![
            PropValue::HrefSet(
                PropName::new(Namespace::CalDav, "calendar-home-set"),
                vec!["/dav/cal/u/".to_string(), "/dav/cal/shared/".to_string()],
            ),
            PropValue::Empty(PropName::new(Namespace::CalendarServer, "getctag")),
        ]);
        assert!(rendered.contains(concat!(
            "<C:calendar-home-set><D:href>/dav/cal/u/</D:href>",
            "<D:href>/dav/cal/shared/</D:href></C:calendar-home-set>"
        )));
        assert!(rendered.contains("<CS:getctag/>"));
    }

    #[test]
    fn a_foreign_namespace_prop_declares_its_namespace_inline() {
        let rendered = props_to_string(vec![
            PropValue::Text(
                PropName::new(
                    Namespace::Other("http://example.com/ns".to_string()),
                    "custom",
                ),
                "v".to_string(),
            ),
            PropValue::Empty(PropName::new(
                Namespace::Other("http://example.com/ns".to_string()),
                "flag",
            )),
        ]);
        assert!(rendered.contains(r#"<x:custom xmlns:x="http://example.com/ns">v</x:custom>"#));
        assert!(rendered.contains(r#"<x:flag xmlns:x="http://example.com/ns"/>"#));
    }

    #[test]
    fn status_lines_map_known_codes() {
        let rendered = MultiStatus {
            responses: vec![DavResponse {
                href: "/dav/cal/u/work/".to_string(),
                propstats: vec![
                    PropStat {
                        status: 403,
                        props: vec![PropValue::Empty(dav("displayname"))],
                    },
                    PropStat {
                        status: 507,
                        props: vec![PropValue::Empty(dav("quota-available-bytes"))],
                    },
                    PropStat {
                        status: 418,
                        props: vec![PropValue::Empty(dav("teapot"))],
                    },
                ],
                status: None,
            }],
            sync_token: None,
        }
        .to_string();
        assert!(rendered.contains("<D:status>HTTP/1.1 403 Forbidden</D:status>"));
        assert!(rendered.contains("<D:status>HTTP/1.1 507 Insufficient Storage</D:status>"));
        assert!(rendered.contains("<D:status>HTTP/1.1 418 Status</D:status>"));
    }

    #[test]
    fn resource_types_render_principal_and_addressbook() {
        let rendered = props_to_string(vec![PropValue::ResourceTypes(vec![
            ResourceType::Collection,
            ResourceType::AddressBook,
            ResourceType::Principal,
        ])]);
        assert!(rendered.contains(concat!(
            "<D:resourcetype><D:collection/><B:addressbook/><D:principal/>",
            "</D:resourcetype>"
        )));
    }

    #[test]
    fn an_error_body_renders_its_condition() {
        assert_eq!(
            error_body(Prefix::C, "valid-calendar-data"),
            format!(
                "{DECL}<D:error xmlns:D=\"DAV:\" xmlns:C=\"urn:ietf:params:xml:ns:caldav\" \
                 xmlns:B=\"urn:ietf:params:xml:ns:carddav\"><C:valid-calendar-data/></D:error>"
            )
        );
        assert!(error_body(Prefix::D, "need-privileges").contains("<D:need-privileges/>"));
        assert!(error_body(Prefix::B, "no-uid-conflict").contains("<B:no-uid-conflict/>"));
    }

    fn props_to_string(props: Vec<PropValue>) -> String {
        MultiStatus {
            responses: vec![DavResponse {
                href: "/dav/".to_string(),
                propstats: vec![PropStat { status: 200, props }],
                status: None,
            }],
            sync_token: None,
        }
        .to_string()
    }
}
