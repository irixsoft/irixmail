use super::Family;

pub const DEPTH_INFINITY: u8 = u8::MAX;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Root,
    Service(Family),
    Principal,
    Home(Family),
    Collection(Family, String),
    Object(Family, String, String),
}

pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                out.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub fn percent_encode(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.as_bytes() {
        if is_path_safe(*byte) {
            out.push(*byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn is_path_safe(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@".contains(&byte)
}

pub fn segments(path: &str) -> Vec<String> {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .map(percent_decode)
        .collect()
}

pub fn parse_target(path: &str, username: &str) -> Result<Target, u16> {
    let parts = segments(path);
    let mut parts = parts.iter().map(String::as_str);
    if parts.next() != Some("dav") {
        return Err(404);
    }
    let rest: Vec<&str> = parts.collect();
    let owned = |user: &str| -> Result<(), u16> {
        if user.eq_ignore_ascii_case(username) {
            Ok(())
        } else {
            Err(403)
        }
    };
    match rest.as_slice() {
        [] => Ok(Target::Root),
        ["principal"] => Err(404),
        ["principal", user] => owned(user).map(|()| Target::Principal),
        [service] => family_of(service).map(Target::Service).ok_or(404),
        [service, user] => {
            let family = family_of(service).ok_or(404u16)?;
            owned(user).map(|()| Target::Home(family))
        }
        [service, user, collection] => {
            let family = family_of(service).ok_or(404u16)?;
            owned(user).map(|()| Target::Collection(family, (*collection).to_string()))
        }
        [service, user, collection, object] => {
            let family = family_of(service).ok_or(404u16)?;
            owned(user)
                .map(|()| Target::Object(family, (*collection).to_string(), (*object).to_string()))
        }
        _ => Err(404),
    }
}

fn family_of(segment: &str) -> Option<Family> {
    match segment {
        "cal" => Some(Family::Cal),
        "card" => Some(Family::Card),
        _ => None,
    }
}

pub fn strip_origin(destination: &str) -> &str {
    let Some(rest) = destination.split_once("://").map(|(_, rest)| rest) else {
        return destination;
    };
    match rest.find('/') {
        Some(index) => &rest[index..],
        None => "/",
    }
}

pub fn principal_href(username: &str) -> String {
    format!("/dav/principal/{}/", percent_encode(username))
}

pub fn home_href(family: Family, username: &str) -> String {
    format!("/dav/{}/{}/", family.segment(), percent_encode(username))
}

pub fn collection_href(family: Family, username: &str, collection: &str) -> String {
    format!(
        "{}{}/",
        home_href(family, username),
        percent_encode(collection)
    )
}

pub fn object_href(family: Family, username: &str, collection: &str, object: &str) -> String {
    format!(
        "{}{}",
        collection_href(family, username, collection),
        percent_encode(object)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_escapes_decode_and_plain_text_passes_through() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("caf%C3%A9"), "café");
        assert_eq!(percent_decode("already decoded"), "already decoded");
        assert_eq!(percent_decode("a+b"), "a+b");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("50%zz"), "50%zz");
    }

    #[test]
    fn unsafe_segment_characters_are_encoded() {
        assert_eq!(percent_encode("plain-name.ics"), "plain-name.ics");
        assert_eq!(percent_encode("a b"), "a%20b");
        assert_eq!(percent_encode("a#b%c"), "a%23b%25c");
        assert_eq!(percent_encode("<x>"), "%3Cx%3E");
        assert_eq!(percent_encode("café"), "caf%C3%A9");
    }

    #[test]
    fn a_path_splits_into_decoded_non_empty_segments() {
        assert_eq!(segments("/dav/cal/"), vec!["dav", "cal"]);
        assert_eq!(segments("/dav//cal"), vec!["dav", "cal"]);
        assert_eq!(segments("/dav/cal/a%20b/"), vec!["dav", "cal", "a b"]);
        assert!(segments("/").is_empty());
    }

    #[test]
    fn every_url_shape_parses_to_its_target() {
        let user = "saeed@irixsoft.com";
        assert_eq!(parse_target("/dav/", user), Ok(Target::Root));
        assert_eq!(parse_target("/dav", user), Ok(Target::Root));
        assert_eq!(
            parse_target("/dav/cal/", user),
            Ok(Target::Service(Family::Cal))
        );
        assert_eq!(
            parse_target("/dav/card", user),
            Ok(Target::Service(Family::Card))
        );
        assert_eq!(
            parse_target("/dav/principal/saeed@irixsoft.com/", user),
            Ok(Target::Principal)
        );
        assert_eq!(
            parse_target("/dav/cal/SAEED@irixsoft.com/", user),
            Ok(Target::Home(Family::Cal))
        );
        assert_eq!(
            parse_target("/dav/cal/saeed@irixsoft.com/work/", user),
            Ok(Target::Collection(Family::Cal, "work".to_string()))
        );
        assert_eq!(
            parse_target("/dav/card/saeed@irixsoft.com/contacts/one.vcf", user),
            Ok(Target::Object(
                Family::Card,
                "contacts".to_string(),
                "one.vcf".to_string()
            ))
        );
        assert_eq!(
            parse_target("/dav/cal/saeed%40irixsoft.com/work/a%20b.ics", user),
            Ok(Target::Object(
                Family::Cal,
                "work".to_string(),
                "a b.ics".to_string()
            ))
        );
    }

    #[test]
    fn a_foreign_user_is_forbidden_and_junk_is_not_found() {
        let user = "saeed@irixsoft.com";
        assert_eq!(parse_target("/dav/cal/other@irixsoft.com/", user), Err(403));
        assert_eq!(parse_target("/dav/principal/other/", user), Err(403));
        assert_eq!(parse_target("/jmap/", user), Err(404));
        assert_eq!(parse_target("/dav/nope/", user), Err(404));
        assert_eq!(parse_target("/dav/principal/", user), Err(404));
        assert_eq!(
            parse_target("/dav/cal/saeed@irixsoft.com/work/a.ics/extra", user),
            Err(404)
        );
    }

    #[test]
    fn an_absolute_destination_loses_its_origin() {
        assert_eq!(
            strip_origin("https://mail.example.com:443/dav/cal/u/work/a.ics"),
            "/dav/cal/u/work/a.ics"
        );
        assert_eq!(
            strip_origin("/dav/cal/u/work/a.ics"),
            "/dav/cal/u/work/a.ics"
        );
        assert_eq!(strip_origin("http://host/dav/"), "/dav/");
    }

    #[test]
    fn hrefs_are_built_with_encoded_segments_and_trailing_slashes() {
        let user = "saeed@irixsoft.com";
        assert_eq!(principal_href(user), "/dav/principal/saeed@irixsoft.com/");
        assert_eq!(home_href(Family::Cal, user), "/dav/cal/saeed@irixsoft.com/");
        assert_eq!(
            home_href(Family::Card, user),
            "/dav/card/saeed@irixsoft.com/"
        );
        assert_eq!(
            collection_href(Family::Cal, user, "my work"),
            "/dav/cal/saeed@irixsoft.com/my%20work/"
        );
        assert_eq!(
            object_href(Family::Cal, user, "work", "a b.ics"),
            "/dav/cal/saeed@irixsoft.com/work/a%20b.ics"
        );
    }
}
