use mail_parser::{ContentType, Message, MessageParser, MessagePart, MimeHeaders, PartType};

use crate::envelope::envelope_body;

pub fn build_bodystructure(raw: &[u8], extended: bool) -> Option<String> {
    let message = MessageParser::default().parse(raw)?;
    Some(part_structure(&message, 0, raw, extended))
}

fn part_structure(message: &Message<'_>, part_id: usize, raw: &[u8], extended: bool) -> String {
    let Some(part) = message.parts.get(part_id) else {
        return "NIL".to_string();
    };
    match &part.body {
        PartType::Multipart(children) => multipart(message, part, children, raw, extended),
        PartType::Message(nested) => message_part(part, nested, raw, extended),
        _ => single(part, raw, extended),
    }
}

fn single(part: &MessagePart<'_>, raw: &[u8], extended: bool) -> String {
    let ct = part.content_type();
    let (media_type, subtype) = media_type(part, ct);
    let body = part_body(part, raw);
    let mut out = format!(
        "({} {} {} {} {} {} {}",
        qstring(&media_type),
        qstring(&subtype),
        param_list(ct),
        nstring(part.content_id().map(bracket).as_deref()),
        nstring(part.content_description()),
        qstring(part.content_transfer_encoding().unwrap_or("7bit")),
        body.len(),
    );
    if media_type.eq_ignore_ascii_case("text") {
        out.push(' ');
        out.push_str(&line_count(body).to_string());
    }
    if extended {
        out.push_str(&extension(part));
    }
    out.push(')');
    out
}

fn message_part(
    part: &MessagePart<'_>,
    nested: &Message<'_>,
    raw: &[u8],
    extended: bool,
) -> String {
    let body = part_body(part, raw);
    let envelope = envelope_body(nested);
    let inner = part_structure(nested, 0, nested.raw_message.as_ref(), extended);
    let mut out = format!(
        "({} {} {} {} {} {} {} {} {}",
        qstring("message"),
        qstring("rfc822"),
        param_list(part.content_type()),
        nstring(part.content_id().map(bracket).as_deref()),
        nstring(part.content_description()),
        qstring(part.content_transfer_encoding().unwrap_or("7bit")),
        body.len(),
        envelope,
        inner,
    );
    out.push(' ');
    out.push_str(&line_count(body).to_string());
    if extended {
        out.push_str(&extension(part));
    }
    out.push(')');
    out
}

fn multipart(
    message: &Message<'_>,
    part: &MessagePart<'_>,
    children: &[u32],
    raw: &[u8],
    extended: bool,
) -> String {
    let mut out = String::from("(");
    for child in children {
        out.push_str(&part_structure(message, *child as usize, raw, extended));
    }
    let subtype = part
        .content_type()
        .and_then(|ct| ct.subtype())
        .unwrap_or("mixed");
    out.push(' ');
    out.push_str(&qstring(subtype));
    if extended {
        out.push(' ');
        out.push_str(&param_list(part.content_type()));
        out.push(' ');
        out.push_str(&disposition(part));
        out.push(' ');
        out.push_str(&language(part));
        out.push(' ');
        out.push_str(&nstring(part.content_location()));
    }
    out.push(')');
    out
}

fn extension(part: &MessagePart<'_>) -> String {
    format!(
        " {} {} {} {}",
        nstring(None),
        disposition(part),
        language(part),
        nstring(part.content_location()),
    )
}

fn media_type(part: &MessagePart<'_>, ct: Option<&ContentType<'_>>) -> (String, String) {
    let default_subtype = match &part.body {
        PartType::Html(_) => "html",
        PartType::Text(_) => "plain",
        _ => "octet-stream",
    };
    let default_type = match &part.body {
        PartType::Text(_) | PartType::Html(_) => "text",
        _ => "application",
    };
    match ct {
        Some(ct) => (
            ct.ctype().to_string(),
            ct.subtype().unwrap_or(default_subtype).to_string(),
        ),
        None => (default_type.to_string(), default_subtype.to_string()),
    }
}

fn disposition(part: &MessagePart<'_>) -> String {
    match part.content_disposition() {
        Some(cd) => format!("({} {})", qstring(cd.ctype()), param_list(Some(cd))),
        None => "NIL".to_string(),
    }
}

