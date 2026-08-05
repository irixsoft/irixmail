use crate::error::CompileError;
use crate::limits::CompilerLimits;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tok {
    Identifier(String),
    Tag(String),
    Number(u64),
    Str(String),
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semicolon,
}

#[derive(Debug, Clone)]
pub(crate) struct Token {
    pub tok: Tok,
    pub line: usize,
    pub col: usize,
}

struct Lexer<'a> {
    bytes: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
}

pub(crate) fn tokenize(source: &str, limits: &CompilerLimits) -> Result<Vec<Token>, CompileError> {
    let mut lexer = Lexer {
        bytes: source.as_bytes(),
        pos: 0,
        line: 1,
        col: 1,
    };
    let mut tokens = Vec::new();
    while let Some(token) = lexer.next_token(limits)? {
        tokens.push(token);
    }
    Ok(tokens)
}

impl<'a> Lexer<'a> {
    fn error(&self, line: usize, col: usize, message: impl Into<String>) -> CompileError {
        CompileError {
            line,
            column: col,
            message: message.into(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.pos += 1;
        if byte == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(byte)
    }

    fn next_token(&mut self, limits: &CompilerLimits) -> Result<Option<Token>, CompileError> {
        loop {
            let (line, col) = (self.line, self.col);
            let Some(byte) = self.peek() else {
                return Ok(None);
            };
            match byte {
                b' ' | b'\t' | b'\r' | b'\n' => {
                    self.advance();
                }
                b'#' => {
                    while let Some(byte) = self.peek() {
                        if byte == b'\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                b'/' if self.peek_at(1) == Some(b'*') => {
                    self.advance();
                    self.advance();
                    loop {
                        match self.peek() {
                            Some(b'*') if self.peek_at(1) == Some(b'/') => {
                                self.advance();
                                self.advance();
                                break;
                            }
                            Some(_) => {
                                self.advance();
                            }
                            None => {
                                return Err(self.error(line, col, "unterminated comment"));
                            }
                        }
                    }
                }
                b'"' => {
                    let value = self.quoted_string(limits, line, col)?;
                    return Ok(Some(Token {
                        tok: Tok::Str(value),
                        line,
                        col,
                    }));
                }
                b'0'..=b'9' => {
                    let value = self.number(line, col)?;
                    return Ok(Some(Token {
                        tok: Tok::Number(value),
                        line,
                        col,
                    }));
                }
                b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                    let word = self.word();
                    if word == "text" && self.peek() == Some(b':') {
                        self.advance();
                        let value = self.multiline_string(limits, line, col)?;
                        return Ok(Some(Token {
                            tok: Tok::Str(value),
                            line,
                            col,
                        }));
                    }
                    return Ok(Some(Token {
                        tok: Tok::Identifier(word),
                        line,
                        col,
                    }));
                }
                b':' => {
                    self.advance();
                    match self.peek() {
                        Some(b'a'..=b'z' | b'A'..=b'Z' | b'_') => {
                            let word = self.word();
                            return Ok(Some(Token {
                                tok: Tok::Tag(word),
                                line,
                                col,
                            }));
                        }
                        _ => return Err(self.error(line, col, "expected tag name after ':'")),
                    }
                }
                b'(' | b')' | b'{' | b'}' | b'[' | b']' | b',' | b';' => {
                    self.advance();
                    let tok = match byte {
                        b'(' => Tok::LParen,
                        b')' => Tok::RParen,
                        b'{' => Tok::LBrace,
                        b'}' => Tok::RBrace,
                        b'[' => Tok::LBracket,
                        b']' => Tok::RBracket,
                        b',' => Tok::Comma,
                        _ => Tok::Semicolon,
                    };
                    return Ok(Some(Token { tok, line, col }));
                }
                _ => {
                    return Err(self.error(
                        line,
                        col,
                        format!("unexpected character 0x{byte:02x}"),
                    ));
                }
            }
        }
    }

    fn word(&mut self) -> String {
        let mut word = String::new();
        while let Some(byte) = self.peek() {
            match byte {
                b'a'..=b'z' | b'0'..=b'9' | b'_' => word.push(byte as char),
                b'A'..=b'Z' => word.push(byte.to_ascii_lowercase() as char),
                _ => break,
            }
            self.advance();
        }
        word
    }

    fn quoted_string(
        &mut self,
        limits: &CompilerLimits,
        line: usize,
        col: usize,
    ) -> Result<String, CompileError> {
        self.advance();
        let mut value = Vec::new();
        loop {
            match self.advance() {
                Some(b'"') => break,
                Some(b'\\') => match self.advance() {
                    Some(escaped) => value.push(escaped),
                    None => return Err(self.error(line, col, "unterminated string")),
                },
                Some(byte) => value.push(byte),
                None => return Err(self.error(line, col, "unterminated string")),
            }
            if value.len() > limits.max_string_size {
                return Err(self.error(line, col, "string exceeds the maximum length"));
            }
        }
        String::from_utf8(value).map_err(|_| self.error(line, col, "string is not valid utf-8"))
    }

    fn number(&mut self, line: usize, col: usize) -> Result<u64, CompileError> {
        let mut value: u64 = 0;
        while let Some(byte @ b'0'..=b'9') = self.peek() {
            value = value
                .checked_mul(10)
                .and_then(|v| v.checked_add(u64::from(byte - b'0')))
                .ok_or_else(|| self.error(line, col, "number is too large"))?;
            self.advance();
        }
        let multiplier = match self.peek() {
            Some(b'k' | b'K') => Some(1u64 << 10),
            Some(b'm' | b'M') => Some(1u64 << 20),
            Some(b'g' | b'G') => Some(1u64 << 30),
            _ => None,
        };
        if let Some(multiplier) = multiplier {
            self.advance();
            value = value.saturating_mul(multiplier);
        }
        Ok(value)
    }

    fn multiline_string(
        &mut self,
        limits: &CompilerLimits,
        line: usize,
        col: usize,
    ) -> Result<String, CompileError> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r') => {
                    self.advance();
                }
                Some(b'#') => {
                    while let Some(byte) = self.peek() {
                        if byte == b'\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                Some(b'\n') => {
                    self.advance();
                    break;
                }
                Some(_) => return Err(self.error(line, col, "expected a newline after 'text:'")),
                None => return Err(self.error(line, col, "unterminated multiline string")),
            }
        }
        let mut value = String::new();
        loop {
            let mut current = Vec::new();
            let mut terminated = false;
            while let Some(byte) = self.advance() {
                if byte == b'\n' {
                    terminated = true;
                    break;
                }
                current.push(byte);
            }
            if current.last() == Some(&b'\r') {
                current.pop();
            }
            if !terminated && current.is_empty() {
                return Err(self.error(line, col, "unterminated multiline string"));
            }
            if current == b"." {
                return Ok(value);
            }
            let text = if current.starts_with(b"..") {
                &current[1..]
            } else {
                &current[..]
            };
            let text = std::str::from_utf8(text)
                .map_err(|_| self.error(line, col, "string is not valid utf-8"))?;
            value.push_str(text);
            value.push_str("\r\n");
            if value.len() > limits.max_string_size {
                return Err(self.error(line, col, "string exceeds the maximum length"));
            }
            if !terminated {
                return Err(self.error(line, col, "unterminated multiline string"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(source: &str) -> Vec<Tok> {
        tokenize(source, &CompilerLimits::default())
            .unwrap()
            .into_iter()
            .map(|t| t.tok)
            .collect()
    }

    fn lex_err(source: &str) -> CompileError {
        tokenize(source, &CompilerLimits::default()).unwrap_err()
    }

    #[test]
    fn keywords_are_lowercased_and_punctuation_is_tokenized() {
        assert_eq!(
            lex("IF header :CONTAINS [\"a\"] { stop; }"),
            vec![
                Tok::Identifier("if".into()),
                Tok::Identifier("header".into()),
                Tok::Tag("contains".into()),
                Tok::LBracket,
                Tok::Str("a".into()),
                Tok::RBracket,
                Tok::LBrace,
                Tok::Identifier("stop".into()),
                Tok::Semicolon,
                Tok::RBrace,
            ]
        );
    }

    #[test]
    fn quoted_strings_unescape_backslash_sequences() {
        assert_eq!(lex(r#""a\"b\\c\d""#), vec![Tok::Str(r#"a"b\cd"#.into())]);
    }

    #[test]
    fn quoted_strings_keep_raw_newlines() {
        assert_eq!(lex("\"a\r\nb\""), vec![Tok::Str("a\r\nb".into())]);
    }

    #[test]
    fn hash_comments_run_to_end_of_line() {
        assert_eq!(
            lex("keep; # trailing words \"quote\n stop;"),
            vec![
                Tok::Identifier("keep".into()),
                Tok::Semicolon,
                Tok::Identifier("stop".into()),
                Tok::Semicolon,
            ]
        );
    }

    #[test]
    fn bracket_comments_may_span_lines() {
        assert_eq!(
            lex("keep /* one\ntwo */ ;"),
            vec![Tok::Identifier("keep".into()), Tok::Semicolon]
        );
    }

    #[test]
    fn an_unterminated_bracket_comment_is_an_error() {
        assert_eq!(lex_err("keep; /* open").message, "unterminated comment");
    }

    #[test]
    fn an_unterminated_string_is_an_error() {
        assert_eq!(lex_err("\"open").message, "unterminated string");
    }

    #[test]
    fn numbers_scale_with_kmg_suffixes() {
        assert_eq!(
            lex("1 2k 3M 4G"),
            vec![
                Tok::Number(1),
                Tok::Number(2 << 10),
                Tok::Number(3 << 20),
                Tok::Number(4u64 << 30),
            ]
        );
    }

    #[test]
    fn multiline_text_collects_lines_until_a_lone_dot() {
        assert_eq!(
            lex("text: # note\nline one\nline two\n.\n;"),
            vec![Tok::Str("line one\r\nline two\r\n".into()), Tok::Semicolon]
        );
    }

    #[test]
    fn multiline_text_undoes_dot_stuffing() {
        assert_eq!(
            lex("text:\n..hidden\n.\n"),
            vec![Tok::Str(".hidden\r\n".into())]
        );
    }

    #[test]
    fn multiline_text_without_a_terminator_is_an_error() {
        assert_eq!(
            lex_err("text:\nline one\n").message,
            "unterminated multiline string"
        );
    }

    #[test]
    fn a_bare_colon_is_an_error() {
        assert_eq!(lex_err("keep :;").message, "expected tag name after ':'");
    }

    #[test]
    fn token_positions_track_lines_and_columns() {
        let tokens = tokenize("keep;\n  stop;", &CompilerLimits::default()).unwrap();
        assert_eq!((tokens[0].line, tokens[0].col), (1, 1));
        assert_eq!((tokens[2].line, tokens[2].col), (2, 3));
    }

    #[test]
    fn an_oversized_string_is_rejected() {
        let source = format!("\"{}\"", "a".repeat(5000));
        assert_eq!(
            lex_err(&source).message,
            "string exceeds the maximum length"
        );
    }

    #[test]
    fn unexpected_bytes_are_reported_with_their_value() {
        assert_eq!(lex_err("keep %").message, "unexpected character 0x25");
    }
}
