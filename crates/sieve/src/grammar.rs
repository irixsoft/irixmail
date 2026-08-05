use crate::error::CompileError;
use crate::instruction::{AddressPart, Comparator, EnvelopePart, Instruction, MatchType, Test};
use crate::lexer::{Tok, Token};
use crate::limits::CompilerLimits;
use crate::string::decode_encoded_characters;

#[derive(Debug)]
pub(crate) struct Program {
    pub instructions: Vec<Instruction>,
    pub capabilities: Vec<String>,
}

pub(crate) fn compile(
    tokens: Vec<Token>,
    limits: &CompilerLimits,
) -> Result<Program, CompileError> {
    Parser {
        tokens,
        pos: 0,
        limits,
        instructions: Vec::new(),
        capabilities: Vec::new(),
        commands_started: false,
        blocks: Vec::new(),
    }
    .run()
}

struct OpenBlock {
    jz_at: Option<usize>,
    chain_jmps: Vec<usize>,
}

struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    limits: &'a CompilerLimits,
    instructions: Vec<Instruction>,
    capabilities: Vec<String>,
    commands_started: bool,
    blocks: Vec<OpenBlock>,
}

impl Parser<'_> {
    fn run(mut self) -> Result<Program, CompileError> {
        while let Some(token) = self.advance() {
            match token.tok.clone() {
                Tok::Identifier(word) => self.command(&word, &token)?,
                Tok::RBrace => self.close_block(&token)?,
                _ => return Err(err_at(&token, "expected a command")),
            }
        }
        if !self.blocks.is_empty() {
            return Err(self.err("unclosed block at end of script"));
        }
        Ok(Program {
            instructions: self.instructions,
            capabilities: self.capabilities,
        })
    }

    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos).map(|t| &t.tok)
    }

    fn advance(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned()?;
        self.pos += 1;
        Some(token)
    }

    fn err(&self, message: impl Into<String>) -> CompileError {
        let (line, column) = self
            .tokens
            .get(self.pos.min(self.tokens.len().saturating_sub(1)))
            .map(|t| (t.line, t.col))
            .unwrap_or((1, 1));
        CompileError {
            line,
            column,
            message: message.into(),
        }
    }

    fn expect(&mut self, expected: &Tok, description: &str) -> Result<(), CompileError> {
        match self.advance() {
            Some(token) if &token.tok == expected => Ok(()),
            Some(token) => Err(err_at(&token, format!("expected {description}"))),
            None => Err(self.err(format!("expected {description}"))),
        }
    }

    fn has_capability(&self, name: &str) -> bool {
        self.capabilities.iter().any(|c| c == name)
    }

    fn need_capability(&self, name: &str, token: &Token) -> Result<(), CompileError> {
        if self.has_capability(name) {
            Ok(())
        } else {
            Err(err_at(token, format!("\"{name}\" must be required first")))
        }
    }

    fn patch(&mut self, index: usize, target: usize) {
        match &mut self.instructions[index] {
            Instruction::Jmp(t) | Instruction::Jz(t) | Instruction::Jnz(t) => *t = target,
            _ => unreachable!("only jumps are patched"),
        }
    }

    fn command(&mut self, word: &str, token: &Token) -> Result<(), CompileError> {
        match word {
            "require" => {
                if self.commands_started || !self.blocks.is_empty() {
                    return Err(err_at(token, "require must precede all other commands"));
                }
                for name in self.parse_raw_string_list()? {
                    match name.as_str() {
                        "fileinto"
                        | "envelope"
                        | "imap4flags"
                        | "encoded-character"
                        | "comparator-i;octet"
                        | "comparator-i;ascii-casemap" => {
                            if !self.has_capability(&name) {
                                self.capabilities.push(name);
                            }
                        }
                        _ => {
                            return Err(err_at(token, format!("unknown capability \"{name}\"")));
                        }
                    }
                }
                self.expect(&Tok::Semicolon, "';'")
            }
            "if" => {
                self.commands_started = true;
                if self.blocks.len() >= self.limits.max_nested_blocks {
                    return Err(err_at(token, "blocks are nested too deeply"));
                }
                self.parse_test_expr(false, 0)?;
                self.instructions.push(Instruction::Jz(usize::MAX));
                let jz_at = self.instructions.len() - 1;
                self.blocks.push(OpenBlock {
                    jz_at: Some(jz_at),
                    chain_jmps: Vec::new(),
                });
                self.expect(&Tok::LBrace, "'{'")
            }
            "elsif" | "else" => Err(err_at(token, format!("{word} without a preceding if"))),
            "stop" => self.simple_command(Instruction::Stop),
            "keep" => self.simple_command(Instruction::Keep),
            "discard" => self.simple_command(Instruction::Discard),
            "fileinto" => {
                self.commands_started = true;
                self.need_capability("fileinto", token)?;
                let mailbox = self.parse_string()?;
                self.expect(&Tok::Semicolon, "';'")?;
                self.instructions.push(Instruction::FileInto(mailbox));
                Ok(())
            }
            "redirect" => {
                self.commands_started = true;
                let address = self.parse_string()?;
                self.expect(&Tok::Semicolon, "';'")?;
                self.instructions.push(Instruction::Redirect(address));
                Ok(())
            }
            "addflag" | "setflag" | "removeflag" => {
                self.commands_started = true;
                self.need_capability("imap4flags", token)?;
                let flags: Vec<String> = self
                    .parse_string_list()?
                    .iter()
                    .flat_map(|entry| entry.split_ascii_whitespace().map(str::to_string))
                    .collect();
                self.expect(&Tok::Semicolon, "';'")?;
                self.instructions.push(match word {
                    "addflag" => Instruction::AddFlag(flags),
                    "setflag" => Instruction::SetFlag(flags),
                    _ => Instruction::RemoveFlag(flags),
                });
                Ok(())
            }
            _ => Err(err_at(token, format!("unknown command \"{word}\""))),
        }
    }

    fn simple_command(&mut self, instruction: Instruction) -> Result<(), CompileError> {
        self.commands_started = true;
        self.expect(&Tok::Semicolon, "';'")?;
        self.instructions.push(instruction);
        Ok(())
    }

    fn close_block(&mut self, token: &Token) -> Result<(), CompileError> {
        let Some(mut block) = self.blocks.pop() else {
            return Err(err_at(token, "'}' without an open block"));
        };
        let continuation = match self.peek() {
            Some(Tok::Identifier(word)) if word == "elsif" => Some(true),
            Some(Tok::Identifier(word)) if word == "else" => Some(false),
            _ => None,
        };
        match continuation {
            Some(is_elsif) => {
                let Some(jz_at) = block.jz_at else {
                    return Err(self.err("else block cannot be followed by another branch"));
                };
                self.advance();
                self.instructions.push(Instruction::Jmp(usize::MAX));
                block.chain_jmps.push(self.instructions.len() - 1);
                let target = self.instructions.len();
                self.patch(jz_at, target);
                if is_elsif {
                    self.parse_test_expr(false, 0)?;
                    self.instructions.push(Instruction::Jz(usize::MAX));
                    block.jz_at = Some(self.instructions.len() - 1);
                } else {
                    block.jz_at = None;
                }
                self.blocks.push(block);
                self.expect(&Tok::LBrace, "'{'")
            }
            None => {
                let target = self.instructions.len();
                if let Some(jz_at) = block.jz_at {
                    self.patch(jz_at, target);
                }
                for jmp in block.chain_jmps {
                    self.patch(jmp, target);
                }
                Ok(())
            }
        }
    }

    fn parse_test_expr(&mut self, negate: bool, depth: usize) -> Result<(), CompileError> {
        if depth >= self.limits.max_nested_tests {
            return Err(self.err("tests are nested too deeply"));
        }
        let Some(token) = self.advance() else {
            return Err(self.err("expected a test"));
        };
        let Tok::Identifier(word) = token.tok.clone() else {
            return Err(err_at(&token, "expected a test"));
        };
        match word.as_str() {
            "not" => self.parse_test_expr(!negate, depth + 1),
            "allof" | "anyof" => {
                let is_all = (word == "allof") != negate;
                self.expect(&Tok::LParen, "'('")?;
                let mut jumps = Vec::new();
                loop {
                    self.parse_test_expr(negate, depth + 1)?;
                    match self.advance() {
                        Some(Token {
                            tok: Tok::Comma, ..
                        }) => {
                            self.instructions.push(if is_all {
                                Instruction::Jz(usize::MAX)
                            } else {
                                Instruction::Jnz(usize::MAX)
                            });
                            jumps.push(self.instructions.len() - 1);
                        }
                        Some(Token {
                            tok: Tok::RParen, ..
                        }) => break,
                        Some(other) => return Err(err_at(&other, "expected ',' or ')'")),
                        None => return Err(self.err("expected ',' or ')'")),
                    }
                }
                let target = self.instructions.len();
                for jump in jumps {
                    self.patch(jump, target);
                }
                Ok(())
            }
            "true" | "false" => {
                self.instructions
                    .push(Instruction::Test(Test::Bool((word == "true") != negate)));
                Ok(())
            }
            "exists" => {
                let headers = self.parse_header_list(&token)?;
                self.instructions.push(Instruction::Test(Test::Exists {
                    headers,
                    is_not: negate,
                }));
                Ok(())
            }
            "size" => {
                let mut over = None;
                while let Some(Tok::Tag(tag)) = self.peek() {
                    let tag = tag.clone();
                    self.advance();
                    match tag.as_str() {
                        "over" | "under" => {
                            if over.is_some() {
                                return Err(err_at(&token, "duplicate size tag"));
                            }
                            over = Some(tag == "over");
                        }
                        _ => return Err(err_at(&token, format!("unknown tag \":{tag}\""))),
                    }
                }
                let Some(over) = over else {
                    return Err(err_at(&token, "size requires :over or :under"));
                };
                let limit = match self.advance() {
                    Some(Token {
                        tok: Tok::Number(n),
                        ..
                    }) => n,
                    _ => return Err(err_at(&token, "size requires a number")),
                };
                self.instructions.push(Instruction::Test(Test::Size {
                    over,
                    limit,
                    is_not: negate,
                }));
                Ok(())
            }
            "header" => {
                let tags = self.parse_match_tags(&token, false)?;
                let headers = self.parse_header_list(&token)?;
                let keys = self.parse_string_list()?;
                self.instructions.push(Instruction::Test(Test::Header {
                    headers,
                    keys,
                    match_type: tags.match_type,
                    comparator: tags.comparator,
                    is_not: negate,
                }));
                Ok(())
            }
            "address" => {
                let tags = self.parse_match_tags(&token, true)?;
                let headers = self.parse_header_list(&token)?;
                let keys = self.parse_string_list()?;
                self.instructions.push(Instruction::Test(Test::Address {
                    headers,
                    keys,
                    part: tags.part,
                    match_type: tags.match_type,
                    comparator: tags.comparator,
                    is_not: negate,
                }));
                Ok(())
            }
            "envelope" => {
                self.need_capability("envelope", &token)?;
                let tags = self.parse_match_tags(&token, true)?;
                let mut parts = Vec::new();
                for name in self.parse_string_list()? {
                    match name.to_ascii_lowercase().as_str() {
                        "from" => parts.push(EnvelopePart::From),
                        "to" => parts.push(EnvelopePart::To),
                        _ => {
                            return Err(err_at(
                                &token,
                                format!("unsupported envelope part \"{name}\""),
                            ));
                        }
                    }
                }
                let keys = self.parse_string_list()?;
                self.instructions.push(Instruction::Test(Test::Envelope {
                    parts,
                    keys,
                    part: tags.part,
                    match_type: tags.match_type,
                    comparator: tags.comparator,
                    is_not: negate,
                }));
                Ok(())
            }
            _ => Err(err_at(&token, format!("unknown test \"{word}\""))),
        }
    }

    fn parse_match_tags(
        &mut self,
        token: &Token,
        with_part: bool,
    ) -> Result<MatchTags, CompileError> {
        let mut match_type = None;
        let mut comparator = None;
        let mut part = None;
        while let Some(Tok::Tag(tag)) = self.peek() {
            let tag = tag.clone();
            self.advance();
            match tag.as_str() {
                "is" | "contains" | "matches" => {
                    if match_type.is_some() {
                        return Err(err_at(token, "duplicate match type"));
                    }
                    match_type = Some(match tag.as_str() {
                        "is" => MatchType::Is,
                        "contains" => MatchType::Contains,
                        _ => MatchType::Matches,
                    });
                }
                "comparator" => {
                    if comparator.is_some() {
                        return Err(err_at(token, "duplicate comparator"));
                    }
                    let name = self.parse_string()?;
                    comparator = Some(match name.as_str() {
                        "i;octet" => {
                            self.need_capability("comparator-i;octet", token)?;
                            Comparator::Octet
                        }
                        "i;ascii-casemap" => Comparator::AsciiCaseMap,
                        _ => {
                            return Err(err_at(token, format!("unknown comparator \"{name}\"")));
                        }
                    });
                }
                "all" | "localpart" | "domain" if with_part => {
                    if part.is_some() {
                        return Err(err_at(token, "duplicate address part"));
                    }
                    part = Some(match tag.as_str() {
                        "all" => AddressPart::All,
                        "localpart" => AddressPart::Localpart,
                        _ => AddressPart::Domain,
                    });
                }
                _ => return Err(err_at(token, format!("unknown tag \":{tag}\""))),
            }
        }
        Ok(MatchTags {
            match_type: match_type.unwrap_or(MatchType::Is),
            comparator: comparator.unwrap_or(Comparator::AsciiCaseMap),
            part: part.unwrap_or(AddressPart::All),
        })
    }

    fn parse_string(&mut self) -> Result<String, CompileError> {
        match self.advance() {
            Some(Token {
                tok: Tok::Str(value),
                line,
                col,
            }) => self.decode(value, line, col),
            Some(token) => Err(err_at(&token, "expected a string")),
            None => Err(self.err("expected a string")),
        }
    }

    fn parse_string_list(&mut self) -> Result<Vec<String>, CompileError> {
        let raw = self.parse_raw_string_list()?;
        let mut decoded = Vec::with_capacity(raw.len());
        for value in raw {
            decoded.push(self.decode(value, 0, 0)?);
        }
        Ok(decoded)
    }

    fn parse_raw_string_list(&mut self) -> Result<Vec<String>, CompileError> {
        match self.advance() {
            Some(Token {
                tok: Tok::Str(value),
                ..
            }) => Ok(vec![value]),
            Some(Token {
                tok: Tok::LBracket, ..
            }) => {
                let mut values = Vec::new();
                loop {
                    match self.advance() {
                        Some(Token {
                            tok: Tok::Str(value),
                            ..
                        }) => {
                            values.push(value);
                            if values.len() > self.limits.max_list_items {
                                return Err(self.err("list has too many items"));
                            }
                        }
                        Some(token) => return Err(err_at(&token, "expected a string")),
                        None => return Err(self.err("expected a string")),
                    }
                    match self.advance() {
                        Some(Token {
                            tok: Tok::Comma, ..
                        }) => {}
                        Some(Token {
                            tok: Tok::RBracket, ..
                        }) => break,
                        Some(token) => return Err(err_at(&token, "expected ',' or ']'")),
                        None => return Err(self.err("expected ',' or ']'")),
                    }
                }
                Ok(values)
            }
            Some(token) => Err(err_at(&token, "expected a string or string list")),
            None => Err(self.err("expected a string or string list")),
        }
    }

    fn parse_header_list(&mut self, token: &Token) -> Result<Vec<String>, CompileError> {
        let headers = self.parse_string_list()?;
        for header in &headers {
            let valid =
                !header.is_empty() && header.bytes().all(|b| (33..=126).contains(&b) && b != b':');
            if !valid {
                return Err(err_at(token, format!("invalid header name \"{header}\"")));
            }
        }
        Ok(headers)
    }

    fn decode(&self, value: String, line: usize, col: usize) -> Result<String, CompileError> {
        if !self.has_capability("encoded-character") {
            return Ok(value);
        }
        decode_encoded_characters(&value).map_err(|message| CompileError {
            line: if line == 0 { 1 } else { line },
            column: col.max(1),
            message,
        })
    }
}

