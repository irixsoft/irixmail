pub const NS_DAV: &str = "DAV:";
pub const NS_CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
pub const NS_CARDDAV: &str = "urn:ietf:params:xml:ns:carddav";
pub const NS_CALENDARSERVER: &str = "http://calendarserver.org/ns/";
pub const NS_APPLE_ICAL: &str = "http://apple.com/ns/ical/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Namespace {
    Dav,
    CalDav,
    CardDav,
    CalendarServer,
    AppleICal,
    Other(String),
}

impl Namespace {
    pub fn uri(&self) -> &str {
        match self {
            Self::Dav => NS_DAV,
            Self::CalDav => NS_CALDAV,
            Self::CardDav => NS_CARDDAV,
            Self::CalendarServer => NS_CALENDARSERVER,
            Self::AppleICal => NS_APPLE_ICAL,
            Self::Other(uri) => uri,
        }
    }

    pub fn from_uri(uri: &str) -> Namespace {
        match uri {
            NS_DAV => Self::Dav,
            NS_CALDAV => Self::CalDav,
            NS_CARDDAV => Self::CardDav,
            NS_CALENDARSERVER => Self::CalendarServer,
            NS_APPLE_ICAL => Self::AppleICal,
            other => Self::Other(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropName {
    pub ns: Namespace,
    pub name: String,
}

impl PropName {
    pub fn new(ns: Namespace, name: impl Into<String>) -> Self {
        Self {
            ns,
            name: name.into(),
        }
    }

    pub fn is(&self, ns: &Namespace, name: &str) -> bool {
        &self.ns == ns && self.name == name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_namespace_uri_maps_back_to_itself() {
        for ns in [
            Namespace::Dav,
            Namespace::CalDav,
            Namespace::CardDav,
            Namespace::CalendarServer,
            Namespace::AppleICal,
        ] {
            assert_eq!(Namespace::from_uri(ns.uri()), ns);
        }
        assert_eq!(Namespace::Dav.uri(), "DAV:");
        assert_eq!(Namespace::CalDav.uri(), "urn:ietf:params:xml:ns:caldav");
        assert_eq!(Namespace::CardDav.uri(), "urn:ietf:params:xml:ns:carddav");
        assert_eq!(
            Namespace::CalendarServer.uri(),
            "http://calendarserver.org/ns/"
        );
        assert_eq!(Namespace::AppleICal.uri(), "http://apple.com/ns/ical/");
    }

    #[test]
    fn an_unknown_namespace_uri_becomes_other() {
        let ns = Namespace::from_uri("http://example.com/ns");
        assert_eq!(ns, Namespace::Other("http://example.com/ns".to_string()));
        assert_eq!(ns.uri(), "http://example.com/ns");
    }

    #[test]
    fn a_prop_name_keeps_its_local_name_verbatim() {
        let name = PropName::new(Namespace::AppleICal, "calendar-color");
        assert_eq!(name.ns, Namespace::AppleICal);
        assert_eq!(name.name, "calendar-color");
    }
}
