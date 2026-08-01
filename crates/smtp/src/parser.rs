use std::fmt;

use smtp_proto::{
    Error as ProtoError, Request, AUTH_LOGIN, AUTH_PLAIN, MAIL_BODY_8BITMIME, MAIL_SMTPUTF8,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Ehlo {
        host: String,
    },
    Helo {
        host: String,
    },
    Mail {
        from: MailParams,
    },
    Rcpt {
        to: RcptParams,
    },
    Data,
    Bdat {
        chunk_size: usize,
        is_last: bool,
    },
    StartTls,
    Auth {
        mechanism: AuthMechanism,
        initial_response: String,
    },
    Rset,
    Noop,
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailParams {
    pub address: String,
    pub size: usize,
    pub body_8bitmime: bool,
    pub smtputf8: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RcptParams {
    pub address: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthMechanism {
    Plain,
    Login,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    Incomplete,
    UnknownCommand,
    Unsupported,
    InvalidSender,
    InvalidRecipient,
    Syntax(&'static str),
    InvalidParameter(&'static str),
    UnsupportedParameter(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Incomplete => f.write_str("command line incomplete"),
            ParseError::UnknownCommand => f.write_str("unrecognized command"),
            ParseError::Unsupported => f.write_str("command not supported"),
            ParseError::InvalidSender => f.write_str("invalid sender address"),
            ParseError::InvalidRecipient => f.write_str("invalid recipient address"),
            ParseError::Syntax(syntax) => write!(f, "syntax error, expected {syntax}"),
            ParseError::InvalidParameter(param) => write!(f, "invalid {param} parameter"),
            ParseError::UnsupportedParameter(param) => write!(f, "unsupported parameter {param}"),
        }
    }
}

impl std::error::Error for ParseError {}

pub fn parse_command(line: &[u8]) -> Result<Command, ParseError> {
    let mut bytes = line.iter();
    let request = Request::parse(&mut bytes).map_err(map_proto_error)?;
    match request {
        Request::Ehlo { host } => Ok(Command::Ehlo {
            host: host.into_owned(),
        }),
        Request::Helo { host } => Ok(Command::Helo {
            host: host.into_owned(),
        }),
        Request::Mail { from } => Ok(Command::Mail {
            from: MailParams {
                address: from.address.into_owned(),
                size: from.size,
                body_8bitmime: from.flags & MAIL_BODY_8BITMIME != 0,
                smtputf8: from.flags & MAIL_SMTPUTF8 != 0,
            },
        }),
        Request::Rcpt { to } => Ok(Command::Rcpt {
            to: RcptParams {
                address: to.address.into_owned(),
            },
        }),
        Request::Data => Ok(Command::Data),
        Request::Bdat {
            chunk_size,
            is_last,
        } => Ok(Command::Bdat {
            chunk_size,
            is_last,
        }),
        Request::StartTls => Ok(Command::StartTls),
        Request::Auth {
            mechanism,
            initial_response,
        } => Ok(Command::Auth {
            mechanism: auth_mechanism(mechanism),
            initial_response: initial_response.into_owned(),
        }),
        Request::Rset => Ok(Command::Rset),
        Request::Noop { .. } => Ok(Command::Noop),
        Request::Quit => Ok(Command::Quit),
        _ => Err(ParseError::Unsupported),
    }
}

fn auth_mechanism(mechanism: u64) -> AuthMechanism {
    if mechanism == AUTH_PLAIN {
        AuthMechanism::Plain
    } else if mechanism == AUTH_LOGIN {
        AuthMechanism::Login
    } else {
        AuthMechanism::Unsupported
    }
}

fn map_proto_error(error: ProtoError) -> ParseError {
    match error {
        ProtoError::NeedsMoreData { .. } => ParseError::Incomplete,
        ProtoError::UnknownCommand => ParseError::UnknownCommand,
        ProtoError::InvalidSenderAddress => ParseError::InvalidSender,
        ProtoError::InvalidRecipientAddress => ParseError::InvalidRecipient,
        ProtoError::SyntaxError { syntax } => ParseError::Syntax(syntax),
        ProtoError::InvalidParameter { param } => ParseError::InvalidParameter(param),
        ProtoError::UnsupportedParameter { param } => ParseError::UnsupportedParameter(param),
        ProtoError::ResponseTooLong | ProtoError::InvalidResponse { .. } => {
            ParseError::Syntax("command")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ehlo_and_helo_carry_the_host() {
        assert_eq!(
            parse_command(b"EHLO client.example\r\n").unwrap(),
            Command::Ehlo {
                host: "client.example".into()
            }
        );
        assert_eq!(
            parse_command(b"helo other.example\r\n").unwrap(),
            Command::Helo {
                host: "other.example".into()
            }
        );
    }

    #[test]
    fn mail_captures_size_and_extensions() {
        let command =
            parse_command(b"MAIL FROM:<a@b.example> SIZE=2048 BODY=8BITMIME SMTPUTF8\r\n").unwrap();
        let Command::Mail { from } = command else {
            panic!("expected MAIL");
        };
        assert_eq!(from.address, "a@b.example");
        assert_eq!(from.size, 2048);
        assert!(from.body_8bitmime);
        assert!(from.smtputf8);
    }

    #[test]
    fn null_sender_is_accepted() {
        let command = parse_command(b"MAIL FROM:<>\r\n").unwrap();
        let Command::Mail { from } = command else {
            panic!("expected MAIL");
        };
        assert!(from.address.is_empty());
    }

    #[test]
    fn rcpt_carries_the_recipient() {
        assert_eq!(
            parse_command(b"RCPT TO:<c@d.example>\r\n").unwrap(),
            Command::Rcpt {
                to: RcptParams {
                    address: "c@d.example".into()
                }
            }
        );
    }

    #[test]
    fn simple_verbs_parse() {
        assert_eq!(parse_command(b"DATA\r\n").unwrap(), Command::Data);
        assert_eq!(parse_command(b"RSET\r\n").unwrap(), Command::Rset);
        assert_eq!(parse_command(b"NOOP\r\n").unwrap(), Command::Noop);
        assert_eq!(parse_command(b"QUIT\r\n").unwrap(), Command::Quit);
        assert_eq!(parse_command(b"STARTTLS\r\n").unwrap(), Command::StartTls);
    }

    #[test]
    fn bdat_reports_chunk_size_and_last_flag() {
        assert_eq!(
            parse_command(b"BDAT 32 LAST\r\n").unwrap(),
            Command::Bdat {
                chunk_size: 32,
                is_last: true
            }
        );
        assert_eq!(
            parse_command(b"BDAT 16\r\n").unwrap(),
            Command::Bdat {
                chunk_size: 16,
                is_last: false
            }
        );
    }

    #[test]
    fn auth_mechanisms_are_classified() {
        assert_eq!(
            parse_command(b"AUTH PLAIN\r\n").unwrap(),
            Command::Auth {
                mechanism: AuthMechanism::Plain,
                initial_response: String::new()
            }
        );
        assert_eq!(
            parse_command(b"AUTH LOGIN\r\n").unwrap(),
            Command::Auth {
                mechanism: AuthMechanism::Login,
                initial_response: String::new()
            }
        );
        let command = parse_command(b"AUTH CRAM-MD5\r\n").unwrap();
        assert_eq!(
            command,
            Command::Auth {
                mechanism: AuthMechanism::Unsupported,
                initial_response: String::new()
            }
        );
    }

    #[test]
    fn auth_plain_keeps_the_initial_response() {
        let command = parse_command(b"AUTH PLAIN dGVzdAB0ZXN0AHNlY3JldA==\r\n").unwrap();
        assert_eq!(
            command,
            Command::Auth {
                mechanism: AuthMechanism::Plain,
                initial_response: "dGVzdAB0ZXN0AHNlY3JldA==".into(),
            }
        );
    }

    #[test]
    fn an_incomplete_line_is_reported() {
        assert_eq!(parse_command(b"QUIT"), Err(ParseError::Incomplete));
    }

    #[test]
    fn an_unknown_verb_is_reported() {
        assert_eq!(
            parse_command(b"FROBNICATE\r\n"),
            Err(ParseError::UnknownCommand)
        );
    }

    #[test]
    fn an_unsupported_verb_is_rejected() {
        assert_eq!(
            parse_command(b"VRFY user\r\n"),
            Err(ParseError::Unsupported)
        );
        assert_eq!(parse_command(b"HELP\r\n"), Err(ParseError::Unsupported));
    }

    #[test]
    fn a_malformed_sender_is_reported() {
        assert_eq!(
            parse_command(b"MAIL FROM:<@invalid>\r\n"),
            Err(ParseError::InvalidSender)
        );
    }
}
