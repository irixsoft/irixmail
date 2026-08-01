use crate::proto::element::{Namespace, PropName};
use crate::proto::request::PropRequest;
use crate::proto::response::{DavResponse, Prefix, PropStat, PropValue, ResourceType};

use super::path::{home_href, principal_href};
use super::view::{CollectionView, ObjectView};
use super::Family;

pub fn dav(name: &str) -> PropName {
    PropName::new(Namespace::Dav, name)
}

pub fn caldav(name: &str) -> PropName {
    PropName::new(Namespace::CalDav, name)
}

pub fn carddav(name: &str) -> PropName {
    PropName::new(Namespace::CardDav, name)
}

pub fn calendar_server(name: &str) -> PropName {
    PropName::new(Namespace::CalendarServer, name)
}

pub fn apple(name: &str) -> PropName {
    PropName::new(Namespace::AppleICal, name)
}

pub fn name_of(value: &PropValue) -> PropName {
    match value {
        PropValue::Text(name, _) | PropValue::Empty(name) => name.clone(),
        PropValue::Href(name, _) | PropValue::HrefSet(name, _) => name.clone(),
        PropValue::ResourceTypes(_) => dav("resourcetype"),
        PropValue::SupportedReports(_) => dav("supported-report-set"),
        PropValue::SupportedCalendarComponents(_) => caldav("supported-calendar-component-set"),
        PropValue::CalendarData(_) => caldav("calendar-data"),
        PropValue::AddressData(_) => carddav("address-data"),
    }
}

fn is_data(name: &PropName) -> bool {
    name.is(&Namespace::CalDav, "calendar-data") || name.is(&Namespace::CardDav, "address-data")
}

pub fn respond(href: String, request: &PropRequest, available: Vec<PropValue>) -> DavResponse {
    let propstats = match request {
        PropRequest::AllProp => vec![PropStat {
            status: 200,
            props: available
                .into_iter()
                .filter(|value| !is_data(&name_of(value)))
                .collect(),
        }],
        PropRequest::PropName => vec![PropStat {
            status: 200,
            props: available
                .iter()
                .map(name_of)
                .filter(|name| !is_data(name))
                .map(PropValue::Empty)
                .collect(),
        }],
        PropRequest::Named(names) => {
            let mut found = Vec::new();
            let mut missing = Vec::new();
            for name in names {
                match available.iter().find(|value| name_of(value) == *name) {
                    Some(value) => found.push(value.clone()),
                    None => missing.push(PropValue::Empty(name.clone())),
                }
            }
            let mut propstats = Vec::new();
            if !found.is_empty() || missing.is_empty() {
                propstats.push(PropStat {
                    status: 200,
                    props: found,
                });
            }
            if !missing.is_empty() {
                propstats.push(PropStat {
                    status: 404,
                    props: missing,
                });
            }
            propstats
        }
    };
    DavResponse {
        href,
        propstats,
        status: None,
    }
}

pub fn with_default_etag(request: &PropRequest) -> PropRequest {
    match request {
        PropRequest::Named(names) if names.is_empty() => PropRequest::Named(vec![dav("getetag")]),
        other => other.clone(),
    }
}

pub fn service_root_props(username: &str) -> Vec<PropValue> {
    vec![
        PropValue::ResourceTypes(vec![ResourceType::Collection]),
        PropValue::Href(dav("current-user-principal"), principal_href(username)),
        PropValue::Href(dav("principal-URL"), principal_href(username)),
    ]
}

pub fn principal_props(username: &str) -> Vec<PropValue> {
    vec![
        PropValue::ResourceTypes(vec![ResourceType::Collection, ResourceType::Principal]),
        PropValue::Text(dav("displayname"), username.to_string()),
        PropValue::Href(dav("current-user-principal"), principal_href(username)),
        PropValue::Href(dav("principal-URL"), principal_href(username)),
        PropValue::HrefSet(
            caldav("calendar-home-set"),
            vec![home_href(Family::Cal, username)],
        ),
        PropValue::HrefSet(
            carddav("addressbook-home-set"),
            vec![home_href(Family::Card, username)],
        ),
        PropValue::HrefSet(
            caldav("calendar-user-address-set"),
            vec![format!("mailto:{username}")],
        ),
        PropValue::Text(caldav("calendar-user-type"), "INDIVIDUAL".to_string()),
        PropValue::SupportedReports(Vec::new()),
    ]
}

pub fn home_props(family: Family, username: &str) -> Vec<PropValue> {
    let display = match family {
        Family::Cal => "Calendars",
        Family::Card => "Address Books",
    };
    vec![
        PropValue::ResourceTypes(vec![ResourceType::Collection]),
        PropValue::Text(dav("displayname"), display.to_string()),
        PropValue::Href(dav("current-user-principal"), principal_href(username)),
        PropValue::Href(dav("owner"), principal_href(username)),
    ]
}

fn supported_reports(family: Family) -> Vec<(Prefix, &'static str)> {
    match family {
        Family::Cal => vec![
            (Prefix::D, "sync-collection"),
            (Prefix::C, "calendar-query"),
            (Prefix::C, "calendar-multiget"),
        ],
        Family::Card => vec![
            (Prefix::D, "sync-collection"),
            (Prefix::B, "addressbook-query"),
            (Prefix::B, "addressbook-multiget"),
        ],
    }
}

