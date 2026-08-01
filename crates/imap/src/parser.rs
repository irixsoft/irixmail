use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub tag: String,
    pub name: String,
    pub args: Vec<Token>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Atom(String),
    Quoted(String),
    List(Vec<Token>),
    Literal { length: u32, sync: bool },
    LiteralValue(Vec<u8>),
    Nil,
}

impl Token {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Token::Atom(value) | Token::Quoted(value) => Some(value),
            Token::LiteralValue(bytes) => std::str::from_utf8(bytes).ok(),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Token]> {
        match self {
            Token::List(items) => Some(items),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    MissingTag,
    MissingCommand,
    UnterminatedQuoted,
    UnterminatedList,
    UnbalancedParen,
    BadLiteral,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Empty => f.write_str("empty command line"),
            ParseError::MissingTag => f.write_str("missing command tag"),
            ParseError::MissingCommand => f.write_str("missing command name"),
            ParseError::UnterminatedQuoted => f.write_str("unterminated quoted string"),
            ParseError::UnterminatedList => f.write_str("unterminated parenthesized list"),
            ParseError::UnbalancedParen => f.write_str("unbalanced parenthesis"),
            ParseError::BadLiteral => f.write_str("malformed literal length"),
        }
    }
}

impl std::error::Error for ParseError {}

pub fn parse_command(line: &[u8]) -> Result<Command, ParseError> {
    let line = strip_crlf(line);
    if line.is_empty() {
        return Err(ParseError::Empty);
    }

    let tag_end = line
        .iter()
        .position(|b| *b == b' ')
        .ok_or(ParseError::MissingCommand)?;
    let tag = &line[..tag_end];
    if tag.is_empty() {
        return Err(ParseError::MissingTag);
    }

    let after_tag = &line[tag_end + 1..];
    let name_start = after_tag
        .iter()
        .position(|b| *b != b' ')
        .ok_or(ParseError::MissingCommand)?;
    let after_tag = &after_tag[name_start..];
    let name_end = after_tag
        .iter()
        .position(|b| *b == b' ')
        .unwrap_or(after_tag.len());
    let name = &after_tag[..name_end];
    if name.is_empty() {
        return Err(ParseError::MissingCommand);
    }

    let args_input = after_tag.get(name_end + 1..).unwrap_or(&[]);
    let mut tokenizer = Tokenizer {
        input: args_input,
        pos: 0,
    };
    let args = tokenizer.tokens()?;

    Ok(Command {
        tag: String::from_utf8_lossy(tag).into_owned(),
        name: String::from_utf8_lossy(name).into_owned(),
        args,
    })
}

pub fn tokenize_args(input: &[u8]) -> Result<Vec<Token>, ParseError> {
    let mut tokenizer = Tokenizer {
        input: strip_crlf(input),
        pos: 0,
    };
    tokenizer.tokens()
}

struct Tokenizer<'a> {
    input: &'a [u8],
    pos: usize,
}

impl Tokenizer<'_> {
    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn skip_spaces(&mut self) {
        while self.peek() == Some(b' ') {
            self.pos += 1;
        }
    }

    fn tokens(&mut self) -> Result<Vec<Token>, ParseError> {
        let mut out = Vec::new();
        loop {
            self.skip_spaces();
            if self.peek().is_none() {
                return Ok(out);
            }
            out.push(self.token()?);
        }
    }

    fn token(&mut self) -> Result<Token, ParseError> {
        match self.peek() {
            Some(b'(') => self.list(),
            Some(b'"') => self.quoted(),
            Some(b'{') => self.literal(),
            Some(b')') => Err(ParseError::UnbalancedParen),
            _ => self.atom(),
        }
    }

    fn list(&mut self) -> Result<Token, ParseError> {
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.skip_spaces();
            match self.peek() {
                None => return Err(ParseError::UnterminatedList),
                Some(b')') => {
                    self.pos += 1;
                    return Ok(Token::List(items));
                }
                Some(_) => items.push(self.token()?),
            }
        }
    }

    fn quoted(&mut self) -> Result<Token, ParseError> {
        self.pos += 1;
        let mut buf = Vec::new();
        while let Some(byte) = self.peek() {
            self.pos += 1;
            match byte {
                b'"' => return Ok(Token::Quoted(String::from_utf8_lossy(&buf).into_owned())),
                b'\\' => {
                    let escaped = self.peek().ok_or(ParseError::UnterminatedQuoted)?;
                    self.pos += 1;
                    buf.push(escaped);
                }
                _ => buf.push(byte),
            }
        }
        Err(ParseError::UnterminatedQuoted)
    }

    fn literal(&mut self) -> Result<Token, ParseError> {
        self.pos += 1;
        let start = self.pos;
        while matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
            self.pos += 1;
        }
        let digits = &self.input[start..self.pos];
        if digits.is_empty() {
            return Err(ParseError::BadLiteral);
        }
        let mut sync = true;
        if self.peek() == Some(b'+') {
            self.pos += 1;
            sync = false;
        }
        if self.peek() != Some(b'}') {
            return Err(ParseError::BadLiteral);
        }
        self.pos += 1;
        if self.pos != self.input.len() {
            return Err(ParseError::BadLiteral);
        }
        let length = std::str::from_utf8(digits)
            .ok()
            .and_then(|text| text.parse().ok())
            .ok_or(ParseError::BadLiteral)?;
        Ok(Token::Literal { length, sync })
    }

    fn atom(&mut self) -> Result<Token, ParseError> {
        let start = self.pos;
        while matches!(self.peek(), Some(b) if is_atom_byte(b)) {
            self.pos += 1;
        }
        let raw = &self.input[start..self.pos];
        if raw.eq_ignore_ascii_case(b"NIL") {
            Ok(Token::Nil)
        } else {
            Ok(Token::Atom(String::from_utf8_lossy(raw).into_owned()))
        }
    }
}

