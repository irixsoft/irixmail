const BLOCK_TAGS: [&str; 8] = ["p", "div", "li", "tr", "h1", "h2", "h3", "blockquote"];

const DROPPED_TAGS: [&str; 2] = ["script", "style"];

pub fn text_from_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('>') else {
            out.push_str(after);
            rest = "";
            break;
        };
        let tag = &after[..close];
        let name = tag_name(tag);
        rest = &after[close + 1..];
        if DROPPED_TAGS.contains(&name.as_str()) && !tag.starts_with('/') {
            rest = skip_to_close(rest, &name);
            continue;
        }
        if name == "br" {
            out.push('\n');
        } else if BLOCK_TAGS.contains(&name.as_str()) && tag.starts_with('/') {
            out.push_str("\n\n");
        }
    }
    out.push_str(rest);
    collapse(&decode(&out))
}

fn tag_name(tag: &str) -> String {
    tag.trim_start_matches('/')
        .split(|c: char| c.is_whitespace() || c == '/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn skip_to_close<'a>(rest: &'a str, name: &str) -> &'a str {
    let needle = format!("</{name}");
    match rest.to_ascii_lowercase().find(&needle) {
        Some(at) => match rest[at..].find('>') {
            Some(end) => &rest[at + end + 1..],
            None => "",
        },
        None => "",
    }
}

fn decode(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

fn collapse(text: &str) -> String {
    let mut blocks: Vec<String> = Vec::new();
    for block in text.split("\n\n") {
        let cleaned: Vec<&str> = block
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if !cleaned.is_empty() {
            blocks.push(cleaned.join("\n"));
        }
    }
    blocks.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_stripped_and_the_words_survive() {
        assert_eq!(text_from_html("<p>Hello <b>world</b></p>"), "Hello world");
    }

    #[test]
    fn block_boundaries_become_newlines() {
        assert_eq!(text_from_html("<p>a</p><p>b</p>"), "a\n\nb");
    }

    #[test]
    fn a_line_break_becomes_a_newline() {
        assert_eq!(text_from_html("a<br>b"), "a\nb");
    }

    #[test]
    fn entities_are_decoded() {
        assert_eq!(
            text_from_html("<p>tom &amp; jerry &lt;here&gt;&nbsp;now</p>"),
            "tom & jerry <here> now"
        );
    }

    #[test]
    fn script_and_style_content_is_dropped() {
        assert_eq!(
            text_from_html("<style>p{color:red}</style><p>visible</p><script>alert(1)</script>"),
            "visible"
        );
    }
}
