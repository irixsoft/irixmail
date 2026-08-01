#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sanitized {
    pub html: String,
    pub blocked_remote_content: bool,
}

pub fn sanitize_html(input: &str, allow_remote_content: bool) -> Sanitized {
    let mut out = String::with_capacity(input.len());
    let mut blocked_remote_content = false;
    let mut suppress_depth: usize = 0;

    for token in tokenize(input) {
        match token {
            Token::Text(text) => {
                if suppress_depth == 0 {
                    escape_text_into(text, &mut out);
                }
            }
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } => {
                if suppress_depth > 0 {
                    if drops_content(&name) {
                        suppress_depth += 1;
                    }
                    continue;
                }
                if drops_content(&name) {
                    suppress_depth = 1;
                    continue;
                }
                let Some(allowed_attrs) = allowed_tag_attributes(&name) else {
                    continue;
                };
                out.push('<');
                out.push_str(&name);
                for (attr, value) in attributes {
                    if let Some(rendered) = sanitize_attribute(
                        &name,
                        &attr,
                        value.as_deref(),
                        allowed_attrs,
                        allow_remote_content,
                        &mut blocked_remote_content,
                    ) {
                        out.push(' ');
                        out.push_str(&rendered);
                    }
                }
                if self_closing || is_void(&name) {
                    out.push_str(" />");
                } else {
                    out.push('>');
                }
            }
            Token::EndTag { name } => {
                if suppress_depth > 0 {
                    if drops_content(&name) {
                        suppress_depth -= 1;
                    }
                    continue;
                }
                if drops_content(&name) || is_void(&name) {
                    continue;
                }
                if allowed_tag_attributes(&name).is_some() {
                    out.push_str("</");
                    out.push_str(&name);
                    out.push('>');
                }
            }
        }
    }

    Sanitized {
        html: out,
        blocked_remote_content,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token<'a> {
    Text(&'a str),
    StartTag {
        name: String,
        attributes: Vec<(String, Option<String>)>,
        self_closing: bool,
    },
    EndTag {
        name: String,
    },
}

fn tokenize(input: &str) -> Vec<Token<'_>> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    let len = bytes.len();
    let mut text_start = 0;

    while i < len {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }

        let next = bytes.get(i + 1).copied();
        let starts_tag = matches!(next, Some(c) if c.is_ascii_alphabetic())
            || matches!(next, Some(b'/' | b'!' | b'?'));
        if !starts_tag {
            i += 1;
            continue;
        }

        if text_start < i {
            tokens.push(Token::Text(&input[text_start..i]));
        }

        if next == Some(b'!') {
            if bytes.get(i + 2) == Some(&b'-') && bytes.get(i + 3) == Some(&b'-') {
                i = skip_until(bytes, i + 4, b"-->");
            } else {
                i = skip_until(bytes, i + 2, b">");
            }
            text_start = i;
            continue;
        }
        if next == Some(b'?') {
            i = skip_until(bytes, i + 2, b">");
            text_start = i;
            continue;
        }

        if let Some((token, end)) = parse_tag(input, i) {
            tokens.push(token);
            i = end;
            text_start = i;
        } else {
            text_start = len;
            break;
        }
    }

    if text_start < len {
        tokens.push(Token::Text(&input[text_start..len]));
    }

    tokens
}

fn skip_until(bytes: &[u8], from: usize, needle: &[u8]) -> usize {
    let mut i = from;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            return i + needle.len();
        }
        i += 1;
    }
    bytes.len()
}

