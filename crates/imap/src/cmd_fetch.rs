use std::borrow::Cow;

use irixmail_mail::{ByteRange, MessageCacheEntry, MessagePart, PartBody};

use crate::internaldate::format_internaldate;
use crate::parser::Token;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeqPoint {
    Num(u32),
    Star,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeqRange {
    pub from: SeqPoint,
    pub to: SeqPoint,
}

impl SeqRange {
    pub fn contains(&self, value: u32, largest: u32) -> bool {
        let lo = resolve(self.from, largest);
        let hi = resolve(self.to, largest);
        let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
        value >= lo && value <= hi
    }
}

fn resolve(point: SeqPoint, largest: u32) -> u32 {
    match point {
        SeqPoint::Num(value) => value,
        SeqPoint::Star => largest,
    }
}

pub fn parse_sequence_set(input: &str) -> Option<Vec<SeqRange>> {
    let mut ranges = Vec::new();
    for part in input.split(',') {
        if part.is_empty() {
            return None;
        }
        let range = match part.split_once(':') {
            Some((from, to)) => SeqRange {
                from: parse_point(from)?,
                to: parse_point(to)?,
            },
            None => {
                let point = parse_point(part)?;
                SeqRange {
                    from: point,
                    to: point,
                }
            }
        };
        ranges.push(range);
    }
    (!ranges.is_empty()).then_some(ranges)
}

pub fn sequence_contains(ranges: &[SeqRange], value: u32, largest: u32) -> bool {
    ranges.iter().any(|range| range.contains(value, largest))
}

pub fn compress_sequence(values: &[u32]) -> String {
    let mut sorted: Vec<u32> = values.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut parts: Vec<String> = Vec::new();
    let mut run: Option<(u32, u32)> = None;
    for value in sorted {
        run = match run {
            Some((start, end)) if value == end + 1 => Some((start, value)),
            Some((start, end)) => {
                parts.push(render_run(start, end));
                Some((value, value))
            }
            None => Some((value, value)),
        };
    }
    if let Some((start, end)) = run {
        parts.push(render_run(start, end));
    }
    parts.join(",")
}

fn render_run(start: u32, end: u32) -> String {
    if start == end {
        start.to_string()
    } else {
        format!("{start}:{end}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FetchMods {
    pub changed_since: u64,
    pub vanished: bool,
}

pub fn split_fetch_modifiers(args: &[Token]) -> (&[Token], Option<FetchMods>) {
    let Some(Token::List(inner)) = args.last() else {
        return (args, None);
    };
    let mut words = inner.iter().filter_map(Token::as_str);
    let Some(first) = words.next() else {
        return (args, None);
    };
    if !first.eq_ignore_ascii_case("CHANGEDSINCE") {
        return (args, None);
    }
    let Some(changed_since) = words.next().and_then(|value| value.parse().ok()) else {
        return (args, None);
    };
    let vanished = words.any(|word| word.eq_ignore_ascii_case("VANISHED"));
    (
        &args[..args.len() - 1],
        Some(FetchMods {
            changed_since,
            vanished,
        }),
    )
}

fn parse_point(input: &str) -> Option<SeqPoint> {
    if input == "*" {
        Some(SeqPoint::Star)
    } else {
        input
            .parse::<u32>()
            .ok()
            .filter(|n| *n > 0)
            .map(SeqPoint::Num)
    }
}

pub fn fetch_items(args: &[Token]) -> Vec<String> {
    let tokens: &[Token] = match args {
        [Token::List(items)] => items,
        _ => args,
    };
    let mut raw: Vec<String> = Vec::new();
    for token in tokens {
        let rendered = match token {
            Token::List(items) => {
                let inner: Vec<&str> = items.iter().filter_map(Token::as_str).collect();
                format!("({})", inner.join(" "))
            }
            other => match other.as_str() {
                Some(text) => text.to_string(),
                None => continue,
            },
        };
        // an item like BODY[HEADER.FIELDS (A B)] tokenizes as three tokens; rejoin
        match raw.last_mut() {
            Some(last) if last.contains('[') && !last.contains(']') => {
                if matches!(token, Token::List(_)) {
                    last.push(' ');
                }
                last.push_str(&rendered);
            }
            _ => raw.push(rendered),
        }
    }

    let mut out = Vec::new();
    for item in raw.into_iter().map(|item| item.to_ascii_uppercase()) {
        match item.as_str() {
            "FAST" => out.extend(["FLAGS", "INTERNALDATE", "RFC822.SIZE"].map(String::from)),
            "ALL" => {
                out.extend(["FLAGS", "INTERNALDATE", "RFC822.SIZE", "ENVELOPE"].map(String::from))
            }
            "FULL" => out.extend(
                ["FLAGS", "INTERNALDATE", "RFC822.SIZE", "ENVELOPE", "BODY"].map(String::from),
            ),
            _ => out.push(item),
        }
    }
    out
}

pub struct BodyData<'a> {
    pub full: &'a [u8],
    pub header: &'a [u8],
    pub text: &'a [u8],
    pub parts: &'a [MessagePart],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SectionText {
    Full,
    Header,
    HeaderFields { not: bool, fields: Vec<String> },
    Text,
    Mime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub path: Vec<u32>,
    pub text: SectionText,
}

pub fn parse_section(spec: &str) -> Option<Section> {
    let mut rest = spec.trim();
    let mut path = Vec::new();
    loop {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            break;
        }
        let number: u32 = rest[..digits].parse().ok()?;
        if number == 0 {
            return None;
        }
        path.push(number);
        rest = &rest[digits..];
        match rest.strip_prefix('.') {
            Some(after) => rest = after,
            None => {
                return rest.is_empty().then_some(Section {
                    path,
                    text: SectionText::Full,
                });
            }
        }
    }
    let text = if rest.is_empty() {
        if path.is_empty() {
            SectionText::Full
        } else {
            return None;
        }
    } else if let Some(fields) = rest.strip_prefix("HEADER.FIELDS.NOT ") {
        SectionText::HeaderFields {
            not: true,
            fields: field_list(fields)?,
        }
    } else if let Some(fields) = rest.strip_prefix("HEADER.FIELDS ") {
        SectionText::HeaderFields {
            not: false,
            fields: field_list(fields)?,
        }
    } else if rest == "HEADER" {
        SectionText::Header
    } else if rest == "TEXT" {
        SectionText::Text
    } else if rest == "MIME" {
        if path.is_empty() {
            return None;
        }
        SectionText::Mime
    } else {
        return None;
    };
    Some(Section { path, text })
}

fn field_list(input: &str) -> Option<Vec<String>> {
    let inner = input.trim().strip_prefix('(')?.strip_suffix(')')?;
    Some(inner.split_whitespace().map(str::to_string).collect())
}

fn body_section(base: &str) -> Option<(bool, Section)> {
    let (peek, spec) = if let Some(spec) = base.strip_prefix("BODY.PEEK[") {
        (true, spec)
    } else if let Some(spec) = base.strip_prefix("BODY[") {
        (false, spec)
    } else {
        return None;
    };
    parse_section(spec.strip_suffix(']')?).map(|section| (peek, section))
}

fn section_label(section: &Section) -> String {
    let mut parts: Vec<String> = section.path.iter().map(u32::to_string).collect();
    match &section.text {
        SectionText::Full => {}
        SectionText::Header => parts.push("HEADER".to_string()),
        SectionText::Text => parts.push("TEXT".to_string()),
        SectionText::Mime => parts.push("MIME".to_string()),
        SectionText::HeaderFields { not, fields } => {
            let name = if *not {
                "HEADER.FIELDS.NOT"
            } else {
                "HEADER.FIELDS"
            };
            parts.push(format!("{name} ({})", fields.join(" ")));
        }
    }
    parts.join(".")
}

// RFC 3501: numbering descends multiparts by child position; a message/rfc822
// part is transparent (its embedded message renumbers from its own root), and
// part 1 of a non-multipart message is the message body itself.
fn resolve_part(parts: &[MessagePart], path: &[u32]) -> Option<(usize, u32)> {
    let mut index = 0usize;
    let mut base = 0u32;
    for (step, &number) in path.iter().enumerate() {
        let mut part = parts.get(index)?;
        let mut at_message_root = step == 0;
        if step > 0 {
            if let PartBody::Message(sub) = &part.kind {
                base = base.checked_add(part.body.start)?;
                index = *sub as usize;
                part = parts.get(index)?;
                at_message_root = true;
            }
        }
        match &part.kind {
            PartBody::Multipart(children) => {
                index = *children.get(number.checked_sub(1)? as usize)? as usize;
            }
            _ => {
                if !at_message_root || number != 1 {
                    return None;
                }
            }
        }
    }
    Some((index, base))
}

fn resolve_section<'a>(body: &BodyData<'a>, section: &Section) -> Option<Cow<'a, [u8]>> {
    if section.path.is_empty() {
        return match &section.text {
            SectionText::Full => Some(Cow::Borrowed(body.full)),
            SectionText::Header => Some(Cow::Borrowed(body.header)),
            SectionText::Text => Some(Cow::Borrowed(body.text)),
            SectionText::HeaderFields { not, fields } => {
                Some(Cow::Owned(header_fields(body.header, fields, *not)))
            }
            SectionText::Mime => None,
        };
    }
    let (index, base) = resolve_part(body.parts, &section.path)?;
    let part = body.parts.get(index)?;
    let slice = |range: &ByteRange| -> Option<&'a [u8]> {
        body.full
            .get((base + range.start) as usize..(base + range.end) as usize)
    };
    match &section.text {
        SectionText::Full => slice(&part.body).map(Cow::Borrowed),
        SectionText::Mime => slice(&part.header).map(Cow::Borrowed),
        SectionText::Header | SectionText::Text | SectionText::HeaderFields { .. } => {
            let PartBody::Message(sub) = &part.kind else {
                return None;
            };
            let root = body.parts.get(*sub as usize)?;
            let inner_base = base.checked_add(part.body.start)?;
            let inner = |range: &ByteRange| -> Option<&'a [u8]> {
                body.full
                    .get((inner_base + range.start) as usize..(inner_base + range.end) as usize)
            };
            match &section.text {
                SectionText::Header => inner(&root.header).map(Cow::Borrowed),
                SectionText::Text => inner(&root.body).map(Cow::Borrowed),
                SectionText::HeaderFields { not, fields } => {
                    inner(&root.header).map(|block| Cow::Owned(header_fields(block, fields, *not)))
                }
                _ => None,
            }
        }
    }
}

