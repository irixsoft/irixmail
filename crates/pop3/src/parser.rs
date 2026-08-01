#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedCommand {
    pub verb: String,
    pub rest: String,
    pub args: Vec<String>,
}

pub fn parse_command(line: &[u8]) -> ParsedCommand {
    let line = strip_crlf(line);
    let text = String::from_utf8_lossy(line);
    let trimmed = text.trim_start();
    let (verb, rest) = match trimmed.split_once(char::is_whitespace) {
        Some((verb, rest)) => (verb.to_string(), rest.trim_start().to_string()),
        None => (trimmed.to_string(), String::new()),
    };
    let args = rest.split_whitespace().map(str::to_string).collect();
    ParsedCommand { verb, rest, args }
}

fn strip_crlf(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    if end > 0 && line[end - 1] == b'\n' {
        end -= 1;
    }
    if end > 0 && line[end - 1] == b'\r' {
        end -= 1;
    }
    &line[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_verb_has_no_arguments() {
        let command = parse_command(b"STAT\r\n");
        assert_eq!(command.verb, "STAT");
        assert!(command.rest.is_empty());
        assert!(command.args.is_empty());
    }

    #[test]
    fn the_username_is_the_remainder() {
        let command = parse_command(b"USER alice@example.com\r\n");
        assert_eq!(command.verb, "USER");
        assert_eq!(command.rest, "alice@example.com");
        assert_eq!(command.args, vec!["alice@example.com"]);
    }

    #[test]
    fn a_password_keeps_internal_spaces_in_the_remainder() {
        let command = parse_command(b"PASS correct horse battery\r\n");
        assert_eq!(command.verb, "PASS");
        assert_eq!(command.rest, "correct horse battery");
        assert_eq!(command.args.len(), 3);
    }

    #[test]
    fn numeric_arguments_split_apart() {
        let command = parse_command(b"TOP 3 10\r\n");
        assert_eq!(command.verb, "TOP");
        assert_eq!(command.args, vec!["3", "10"]);
    }

    #[test]
    fn an_empty_line_yields_an_empty_verb() {
        let command = parse_command(b"\r\n");
        assert!(command.verb.is_empty());
    }
}
