use std::fmt;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("storage error: {0}")]
    Store(String),

    #[error("serialization error: {0}")]
    Serialize(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl Error {
    pub fn config(detail: impl fmt::Display) -> Self {
        Self::Config(detail.to_string())
    }

    pub fn store(detail: impl fmt::Display) -> Self {
        Self::Store(detail.to_string())
    }

    pub fn serialize(detail: impl fmt::Display) -> Self {
        Self::Serialize(detail.to_string())
    }

    pub fn not_found(detail: impl fmt::Display) -> Self {
        Self::NotFound(detail.to_string())
    }

    pub fn invalid_input(detail: impl fmt::Display) -> Self {
        Self::InvalidInput(detail.to_string())
    }

    pub fn unauthorized(detail: impl fmt::Display) -> Self {
        Self::Unauthorized(detail.to_string())
    }

    pub fn forbidden(detail: impl fmt::Display) -> Self {
        Self::Forbidden(detail.to_string())
    }

    pub fn protocol(detail: impl fmt::Display) -> Self {
        Self::Protocol(detail.to_string())
    }

    pub fn internal(detail: impl fmt::Display) -> Self {
        Self::Internal(detail.to_string())
    }

    pub fn is_client_error(&self) -> bool {
        matches!(
            self,
            Error::NotFound(_)
                | Error::InvalidInput(_)
                | Error::Unauthorized(_)
                | Error::Forbidden(_)
                | Error::Protocol(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_category_and_detail() {
        let err = Error::config("missing hostname");
        assert_eq!(err.to_string(), "configuration error: missing hostname");

        let err = Error::not_found("account 42");
        assert_eq!(err.to_string(), "not found: account 42");
    }

    #[test]
    fn io_errors_convert_via_from() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");
        let err: Error = io.into();
        assert!(matches!(err, Error::Io(_)));
        assert!(err.to_string().contains("no such file"));
    }

    #[test]
    fn constructors_accept_any_display() {
        let err = Error::store(format_args!("rocksdb code {}", 7));
        assert_eq!(err.to_string(), "storage error: rocksdb code 7");

        let err = Error::internal(42);
        assert_eq!(err.to_string(), "internal error: 42");
    }

    #[test]
    fn client_errors_are_classified() {
        assert!(Error::not_found("x").is_client_error());
        assert!(Error::invalid_input("x").is_client_error());
        assert!(Error::unauthorized("x").is_client_error());
        assert!(Error::forbidden("x").is_client_error());
        assert!(Error::protocol("x").is_client_error());

        assert!(!Error::config("x").is_client_error());
        assert!(!Error::store("x").is_client_error());
        assert!(!Error::internal("x").is_client_error());
        let io = std::io::Error::other("boom");
        assert!(!Error::from(io).is_client_error());
    }

    #[test]
    fn result_alias_defaults_to_crate_error() {
        fn fallible(ok: bool) -> Result<u32> {
            if ok {
                Ok(7)
            } else {
                Err(Error::invalid_input("nope"))
            }
        }

        assert_eq!(fallible(true).unwrap(), 7);
        assert!(fallible(false).is_err());
    }
}