fn header_fields(header: &[u8], fields: &[String], not: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut include = false;
    for line in header_lines(header) {
        if line.is_empty() || line == b"\r\n" || line == b"\n" {
            break;
        }
        if !matches!(line.first(), Some(b' ' | b'\t')) {
            let name = line.split(|&byte| byte == b':').next().unwrap_or(&[]);
            let name = std::str::from_utf8(name).unwrap_or("").trim();
            let listed = fields.iter().any(|field| field.eq_ignore_ascii_case(name));
            include = listed != not;
        }
        if include {
            out.extend_from_slice(line);
        }
    }
    out.extend_from_slice(b"\r\n");
    out
}

fn header_lines(block: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = block;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let end = rest
            .iter()
            .position(|&byte| byte == b'\n')
            .map(|at| at + 1)
            .unwrap_or(rest.len());
        let (line, tail) = rest.split_at(end);
        rest = tail;
        Some(line)
    })
}

#[derive(Default)]
pub struct FetchExtras<'a> {
    pub body: Option<BodyData<'a>>,
    pub envelope: Option<String>,
    pub structure: Option<String>,
    pub structure_brief: Option<String>,
    pub modseq: Option<u64>,
}

pub fn is_structure_item(item: &str) -> bool {
    matches!(item, "BODY" | "BODYSTRUCTURE")
}

