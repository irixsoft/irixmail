use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

use crate::parser::AuthMechanism;

const ENCRYPTION_REQUIRED: &[u8] = b"530 5.7.0 Must issue a STARTTLS command first\r\n";
const ALREADY_AUTHENTICATED: &[u8] = b"503 5.5.1 Already authenticated\r\n";
const UNSUPPORTED_MECHANISM: &[u8] = b"504 5.5.4 Unsupported authentication mechanism\r\n";
const INVALID_CHALLENGE: &[u8] = b"501 5.5.2 Invalid authentication challenge\r\n";
const CREDENTIALS_INVALID: &[u8] = b"535 5.7.8 Authentication credentials invalid\r\n";
const SUCCESS: &[u8] = b"235 2.7.0 Authentication successful\r\n";
const TOO_MANY_ATTEMPTS: &[u8] = b"454 4.7.0 Too many failed authentication attempts\r\n";
const CHALLENGE_USERNAME: &[u8] = b"334 VXNlcm5hbWU6\r\n";
const CHALLENGE_PASSWORD: &[u8] = b"334 UGFzc3dvcmQ6\r\n";
const CHALLENGE_EMPTY: &[u8] = b"334 \r\n";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credentials {
    pub authcid: String,
    pub password: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaslStep {
    Challenge(&'static [u8]),
    Resolved(Credentials),
    Reply { bytes: &'static [u8], success: bool },
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
        mechanism: AuthMechanism,
        is_tls: bool,
        authenticated: bool,
        initial_response: &str,
    ) -> SaslStart {
        if authenticated {
            return SaslStart::Reply {
                bytes: ALREADY_AUTHENTICATED,
                success: false,
            };
        }
        if !is_tls {
            return SaslStart::Reply {
                bytes: ENCRYPTION_REQUIRED,
                success: false,
            };
        }
        match mechanism {
            AuthMechanism::Plain => {
                let mut exchange = SaslExchange {
                    state: State::PlainAwaitingResponse,
                };
                if initial_response.is_empty() {
                    SaslStart::Continue {
                        exchange,
                        step: SaslStep::Challenge(CHALLENGE_EMPTY),
                    }
                } else {
                    let step = exchange.advance(initial_response);
                    SaslStart::Continue { exchange, step }
                }
            }
            AuthMechanism::Login => {
                let mut exchange = SaslExchange {
                    state: State::LoginAwaitingUsername,
                };
                if initial_response.is_empty() {
                    SaslStart::Continue {
                        exchange,
                        step: SaslStep::Challenge(CHALLENGE_USERNAME),
                    }
                } else {
                    let step = exchange.advance(initial_response);
                    SaslStart::Continue { exchange, step }
                }
            }
            AuthMechanism::Unsupported => SaslStart::Reply {
                bytes: UNSUPPORTED_MECHANISM,
                success: false,
            },
        }
    }

    pub fn advance(&mut self, response: &str) -> SaslStep {
        let decoded = match STANDARD.decode(response.trim()) {
            Ok(decoded) => decoded,
            Err(_) => {
                return SaslStep::Reply {
                    bytes: INVALID_CHALLENGE,
                    success: false,
                }
            }
        };
        match std::mem::replace(&mut self.state, State::PlainAwaitingResponse) {
            State::PlainAwaitingResponse => match decode_plain(&decoded) {
                Some(credentials) => SaslStep::Resolved(credentials),
                None => SaslStep::Reply {
                    bytes: INVALID_CHALLENGE,
                    success: false,
                },
            },
            State::LoginAwaitingUsername => match String::from_utf8(decoded) {
                Ok(username) => {
                    self.state = State::LoginAwaitingPassword { username };
                    SaslStep::Challenge(CHALLENGE_PASSWORD)
                }
                Err(_) => SaslStep::Reply {
                    bytes: INVALID_CHALLENGE,
                    success: false,
                },
            },
            State::LoginAwaitingPassword { username } => match String::from_utf8(decoded) {
                Ok(password) => SaslStep::Resolved(Credentials {
                    authcid: username,
                    password,
                }),
                Err(_) => SaslStep::Reply {
                    bytes: INVALID_CHALLENGE,
                    success: false,
                },
            },
        }
    }
}

pub enum SaslStart {
    Reply {
        bytes: &'static [u8],
        success: bool,
    },
    Continue {
        exchange: SaslExchange,
        step: SaslStep,
    },
}

pub fn credentials_invalid_reply() -> &'static [u8] {
    CREDENTIALS_INVALID
}

pub fn success_reply() -> &'static [u8] {
    SUCCESS
}

