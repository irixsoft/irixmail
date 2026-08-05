use mail_parser::{HeaderValue, Message, MessageParser};

use crate::instruction::AddressPart;

pub(crate) struct MessageView<'a> {
    raw: &'a [u8],
    parsed: Option<Message<'a>>,
}

impl<'a> MessageView<'a> {
    pub fn new(raw: &'a [u8]) -> Self {
        Self {
            raw,
            parsed: MessageParser::default().parse(raw),
        }
    }

    pub fn size(&self) -> u64 {
        self.raw.len() as u64
    }

    pub fn header_exists(&self, name: &str) -> bool {
        !self.headers_named(name).is_empty()
    }

    pub fn header_values(&self, name: &str) -> Vec<String> {
        self.headers_named(name)
            .into_iter()
            .map(|header| match &header.value {
                HeaderValue::Text(text) => text.to_string(),
                HeaderValue::TextList(list) => list.join(", "),
                HeaderValue::Address(address) => address
                    .iter()
                    .map(|addr| match (addr.name(), addr.address()) {
                        (Some(name), Some(email)) => format!("{name} <{email}>"),
                        (None, Some(email)) => email.to_string(),
                        (Some(name), None) => name.to_string(),
                        (None, None) => String::new(),
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                _ => self.raw_value(header.offset_start as usize, header.offset_end as usize),
            })
            .collect()
    }

    pub fn header_addresses(&self, name: &str, part: AddressPart) -> Vec<String> {
        self.headers_named(name)
            .into_iter()
            .flat_map(|header| match &header.value {
                HeaderValue::Address(address) => {
                    let addresses: Vec<String> = address
                        .iter()
                        .map(|addr| {
                            let full = addr.address().or(addr.name()).unwrap_or_default();
                            address_part(full, part)
                        })
                        .collect();
                    if addresses.is_empty() {
                        vec![String::new()]
                    } else {
                        addresses
                    }
                }
                _ => vec![String::new()],
            })
            .collect()
    }

    fn headers_named(&self, name: &str) -> Vec<&mail_parser::Header<'a>> {
        self.parsed
            .iter()
            .flat_map(|message| message.headers())
            .filter(|header| header.name.as_str().eq_ignore_ascii_case(name))
            .collect()
    }

    fn raw_value(&self, start: usize, end: usize) -> String {
        self.raw
            .get(start..end)
            .map(|bytes| String::from_utf8_lossy(bytes).trim().to_string())
            .unwrap_or_default()
    }
}

pub(crate) fn address_part(address: &str, part: AddressPart) -> String {
    match part {
        AddressPart::All => address.to_string(),
        AddressPart::Localpart => address
            .rsplit_once('@')
            .map(|(local, _)| local.to_string())
            .unwrap_or_else(|| address.to_string()),
        AddressPart::Domain => address
            .rsplit_once('@')
            .map(|(_, domain)| domain.to_string())
            .unwrap_or_default(),
    }
}

pub(crate) fn parse_envelope_address(input: &str) -> Option<String> {
    let mut value = input.trim();
    if let Some(stripped) = value.strip_prefix('<') {
        value = stripped.strip_suffix('>')?.trim();
    }
    if value.is_empty() {
        return Some(String::new());
    }
    if value.starts_with('@') {
        value = value.split_once(':').map(|(_, rest)| rest)?;
    }
    if value
        .bytes()
        .any(|b| b.is_ascii_whitespace() || b.is_ascii_control() || !b.is_ascii())
    {
        return None;
    }
    let Some((local, domain)) = value.rsplit_once('@') else {
        return value
            .eq_ignore_ascii_case("mailer-daemon")
            .then(|| value.to_ascii_lowercase());
    };
    if local.is_empty() || domain.is_empty() || local.contains('@') || domain.contains("..") {
        return None;
    }
    Some(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MESSAGE: &[u8] = concat!(
        "From: Weekly News <newsletter@example.com>\r\n",
        "To: me@example.org, Other <other@example.org>\r\n",
        "Subject: =?utf-8?q?Weekly_deals?=\r\n",
        "X-Campaign: summer\r\n",
        "X-Campaign: winter\r\n",
        "\r\n",
        "Body\r\n"
    )
    .as_bytes();

    #[test]
    fn header_values_decode_encoded_words() {
        let view = MessageView::new(MESSAGE);
        assert_eq!(view.header_values("subject"), vec!["Weekly deals"]);
    }

    #[test]
    fn header_values_render_address_headers_with_names() {
        let view = MessageView::new(MESSAGE);
        assert_eq!(
            view.header_values("From"),
            vec!["Weekly News <newsletter@example.com>"]
        );
        assert_eq!(
            view.header_values("to"),
            vec!["me@example.org, Other <other@example.org>"]
        );
    }

    #[test]
    fn header_values_return_every_instance_of_a_repeated_header() {
        let view = MessageView::new(MESSAGE);
        assert_eq!(view.header_values("x-campaign"), vec!["summer", "winter"]);
    }

    #[test]
    fn a_missing_header_yields_no_values_and_does_not_exist() {
        let view = MessageView::new(MESSAGE);
        assert!(view.header_values("x-nonsense").is_empty());
        assert!(!view.header_exists("x-nonsense"));
        assert!(view.header_exists("from"));
    }

    #[test]
    fn header_addresses_extract_each_requested_part() {
        let view = MessageView::new(MESSAGE);
        assert_eq!(
            view.header_addresses("to", AddressPart::All),
            vec!["me@example.org", "other@example.org"]
        );
        assert_eq!(
            view.header_addresses("to", AddressPart::Localpart),
            vec!["me", "other"]
        );
        assert_eq!(
            view.header_addresses("to", AddressPart::Domain),
            vec!["example.org", "example.org"]
        );
    }

    #[test]
    fn header_addresses_on_a_non_address_header_visit_an_empty_string() {
        let view = MessageView::new(MESSAGE);
        assert_eq!(
            view.header_addresses("x-campaign", AddressPart::All),
            vec!["", ""]
        );
    }

    #[test]
    fn an_unparseable_message_has_no_headers_but_keeps_its_size() {
        let view = MessageView::new(b"");
        assert!(view.header_values("from").is_empty());
        assert_eq!(view.size(), 0);
    }

    #[test]
    fn envelope_addresses_are_normalized_at_ingress() {
        assert_eq!(
            parse_envelope_address("<user@example.com>").as_deref(),
            Some("user@example.com")
        );
        assert_eq!(parse_envelope_address("<>").as_deref(), Some(""));
        assert_eq!(parse_envelope_address("").as_deref(), Some(""));
        assert_eq!(
            parse_envelope_address("@relay.example,@other.example:user@example.com").as_deref(),
            Some("user@example.com")
        );
        assert_eq!(
            parse_envelope_address("MAILER-DAEMON").as_deref(),
            Some("mailer-daemon")
        );
    }

    #[test]
    fn junk_envelope_addresses_are_dropped() {
        for junk in [
            "a b@example.com",
            "user@@example.com",
            "user@exa\u{7f}mple.com",
            "usér@example.com",
            "user@bad..example",
            "user@",
            "@example.com",
            "plainuser",
            "<unclosed@example.com",
        ] {
            assert_eq!(parse_envelope_address(junk), None, "{junk}");
        }
    }

    #[test]
    fn address_parts_split_on_the_last_at_sign() {
        assert_eq!(address_part("a@b@c.example", AddressPart::Localpart), "a@b");
        assert_eq!(
            address_part("a@b@c.example", AddressPart::Domain),
            "c.example"
        );
        assert_eq!(
            address_part("no-at-sign", AddressPart::Localpart),
            "no-at-sign"
        );
        assert_eq!(address_part("no-at-sign", AddressPart::Domain), "");
    }
}
