use mail_parser::{Addr, Address, Message, MessageParser};

pub fn build_envelope(raw: &[u8]) -> Option<String> {
    let message = MessageParser::default().parse(raw)?;
    Some(envelope_of(&message))
}

fn envelope_of(message: &Message<'_>) -> String {
    format!("ENVELOPE {}", envelope_body(message))
}

pub(crate) fn envelope_body(message: &Message<'_>) -> String {
    let date = nstring(message.date().map(|date| date.to_rfc822()).as_deref());
    let subject = nstring(message.subject());
    let from = address_list(message.from());
    let sender = match message.sender() {
        Some(_) => address_list(message.sender()),
        None => from.clone(),
    };
    let reply_to = match message.reply_to() {
        Some(_) => address_list(message.reply_to()),
        None => from.clone(),
    };
    let to = address_list(message.to());
    let cc = address_list(message.cc());
    let bcc = address_list(message.bcc());
    let in_reply_to = nstring(message.in_reply_to().as_text().map(bracket_id).as_deref());
    let message_id = nstring(message.message_id().map(bracket_id).as_deref());
    format!(
        "({date} {subject} {from} {sender} {reply_to} {to} {cc} {bcc} {in_reply_to} {message_id})"
    )
}

fn address_list(address: Option<&Address<'_>>) -> String {
    let Some(address) = address else {
        return "NIL".to_string();
    };
    let entries: String = address.iter().map(format_address).collect();
    if entries.is_empty() {
        "NIL".to_string()
    } else {
        format!("({entries})")
    }
}

fn format_address(addr: &Addr<'_>) -> String {
    let name = nstring(addr.name());
    let (mailbox, host) = match addr.address() {
        Some(value) => match value.rsplit_once('@') {
            Some((mailbox, host)) => (nstring(Some(mailbox)), nstring(Some(host))),
            None => (nstring(Some(value)), "NIL".to_string()),
        },
        None => ("NIL".to_string(), "NIL".to_string()),
    };
    format!("({name} NIL {mailbox} {host})")
}

fn bracket_id(id: &str) -> String {
    let trimmed = id.trim();
    if trimmed.starts_with('<') {
        trimmed.to_string()
    } else {
        format!("<{trimmed}>")
    }
}

fn nstring(value: Option<&str>) -> String {
    match value {
        Some(value) => format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")),
        None => "NIL".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_message_builds_a_full_envelope() {
        let raw = b"From: Alice <alice@example.com>\r\nTo: Bob <bob@example.org>\r\nSubject: Hello\r\nMessage-ID: <abc@example.com>\r\n\r\nBody\r\n";
        let envelope = build_envelope(raw).unwrap();
        assert_eq!(
            envelope,
            "ENVELOPE (NIL \"Hello\" \
             ((\"Alice\" NIL \"alice\" \"example.com\")) \
             ((\"Alice\" NIL \"alice\" \"example.com\")) \
             ((\"Alice\" NIL \"alice\" \"example.com\")) \
             ((\"Bob\" NIL \"bob\" \"example.org\")) \
             NIL NIL NIL \"<abc@example.com>\")"
        );
    }

    #[test]
    fn a_message_without_addresses_or_subject_is_all_nil() {
        let raw = b"X-Other: marker\r\n\r\nbody\r\n";
        let envelope = build_envelope(raw).unwrap();
        assert_eq!(
            envelope,
            "ENVELOPE (NIL NIL NIL NIL NIL NIL NIL NIL NIL NIL)"
        );
    }

    #[test]
    fn the_date_header_is_carried_as_a_quoted_string() {
        let raw = b"From: a@b.com\r\nDate: Mon, 7 Feb 1994 21:52:25 -0800\r\n\r\nx\r\n";
        let envelope = build_envelope(raw).unwrap();
        assert!(envelope.starts_with("ENVELOPE (\""), "{envelope}");
        assert!(!envelope.starts_with("ENVELOPE (NIL"), "{envelope}");
        assert!(envelope.contains("(NIL NIL \"a\" \"b.com\")"), "{envelope}");
    }

    #[test]
    fn a_quote_in_a_header_value_is_escaped() {
        let raw = b"Subject: a \"quoted\" word\r\n\r\nx\r\n";
        let envelope = build_envelope(raw).unwrap();
        assert!(envelope.contains("\"a \\\"quoted\\\" word\""), "{envelope}");
    }
}
