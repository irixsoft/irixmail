#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Token {
    Atom(String),
    Str(String),
    Literal(usize),
}

impl Token {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Token::Atom(value) | Token::Str(value) => Some(value),
            Token::Literal(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParseError {
    UnterminatedQuoted,
    BadLiteral,
    LiteralNotLast,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ParseError::UnterminatedQuoted => "unterminated quoted string",
            ParseError::BadLiteral => "malformed literal",
            ParseError::LiteralNotLast => "a literal must end the line",
        })
    }
}

pub(crate) fn tokenize_line(line: &[u8]) -> Result<Vec<Token>, ParseError> {
    let mut tokens = Vec::new();
    let mut pos = 0;
    while pos < line.len() {
        match line[pos] {
            b' ' | b'\t' => pos += 1,
            b'"' => {
                let mut value = Vec::new();
                pos += 1;
                loop {
                    match line.get(pos) {
                        Some(b'"') => {
                            pos += 1;
                            break;
                        }
                        Some(b'\\') => {
                            pos += 1;
                            match line.get(pos) {
                                Some(byte) => {
                                    value.push(*byte);
                                    pos += 1;
                                }
                                None => return Err(ParseError::UnterminatedQuoted),
                            }
                        }
                        Some(byte) => {
                            value.push(*byte);
                            pos += 1;
                        }
                        None => return Err(ParseError::UnterminatedQuoted),
                    }
                }
                tokens.push(Token::Str(String::from_utf8_lossy(&value).into_owned()));
            }
            b'{' => {
                let close = line[pos..]
                    .iter()
                    .position(|b| *b == b'}')
                    .ok_or(ParseError::BadLiteral)?
                    + pos;
                let digits = &line[pos + 1..close];
                let digits = digits.strip_suffix(b"+").unwrap_or(digits);
                if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
                    return Err(ParseError::BadLiteral);
                }
                let length: usize = std::str::from_utf8(digits)
                    .ok()
                    .and_then(|text| text.parse().ok())
                    .ok_or(ParseError::BadLiteral)?;
                if line[close + 1..].iter().any(|b| !b" \t".contains(b)) {
                    return Err(ParseError::LiteralNotLast);
                }
                tokens.push(Token::Literal(length));
                return Ok(tokens);
            }
            _ => {
                let start = pos;
                while pos < line.len() && !b" \t".contains(&line[pos]) {
                    pos += 1;
                }
                tokens.push(Token::Atom(
                    String::from_utf8_lossy(&line[start..pos]).into_owned(),
                ));
            }
        }
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atoms_and_quoted_strings_are_tokenized() {
        assert_eq!(
            tokenize_line(b"PUTSCRIPT \"my script\" trailing").unwrap(),
            vec![
                Token::Atom("PUTSCRIPT".into()),
                Token::Str("my script".into()),
                Token::Atom("trailing".into()),
            ]
        );
    }

    #[test]
    fn quoted_escapes_are_unescaped() {
        assert_eq!(
            tokenize_line(br#""a\"b\\c""#).unwrap(),
            vec![Token::Str(r#"a"b\c"#.into())]
        );
    }

    #[test]
    fn a_trailing_literal_reports_its_length() {
        assert_eq!(
            tokenize_line(b"PUTSCRIPT \"name\" {42+}").unwrap(),
            vec![
                Token::Atom("PUTSCRIPT".into()),
                Token::Str("name".into()),
                Token::Literal(42),
            ]
        );
        assert_eq!(
            tokenize_line(b"GETSCRIPT {3}").unwrap(),
            vec![Token::Atom("GETSCRIPT".into()), Token::Literal(3)]
        );
    }

    #[test]
    fn malformed_literals_are_rejected() {
        assert_eq!(tokenize_line(b"X {}").unwrap_err(), ParseError::BadLiteral);
        assert_eq!(
            tokenize_line(b"X {abc}").unwrap_err(),
            ParseError::BadLiteral
        );
        assert_eq!(tokenize_line(b"X {1").unwrap_err(), ParseError::BadLiteral);
        assert_eq!(
            tokenize_line(b"X {1+} more").unwrap_err(),
            ParseError::LiteralNotLast
        );
    }

    #[test]
    fn an_unterminated_quote_is_rejected() {
        assert_eq!(
            tokenize_line(b"\"open").unwrap_err(),
            ParseError::UnterminatedQuoted
        );
        assert_eq!(
            tokenize_line(b"\"open\\").unwrap_err(),
            ParseError::UnterminatedQuoted
        );
    }

    #[test]
    fn an_empty_line_yields_no_tokens() {
        assert!(tokenize_line(b"").unwrap().is_empty());
        assert!(tokenize_line(b"   ").unwrap().is_empty());
    }
}
