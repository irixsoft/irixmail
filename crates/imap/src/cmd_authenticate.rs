use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

use crate::cmd_login::LoginCredentials;

const PRIVACY_REQUIRED: &str = "[PRIVACYREQUIRED] AUTHENTICATE requires TLS";
const UNSUPPORTED: &str = "Unsupported authentication mechanism";

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
pub enum SaslStep {
    Challenge(String),
    Resolved(LoginCredentials),
    Failed {
        status: &'static str,
        text: &'static str,
    },
}

pub enum SaslStart {
    Reply {
        status: &'static str,
        text: &'static str,
    },
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
    pub fn begin(
        mechanism: Mechanism,
        is_tls: bool,
        authenticated: bool,
        initial_response: Option<&str>,
    ) -> SaslStart {
        if authenticated {
            return SaslStart::Reply {
                status: "NO",
                text: "Already authenticated",
            };
        }
        if !is_tls {
            return SaslStart::Reply {
                status: "NO",
                text: PRIVACY_REQUIRED,
            };
        }
        match mechanism {
            Mechanism::Plain => {
                let mut exchange = SaslExchange {
                    state: State::PlainAwaitingResponse,
                };
                match initial_response {
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
                match initial_response {
                    Some(response) => {
                        let step = exchange.advance(response);
                        SaslStart::Continue { exchange, step }
                    }
                    None => SaslStart::Continue {
                        exchange,
                        step: SaslStep::Challenge(challenge("Username:")),
                    },
                }
            }
            Mechanism::Unsupported => SaslStart::Reply {
                status: "NO",
                text: UNSUPPORTED,
            },
        }
    }

    pub fn advance(&mut self, response: &str) -> SaslStep {
        let decoded = match STANDARD.decode(response.trim()) {
            Ok(decoded) => decoded,
            Err(_) => {
                return SaslStep::Failed {
                    status: "BAD",
                    text: "Invalid base64 in authentication response",
                }
            }
        };
        match std::mem::replace(&mut self.state, State::PlainAwaitingResponse) {
            State::PlainAwaitingResponse => match decode_plain(&decoded) {
                Some(credentials) => SaslStep::Resolved(credentials),
                None => SaslStep::Failed {
                    status: "BAD",
                    text: "Malformed PLAIN authentication response",
                },
            },
            State::LoginAwaitingUsername => match String::from_utf8(decoded) {
                Ok(username) => {
                    self.state = State::LoginAwaitingPassword { username };
                    SaslStep::Challenge(challenge("Password:"))
                }
                Err(_) => SaslStep::Failed {
                    status: "BAD",
                    text: "Invalid username encoding",
                },
            },
            State::LoginAwaitingPassword { username } => match String::from_utf8(decoded) {
                Ok(password) => SaslStep::Resolved(LoginCredentials { username, password }),
                Err(_) => SaslStep::Failed {
                    status: "BAD",
                    text: "Invalid password encoding",
                },
            },
        }
    }
}

fn challenge(prompt: &str) -> String {
    STANDARD.encode(prompt)
}

fn decode_plain(payload: &[u8]) -> Option<LoginCredentials> {
    let mut parts = payload.splitn(3, |byte| *byte == 0);
    let _authzid = parts.next()?;
    let authcid = parts.next()?;
    let password = parts.next()?;
    if password.contains(&0) {
        return None;
    }
    Some(LoginCredentials {
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

    fn resolved(start: SaslStart) -> LoginCredentials {
        match start {
            SaslStart::Continue {
                step: SaslStep::Resolved(credentials),
                ..
            } => credentials,
            _ => panic!("expected resolved credentials"),
        }
    }

    #[test]
    fn mechanisms_parse_case_insensitively() {
        assert_eq!(Mechanism::parse("plain"), Mechanism::Plain);
        assert_eq!(Mechanism::parse("LOGIN"), Mechanism::Login);
        assert_eq!(Mechanism::parse("CRAM-MD5"), Mechanism::Unsupported);
    }

    #[test]
    fn authentication_is_refused_on_a_cleartext_channel() {
        match SaslExchange::begin(Mechanism::Plain, false, false, None) {
            SaslStart::Reply { status, text } => {
                assert_eq!(status, "NO");
                assert_eq!(text, PRIVACY_REQUIRED);
            }
            _ => panic!("expected a refusal"),
        }
    }

    #[test]
    fn authentication_is_refused_once_authenticated() {
        match SaslExchange::begin(Mechanism::Plain, true, true, None) {
            SaslStart::Reply { text, .. } => assert_eq!(text, "Already authenticated"),
            _ => panic!("expected a refusal"),
        }
    }

    #[test]
    fn an_unsupported_mechanism_is_rejected() {
        match SaslExchange::begin(Mechanism::Unsupported, true, false, None) {
            SaslStart::Reply { text, .. } => assert_eq!(text, UNSUPPORTED),
            _ => panic!("expected a rejection"),
        }
    }

    #[test]
    fn plain_with_an_inline_response_resolves_immediately() {
        let payload = b64(b"\0alice\0secret");
        let credentials = resolved(SaslExchange::begin(
            Mechanism::Plain,
            true,
            false,
            Some(&payload),
        ));
        assert_eq!(credentials.username, "alice");
        assert_eq!(credentials.password, "secret");
    }

    #[test]
    fn plain_challenges_then_resolves() {
        let SaslStart::Continue { mut exchange, step } =
            SaslExchange::begin(Mechanism::Plain, true, false, None)
        else {
            panic!("expected a live exchange");
        };
        assert_eq!(step, SaslStep::Challenge(String::new()));
        match exchange.advance(&b64(b"\0bob\0hunter2")) {
            SaslStep::Resolved(credentials) => {
                assert_eq!(credentials.username, "bob");
                assert_eq!(credentials.password, "hunter2");
            }
            _ => panic!("expected resolved credentials"),
        }
    }

    #[test]
    fn login_walks_the_username_and_password_challenges() {
        let SaslStart::Continue { mut exchange, step } =
            SaslExchange::begin(Mechanism::Login, true, false, None)
        else {
            panic!("expected a live exchange");
        };
        assert_eq!(step, SaslStep::Challenge(challenge("Username:")));
        assert_eq!(
            exchange.advance(&b64(b"carol")),
            SaslStep::Challenge(challenge("Password:"))
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
    fn a_non_base64_response_is_a_protocol_error() {
        let SaslStart::Continue { mut exchange, .. } =
            SaslExchange::begin(Mechanism::Plain, true, false, None)
        else {
            panic!("expected a live exchange");
        };
        assert_eq!(
            exchange.advance("not base64 !!!"),
            SaslStep::Failed {
                status: "BAD",
                text: "Invalid base64 in authentication response"
            }
        );
    }

    #[test]
    fn a_plain_payload_with_an_authzid_keeps_the_authcid() {
        let credentials = resolved(SaslExchange::begin(
            Mechanism::Plain,
            true,
            false,
            Some(&b64(b"admin\0alice\0pw")),
        ));
        assert_eq!(credentials.username, "alice");
        assert_eq!(credentials.password, "pw");
    }

    #[test]
    fn a_password_may_be_empty() {
        let credentials = resolved(SaslExchange::begin(
            Mechanism::Plain,
            true,
            false,
            Some(&b64(b"\0alice\0")),
        ));
        assert!(credentials.password.is_empty());
    }
}