fn parse_tag(input: &str, start: usize) -> Option<(Token<'_>, usize)> {
    let bytes = input.as_bytes();
    let mut i = start + 1;
    let is_end = bytes.get(i) == Some(&b'/');
    if is_end {
        i += 1;
    }

    let name_start = i;
    while i < bytes.len()
        && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b':' || bytes[i] == b'-')
    {
        i += 1;
    }
    let name = input[name_start..i].to_ascii_lowercase();
    if name.is_empty() {
        let end = skip_until(bytes, i, b">");
        return Some((Token::Text(""), end));
    }

    if is_end {
        let end = skip_until(bytes, i, b">");
        return Some((Token::EndTag { name }, end));
    }

    let mut attributes: Vec<(String, Option<String>)> = Vec::new();
    let mut self_closing = false;

    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        match bytes.get(i) {
            None => return None,
            Some(b'>') => {
                i += 1;
                break;
            }
            Some(b'/') => {
                self_closing = true;
                i += 1;
                continue;
            }
            _ => {}
        }

        let attr_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'>'
            && bytes[i] != b'/'
        {
            i += 1;
        }
        let attr_name = input[attr_start..i].to_ascii_lowercase();

        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }

        let value = if bytes.get(i) == Some(&b'=') {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            Some(read_attr_value(input, &mut i))
        } else {
            None
        };

        if !attr_name.is_empty() {
            attributes.push((attr_name, value));
        }
    }

    Some((
        Token::StartTag {
            name,
            attributes,
            self_closing,
        },
        i,
    ))
}

fn read_attr_value(input: &str, i: &mut usize) -> String {
    let bytes = input.as_bytes();
    match bytes.get(*i) {
        Some(&q @ (b'"' | b'\'')) => {
            *i += 1;
            let start = *i;
            while *i < bytes.len() && bytes[*i] != q {
                *i += 1;
            }
            let value = input[start..*i].to_string();
            if *i < bytes.len() {
                *i += 1; // Consume the closing quote.
            }
            value
        }
        _ => {
            let start = *i;
            while *i < bytes.len() && !bytes[*i].is_ascii_whitespace() && bytes[*i] != b'>' {
                *i += 1;
            }
            input[start..*i].to_string()
        }
    }
}

fn sanitize_attribute(
    tag: &str,
    attr: &str,
    value: Option<&str>,
    allowed_attrs: &[&str],
    allow_remote_content: bool,
    blocked_remote_content: &mut bool,
) -> Option<String> {
    if attr.starts_with("on") {
        return None;
    }
    if !allowed_attrs.contains(&attr) {
        return None;
    }

    let value = value.unwrap_or("");

    if is_url_attribute(tag, attr) {
        let scheme = url_scheme(value);
        match scheme {
            UrlScheme::Cid | UrlScheme::Data => {}
            UrlScheme::Safe => {
                if is_fetching_attribute(attr) && !allow_remote_content {
                    *blocked_remote_content = true;
                    return Some(format!("{}=\"\"", attr));
                }
            }
            UrlScheme::Relative => {}
            UrlScheme::Unsafe => {
                return None;
            }
        }
    }

    Some(format!("{}=\"{}\"", attr, escape_attribute(value)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UrlScheme {
    Cid,
    Data,
    Safe,
    Relative,
    Unsafe,
}

fn url_scheme(value: &str) -> UrlScheme {
    let mut scheme = String::new();
    for ch in value.chars() {
        match ch {
            c if c.is_whitespace() || c.is_control() => continue,
            ':' => break,
            '/' | '?' | '#' => return UrlScheme::Relative,
            c => scheme.push(c.to_ascii_lowercase()),
        }
    }
    if scheme.is_empty() {
        return UrlScheme::Relative;
    }
    match scheme.as_str() {
        "cid" => UrlScheme::Cid,
        "data" => UrlScheme::Data,
        "http" | "https" | "mailto" | "ftp" | "ftps" | "tel" => UrlScheme::Safe,
        _ => UrlScheme::Unsafe,
    }
}

fn is_url_attribute(_tag: &str, attr: &str) -> bool {
    matches!(
        attr,
        "href" | "src" | "poster" | "background" | "cite" | "longdesc" | "action"
    )
}

fn is_fetching_attribute(attr: &str) -> bool {
    matches!(attr, "src" | "poster" | "background")
}

fn drops_content(name: &str) -> bool {
    matches!(
        name,
        "script" | "style" | "iframe" | "object" | "embed" | "applet" | "noscript" | "template"
    )
}

fn is_void(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "source"
            | "wbr"
    )
}

fn allowed_tag_attributes(name: &str) -> Option<&'static [&'static str]> {
    const COMMON: &[&str] = &["class", "id", "title", "dir", "lang", "align"];
    let extra: &[&str] = match name {
        "a" => &["href", "name", "target", "rel"],
        "img" => &["src", "alt", "width", "height"],
        "td" | "th" => &[
            "colspan",
            "rowspan",
            "scope",
            "headers",
            "valign",
            "width",
            "background",
        ],
        "table" => &["border", "cellpadding", "cellspacing", "width", "summary"],
        "col" | "colgroup" => &["span", "width"],
        "ol" => &["start", "type", "reversed"],
        "li" => &["value"],
        "blockquote" | "q" => &["cite"],
        "p" | "div" | "span" | "br" | "hr" | "pre" | "code" | "kbd" | "samp" | "var" | "b"
        | "i" | "u" | "s" | "strong" | "em" | "small" | "sub" | "sup" | "mark" | "del" | "ins"
        | "abbr" | "cite" | "dfn" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "dl"
        | "dt" | "dd" | "figure" | "figcaption" | "caption" | "thead" | "tbody" | "tfoot"
        | "tr" | "address" | "article" | "section" | "header" | "footer" | "main" | "nav"
        | "aside" | "wbr" => &[],
        _ => return None,
    };
    Some(merge_attrs(name, COMMON, extra))
}

fn merge_attrs(
    name: &str,
    common: &[&'static str],
    extra: &[&'static str],
) -> &'static [&'static str] {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static TABLE: OnceLock<Mutex<HashMap<String, &'static [&'static str]>>> = OnceLock::new();
    let table = TABLE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = table.lock().expect("attribute table mutex poisoned");
    if let Some(slice) = guard.get(name) {
        return slice;
    }
    let mut combined: Vec<&'static str> = Vec::with_capacity(common.len() + extra.len());
    combined.extend_from_slice(common);
    combined.extend_from_slice(extra);
    let leaked: &'static [&'static str] = Box::leak(combined.into_boxed_slice());
    guard.insert(name.to_string(), leaked);
    leaked
}

