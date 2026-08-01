use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

pub fn auth_list() -> &'static [u8] {
    b"+OK\r\nPLAIN\r\nLOGIN\r\n.\r\n"
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mechanism {
    Plain,
    Login,
    Unsupported,
}

impl Mechanism {
    pub fn parse(name: &str) -> Self {
        if name.eq_ignore_ascii_case("PLAIN") {
            Mechanism::Plain
        } else if name.eq_ignore_ascii_case("LOGIN") {
            Mechanism::Login
        } else {
            Mechanism::Unsupported
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaslStep {
    Challenge(String),
    Resolved(Credentials),
    Failed(&'static str),
}

pub enum SaslStart {
    Reply(&'static [u8]),
    Continue {
        exchange: SaslExchange,
        step: SaslStep,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    PlainAwaitingResponse,
    LoginAwaitingUsername,
    LoginAwaitingPassword { username: String },
}

pub struct SaslExchange {
    state: State,
}

impl SaslExchange {
    pub fn begin(mechanism: Mechanism, is_tls: bool, initial: Option<&str>) -> SaslStart {
        if !is_tls {
            return SaslStart::Reply(b"-ERR [AUTH] STLS required before AUTH\r\n");
        }
        match mechanism {
            Mechanism::Plain => {
                let mut exchange = SaslExchange {
                    state: State::PlainAwaitingResponse,
                };
                match initial {
                    Some(response) => {
                        let step = exchange.advance(response);
                        SaslStart::Continue { exchange, step }
                    }
                    None => SaslStart::Continue {
                        exchange,
                        step: SaslStep::Challenge(String::new()),
                    },
                }
            }
            Mechanism::Login => {
                let mut exchange = SaslExchange {
                    state: State::LoginAwaitingUsername,
                };
                match initial {
                    Some(response) => {
                        let step = exchange.advance(response);
                        SaslStart::Continue { exchange, step }
                    }
                    None => SaslStart::Continue {
                        exchange,
                        step: SaslStep::Challenge(STANDARD.encode("Username:")),
                    },
                }
            }
            Mechanism::Unsupported => {
                SaslStart::Reply(b"-ERR unsupported authentication mechanism\r\n")
            }
        }
    }

    pub fn advance(&mut self, response: &str) -> SaslStep {
        let decoded = match STANDARD.decode(response.trim()) {
            Ok(decoded) => decoded,
            Err(_) => return SaslStep::Failed("invalid base64 in authentication response"),
        };
        match std::mem::replace(&mut self.state, State::PlainAwaitingResponse) {
            State::PlainAwaitingResponse => match decode_plain(&decoded) {
                Some(credentials) => SaslStep::Resolved(credentials),
                None => SaslStep::Failed("malformed PLAIN authentication response"),
            },
            State::LoginAwaitingUsername => match String::from_utf8(decoded) {
                Ok(username) => {
                    self.state = State::LoginAwaitingPassword { username };
                    SaslStep::Challenge(STANDARD.encode("Password:"))
                }
                Err(_) => SaslStep::Failed("invalid username encoding"),
            },
            State::LoginAwaitingPassword { username } => match String::from_utf8(decoded) {
                Ok(password) => SaslStep::Resolved(Credentials { username, password }),
                Err(_) => SaslStep::Failed("invalid password encoding"),
            },
        }
    }
}

fn decode_plain(payload: &[u8]) -> Option<Credentials> {
    let mut parts = payload.splitn(3, |byte| *byte == 0);
    let _authzid = parts.next()?;
    let authcid = parts.next()?;
    let password = parts.next()?;
    if password.contains(&0) {
        return None;
    }
    Some(Credentials {
        username: String::from_utf8(authcid.to_vec()).ok()?,
        password: String::from_utf8(password.to_vec()).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(bytes: &[u8]) -> String {
        STANDARD.encode(bytes)
    }

    fn resolved(start: SaslStart) -> Credentials {
        match start {
            SaslStart::Continue {
                step: SaslStep::Resolved(credentials),
                ..
            } => credentials,
            _ => panic!("expected resolved credentials"),
        }
    }

    #[test]
    fn the_mechanism_list_terminates_with_a_dot() {
        assert!(auth_list().ends_with(b".\r\n"));
    }

    #[test]
    fn auth_is_refused_on_a_cleartext_channel() {
        match SaslExchange::begin(Mechanism::Plain, false, None) {
            SaslStart::Reply(line) => assert!(line.starts_with(b"-ERR")),
            _ => panic!("expected a refusal"),
        }
    }

    #[test]
    fn an_unsupported_mechanism_is_refused() {
        match SaslExchange::begin(Mechanism::Unsupported, true, None) {
            SaslStart::Reply(line) => assert!(line.starts_with(b"-ERR")),
            _ => panic!("expected a refusal"),
        }
    }

    #[test]
    fn plain_with_an_inline_response_resolves() {
        let credentials = resolved(SaslExchange::begin(
            Mechanism::Plain,
            true,
            Some(&b64(b"\0alice\0secret")),
        ));
        assert_eq!(credentials.username, "alice");
        assert_eq!(credentials.password, "secret");
    }

    #[test]
    fn login_walks_the_username_and_password_challenges() {
        let SaslStart::Continue { mut exchange, step } =
            SaslExchange::begin(Mechanism::Login, true, None)
        else {
            panic!("expected a live exchange");
        };
        assert_eq!(step, SaslStep::Challenge(STANDARD.encode("Username:")));
        assert_eq!(
            exchange.advance(&b64(b"carol")),
            SaslStep::Challenge(STANDARD.encode("Password:"))
        );
        match exchange.advance(&b64(b"open sesame")) {
            SaslStep::Resolved(credentials) => {
                assert_eq!(credentials.username, "carol");
                assert_eq!(credentials.password, "open sesame");
            }
            _ => panic!("expected resolved credentials"),
        }
    }

    #[test]
    fn a_non_base64_response_is_a_failure() {
        let SaslStart::Continue { mut exchange, .. } =
            SaslExchange::begin(Mechanism::Plain, true, None)
        else {
            panic!("expected a live exchange");
        };
        assert!(matches!(
            exchange.advance("not base64 !!!"),
            SaslStep::Failed(_)
        ));
    }
}