struct MatchTags {
    match_type: MatchType,
    comparator: Comparator,
    part: AddressPart,
}

fn err_at(token: &Token, message: impl Into<String>) -> CompileError {
    CompileError {
        line: token.line,
        column: token.col,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;

    fn parse(source: &str) -> Result<Program, CompileError> {
        let limits = CompilerLimits::default();
        compile(tokenize(source, &limits)?, &limits)
    }

    fn instructions(source: &str) -> Vec<Instruction> {
        parse(source).unwrap().instructions
    }

    fn error(source: &str) -> CompileError {
        parse(source).unwrap_err()
    }

    #[test]
    fn an_empty_script_compiles_to_no_instructions() {
        assert!(instructions("").is_empty());
        assert!(instructions("# only a comment\n").is_empty());
    }

    #[test]
    fn an_if_test_compiles_to_a_forward_jz_over_its_block() {
        assert_eq!(
            instructions("if true { discard; }"),
            vec![
                Instruction::Test(Test::Bool(true)),
                Instruction::Jz(3),
                Instruction::Discard,
            ]
        );
    }

    #[test]
    fn an_elsif_else_chain_jumps_over_the_remaining_branches() {
        assert_eq!(
            instructions("if true { keep; } elsif false { discard; } else { stop; }"),
            vec![
                Instruction::Test(Test::Bool(true)),
                Instruction::Jz(4),
                Instruction::Keep,
                Instruction::Jmp(9),
                Instruction::Test(Test::Bool(false)),
                Instruction::Jz(8),
                Instruction::Discard,
                Instruction::Jmp(9),
                Instruction::Stop,
            ]
        );
    }

    #[test]
    fn allof_short_circuits_with_jz_between_tests() {
        assert_eq!(
            instructions("if allof (true, false) { keep; }"),
            vec![
                Instruction::Test(Test::Bool(true)),
                Instruction::Jz(3),
                Instruction::Test(Test::Bool(false)),
                Instruction::Jz(5),
                Instruction::Keep,
            ]
        );
    }

    #[test]
    fn anyof_short_circuits_with_jnz_between_tests() {
        assert_eq!(
            instructions("if anyof (true, false) { keep; }"),
            vec![
                Instruction::Test(Test::Bool(true)),
                Instruction::Jnz(3),
                Instruction::Test(Test::Bool(false)),
                Instruction::Jz(5),
                Instruction::Keep,
            ]
        );
    }

    #[test]
    fn not_is_folded_into_leaf_tests_by_de_morgan() {
        assert_eq!(
            instructions("if not allof (exists \"a\", not exists \"b\") { keep; }"),
            vec![
                Instruction::Test(Test::Exists {
                    headers: vec!["a".into()],
                    is_not: true,
                }),
                Instruction::Jnz(3),
                Instruction::Test(Test::Exists {
                    headers: vec!["b".into()],
                    is_not: false,
                }),
                Instruction::Jz(5),
                Instruction::Keep,
            ]
        );
    }

    #[test]
    fn not_true_folds_to_a_false_literal() {
        assert_eq!(
            instructions("if not true { keep; }"),
            vec![
                Instruction::Test(Test::Bool(false)),
                Instruction::Jz(3),
                Instruction::Keep,
            ]
        );
    }

    #[test]
    fn header_tests_default_to_is_with_the_casemap_comparator() {
        assert_eq!(
            instructions("if header \"subject\" \"hi\" { keep; }")[0],
            Instruction::Test(Test::Header {
                headers: vec!["subject".into()],
                keys: vec!["hi".into()],
                match_type: MatchType::Is,
                comparator: Comparator::AsciiCaseMap,
                is_not: false,
            })
        );
    }

    #[test]
    fn address_tests_accept_an_address_part_tag() {
        assert_eq!(
            instructions("if address :domain :is \"from\" \"example.com\" { keep; }")[0],
            Instruction::Test(Test::Address {
                headers: vec!["from".into()],
                keys: vec!["example.com".into()],
                part: AddressPart::Domain,
                match_type: MatchType::Is,
                comparator: Comparator::AsciiCaseMap,
                is_not: false,
            })
        );
    }

    #[test]
    fn envelope_tests_need_the_envelope_capability() {
        assert_eq!(
            error("if envelope \"from\" \"a@b.example\" { keep; }").message,
            "\"envelope\" must be required first"
        );
        assert_eq!(
            instructions("require \"envelope\";\nif envelope :localpart \"FROM\" \"a\" { keep; }")
                [0],
            Instruction::Test(Test::Envelope {
                parts: vec![EnvelopePart::From],
                keys: vec!["a".into()],
                part: AddressPart::Localpart,
                match_type: MatchType::Is,
                comparator: Comparator::AsciiCaseMap,
                is_not: false,
            })
        );
    }

    #[test]
    fn an_unsupported_envelope_part_is_rejected() {
        assert_eq!(
            error("require \"envelope\";\nif envelope \"auth\" \"x\" { keep; }").message,
            "unsupported envelope part \"auth\""
        );
    }

    #[test]
    fn size_requires_exactly_one_direction_tag() {
        assert_eq!(
            instructions("if size :over 100K { discard; }")[0],
            Instruction::Test(Test::Size {
                over: true,
                limit: 100 << 10,
                is_not: false,
            })
        );
        assert_eq!(
            error("if size 100 { keep; }").message,
            "size requires :over or :under"
        );
        assert_eq!(
            error("if size :over :under 1 { keep; }").message,
            "duplicate size tag"
        );
    }

    #[test]
    fn fileinto_and_flag_actions_need_their_capabilities() {
        assert_eq!(
            error("fileinto \"Spam\";").message,
            "\"fileinto\" must be required first"
        );
        assert_eq!(
            error("addflag \"\\\\Seen\";").message,
            "\"imap4flags\" must be required first"
        );
    }

    #[test]
    fn flag_arguments_split_on_whitespace() {
        assert_eq!(
            instructions("require \"imap4flags\";\nsetflag \"\\\\Seen \\\\Flagged\";").pop(),
            Some(Instruction::SetFlag(vec![
                "\\Seen".into(),
                "\\Flagged".into()
            ]))
        );
    }

    #[test]
    fn require_after_a_command_is_rejected() {
        assert_eq!(
            error("keep;\nrequire \"fileinto\";").message,
            "require must precede all other commands"
        );
    }

    #[test]
    fn an_unknown_capability_is_rejected() {
        assert_eq!(
            error("require \"vacation\";").message,
            "unknown capability \"vacation\""
        );
    }

    #[test]
    fn unknown_commands_and_tests_are_rejected_with_positions() {
        let error_command = error("keep;\nfrobnicate;");
        assert_eq!(error_command.message, "unknown command \"frobnicate\"");
        assert_eq!(error_command.line, 2);
        let error_test = error("if frobnicate { keep; }");
        assert_eq!(error_test.message, "unknown test \"frobnicate\"");
    }

    #[test]
    fn duplicate_match_type_tags_are_rejected() {
        assert_eq!(
            error("if header :is :contains \"a\" \"b\" { keep; }").message,
            "duplicate match type"
        );
    }

    #[test]
    fn the_octet_comparator_needs_its_capability() {
        assert_eq!(
            error("if header :comparator \"i;octet\" \"a\" \"b\" { keep; }").message,
            "\"comparator-i;octet\" must be required first"
        );
        assert!(parse(
            "require \"comparator-i;octet\";\nif header :comparator \"i;octet\" \"a\" \"b\" { keep; }"
        )
        .is_ok());
        assert_eq!(
            error("if header :comparator \"i;unicode\" \"a\" \"b\" { keep; }").message,
            "unknown comparator \"i;unicode\""
        );
    }

    #[test]
    fn invalid_header_names_are_rejected() {
        assert_eq!(
            error("if exists \"bad name\" { keep; }").message,
            "invalid header name \"bad name\""
        );
        assert_eq!(
            error("if exists \"colon:name\" { keep; }").message,
            "invalid header name \"colon:name\""
        );
        assert_eq!(
            error("if exists \"\" { keep; }").message,
            "invalid header name \"\""
        );
    }

    #[test]
    fn unclosed_blocks_and_stray_braces_are_rejected() {
        assert_eq!(
            error("if true { keep;").message,
            "unclosed block at end of script"
        );
        assert_eq!(error("}").message, "'}' without an open block");
        assert_eq!(
            error("elsif true { keep; }").message,
            "elsif without a preceding if"
        );
    }

    #[test]
    fn else_cannot_be_followed_by_another_branch() {
        assert_eq!(
            error("if true { keep; } else { stop; } else { discard; }").message,
            "else block cannot be followed by another branch"
        );
    }

    #[test]
    fn deep_block_nesting_is_rejected() {
        let mut source = String::new();
        for _ in 0..20 {
            source.push_str("if true { ");
        }
        source.push_str("keep;");
        for _ in 0..20 {
            source.push_str(" }");
        }
        assert_eq!(error(&source).message, "blocks are nested too deeply");
    }

    #[test]
    fn deep_test_nesting_is_rejected() {
        let mut source = String::from("if ");
        for _ in 0..20 {
            source.push_str("allof (");
        }
        source.push_str("true");
        for _ in 0..20 {
            source.push(')');
        }
        source.push_str(" { keep; }");
        assert_eq!(error(&source).message, "tests are nested too deeply");
    }

    #[test]
    fn oversized_string_lists_are_rejected() {
        let items: Vec<String> = (0..200).map(|i| format!("\"h{i}\"")).collect();
        let source = format!("if exists [{}] {{ keep; }}", items.join(", "));
        assert_eq!(error(&source).message, "list has too many items");
    }

    #[test]
    fn encoded_characters_decode_only_when_required() {
        assert_eq!(
            instructions("require [\"fileinto\", \"encoded-character\"];\nfileinto \"${hex:40}\";")
                .pop(),
            Some(Instruction::FileInto("@".into()))
        );
        assert_eq!(
            instructions("require \"fileinto\";\nfileinto \"${hex:40}\";").pop(),
            Some(Instruction::FileInto("${hex:40}".into()))
        );
    }

    #[test]
    fn capabilities_are_recorded_once_each() {
        let program =
            parse("require [\"fileinto\", \"imap4flags\"];\nrequire \"fileinto\";\nkeep;").unwrap();
        assert_eq!(program.capabilities, vec!["fileinto", "imap4flags"]);
    }

    #[test]
    fn missing_semicolons_are_reported() {
        assert_eq!(error("keep").message, "expected ';'");
        assert_eq!(error("keep stop;").message, "expected ';'");
    }
}
