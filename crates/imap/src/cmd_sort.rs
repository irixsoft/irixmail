use crate::parser::Token;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Arrival,
    Date,
    Size,
    From,
    To,
    Cc,
    Subject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortSpec {
    pub key: SortKey,
    pub reverse: bool,
}

impl SortSpec {
    pub fn needs_headers(&self) -> bool {
        matches!(
            self.key,
            SortKey::From | SortKey::To | SortKey::Cc | SortKey::Subject
        )
    }
}

pub fn parse_sort_keys(arg: Option<&Token>) -> Option<Vec<SortSpec>> {
    let Some(Token::List(items)) = arg else {
        return None;
    };
    let mut specs = Vec::new();
    let mut reverse = false;
    for word in items.iter().filter_map(Token::as_str) {
        let key = match word.to_ascii_uppercase().as_str() {
            "REVERSE" => {
                reverse = true;
                continue;
            }
            "ARRIVAL" => SortKey::Arrival,
            "DATE" => SortKey::Date,
            "SIZE" => SortKey::Size,
            "FROM" => SortKey::From,
            "TO" => SortKey::To,
            "CC" => SortKey::Cc,
            "SUBJECT" => SortKey::Subject,
            _ => return None,
        };
        specs.push(SortSpec { key, reverse });
        reverse = false;
    }
    (!specs.is_empty() && !reverse).then_some(specs)
}

pub fn base_subject(subject: &str) -> String {
    let mut current = subject.trim();
    loop {
        let lower = current.to_ascii_lowercase();
        let stripped = ["re:", "fw:", "fwd:"]
            .iter()
            .find(|prefix| lower.starts_with(**prefix))
            .map(|prefix| current[prefix.len()..].trim_start());
        match stripped {
            Some(rest) => current = rest,
            None => break,
        }
    }
    current.trim().to_ascii_lowercase()
}

pub fn header_value(raw_headers: &[u8], name: &str) -> Option<String> {
    let text = String::from_utf8_lossy(raw_headers);
    let mut collecting = false;
    let mut value = String::new();
    for line in text.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            if collecting {
                value.push(' ');
                value.push_str(line.trim());
            }
            continue;
        }
        if collecting {
            return Some(value);
        }
        if let Some((field, rest)) = line.split_once(':') {
            if field.eq_ignore_ascii_case(name) {
                collecting = true;
                value.push_str(rest.trim_start());
            }
        }
    }
    collecting.then_some(value)
}

pub fn sort_response(ids: &[u32]) -> String {
    let mut line = String::from("* SORT");
    for id in ids {
        line.push(' ');
        line.push_str(&id.to_string());
    }
    line.push_str("\r\n");
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(words: &[&str]) -> Token {
        Token::List(
            words
                .iter()
                .map(|word| Token::Atom((*word).into()))
                .collect(),
        )
    }

    #[test]
    fn reverse_binds_to_the_following_key() {
        let specs = parse_sort_keys(Some(&list(&["REVERSE", "DATE", "SUBJECT"]))).unwrap();
        assert_eq!(
            specs,
            vec![
                SortSpec {
                    key: SortKey::Date,
                    reverse: true
                },
                SortSpec {
                    key: SortKey::Subject,
                    reverse: false
                },
            ]
        );
    }

    #[test]
    fn an_unknown_key_or_dangling_reverse_is_rejected() {
        assert_eq!(parse_sort_keys(Some(&list(&["BOGUS"]))), None);
        assert_eq!(parse_sort_keys(Some(&list(&["DATE", "REVERSE"]))), None);
        assert_eq!(parse_sort_keys(Some(&list(&[]))), None);
    }

    #[test]
    fn base_subject_strips_stacked_reply_prefixes() {
        assert_eq!(base_subject("Re: Fwd: RE: Hello"), "hello");
        assert_eq!(base_subject("  plain  "), "plain");
    }

    #[test]
    fn header_value_unfolds_continuation_lines() {
        let raw = b"Subject: one\r\n two\r\nFrom: a@example.com\r\n\r\n";
        assert_eq!(header_value(raw, "subject").as_deref(), Some("one two"));
        assert_eq!(header_value(raw, "from").as_deref(), Some("a@example.com"));
        assert_eq!(header_value(raw, "cc"), None);
    }

    #[test]
    fn the_sort_response_lists_ids_in_order() {
        assert_eq!(sort_response(&[3, 1, 2]), "* SORT 3 1 2\r\n");
        assert_eq!(sort_response(&[]), "* SORT\r\n");
    }
}
