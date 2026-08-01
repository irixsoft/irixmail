pub fn from_domain(raw: &[u8]) -> Option<String> {
    let message = mail_auth::AuthenticatedMessage::parse(raw)?;
    message
        .from
        .iter()
        .find_map(|address| match address.rsplit_once('@') {
            Some((_, domain)) if !domain.is_empty() => Some(domain.to_string()),
            _ => None,
        })
}

pub fn complete_headers(raw: &[u8], host: &str, now: u64) -> Vec<u8> {
    let block = header_block(raw);

    let mut prepend = Vec::new();
    if !has_field(block, b"date") {
        let date = crate::dsn::Rfc822Date::from_timestamp(now as i64).to_string();
        prepend.extend_from_slice(b"Date: ");
        prepend.extend_from_slice(date.as_bytes());
        prepend.extend_from_slice(b"\r\n");
    }
    if !has_field(block, b"message-id") {
        prepend.extend_from_slice(b"Message-ID: ");
        prepend.extend_from_slice(message_id(raw, host, now).as_bytes());
        prepend.extend_from_slice(b"\r\n");
    }
    if !has_field(block, b"mime-version") {
        prepend.extend_from_slice(b"MIME-Version: 1.0\r\n");
    }

    if prepend.is_empty() {
        return raw.to_vec();
    }
    let mut out = Vec::with_capacity(prepend.len() + raw.len());
    out.extend_from_slice(&prepend);
    out.extend_from_slice(raw);
    out
}

fn header_block(raw: &[u8]) -> &[u8] {
    match raw.windows(4).position(|w| w == b"\r\n\r\n") {
        Some(end) => &raw[..end + 2],
        None => raw,
    }
}

fn has_field(block: &[u8], name_lower: &[u8]) -> bool {
    for line in block.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.first().is_some_and(|b| *b == b' ' || *b == b'\t') {
            continue;
        }
        if line.len() > name_lower.len()
            && line[name_lower.len()] == b':'
            && line[..name_lower.len()].eq_ignore_ascii_case(name_lower)
        {
            return true;
        }
    }
    false
}

fn message_id(raw: &[u8], host: &str, now: u64) -> String {
    let mut acc = now;
    for byte in raw {
        acc = acc.wrapping_mul(31).wrapping_add(*byte as u64);
    }
    format!("<{now:x}.{acc:016x}@{host}>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_from_domain_is_read_from_an_angle_addressed_header() {
        let raw = b"From: \"Alice Q.\" <Alice@Foo.Example>\r\nTo: b@h\r\n\r\nbody\r\n";
        assert_eq!(from_domain(raw), Some("foo.example".to_string()));
        assert_eq!(from_domain(b"To: b@h\r\n\r\nbody\r\n"), None);
    }

    #[test]
    fn a_message_without_originator_headers_receives_all_three() {
        let raw = b"From: a@irix.example\r\nSubject: hi\r\n\r\nbody\r\n";
        let out = complete_headers(raw, "irix.example", 1_700_000_000);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Date: Tue, 14 Nov 2023 22:13:20 +0000\r\n"));
        assert!(text.contains("Message-ID: <"));
        assert!(text.contains("@irix.example>\r\n"));
        assert!(text.contains("MIME-Version: 1.0\r\n"));
        assert!(text.ends_with("From: a@irix.example\r\nSubject: hi\r\n\r\nbody\r\n"));
    }

    #[test]
    fn present_headers_are_not_duplicated() {
        let raw = b"Date: yesterday\r\nMessage-ID: <x@h>\r\nMIME-Version: 1.0\r\nFrom: a@h\r\n\r\nbody\r\n";
        let out = complete_headers(raw, "h", 0);
        assert_eq!(out, raw);
    }

    #[test]
    fn a_folded_continuation_line_is_not_mistaken_for_a_field() {
        let raw = b"Subject: a very long\r\n Date: not a header\r\nFrom: a@h\r\n\r\nbody\r\n";
        let out = complete_headers(raw, "h", 1_700_000_000);
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("Date: Tue, 14 Nov 2023"));
    }

    #[test]
    fn a_body_occurrence_does_not_count_as_a_header() {
        let raw = b"From: a@h\r\n\r\nMessage-ID: in the body\r\n";
        let out = complete_headers(raw, "h", 0);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Message-ID: <"));
    }
}
