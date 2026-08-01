use crate::proto::element::{Namespace, PropName};
use irixmail_core::{Error, Result};
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use quick_xml::NsReader;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropRequest {
    AllProp,
    PropName,
    Named(Vec<PropName>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropFind {
    pub props: PropRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyUpdate {
    pub set: Vec<(PropName, String)>,
    pub remove: Vec<PropName>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MkCol {
    pub display_name: Option<String>,
    pub color: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    SyncCollection(SyncCollection),
    CalendarQuery(CalendarQuery),
    CalendarMultiget(Multiget),
    AddressbookQuery(AddressbookQuery),
    AddressbookMultiget(Multiget),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncCollection {
    pub sync_token: Option<String>,
    pub limit: Option<usize>,
    pub props: PropRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarQuery {
    pub props: PropRequest,
    pub time_range: Option<(Option<i64>, Option<i64>)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Multiget {
    pub props: PropRequest,
    pub hrefs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressbookQuery {
    pub props: PropRequest,
    pub filters: Vec<TextFilter>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFilter {
    pub prop: String,
    pub value: String,
}

pub fn parse_propfind(body: &[u8]) -> Result<PropFind> {
    if is_blank(body) {
        return Ok(PropFind {
            props: PropRequest::AllProp,
        });
    }
    let root = root_element(body)?;
    expect(&root, &Namespace::Dav, "propfind")?;
    Ok(PropFind {
        props: prop_request(&root, PropRequest::AllProp),
    })
}

pub fn parse_propertyupdate(body: &[u8]) -> Result<PropertyUpdate> {
    let root = root_element(body)?;
    expect(&root, &Namespace::Dav, "propertyupdate")?;
    let mut update = PropertyUpdate {
        set: Vec::new(),
        remove: Vec::new(),
    };
    for section in &root.children {
        let removing = match &section.name {
            name if name.is(&Namespace::Dav, "set") => false,
            name if name.is(&Namespace::Dav, "remove") => true,
            _ => continue,
        };
        for prop in section.children_named(&Namespace::Dav, "prop") {
            for entry in &prop.children {
                if removing {
                    update.remove.push(entry.name.clone());
                } else {
                    update.set.push((entry.name.clone(), entry.deep_text()));
                }
            }
        }
    }
    Ok(update)
}

pub fn parse_mkcol(body: &[u8]) -> Result<MkCol> {
    if is_blank(body) {
        return Ok(MkCol::default());
    }
    let root = root_element(body)?;
    if !root.name.is(&Namespace::CalDav, "mkcalendar") && !root.name.is(&Namespace::Dav, "mkcol") {
        return Err(Error::invalid_input("expected a mkcalendar or mkcol body"));
    }
    let mut mkcol = MkCol::default();
    for section in root.children_named(&Namespace::Dav, "set") {
        for prop in section.children_named(&Namespace::Dav, "prop") {
            for entry in &prop.children {
                let value = entry.deep_text();
                match &entry.name {
                    name if name.is(&Namespace::Dav, "displayname") => {
                        mkcol.display_name = Some(value)
                    }
                    name if name.is(&Namespace::AppleICal, "calendar-color") => {
                        mkcol.color = Some(value)
                    }
                    name if name.is(&Namespace::CalDav, "calendar-description")
                        || name.is(&Namespace::CardDav, "addressbook-description") =>
                    {
                        mkcol.description = Some(value)
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(mkcol)
}

pub fn parse_report(body: &[u8]) -> Result<Report> {
    let root = root_element(body)?;
    let name = &root.name;
    if name.is(&Namespace::Dav, "sync-collection") {
        return Ok(Report::SyncCollection(SyncCollection {
            sync_token: root
                .child(&Namespace::Dav, "sync-token")
                .map(Node::deep_text)
                .filter(|token| !token.is_empty()),
            limit: limit_of(&root),
            props: prop_request(&root, PropRequest::Named(Vec::new())),
        }));
    }
    if name.is(&Namespace::CalDav, "calendar-query") {
        return Ok(Report::CalendarQuery(CalendarQuery {
            props: prop_request(&root, PropRequest::Named(Vec::new())),
            time_range: root
                .child(&Namespace::CalDav, "filter")
                .and_then(|filter| filter.find(&Namespace::CalDav, "time-range"))
                .map(|range| {
                    (
                        range.attr("start").and_then(utc_basic_to_epoch),
                        range.attr("end").and_then(utc_basic_to_epoch),
                    )
                }),
        }));
    }
    if name.is(&Namespace::CardDav, "addressbook-query") {
        let filters = root
            .child(&Namespace::CardDav, "filter")
            .map(|filter| {
                filter
                    .children_named(&Namespace::CardDav, "prop-filter")
                    .filter_map(|entry| {
                        let prop = entry.attr("name")?.to_ascii_uppercase();
                        let value = entry.child(&Namespace::CardDav, "text-match")?.deep_text();
                        Some(TextFilter { prop, value })
                    })
                    .collect()
            })
            .unwrap_or_default();
        return Ok(Report::AddressbookQuery(AddressbookQuery {
            props: prop_request(&root, PropRequest::Named(Vec::new())),
            filters,
            limit: limit_of(&root),
        }));
    }
    if name.is(&Namespace::CalDav, "calendar-multiget") {
        return Ok(Report::CalendarMultiget(multiget(&root)));
    }
    if name.is(&Namespace::CardDav, "addressbook-multiget") {
        return Ok(Report::AddressbookMultiget(multiget(&root)));
    }
    Err(Error::invalid_input(format!(
        "unsupported report {}",
        root.name.name
    )))
}

pub fn utc_basic_to_epoch(value: &str) -> Option<i64> {
    let bytes = value.as_bytes();
    if bytes.len() != 16 || bytes[8] != b'T' || bytes[15] != b'Z' {
        return None;
    }
    if !bytes
        .iter()
        .enumerate()
        .all(|(index, byte)| index == 8 || index == 15 || byte.is_ascii_digit())
    {
        return None;
    }
    let field = |range: std::ops::Range<usize>| value[range].parse::<i64>().ok();
    let year = field(0..4)?;
    let month = field(4..6)?;
    let day = field(6..8)?;
    let hour = field(9..11)?;
    let minute = field(11..13)?;
    let second = field(13..15)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86400 + hour * 3600 + minute * 60 + second)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted = (month + 9) % 12;
    let day_of_year = (153 * shifted + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146097 + day_of_era - 719468
}

fn multiget(root: &Node) -> Multiget {
    Multiget {
        props: prop_request(root, PropRequest::Named(Vec::new())),
        hrefs: root
            .children_named(&Namespace::Dav, "href")
            .map(Node::deep_text)
            .collect(),
    }
}

fn limit_of(root: &Node) -> Option<usize> {
    root.children
        .iter()
        .find(|child| {
            child.name.name == "limit"
                && matches!(child.name.ns, Namespace::Dav | Namespace::CardDav)
        })?
        .children
        .iter()
        .find(|child| child.name.name == "nresults")?
        .deep_text()
        .trim()
        .parse()
        .ok()
}

fn prop_request(root: &Node, default: PropRequest) -> PropRequest {
    for child in &root.children {
        if child.name.ns != Namespace::Dav {
            continue;
        }
        match child.name.name.as_str() {
            "allprop" => return PropRequest::AllProp,
            "propname" => return PropRequest::PropName,
            "prop" => {
                return PropRequest::Named(
                    child
                        .children
                        .iter()
                        .map(|prop| prop.name.clone())
                        .collect(),
                );
            }
            _ => {}
        }
    }
    default
}

fn expect(root: &Node, ns: &Namespace, name: &str) -> Result<()> {
    if root.name.is(ns, name) {
        Ok(())
    } else {
        Err(Error::invalid_input(format!(
            "expected a {name} body, got {}",
            root.name.name
        )))
    }
}

fn is_blank(body: &[u8]) -> bool {
    body.iter().all(u8::is_ascii_whitespace)
}

#[derive(Debug)]
struct Node {
    name: PropName,
    attrs: Vec<(String, String)>,
    text: String,
    children: Vec<Node>,
}

impl Node {
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    fn child(&self, ns: &Namespace, name: &str) -> Option<&Node> {
        self.children.iter().find(|child| child.name.is(ns, name))
    }

    fn children_named<'a>(
        &'a self,
        ns: &'a Namespace,
        name: &'a str,
    ) -> impl Iterator<Item = &'a Node> {
        self.children
            .iter()
            .filter(move |child| child.name.is(ns, name))
    }

    fn find(&self, ns: &Namespace, name: &str) -> Option<&Node> {
        for child in &self.children {
            if child.name.is(ns, name) {
                return Some(child);
            }
            if let Some(found) = child.find(ns, name) {
                return Some(found);
            }
        }
        None
    }

    fn deep_text(&self) -> String {
        let mut out = String::new();
        self.write_text(&mut out);
        out.trim().to_string()
    }

    fn write_text(&self, out: &mut String) {
        out.push_str(&self.text);
        for child in &self.children {
            child.write_text(out);
        }
    }
}

fn root_element(body: &[u8]) -> Result<Node> {
    let mut reader = NsReader::from_reader(body);
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    loop {
        let (ns, event) = reader
            .read_resolved_event()
            .map(|(resolved, event)| (resolve(resolved), event))
            .map_err(|err| Error::invalid_input(format!("malformed xml: {err}")))?;
        match event {
            Event::Start(start) => {
                stack.push(node(ns, start.local_name().as_ref(), start.attributes())?)
            }
            Event::Empty(empty) => {
                let node = node(ns, empty.local_name().as_ref(), empty.attributes())?;
                attach(&mut stack, &mut root, node)?;
            }
            Event::End(_) => {
                let node = stack
                    .pop()
                    .ok_or_else(|| Error::invalid_input("unbalanced xml"))?;
                attach(&mut stack, &mut root, node)?;
            }
            Event::Text(text) => {
                if let Some(open) = stack.last_mut() {
                    let value = text
                        .xml10_content()
                        .map_err(|err| Error::invalid_input(format!("bad encoding: {err}")))?;
                    open.text.push_str(&value);
                }
            }
            Event::GeneralRef(reference) => {
                if let Some(open) = stack.last_mut() {
                    let resolved = reference
                        .resolve_char_ref()
                        .map_err(|err| Error::invalid_input(format!("bad char ref: {err}")))?;
                    match resolved {
                        Some(character) => open.text.push(character),
                        None => {
                            let name = reference.decode().map_err(|err| {
                                Error::invalid_input(format!("bad entity: {err}"))
                            })?;
                            let value = quick_xml::escape::resolve_predefined_entity(&name)
                                .ok_or_else(|| {
                                    Error::invalid_input(format!("unknown entity {name}"))
                                })?;
                            open.text.push_str(value);
                        }
                    }
                }
            }
            Event::CData(data) => {
                if let Some(open) = stack.last_mut() {
                    let value = data
                        .xml10_content()
                        .map_err(|err| Error::invalid_input(format!("bad encoding: {err}")))?;
                    open.text.push_str(&value);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err(Error::invalid_input("unclosed xml element"));
    }
    root.ok_or_else(|| Error::invalid_input("no xml root element"))
}

fn attach(stack: &mut [Node], root: &mut Option<Node>, node: Node) -> Result<()> {
    match stack.last_mut() {
        Some(parent) => parent.children.push(node),
        None if root.is_some() => return Err(Error::invalid_input("multiple xml roots")),
        None => *root = Some(node),
    }
    Ok(())
}

fn node(
    ns: Option<Namespace>,
    local: &[u8],
    attrs: quick_xml::events::attributes::Attributes<'_>,
) -> Result<Node> {
    let name = std::str::from_utf8(local)
        .map_err(|_| Error::invalid_input("element name is not utf-8"))?
        .to_string();
    let mut collected = Vec::new();
    for attr in attrs {
        let attr = attr.map_err(|err| Error::invalid_input(format!("bad attribute: {err}")))?;
        let qname = std::str::from_utf8(attr.key.as_ref())
            .map_err(|_| Error::invalid_input("attribute name is not utf-8"))?;
        if qname == "xmlns" || qname.starts_with("xmlns:") {
            continue;
        }
        let local = attr.key.local_name();
        let key = std::str::from_utf8(local.as_ref())
            .map_err(|_| Error::invalid_input("attribute name is not utf-8"))?;
        let value = attr
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|err| Error::invalid_input(format!("bad attribute value: {err}")))?;
        collected.push((key.to_string(), value.into_owned()));
    }
    Ok(Node {
        name: PropName {
            ns: ns.unwrap_or_else(|| Namespace::Other(String::new())),
            name,
        },
        attrs: collected,
        text: String::new(),
        children: Vec::new(),
    })
}

fn resolve(resolved: ResolveResult<'_>) -> Option<Namespace> {
    match resolved {
        ResolveResult::Bound(ns) => std::str::from_utf8(ns.as_ref())
            .ok()
            .map(Namespace::from_uri),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::element::{Namespace, PropName};

    fn dav(name: &str) -> PropName {
        PropName::new(Namespace::Dav, name)
    }

    fn caldav(name: &str) -> PropName {
        PropName::new(Namespace::CalDav, name)
    }

    fn carddav(name: &str) -> PropName {
        PropName::new(Namespace::CardDav, name)
    }

    const PROPFIND_MIXED_PREFIXES: &str = concat!(
        r#"<?xml version="1.0" encoding="utf-8"?>"#,
        r#"<A:propfind xmlns:A="DAV:"><A:prop><A:displayname/>"#,
        r#"<B:calendar-color xmlns:B="http://apple.com/ns/ical/"/>"#,
        r#"</A:prop></A:propfind>"#
    );

    const PROPFIND_ALLPROP: &str =
        r#"<?xml version="1.0"?><propfind xmlns="DAV:"><allprop/></propfind>"#;

    const PROPFIND_PROPNAME: &str = r#"<D:propfind xmlns:D="DAV:"><D:propname/></D:propfind>"#;

    const PROPFIND_MALFORMED: &str = r#"<D:propfind xmlns:D="DAV:"><D:prop></D:propfind>"#;

    const PROPFIND_APPLE_CALENDAR: &str = concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        "\n",
        r#"<A:propfind xmlns:A="DAV:" xmlns:B="urn:ietf:params:xml:ns:caldav" xmlns:C="http://calendarserver.org/ns/">"#,
        "\n  <A:prop>\n    <A:resourcetype/>\n    <A:displayname/>\n    <C:getctag/>\n",
        "    <B:supported-calendar-component-set/>\n    <A:sync-token/>\n  </A:prop>\n",
        "</A:propfind>\n"
    );

    const PROPFIND_THUNDERBIRD: &str = concat!(
        r#"<D:propfind xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">"#,
        r#"<D:prop><D:current-user-principal/><C:calendar-home-set/></D:prop></D:propfind>"#
    );

    const PROPERTYUPDATE: &str = concat!(
        r#"<?xml version="1.0" encoding="utf-8"?>"#,
        r#"<D:propertyupdate xmlns:D="DAV:" xmlns:IC="http://apple.com/ns/ical/">"#,
        r#"<D:set><D:prop><D:displayname>Work &amp; Life</D:displayname>"#,
        r#"<IC:calendar-color>#FF2968FF</IC:calendar-color></D:prop></D:set>"#,
        r#"<D:remove><D:prop><D:getcontentlanguage/></D:prop></D:remove>"#,
        r#"</D:propertyupdate>"#
    );

    const PROPERTYUPDATE_CDATA: &str = concat!(
        r#"<D:propertyupdate xmlns:D="DAV:"><D:set><D:prop>"#,
        r#"<D:displayname><![CDATA[a<b]]> &amp; more</D:displayname>"#,
        r#"</D:prop></D:set></D:propertyupdate>"#
    );

    const MKCALENDAR: &str = concat!(
        r#"<?xml version="1.0" encoding="utf-8"?>"#,
        r#"<C:mkcalendar xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav" xmlns:IC="http://apple.com/ns/ical/">"#,
        r#"<D:set><D:prop><D:displayname>Travel</D:displayname>"#,
        r#"<IC:calendar-color>#711A76FF</IC:calendar-color>"#,
        r#"<C:calendar-description>Flights and hotels</C:calendar-description>"#,
        r#"</D:prop></D:set></C:mkcalendar>"#
    );

    const MKCOL_EXTENDED: &str = concat!(
        r#"<D:mkcol xmlns:D="DAV:" xmlns:B="urn:ietf:params:xml:ns:carddav">"#,
        r#"<D:set><D:prop><D:resourcetype><D:collection/><B:addressbook/></D:resourcetype>"#,
        r#"<D:displayname>Contacts</D:displayname>"#,
        r#"<B:addressbook-description>People</B:addressbook-description>"#,
        r#"</D:prop></D:set></D:mkcol>"#
    );

    const SYNC_INITIAL: &str = concat!(
        r#"<?xml version="1.0" encoding="utf-8"?>"#,
        r#"<D:sync-collection xmlns:D="DAV:"><D:sync-token/><D:sync-level>1</D:sync-level>"#,
        r#"<D:prop><D:getetag/></D:prop></D:sync-collection>"#
    );

    const SYNC_RESUME: &str = concat!(
        r#"<D:sync-collection xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">"#,
        r#"<D:sync-token>http://irixmail/ns/sync/42</D:sync-token>"#,
        r#"<D:sync-level>1</D:sync-level><D:limit><D:nresults>25</D:nresults></D:limit>"#,
        r#"<D:prop><D:getetag/><C:calendar-data/></D:prop></D:sync-collection>"#
    );

    const CALENDAR_QUERY: &str = concat!(
        r#"<?xml version="1.0" encoding="utf-8"?>"#,
        r#"<C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">"#,
        r#"<D:prop><D:getetag/><C:calendar-data/></D:prop>"#,
        r#"<C:filter><C:comp-filter name="VCALENDAR"><C:comp-filter name="VEVENT">"#,
        r#"<C:time-range start="20260210T100000Z" end="20260211T100000Z"/>"#,
        r#"</C:comp-filter></C:comp-filter></C:filter></C:calendar-query>"#
    );

    const CALENDAR_QUERY_NO_RANGE: &str = concat!(
        r#"<C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">"#,
        r#"<D:prop><D:getetag/></D:prop>"#,
        r#"<C:filter><C:comp-filter name="VCALENDAR"><C:comp-filter name="VTODO"/>"#,
        r#"</C:comp-filter></C:filter></C:calendar-query>"#
    );

    const CALENDAR_MULTIGET: &str = concat!(
        r#"<C:calendar-multiget xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">"#,
        r#"<D:prop><D:getetag/><C:calendar-data/></D:prop>"#,
        r#"<D:href>/dav/cal/u/work/one%20two.ics</D:href>"#,
        r#"<D:href>/dav/cal/u/work/b%26c.ics</D:href>"#,
        r#"</C:calendar-multiget>"#
    );

    const ADDRESSBOOK_QUERY: &str = concat!(
        r#"<?xml version="1.0" encoding="utf-8"?>"#,
        r#"<B:addressbook-query xmlns:D="DAV:" xmlns:B="urn:ietf:params:xml:ns:carddav">"#,
        r#"<D:prop><D:getetag/><B:address-data/></D:prop>"#,
        r#"<B:filter><B:prop-filter name="FN"><B:text-match collation="i;unicode-casemap" match-type="contains">sakib</B:text-match></B:prop-filter>"#,
        r#"<B:prop-filter name="email"><B:text-match>@example.com</B:text-match></B:prop-filter>"#,
        r#"<B:prop-filter name="NICKNAME"/>"#,
        r#"</B:filter><B:limit><B:nresults>10</B:nresults></B:limit></B:addressbook-query>"#
    );

    const ADDRESSBOOK_MULTIGET: &str = concat!(
        r#"<B:addressbook-multiget xmlns:D="DAV:" xmlns:B="urn:ietf:params:xml:ns:carddav">"#,
        r#"<D:prop><D:getetag/><B:address-data/></D:prop>"#,
        r#"<D:href>/dav/card/u/default/one.vcf</D:href></B:addressbook-multiget>"#
    );

    const UNKNOWN_REPORT: &str = r#"<D:principal-search-property-set xmlns:D="DAV:"/>"#;

    #[test]
    fn a_propfind_with_named_props_and_mixed_prefixes_parses() {
        let found = parse_propfind(PROPFIND_MIXED_PREFIXES.as_bytes()).unwrap();
        assert_eq!(
            found.props,
            PropRequest::Named(vec![
                dav("displayname"),
                PropName::new(Namespace::AppleICal, "calendar-color"),
            ])
        );
    }

    #[test]
    fn a_propfind_with_allprop_in_a_default_namespace_parses() {
        let found = parse_propfind(PROPFIND_ALLPROP.as_bytes()).unwrap();
        assert_eq!(found.props, PropRequest::AllProp);
    }

    #[test]
    fn a_propfind_with_propname_parses() {
        let found = parse_propfind(PROPFIND_PROPNAME.as_bytes()).unwrap();
        assert_eq!(found.props, PropRequest::PropName);
    }

    #[test]
    fn an_empty_propfind_body_means_allprop() {
        assert_eq!(parse_propfind(b"").unwrap().props, PropRequest::AllProp);
        assert_eq!(
            parse_propfind(b"  \r\n\t ").unwrap().props,
            PropRequest::AllProp
        );
    }

    #[test]
    fn malformed_propfind_xml_is_rejected() {
        assert!(parse_propfind(PROPFIND_MALFORMED.as_bytes()).is_err());
        assert!(parse_propfind(b"not xml at all").is_err());
    }

    #[test]
    fn a_propfind_with_a_foreign_root_is_rejected() {
        assert!(parse_propfind(br#"<hello xmlns="DAV:"/>"#).is_err());
    }

    #[test]
    fn an_apple_calendar_collection_propfind_parses() {
        let found = parse_propfind(PROPFIND_APPLE_CALENDAR.as_bytes()).unwrap();
        assert_eq!(
            found.props,
            PropRequest::Named(vec![
                dav("resourcetype"),
                dav("displayname"),
                PropName::new(Namespace::CalendarServer, "getctag"),
                caldav("supported-calendar-component-set"),
                dav("sync-token"),
            ])
        );
    }

    #[test]
    fn a_thunderbird_principal_propfind_parses() {
        let found = parse_propfind(PROPFIND_THUNDERBIRD.as_bytes()).unwrap();
        assert_eq!(
            found.props,
            PropRequest::Named(vec![
                dav("current-user-principal"),
                caldav("calendar-home-set"),
            ])
        );
    }

    #[test]
    fn a_propertyupdate_sets_and_removes_props() {
        let update = parse_propertyupdate(PROPERTYUPDATE.as_bytes()).unwrap();
        assert_eq!(
            update.set,
            vec![
                (dav("displayname"), "Work & Life".to_string()),
                (
                    PropName::new(Namespace::AppleICal, "calendar-color"),
                    "#FF2968FF".to_string()
                ),
            ]
        );
        assert_eq!(update.remove, vec![dav("getcontentlanguage")]);
    }

    #[test]
    fn cdata_and_entities_in_a_property_value_are_unescaped() {
        let update = parse_propertyupdate(PROPERTYUPDATE_CDATA.as_bytes()).unwrap();
        assert_eq!(
            update.set,
            vec![(dav("displayname"), "a<b & more".to_string())]
        );
    }

    #[test]
    fn a_mkcalendar_body_parses_display_name_color_and_description() {
        let mkcol = parse_mkcol(MKCALENDAR.as_bytes()).unwrap();
        assert_eq!(mkcol.display_name.as_deref(), Some("Travel"));
        assert_eq!(mkcol.color.as_deref(), Some("#711A76FF"));
        assert_eq!(mkcol.description.as_deref(), Some("Flights and hotels"));
    }

    #[test]
    fn an_extended_mkcol_body_parses_display_name_and_description() {
        let mkcol = parse_mkcol(MKCOL_EXTENDED.as_bytes()).unwrap();
        assert_eq!(mkcol.display_name.as_deref(), Some("Contacts"));
        assert_eq!(mkcol.color, None);
        assert_eq!(mkcol.description.as_deref(), Some("People"));
    }

    #[test]
    fn an_empty_mkcol_body_is_default() {
        assert_eq!(parse_mkcol(b"").unwrap(), MkCol::default());
        assert_eq!(parse_mkcol(b"\n  ").unwrap(), MkCol::default());
    }

    #[test]
    fn a_sync_collection_report_with_an_empty_token_parses() {
        let Report::SyncCollection(sync) = parse_report(SYNC_INITIAL.as_bytes()).unwrap() else {
            panic!("expected a sync-collection report");
        };
        assert_eq!(sync.sync_token, None);
        assert_eq!(sync.limit, None);
        assert_eq!(sync.props, PropRequest::Named(vec![dav("getetag")]));
    }

    #[test]
    fn a_sync_collection_report_with_a_token_and_limit_parses() {
        let Report::SyncCollection(sync) = parse_report(SYNC_RESUME.as_bytes()).unwrap() else {
            panic!("expected a sync-collection report");
        };
        assert_eq!(
            sync.sync_token.as_deref(),
            Some("http://irixmail/ns/sync/42")
        );
        assert_eq!(sync.limit, Some(25));
        assert_eq!(
            sync.props,
            PropRequest::Named(vec![dav("getetag"), caldav("calendar-data")])
        );
    }

    #[test]
    fn a_calendar_query_report_parses_its_time_range() {
        let Report::CalendarQuery(query) = parse_report(CALENDAR_QUERY.as_bytes()).unwrap() else {
            panic!("expected a calendar-query report");
        };
        assert_eq!(
            query.props,
            PropRequest::Named(vec![dav("getetag"), caldav("calendar-data")])
        );
        assert_eq!(query.time_range, Some((Some(1770717600), Some(1770804000))));
    }

    #[test]
    fn a_calendar_query_without_a_time_range_has_none() {
        let Report::CalendarQuery(query) =
            parse_report(CALENDAR_QUERY_NO_RANGE.as_bytes()).unwrap()
        else {
            panic!("expected a calendar-query report");
        };
        assert_eq!(query.time_range, None);
    }

    #[test]
    fn a_calendar_multiget_report_keeps_raw_hrefs() {
        let Report::CalendarMultiget(multiget) =
            parse_report(CALENDAR_MULTIGET.as_bytes()).unwrap()
        else {
            panic!("expected a calendar-multiget report");
        };
        assert_eq!(
            multiget.hrefs,
            vec![
                "/dav/cal/u/work/one%20two.ics".to_string(),
                "/dav/cal/u/work/b%26c.ics".to_string(),
            ]
        );
        assert_eq!(
            multiget.props,
            PropRequest::Named(vec![dav("getetag"), caldav("calendar-data")])
        );
    }

    #[test]
    fn an_addressbook_query_report_parses_text_filters() {
        let Report::AddressbookQuery(query) = parse_report(ADDRESSBOOK_QUERY.as_bytes()).unwrap()
        else {
            panic!("expected an addressbook-query report");
        };
        assert_eq!(
            query.props,
            PropRequest::Named(vec![dav("getetag"), carddav("address-data")])
        );
        assert_eq!(
            query.filters,
            vec![
                TextFilter {
                    prop: "FN".to_string(),
                    value: "sakib".to_string()
                },
                TextFilter {
                    prop: "EMAIL".to_string(),
                    value: "@example.com".to_string()
                },
            ]
        );
        assert_eq!(query.limit, Some(10));
    }

    #[test]
    fn an_addressbook_multiget_report_parses() {
        let Report::AddressbookMultiget(multiget) =
            parse_report(ADDRESSBOOK_MULTIGET.as_bytes()).unwrap()
        else {
            panic!("expected an addressbook-multiget report");
        };
        assert_eq!(
            multiget.hrefs,
            vec!["/dav/card/u/default/one.vcf".to_string()]
        );
    }

    #[test]
    fn an_unknown_report_root_is_rejected() {
        assert!(parse_report(UNKNOWN_REPORT.as_bytes()).is_err());
        assert!(parse_report(b"").is_err());
    }

    #[test]
    fn utc_basic_timestamps_convert_to_epoch_seconds() {
        assert_eq!(utc_basic_to_epoch("20260210T100000Z"), Some(1770717600));
        assert_eq!(utc_basic_to_epoch("19700101T000000Z"), Some(0));
        assert_eq!(utc_basic_to_epoch("20260210"), None);
        assert_eq!(utc_basic_to_epoch("20261310T100000Z"), None);
        assert_eq!(utc_basic_to_epoch("2026021.T100000Z"), None);
    }
}