fn is_atom_byte(byte: u8) -> bool {
    !matches!(byte, b' ' | b'\t' | b'(' | b')' | b'"' | b'{')
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
    fn a_tag_command_and_atom_arguments_are_separated() {
        let command = parse_command(b"a1 SELECT INBOX\r\n").unwrap();
        assert_eq!(command.tag, "a1");
        assert_eq!(command.name, "SELECT");
        assert_eq!(command.args, vec![Token::Atom("INBOX".into())]);
    }

    #[test]
    fn a_command_without_arguments_parses() {
        let command = parse_command(b"a NOOP\r\n").unwrap();
        assert_eq!(command.name, "NOOP");
        assert!(command.args.is_empty());
    }

    #[test]
    fn quoted_strings_unescape_backslash_sequences() {
        let command = parse_command(b"a LOGIN \"al\\\"ice\" \"p\\\\w\"\r\n").unwrap();
        assert_eq!(
            command.args,
            vec![
                Token::Quoted("al\"ice".into()),
                Token::Quoted("p\\w".into())
            ]
        );
    }

    #[test]
    fn parenthesized_lists_collect_their_items() {
        let command = parse_command(b"a STORE 1 FLAGS (\\Seen \\Deleted)\r\n").unwrap();
        assert_eq!(command.args[0], Token::Atom("1".into()));
        assert_eq!(command.args[1], Token::Atom("FLAGS".into()));
        assert_eq!(
            command.args[2],
            Token::List(vec![
                Token::Atom("\\Seen".into()),
                Token::Atom("\\Deleted".into())
            ])
        );
    }

    #[test]
    fn lists_nest() {
        let command = parse_command(b"a X (A (B C) D)\r\n").unwrap();
        assert_eq!(
            command.args[0],
            Token::List(vec![
                Token::Atom("A".into()),
                Token::List(vec![Token::Atom("B".into()), Token::Atom("C".into())]),
                Token::Atom("D".into()),
            ])
        );
    }

    #[test]
    fn nil_is_recognized_case_insensitively() {
        let command = parse_command(b"a X nil NIL\r\n").unwrap();
        assert_eq!(command.args, vec![Token::Nil, Token::Nil]);
    }

    #[test]
    fn a_trailing_literal_marker_reports_its_length_and_is_synchronizing() {
        let command = parse_command(b"a APPEND INBOX {310}\r\n").unwrap();
        assert_eq!(command.args[0], Token::Atom("INBOX".into()));
        assert_eq!(
            command.args[1],
            Token::Literal {
                length: 310,
                sync: true
            }
        );
    }

    #[test]
    fn a_non_synchronizing_literal_marker_records_its_plus() {
        let command = parse_command(b"a LOGIN {5+}\r\n").unwrap();
        assert_eq!(
            command.args,
            vec![Token::Literal {
                length: 5,
                sync: false
            }]
        );
    }

    #[test]
    fn tokenize_args_reads_a_trailing_continuation_literal() {
        let tokens = tokenize_args(b" {6}\r\n").unwrap();
        assert_eq!(
            tokens,
            vec![Token::Literal {
                length: 6,
                sync: true
            }]
        );
    }

    #[test]
    fn a_literal_value_reads_back_as_a_string() {
        assert_eq!(
            Token::LiteralValue(b"alice".to_vec()).as_str(),
            Some("alice")
        );
    }

    #[test]
    fn a_sequence_set_stays_one_atom() {
        let command = parse_command(b"a UID FETCH 1:* (FLAGS)\r\n").unwrap();
        assert_eq!(command.name, "UID");
        assert_eq!(command.args[0], Token::Atom("FETCH".into()));
        assert_eq!(command.args[1], Token::Atom("1:*".into()));
    }

    #[test]
    fn an_empty_line_is_rejected() {
        assert_eq!(parse_command(b"\r\n"), Err(ParseError::Empty));
    }

    #[test]
    fn a_leading_space_means_a_missing_tag() {
        assert_eq!(parse_command(b" NOOP\r\n"), Err(ParseError::MissingTag));
    }

    #[test]
    fn a_tag_with_no_command_is_rejected() {
        assert_eq!(parse_command(b"a\r\n"), Err(ParseError::MissingCommand));
        assert_eq!(parse_command(b"a   \r\n"), Err(ParseError::MissingCommand));
    }

    #[test]
    fn an_unterminated_quoted_string_is_rejected() {
        assert_eq!(
            parse_command(b"a LOGIN \"alice\r\n"),
            Err(ParseError::UnterminatedQuoted)
        );
    }

    #[test]
    fn an_unterminated_list_is_rejected() {
        assert_eq!(
            parse_command(b"a STORE 1 FLAGS (\\Seen\r\n"),
            Err(ParseError::UnterminatedList)
        );
    }

    #[test]
    fn a_stray_close_paren_is_rejected() {
        assert_eq!(
            parse_command(b"a X )\r\n"),
            Err(ParseError::UnbalancedParen)
        );
    }

    #[test]
    fn the_accessors_expose_atoms_and_lists() {
        let command = parse_command(b"a X atom (a b)\r\n").unwrap();
        assert_eq!(command.args[0].as_str(), Some("atom"));
        assert_eq!(command.args[1].as_list().unwrap().len(), 2);
        assert_eq!(command.args[0].as_list(), None);
    }
}