pub fn split_partial(item: &str) -> (&str, Option<(u32, u32)>) {
    let Some(open) = item.rfind('<') else {
        return (item, None);
    };
    let Some(inner) = item[open + 1..].strip_suffix('>') else {
        return (item, None);
    };
    let Some((start, count)) = inner.split_once('.') else {
        return (item, None);
    };
    match (start.parse::<u32>(), count.parse::<u32>()) {
        (Ok(start), Ok(count)) if count > 0 => (&item[..open], Some((start, count))),
        _ => (item, None),
    }
}

pub fn is_seen_setting_item(item: &str) -> bool {
    let base = split_partial(item).0;
    if matches!(base, "RFC822" | "RFC822.TEXT") {
        return true;
    }
    match body_section(base) {
        Some((false, section)) => {
            // a header-only fetch does not imply the client read the message
            !(section.path.is_empty()
                && matches!(
                    section.text,
                    SectionText::Header | SectionText::HeaderFields { .. }
                ))
        }
        _ => false,
    }
}

pub fn is_body_item(item: &str) -> bool {
    let base = split_partial(item).0;
    matches!(base, "RFC822" | "RFC822.HEADER" | "RFC822.TEXT") || body_section(base).is_some()
}

pub fn fetch_line(
    seqno: u32,
    uid: u32,
    entry: &MessageCacheEntry,
    items: &[String],
    uid_mode: bool,
    extras: &FetchExtras<'_>,
) -> Vec<u8> {
    let mut parts: Vec<Vec<u8>> = Vec::new();
    let mut has_uid = false;
    for item in items {
        match item.as_str() {
            "FLAGS" => parts.push(format!("FLAGS ({})", flags_render(entry)).into_bytes()),
            "MODSEQ" => parts.push(format!("MODSEQ ({})", extras.modseq.unwrap_or(1)).into_bytes()),
            "UID" => {
                parts.push(format!("UID {uid}").into_bytes());
                has_uid = true;
            }
            "RFC822.SIZE" => parts.push(format!("RFC822.SIZE {}", entry.size).into_bytes()),
            "INTERNALDATE" => parts.push(
                format!(
                    "INTERNALDATE \"{}\"",
                    format_internaldate(entry.received_at)
                )
                .into_bytes(),
            ),
            "ENVELOPE" => {
                if let Some(envelope) = &extras.envelope {
                    parts.push(envelope.clone().into_bytes());
                }
            }
            "BODYSTRUCTURE" => {
                if let Some(structure) = &extras.structure {
                    parts.push(format!("BODYSTRUCTURE {structure}").into_bytes());
                }
            }
            "BODY" => {
                if let Some(structure) = &extras.structure_brief {
                    parts.push(format!("BODY {structure}").into_bytes());
                }
            }
            other => {
                if let Some(rendered) = extras.body.as_ref().and_then(|body| body_item(other, body))
                {
                    parts.push(rendered);
                }
            }
        }
    }
    if uid_mode && !has_uid {
        parts.insert(0, format!("UID {uid}").into_bytes());
    }
    let mut line = format!("* {seqno} FETCH (").into_bytes();
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            line.push(b' ');
        }
        line.extend_from_slice(part);
    }
    line.extend_from_slice(b")\r\n");
    line
}

