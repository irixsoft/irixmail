use crate::parser::Token;

pub const PRIVACY_REQUIRED: &str = "[PRIVACYREQUIRED] LOGIN is disabled on a cleartext connection";
pub const AUTH_FAILED: &str = "[AUTHENTICATIONFAILED] credentials rejected";
pub const THROTTLED: &str = "[LIMIT] Too many failed authentication attempts";
pub const COMPLETED: &str = "LOGIN completed";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginCredentials {
    pub username: String,
    pub password: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginError {
    MissingArguments,
}

pub fn parse_login(args: &[Token]) -> Result<LoginCredentials, LoginError> {
    let username = args
        .first()
        .and_then(Token::as_str)
        .ok_or(LoginError::MissingArguments)?;
    let password = args
        .get(1)
        .and_then(Token::as_str)
        .ok_or(LoginError::MissingArguments)?;
    Ok(LoginCredentials {
        username: username.to_string(),
        password: password.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_atom_arguments_become_the_credentials() {
        let args = vec![
            Token::Atom("alice@example.com".into()),
            Token::Atom("secret".into()),
        ];
        assert_eq!(
            parse_login(&args),
            Ok(LoginCredentials {
                username: "alice@example.com".into(),
                password: "secret".into(),
            })
        );
    }

    #[test]
    fn quoted_arguments_are_accepted() {
        let args = vec![
            Token::Quoted("alice@example.com".into()),
            Token::Quoted("a secret".into()),
        ];
        let creds = parse_login(&args).unwrap();
        assert_eq!(creds.password, "a secret");
    }

    #[test]
    fn a_missing_password_is_an_error() {
        let args = vec![Token::Atom("alice@example.com".into())];
        assert_eq!(parse_login(&args), Err(LoginError::MissingArguments));
    }

    #[test]
    fn no_arguments_is_an_error() {
        assert_eq!(parse_login(&[]), Err(LoginError::MissingArguments));
    }
}