pub fn too_many_attempts_reply() -> &'static [u8] {
    TOO_MANY_ATTEMPTS
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
        authcid: String::from_utf8(authcid.to_vec()).ok()?,
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
    fn auth_is_refused_on_a_plaintext_channel() {
        let start = SaslExchange::begin(AuthMechanism::Plain, false, false, "");
        match start {
            SaslStart::Reply { bytes, success } => {
                assert_eq!(bytes, ENCRYPTION_REQUIRED);
                assert!(!success);
            }
            _ => panic!("expected a refusal"),
        }
    }

    #[test]
    fn auth_is_refused_once_authenticated() {
        let start = SaslExchange::begin(AuthMechanism::Plain, true, true, "");
        match start {
            SaslStart::Reply { bytes, .. } => assert_eq!(bytes, ALREADY_AUTHENTICATED),
            _ => panic!("expected a refusal"),
        }
    }

    #[test]
    fn an_unsupported_mechanism_is_rejected() {
        let start = SaslExchange::begin(AuthMechanism::Unsupported, true, false, "");
        match start {
            SaslStart::Reply { bytes, .. } => assert_eq!(bytes, UNSUPPORTED_MECHANISM),
            _ => panic!("expected a rejection"),
        }
    }

    #[test]
    fn plain_with_an_inline_response_resolves_immediately() {
        let payload = b64(b"\0alice\0secret");
        let credentials = resolved(SaslExchange::begin(
            AuthMechanism::Plain,
            true,
            false,
            &payload,
        ));
        assert_eq!(credentials.authcid, "alice");
        assert_eq!(credentials.password, "secret");
    }

    #[test]
    fn plain_without_an_inline_response_asks_for_a_challenge_then_resolves() {
        let SaslStart::Continue { mut exchange, step } =
            SaslExchange::begin(AuthMechanism::Plain, true, false, "")
        else {
            panic!("expected a live exchange");
        };
        assert_eq!(step, SaslStep::Challenge(CHALLENGE_EMPTY));
        let payload = b64(b"\0bob\0hunter2");
        match exchange.advance(&payload) {
            SaslStep::Resolved(credentials) => {
                assert_eq!(credentials.authcid, "bob");
                assert_eq!(credentials.password, "hunter2");
            }
            _ => panic!("expected resolved credentials"),
        }
    }

    #[test]
    fn login_walks_the_username_and_password_challenges() {
        let SaslStart::Continue { mut exchange, step } =
            SaslExchange::begin(AuthMechanism::Login, true, false, "")
        else {
            panic!("expected a live exchange");
        };
        assert_eq!(step, SaslStep::Challenge(CHALLENGE_USERNAME));
        assert_eq!(
            exchange.advance(&b64(b"carol")),
            SaslStep::Challenge(CHALLENGE_PASSWORD)
        );
        match exchange.advance(&b64(b"open sesame")) {
            SaslStep::Resolved(credentials) => {
                assert_eq!(credentials.authcid, "carol");
                assert_eq!(credentials.password, "open sesame");
            }
            _ => panic!("expected resolved credentials"),
        }
    }

    #[test]
    fn login_accepts_the_username_as_an_inline_response() {
        let SaslStart::Continue { mut exchange, step } =
            SaslExchange::begin(AuthMechanism::Login, true, false, &b64(b"dave"))
        else {
            panic!("expected a live exchange");
        };
        assert_eq!(step, SaslStep::Challenge(CHALLENGE_PASSWORD));
        match exchange.advance(&b64(b"passphrase")) {
            SaslStep::Resolved(credentials) => assert_eq!(credentials.authcid, "dave"),
            _ => panic!("expected resolved credentials"),
        }
    }

    #[test]
    fn a_non_base64_response_is_a_protocol_error() {
        let SaslStart::Continue { mut exchange, .. } =
            SaslExchange::begin(AuthMechanism::Plain, true, false, "")
        else {
            panic!("expected a live exchange");
        };
        assert_eq!(
            exchange.advance("not base64 !!!"),
            SaslStep::Reply {
                bytes: INVALID_CHALLENGE,
                success: false
            }
        );
    }

    #[test]
    fn a_plain_payload_missing_a_field_is_rejected() {
        let SaslStart::Continue { mut exchange, .. } =
            SaslExchange::begin(AuthMechanism::Plain, true, false, "")
        else {
            panic!("expected a live exchange");
        };
        assert_eq!(
            exchange.advance(&b64(b"alice")),
            SaslStep::Reply {
                bytes: INVALID_CHALLENGE,
                success: false
            }
        );
    }

    #[test]
    fn a_plain_payload_with_an_authzid_keeps_the_authcid() {
        let credentials = resolved(SaslExchange::begin(
            AuthMechanism::Plain,
            true,
            false,
            &b64(b"admin\0alice\0pw"),
        ));
        assert_eq!(credentials.authcid, "alice");
        assert_eq!(credentials.password, "pw");
    }

    #[test]
    fn a_password_may_be_empty() {
        let credentials = resolved(SaslExchange::begin(
            AuthMechanism::Plain,
            true,
            false,
            &b64(b"\0alice\0"),
        ));
        assert_eq!(credentials.authcid, "alice");
        assert!(credentials.password.is_empty());
    }

    #[test]
    fn the_decision_replies_carry_their_status_codes() {
        assert!(success_reply().starts_with(b"235"));
        assert!(credentials_invalid_reply().starts_with(b"535"));
    }
}