fn body_item(item: &str, body: &BodyData<'_>) -> Option<Vec<u8>> {
    let (base, partial) = split_partial(item);
    let (label, data): (String, Cow<'_, [u8]>) = match base {
        "RFC822" => ("RFC822".to_string(), Cow::Borrowed(body.full)),
        "RFC822.HEADER" => ("RFC822.HEADER".to_string(), Cow::Borrowed(body.header)),
        "RFC822.TEXT" => ("RFC822.TEXT".to_string(), Cow::Borrowed(body.text)),
        _ => {
            let (_, section) = body_section(base)?;
            let data = resolve_section(body, &section)?;
            (format!("BODY[{}]", section_label(&section)), data)
        }
    };
    // RFC 3501: the response labels only the origin octet, never the count.
    let (label, data) = match partial {
        Some((start, count)) => {
            let lo = start as usize;
            let hi = lo.saturating_add(count as usize).min(data.len());
            (
                format!("{label}<{start}>"),
                data.get(lo..hi).unwrap_or_default(),
            )
        }
        None => (label, &data[..]),
    };
    let mut out = format!("{label} {{{}}}\r\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    Some(out)
}

fn flags_render(entry: &MessageCacheEntry) -> String {
    entry
        .keywords
        .iter()
        .map(|keyword| keyword.to_imap())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_sequence_folds_runs_into_ranges() {
        assert_eq!(compress_sequence(&[1, 3, 4, 5, 7]), "1,3:5,7");
        assert_eq!(compress_sequence(&[2]), "2");
        assert_eq!(compress_sequence(&[5, 1, 2, 2, 3]), "1:3,5");
        assert_eq!(compress_sequence(&[]), "");
    }

    #[test]
    fn fetch_modifiers_split_off_a_trailing_changedsince_list() {
        let args = vec![
            Token::List(vec![Token::Atom("FLAGS".into())]),
            Token::List(vec![
                Token::Atom("CHANGEDSINCE".into()),
                Token::Atom("42".into()),
                Token::Atom("VANISHED".into()),
            ]),
        ];
        let (items, mods) = split_fetch_modifiers(&args);
        assert_eq!(items.len(), 1);
        let mods = mods.unwrap();
        assert_eq!(mods.changed_since, 42);
        assert!(mods.vanished);

        let plain = vec![Token::List(vec![Token::Atom("FLAGS".into())])];
        let (items, mods) = split_fetch_modifiers(&plain);
        assert_eq!(items.len(), 1);
        assert!(mods.is_none());
    }

    #[test]
    fn a_single_number_parses() {
        assert_eq!(
            parse_sequence_set("4"),
            Some(vec![SeqRange {
                from: SeqPoint::Num(4),
                to: SeqPoint::Num(4)
            }])
        );
    }

    #[test]
    fn ranges_and_lists_parse() {
        let set = parse_sequence_set("1:5,7,9:*").unwrap();
        assert_eq!(set.len(), 3);
        assert_eq!(set[2].to, SeqPoint::Star);
    }

    #[test]
    fn zero_and_empty_parts_are_rejected() {
        assert_eq!(parse_sequence_set("0"), None);
        assert_eq!(parse_sequence_set("1,,2"), None);
        assert_eq!(parse_sequence_set(""), None);
        assert_eq!(parse_sequence_set("x"), None);
    }

    #[test]
    fn membership_respects_ranges_and_the_star() {
        let set = parse_sequence_set("2:4,8").unwrap();
        assert!(sequence_contains(&set, 3, 10));
        assert!(sequence_contains(&set, 8, 10));
        assert!(!sequence_contains(&set, 5, 10));

        let tail = parse_sequence_set("5:*").unwrap();
        assert!(sequence_contains(&tail, 9, 9));
        assert!(!sequence_contains(&tail, 4, 9));
    }

    #[test]
    fn a_reversed_range_still_matches() {
        let set = parse_sequence_set("5:2").unwrap();
        assert!(sequence_contains(&set, 3, 10));
    }

    #[test]
    fn the_full_macro_expands() {
        let args = [Token::Atom("FULL".into())];
        assert_eq!(
            fetch_items(&args),
            vec!["FLAGS", "INTERNALDATE", "RFC822.SIZE", "ENVELOPE", "BODY"]
        );
    }

    #[test]
    fn an_item_list_is_uppercased() {
        let args = [Token::List(vec![
            Token::Atom("uid".into()),
            Token::Atom("flags".into()),
        ])];
        assert_eq!(fetch_items(&args), vec!["UID", "FLAGS"]);
    }

    #[test]
    fn a_bracketed_item_split_across_tokens_is_rejoined() {
        let args = [Token::List(vec![
            Token::Atom("BODY.PEEK[HEADER.FIELDS".into()),
            Token::List(vec![
                Token::Atom("Subject".into()),
                Token::Atom("From".into()),
            ]),
            Token::Atom("]".into()),
            Token::Atom("UID".into()),
        ])];
        assert_eq!(
            fetch_items(&args),
            vec!["BODY.PEEK[HEADER.FIELDS (SUBJECT FROM)]", "UID"]
        );
    }

    #[test]
    fn a_bare_bracketed_item_outside_a_list_is_rejoined() {
        let args = [
            Token::Atom("BODY[HEADER.FIELDS".into()),
            Token::List(vec![Token::Atom("Date".into())]),
            Token::Atom("]<0.100>".into()),
        ];
        assert_eq!(
            fetch_items(&args),
            vec!["BODY[HEADER.FIELDS (DATE)]<0.100>"]
        );
    }

    #[test]
    fn section_specs_parse_into_paths_and_texts() {
        assert_eq!(
            parse_section(""),
            Some(Section {
                path: vec![],
                text: SectionText::Full
            })
        );
        assert_eq!(
            parse_section("1.2"),
            Some(Section {
                path: vec![1, 2],
                text: SectionText::Full
            })
        );
        assert_eq!(
            parse_section("2.MIME"),
            Some(Section {
                path: vec![2],
                text: SectionText::Mime
            })
        );
        assert_eq!(
            parse_section("HEADER.FIELDS (SUBJECT FROM)"),
            Some(Section {
                path: vec![],
                text: SectionText::HeaderFields {
                    not: false,
                    fields: vec!["SUBJECT".into(), "FROM".into()]
                }
            })
        );
        assert_eq!(
            parse_section("3.HEADER.FIELDS.NOT (X-A)"),
            Some(Section {
                path: vec![3],
                text: SectionText::HeaderFields {
                    not: true,
                    fields: vec!["X-A".into()]
                }
            })
        );
        assert_eq!(parse_section("MIME"), None);
        assert_eq!(parse_section("0"), None);
        assert_eq!(parse_section("1."), None);
        assert_eq!(parse_section("BOGUS"), None);
    }

    #[test]
    fn header_fields_keeps_matching_lines_with_continuations_and_the_blank_line() {
        let header = b"Subject: a very\r\n long subject\r\nFrom: a@b\r\nTo: c@d\r\n\r\n";
        let picked = header_fields(header, &["SUBJECT".to_string(), "TO".to_string()], false);
        assert_eq!(
            picked,
            b"Subject: a very\r\n long subject\r\nTo: c@d\r\n\r\n"
        );
        let excluded = header_fields(header, &["subject".to_string()], true);
        assert_eq!(excluded, b"From: a@b\r\nTo: c@d\r\n\r\n");
        let none = header_fields(header, &["MISSING".to_string()], false);
        assert_eq!(none, b"\r\n");
    }

    fn leaf(kind: PartBody, header: (u32, u32), body: (u32, u32)) -> MessagePart {
        MessagePart {
            headers: Vec::new(),
            header: ByteRange::new(header.0, header.1),
            body: ByteRange::new(body.0, body.1),
            encoding: irixmail_mail::PartEncoding::None,
            kind,
        }
    }

    #[test]
    fn numeric_sections_resolve_against_a_multipart_tree() {
        // raw: 0..10 root header, 10..20 part1 mime, 20..30 part1 body,
        //      30..40 part2 mime, 40..50 part2 body
        let raw: Vec<u8> = (0..50u8).collect();
        let parts = vec![
            leaf(PartBody::Multipart(vec![1, 2]), (0, 10), (10, 50)),
            leaf(PartBody::Text, (10, 20), (20, 30)),
            leaf(PartBody::Html, (30, 40), (40, 50)),
        ];
        let body = BodyData {
            full: &raw,
            header: &raw[0..10],
            text: &raw[10..50],
            parts: &parts,
        };

        let one = Section {
            path: vec![1],
            text: SectionText::Full,
        };
        assert_eq!(resolve_section(&body, &one).unwrap().as_ref(), &raw[20..30]);
        let two_mime = Section {
            path: vec![2],
            text: SectionText::Mime,
        };
        assert_eq!(
            resolve_section(&body, &two_mime).unwrap().as_ref(),
            &raw[30..40]
        );
        let missing = Section {
            path: vec![3],
            text: SectionText::Full,
        };
        assert!(resolve_section(&body, &missing).is_none());
    }

    #[test]
    fn part_one_of_a_plain_message_is_its_body() {
        let raw = b"Header: x\r\n\r\nplain body";
        let parts = vec![leaf(PartBody::Text, (0, 13), (13, 23))];
        let body = BodyData {
            full: raw,
            header: &raw[0..13],
            text: &raw[13..23],
            parts: &parts,
        };
        let one = Section {
            path: vec![1],
            text: SectionText::Full,
        };
        assert_eq!(
            resolve_section(&body, &one).unwrap().as_ref(),
            b"plain body"
        );
        let deep = Section {
            path: vec![1, 1],
            text: SectionText::Full,
        };
        assert!(resolve_section(&body, &deep).is_none());
    }

    #[test]
    fn sections_of_an_embedded_message_rebase_onto_the_outer_raw() {
        // outer: 0..10 header; body 10..40 is the embedded message
        // embedded (offsets relative to its own start): header 0..12, body 12..30
        let raw: Vec<u8> = (0..40u8).collect();
        let parts = vec![
            leaf(PartBody::Message(1), (0, 10), (10, 40)),
            leaf(PartBody::Text, (0, 12), (12, 30)),
        ];
        let body = BodyData {
            full: &raw,
            header: &raw[0..10],
            text: &raw[10..40],
            parts: &parts,
        };

        let whole = Section {
            path: vec![1],
            text: SectionText::Full,
        };
        assert_eq!(
            resolve_section(&body, &whole).unwrap().as_ref(),
            &raw[10..40]
        );
        let header = Section {
            path: vec![1],
            text: SectionText::Header,
        };
        assert_eq!(
            resolve_section(&body, &header).unwrap().as_ref(),
            &raw[10..22]
        );
        let text = Section {
            path: vec![1],
            text: SectionText::Text,
        };
        assert_eq!(
            resolve_section(&body, &text).unwrap().as_ref(),
            &raw[22..40]
        );
    }

    #[test]
    fn seen_setting_covers_sections_but_not_header_fetches() {
        assert!(is_seen_setting_item("BODY[1]"));
        assert!(is_seen_setting_item("BODY[2.MIME]"));
        assert!(!is_seen_setting_item("BODY[HEADER.FIELDS (SUBJECT)]"));
        assert!(!is_seen_setting_item("BODY[HEADER]"));
        assert!(!is_seen_setting_item("BODY.PEEK[1]"));
        assert!(is_body_item("BODY[HEADER.FIELDS (SUBJECT)]"));
        assert!(is_body_item("BODY.PEEK[1.2.MIME]<0.10>"));
        assert!(!is_body_item("BODY[NONSENSE]"));
    }

    fn entry(uid: u32, keywords: Vec<irixmail_mail::Keyword>, size: u32) -> MessageCacheEntry {
        MessageCacheEntry {
            document_id: 1,
            mailboxes: vec![irixmail_mail::MailboxUid { mailbox_id: 1, uid }],
            keywords,
            thread_id: 0,
            size,
            received_at: 482_374_938,
            sent_at: 0,
        }
    }

    #[test]
    fn a_fetch_line_renders_supported_items_and_skips_the_rest() {
        use irixmail_mail::Keyword;
        let entry = entry(5, vec![Keyword::Seen, Keyword::Flagged], 42);
        let items = vec!["FLAGS".into(), "ENVELOPE".into(), "RFC822.SIZE".into()];
        assert_eq!(
            fetch_line(3, 5, &entry, &items, false, &FetchExtras::default()),
            b"* 3 FETCH (FLAGS (\\Seen \\Flagged) RFC822.SIZE 42)\r\n".to_vec()
        );
    }

    #[test]
    fn the_internaldate_item_renders_the_received_timestamp_quoted() {
        let entry = entry(1, Vec::new(), 0);
        let items = vec!["INTERNALDATE".into()];
        assert_eq!(
            fetch_line(1, 1, &entry, &items, false, &FetchExtras::default()),
            b"* 1 FETCH (INTERNALDATE \"15-Apr-1985 01:02:18 +0000\")\r\n".to_vec()
        );
    }

    #[test]
    fn a_uid_fetch_line_prepends_the_uid_when_not_requested() {
        let entry = entry(9, Vec::new(), 10);
        let items = vec!["FLAGS".into()];
        assert_eq!(
            fetch_line(2, 9, &entry, &items, true, &FetchExtras::default()),
            b"* 2 FETCH (UID 9 FLAGS ())\r\n".to_vec()
        );
    }

    #[test]
    fn body_items_render_as_literals_from_the_body_data() {
        let entry = entry(1, Vec::new(), 3);
        let extras = FetchExtras {
            body: Some(BodyData {
                full: b"RAW",
                header: b"H",
                text: b"T",
                parts: &[],
            }),
            ..FetchExtras::default()
        };
        let items = vec!["BODY[]".into(), "BODY[TEXT]".into()];
        assert_eq!(
            fetch_line(1, 1, &entry, &items, false, &extras),
            b"* 1 FETCH (BODY[] {3}\r\nRAW BODY[TEXT] {1}\r\nT)\r\n".to_vec()
        );
    }

    #[test]
    fn a_partial_body_item_renders_the_requested_slice() {
        let entry = entry(1, Vec::new(), 10);
        let extras = FetchExtras {
            body: Some(BodyData {
                full: b"0123456789",
                header: b"",
                text: b"56789",
                parts: &[],
            }),
            ..FetchExtras::default()
        };
        let items = vec!["BODY.PEEK[]<2.3>".to_string()];
        assert_eq!(
            fetch_line(1, 1, &entry, &items, false, &extras),
            b"* 1 FETCH (BODY[]<2> {3}\r\n234)\r\n".to_vec()
        );
    }

    #[test]
    fn a_partial_range_past_the_end_renders_an_empty_literal() {
        let entry = entry(1, Vec::new(), 10);
        let extras = FetchExtras {
            body: Some(BodyData {
                full: b"0123456789",
                header: b"",
                text: b"56789",
                parts: &[],
            }),
            ..FetchExtras::default()
        };
        let items = vec!["BODY.PEEK[TEXT]<100.5>".to_string()];
        assert_eq!(
            fetch_line(1, 1, &entry, &items, false, &extras),
            b"* 1 FETCH (BODY[TEXT]<100> {0}\r\n)\r\n".to_vec()
        );
    }

    #[test]
    fn partial_suffixes_are_recognized_on_body_and_seen_items() {
        assert!(is_body_item("BODY.PEEK[TEXT]<0.4>"));
        assert!(is_seen_setting_item("BODY[]<0.4>"));
        assert!(!is_seen_setting_item("BODY.PEEK[]<0.4>"));
    }

    #[test]
    fn the_envelope_item_renders_the_prepared_string() {
        let entry = entry(1, Vec::new(), 0);
        let extras = FetchExtras {
            envelope: Some("ENVELOPE (NIL \"Hi\" NIL NIL NIL NIL NIL NIL NIL NIL)".to_string()),
            ..FetchExtras::default()
        };
        let items = vec!["ENVELOPE".into()];
        assert_eq!(
            fetch_line(4, 1, &entry, &items, false, &extras),
            b"* 4 FETCH (ENVELOPE (NIL \"Hi\" NIL NIL NIL NIL NIL NIL NIL NIL))\r\n".to_vec()
        );
    }
}