pub fn collection_props(
    family: Family,
    username: &str,
    view: &CollectionView,
    state: u64,
) -> Vec<PropValue> {
    let kind = match family {
        Family::Cal => ResourceType::Calendar,
        Family::Card => ResourceType::AddressBook,
    };
    let mut props: Vec<PropValue> = view
        .dead
        .iter()
        .map(|entry| {
            PropValue::Text(
                PropName::new(Namespace::from_uri(&entry.ns), entry.name.clone()),
                entry.value.clone(),
            )
        })
        .collect();
    props.push(PropValue::ResourceTypes(vec![
        ResourceType::Collection,
        kind,
    ]));
    props.push(PropValue::Text(
        dav("displayname"),
        view.display_name.clone(),
    ));
    props.push(PropValue::Text(
        calendar_server("getctag"),
        state.to_string(),
    ));
    props.push(PropValue::Text(
        dav("sync-token"),
        super::sync::sync_token(state),
    ));
    props.push(PropValue::SupportedReports(supported_reports(family)));
    props.push(PropValue::Href(dav("owner"), principal_href(username)));
    props.push(PropValue::Href(
        dav("current-user-principal"),
        principal_href(username),
    ));
    if family == Family::Cal {
        props.push(PropValue::SupportedCalendarComponents(vec![
            "VEVENT".to_string()
        ]));
        props.push(PropValue::Text(
            apple("calendar-order"),
            view.order.to_string(),
        ));
        if let Some(color) = &view.color {
            props.push(PropValue::Text(apple("calendar-color"), color.clone()));
        }
        if let Some(description) = &view.description {
            props.push(PropValue::Text(
                caldav("calendar-description"),
                description.clone(),
            ));
        }
        if let Some(zone) = &view.time_zone {
            props.push(PropValue::Text(caldav("calendar-timezone"), zone.clone()));
        }
    } else if let Some(description) = &view.description {
        props.push(PropValue::Text(
            carddav("addressbook-description"),
            description.clone(),
        ));
    }
    dedupe(props)
}

fn dedupe(props: Vec<PropValue>) -> Vec<PropValue> {
    let mut seen: Vec<PropName> = Vec::new();
    let mut kept = Vec::new();
    for prop in props {
        let name = name_of(&prop);
        if seen.contains(&name) {
            continue;
        }
        seen.push(name);
        kept.push(prop);
    }
    kept
}

pub fn object_props(family: Family, view: &ObjectView) -> Vec<PropValue> {
    let data = match family {
        Family::Cal => PropValue::CalendarData(view.data.clone()),
        Family::Card => PropValue::AddressData(view.data.clone()),
    };
    vec![
        PropValue::Text(dav("getetag"), format!("\"{}\"", view.etag)),
        PropValue::Text(dav("getcontenttype"), family.content_type().to_string()),
        PropValue::Text(dav("getcontentlength"), view.size.to_string()),
        PropValue::ResourceTypes(Vec::new()),
        data,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<PropValue> {
        vec![
            PropValue::Text(dav("displayname"), "Work".to_string()),
            PropValue::ResourceTypes(vec![ResourceType::Collection]),
            PropValue::CalendarData("BEGIN:VCALENDAR\r\n".to_string()),
        ]
    }

    #[test]
    fn a_named_request_splits_known_props_from_unknown_ones() {
        let response = respond(
            "/dav/cal/u/work/".to_string(),
            &PropRequest::Named(vec![dav("displayname"), dav("nope")]),
            sample(),
        );
        assert_eq!(response.propstats.len(), 2);
        assert_eq!(response.propstats[0].status, 200);
        assert_eq!(
            response.propstats[0].props,
            vec![PropValue::Text(dav("displayname"), "Work".to_string())]
        );
        assert_eq!(response.propstats[1].status, 404);
        assert_eq!(
            response.propstats[1].props,
            vec![PropValue::Empty(dav("nope"))]
        );
    }

    #[test]
    fn allprop_omits_calendar_data_and_propname_lists_bare_names() {
        let all = respond("/x".to_string(), &PropRequest::AllProp, sample());
        assert_eq!(all.propstats.len(), 1);
        assert_eq!(all.propstats[0].props.len(), 2);

        let names = respond("/x".to_string(), &PropRequest::PropName, sample());
        assert_eq!(
            names.propstats[0].props,
            vec![
                PropValue::Empty(dav("displayname")),
                PropValue::Empty(dav("resourcetype")),
            ]
        );
    }

    #[test]
    fn an_empty_named_request_falls_back_to_getetag() {
        assert_eq!(
            with_default_etag(&PropRequest::Named(Vec::new())),
            PropRequest::Named(vec![dav("getetag")])
        );
        assert_eq!(
            with_default_etag(&PropRequest::AllProp),
            PropRequest::AllProp
        );
    }

    #[test]
    fn a_dead_property_shadows_the_mapped_property_of_the_same_name() {
        let view = CollectionView {
            id: 1,
            name: "work".to_string(),
            display_name: "Work".to_string(),
            color: None,
            order: 3,
            description: None,
            time_zone: None,
            dead: vec![crate::model::DeadProperty {
                ns: "http://apple.com/ns/ical/".to_string(),
                name: "calendar-order".to_string(),
                value: "junk".to_string(),
            }],
            created: 1,
        };
        let props = collection_props(Family::Cal, "u@x.com", &view, 9);
        let orders: Vec<&PropValue> = props
            .iter()
            .filter(|prop| name_of(prop) == apple("calendar-order"))
            .collect();
        assert_eq!(orders.len(), 1);
        assert_eq!(
            orders[0],
            &PropValue::Text(apple("calendar-order"), "junk".to_string())
        );
    }
}