fn language(part: &MessagePart<'_>) -> String {
    match part.content_language().as_text_list() {
        Some(list) if !list.is_empty() => {
            if list.len() == 1 {
                qstring(list[0].as_ref())
            } else {
                let joined: Vec<String> = list.iter().map(|lang| qstring(lang.as_ref())).collect();
                format!("({})", joined.join(" "))
            }
        }
        _ => "NIL".to_string(),
    }
}

fn param_list(ct: Option<&ContentType<'_>>) -> String {
    match ct.and_then(|ct| ct.attributes()) {
        Some(attrs) if !attrs.is_empty() => {
            let pairs: Vec<String> = attrs
                .iter()
                .map(|attr| format!("{} {}", qstring(&attr.name), qstring(&attr.value)))
                .collect();
            format!("({})", pairs.join(" "))
        }
        _ => "NIL".to_string(),
    }
}

fn part_body<'a>(part: &MessagePart<'_>, raw: &'a [u8]) -> &'a [u8] {
    raw.get(part.offset_body as usize..part.offset_end as usize)
        .unwrap_or(&[])
}

fn line_count(body: &[u8]) -> usize {
    body.iter().filter(|byte| **byte == b'\n').count()
}

fn bracket(id: &str) -> String {
    let trimmed = id.trim();
    if trimmed.starts_with('<') {
        trimmed.to_string()
    } else {
        format!("<{trimmed}>")
    }
}

fn qstring(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn nstring(value: Option<&str>) -> String {
    match value {
        Some(value) => qstring(value),
        None => "NIL".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_text_message_has_a_single_text_part() {
        let raw = b"Subject: Hi\r\nFrom: a@example.com\r\n\r\nHello body\r\n";
        assert_eq!(
            build_bodystructure(raw, false).unwrap(),
            "(\"text\" \"plain\" NIL NIL NIL \"7bit\" 12 1)"
        );
    }

    #[test]
    fn bodystructure_appends_the_extension_fields() {
        let raw = b"Subject: Hi\r\n\r\nHello body\r\n";
        assert_eq!(
            build_bodystructure(raw, true).unwrap(),
            "(\"text\" \"plain\" NIL NIL NIL \"7bit\" 12 1 NIL NIL NIL NIL)"
        );
    }

    #[test]
    fn a_content_type_carries_subtype_and_parameters() {
        let raw = b"Content-Type: text/html; charset=utf-8\r\n\r\n<p>hi</p>\r\n";
        let structure = build_bodystructure(raw, false).unwrap();
        assert!(
            structure.starts_with("(\"text\" \"html\" (\"charset\" \"utf-8\") NIL NIL \"7bit\""),
            "{structure}"
        );
    }

    #[test]
    fn a_multipart_alternative_nests_each_child_then_the_subtype() {
        let raw = b"Content-Type: multipart/alternative; boundary=BOUND\r\n\r\n--BOUND\r\nContent-Type: text/plain\r\n\r\nplain\r\n--BOUND\r\nContent-Type: text/html\r\n\r\n<p>html</p>\r\n--BOUND--\r\n";
        let structure = build_bodystructure(raw, false).unwrap();
        assert!(structure.starts_with('('), "{structure}");
        assert!(
            structure.contains("(\"text\" \"plain\" NIL NIL NIL \"7bit\""),
            "{structure}"
        );
        assert!(
            structure.contains("(\"text\" \"html\" NIL NIL NIL \"7bit\""),
            "{structure}"
        );
        assert!(structure.ends_with(" \"alternative\")"), "{structure}");
    }

    #[test]
    fn an_extended_multipart_carries_its_parameters_and_extension_fields() {
        let raw = b"Content-Type: multipart/mixed; boundary=B\r\n\r\n--B\r\nContent-Type: text/plain\r\n\r\nbody\r\n--B--\r\n";
        let structure = build_bodystructure(raw, true).unwrap();
        assert!(
            structure.ends_with(" \"mixed\" (\"boundary\" \"B\") NIL NIL NIL)"),
            "{structure}"
        );
    }

    #[test]
    fn an_attachment_disposition_is_rendered() {
        let raw = b"Content-Type: text/plain\r\nContent-Disposition: attachment; filename=note.txt\r\n\r\nhi\r\n";
        let structure = build_bodystructure(raw, true).unwrap();
        assert!(
            structure.contains("NIL (\"attachment\" (\"filename\" \"note.txt\")) NIL NIL)"),
            "{structure}"
        );
    }
}
