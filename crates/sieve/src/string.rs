pub(crate) fn decode_encoded_characters(input: &str) -> Result<String, String> {
    let mut output = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        if bytes[pos] == b'$' && bytes.get(pos + 1) == Some(&b'{') {
            if let Some((decoded, end)) = decode_sequence(input, pos)? {
                output.push_str(&decoded);
                pos = end;
                continue;
            }
        }
        let ch = input[pos..].chars().next().expect("in-bounds char");
        output.push(ch);
        pos += ch.len_utf8();
    }
    Ok(output)
}

fn decode_sequence(input: &str, start: usize) -> Result<Option<(String, usize)>, String> {
    let Some(close) = input[start..].find('}').map(|i| start + i) else {
        return Ok(None);
    };
    let body = &input[start + 2..close];
    let Some((kind, digits)) = body.split_once(':') else {
        return Ok(None);
    };
    let digits = digits.trim();
    match kind.trim().to_ascii_lowercase().as_str() {
        "hex" => {
            let mut decoded = Vec::new();
            for pair in digits.split_ascii_whitespace() {
                if pair.is_empty() || pair.len() > 2 || !pair.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Ok(None);
                }
                decoded.push(u8::from_str_radix(pair, 16).expect("validated hex"));
            }
            if decoded.is_empty() {
                return Ok(None);
            }
            match String::from_utf8(decoded) {
                Ok(text) => Ok(Some((text, close + 1))),
                Err(_) => Ok(None),
            }
        }
        "unicode" => {
            let mut decoded = String::new();
            let mut seen = false;
            for group in digits.split_ascii_whitespace() {
                if group.is_empty()
                    || group.len() > 6
                    || !group.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Ok(None);
                }
                seen = true;
                let value = u32::from_str_radix(group, 16).expect("validated hex");
                let ch = char::from_u32(value)
                    .ok_or_else(|| format!("invalid unicode code point {group}"))?;
                decoded.push(ch);
            }
            if !seen {
                return Ok(None);
            }
            Ok(Some((decoded, close + 1)))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_sequences_decode_to_their_octets() {
        assert_eq!(
            decode_encoded_characters("${hex:40 21}").unwrap(),
            "@!".to_string()
        );
    }

    #[test]
    fn unicode_sequences_decode_to_code_points() {
        assert_eq!(
            decode_encoded_characters("${unicode:00F6}x").unwrap(),
            "\u{f6}x".to_string()
        );
    }

    #[test]
    fn decoded_output_is_not_rescanned() {
        assert_eq!(
            decode_encoded_characters("${hex:24 7b}hex:40}").unwrap(),
            "${hex:40}".to_string()
        );
    }

    #[test]
    fn malformed_sequences_are_kept_verbatim() {
        for input in [
            "${hex:4Q}",
            "${hex:}",
            "${weird:40}",
            "${hex 40}",
            "${",
            "$",
        ] {
            assert_eq!(decode_encoded_characters(input).unwrap(), input.to_string());
        }
    }

    #[test]
    fn an_out_of_range_code_point_is_an_error() {
        assert!(decode_encoded_characters("${unicode:D800}").is_err());
        assert!(decode_encoded_characters("${unicode:110000}").is_err());
    }

    #[test]
    fn text_without_sequences_passes_through() {
        assert_eq!(
            decode_encoded_characters("plain $ {text}").unwrap(),
            "plain $ {text}".to_string()
        );
    }
}
