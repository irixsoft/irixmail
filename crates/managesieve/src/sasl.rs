use base64::engine::general_purpose::STANDARD;
use base64::Engine;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mechanism {
    Plain,
    Login,
    Unsupported,
}

impl Mechanism {
    pub fn parse(name: &str) -> Self {
        if name.eq_ignore_ascii_case("PLAIN") {
            Mechanism::Plain
        } else if name.eq_ignore_ascii_case("LOGIN") {
            Mechanism::Login
        } else {
            Mechanism::Unsupported
        }
    }
}

pub(crate) fn decode_base64(input: &str) -> Option<String> {
    let bytes = STANDARD.decode(input.trim()).ok()?;
    String::from_utf8(bytes).ok()
}

pub(crate) fn decode_plain(input: &str) -> Option<(String, String)> {
    let decoded = decode_base64(input)?;
    let mut parts = decoded.splitn(3, '\0');
    let _authzid = parts.next()?;
    let authcid = parts.next()?;
    let password = parts.next()?;
    if authcid.is_empty() || password.contains('\0') {
        return None;
    }
    Some((authcid.to_string(), password.to_string()))
}

pub(crate) fn encode_base64(input: &str) -> String {
    STANDARD.encode(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mechanism_names_parse_case_insensitively() {
        assert_eq!(Mechanism::parse("plain"), Mechanism::Plain);
        assert_eq!(Mechanism::parse("LOGIN"), Mechanism::Login);
        assert_eq!(Mechanism::parse("CRAM-MD5"), Mechanism::Unsupported);
    }

    #[test]
    fn plain_responses_split_into_identity_and_password() {
        let encoded = encode_base64("\0alice@example.com\0secret");
        assert_eq!(
            decode_plain(&encoded),
            Some(("alice@example.com".into(), "secret".into()))
        );
    }

    #[test]
    fn plain_responses_ignore_the_authorization_identity() {
        let encoded = encode_base64("admin\0alice@example.com\0secret");
        assert_eq!(
            decode_plain(&encoded),
            Some(("alice@example.com".into(), "secret".into()))
        );
    }

    #[test]
    fn malformed_plain_responses_are_rejected() {
        assert_eq!(decode_plain("not base64!"), None);
        assert_eq!(decode_plain(&encode_base64("no separators")), None);
        assert_eq!(decode_plain(&encode_base64("\0\0password")), None);
    }
}