fn escape_text_into(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            c => out.push(c),
        }
    }
}

fn escape_attribute(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sanitize(input: &str) -> String {
        sanitize_html(input, false).html
    }

    #[test]
    fn plain_text_passes_through_with_entities_escaped() {
        let result = sanitize("a < b & c > d");
        assert_eq!(result, "a &lt; b &amp; c &gt; d");
    }

    #[test]
    fn allowed_formatting_tags_are_kept() {
        let result = sanitize("<p>Hello <strong>world</strong></p>");
        assert_eq!(result, "<p>Hello <strong>world</strong></p>");
    }

    #[test]
    fn script_tags_and_their_content_are_removed() {
        let result = sanitize("<p>before</p><script>alert('x')</script><p>after</p>");
        assert_eq!(result, "<p>before</p><p>after</p>");
    }

    #[test]
    fn style_tags_and_their_content_are_removed() {
        let result = sanitize("<style>body{display:none}</style><p>visible</p>");
        assert_eq!(result, "<p>visible</p>");
    }

    #[test]
    fn iframes_are_removed_with_their_fallback_content() {
        let result = sanitize("<iframe src=\"https://evil.example\">fallback</iframe>text");
        assert_eq!(result, "text");
    }

    #[test]
    fn disallowed_tags_are_unwrapped_but_their_text_is_kept() {
        let result = sanitize("<marquee>scrolling</marquee>");
        assert_eq!(result, "scrolling");
    }

    #[test]
    fn event_handler_attributes_are_stripped() {
        let result = sanitize("<a href=\"https://x.example\" onclick=\"steal()\">link</a>");
        assert_eq!(result, "<a href=\"https://x.example\">link</a>");
    }

    #[test]
    fn javascript_urls_are_dropped_from_href() {
        let result = sanitize("<a href=\"javascript:alert(1)\">x</a>");
        assert_eq!(result, "<a>x</a>");
    }

    #[test]
    fn javascript_urls_with_embedded_whitespace_are_dropped() {
        let result = sanitize("<a href=\"java\tscript:alert(1)\">x</a>");
        assert_eq!(result, "<a>x</a>");
    }

    #[test]
    fn attributes_not_on_the_allowlist_are_dropped() {
        let result = sanitize("<p style=\"color:red\" data-x=\"y\">hi</p>");
        assert_eq!(result, "<p>hi</p>");
    }

    #[test]
    fn remote_images_are_neutralised_by_default_and_reported() {
        let result = sanitize_html("<img src=\"https://tracker.example/p.gif\">", false);
        assert_eq!(result.html, "<img src=\"\" />");
        assert!(result.blocked_remote_content);
    }

    #[test]
    fn remote_images_are_kept_when_remote_content_is_allowed() {
        let result = sanitize_html("<img src=\"https://cdn.example/logo.png\">", true);
        assert_eq!(result.html, "<img src=\"https://cdn.example/logo.png\" />");
        assert!(!result.blocked_remote_content);
    }

    #[test]
    fn inline_cid_images_are_always_kept() {
        let result = sanitize_html("<img src=\"cid:part1@host\" alt=\"logo\">", false);
        assert_eq!(result.html, "<img src=\"cid:part1@host\" alt=\"logo\" />");
        assert!(!result.blocked_remote_content);
    }

    #[test]
    fn data_uri_images_are_always_kept() {
        let result = sanitize_html("<img src=\"data:image/png;base64,AAAA\">", false);
        assert_eq!(result.html, "<img src=\"data:image/png;base64,AAAA\" />");
        assert!(!result.blocked_remote_content);
    }

    #[test]
    fn remote_links_are_not_treated_as_remote_content() {
        let result = sanitize_html("<a href=\"https://site.example\">go</a>", false);
        assert_eq!(result.html, "<a href=\"https://site.example\">go</a>");
        assert!(!result.blocked_remote_content);
    }

    #[test]
    fn attribute_values_are_escaped_to_prevent_breakout() {
        let result = sanitize_html(
            "<a href='https://x.example/?q=\"onmouseover=alert(1)'>x</a>",
            true,
        )
        .html;
        assert_eq!(
            result,
            "<a href=\"https://x.example/?q=&quot;onmouseover=alert(1)\">x</a>"
        );
    }

    #[test]
    fn comments_are_removed() {
        let result = sanitize("<p>a</p><!-- secret <script>x</script> --><p>b</p>");
        assert_eq!(result, "<p>a</p><p>b</p>");
    }

    #[test]
    fn unbalanced_and_stray_angle_brackets_are_escaped_as_text() {
        let result = sanitize("5 < 10 and 10 > 5");
        assert_eq!(result, "5 &lt; 10 and 10 &gt; 5");
    }

    #[test]
    fn void_elements_self_close() {
        let result = sanitize("line1<br>line2<hr>");
        assert_eq!(result, "line1<br />line2<hr />");
    }

    #[test]
    fn unknown_attributes_on_images_are_stripped_but_size_is_kept() {
        let result = sanitize_html(
            "<img src=\"cid:x\" width=\"100\" height=\"50\" onerror=\"x()\" srcset=\"y\">",
            false,
        );
        assert_eq!(
            result.html,
            "<img src=\"cid:x\" width=\"100\" height=\"50\" />"
        );
    }

    #[test]
    fn nested_dangerous_elements_are_fully_suppressed() {
        let result = sanitize("<noscript><img src=\"https://t.example\"><b>x</b></noscript>after");
        assert_eq!(result, "after");
    }

    #[test]
    fn tables_and_their_attributes_survive() {
        let input = "<table border=\"1\"><tr><td colspan=\"2\">cell</td></tr></table>";
        let result = sanitize(input);
        assert_eq!(
            result,
            "<table border=\"1\"><tr><td colspan=\"2\">cell</td></tr></table>"
        );
    }

    #[test]
    fn empty_input_yields_empty_output() {
        let result = sanitize_html("", false);
        assert_eq!(result.html, "");
        assert!(!result.blocked_remote_content);
    }

    #[test]
    fn background_attribute_is_gated_like_src() {
        let result = sanitize_html("<td background=\"https://t.example/bg.png\">x</td>", false);
        assert_eq!(result.html, "<td background=\"\">x</td>");
        assert!(result.blocked_remote_content);
    }
}
