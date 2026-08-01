use axum::http::StatusCode;
use axum::response::Response;

use crate::app::error_response;

pub fn is_valid_email(email: &str) -> bool {
    let Some((local, domain)) = email.rsplit_once('@') else {
        return false;
    };
    !local.is_empty() && !local.contains(char::is_whitespace) && is_valid_domain(domain)
}

pub fn is_valid_domain(domain: &str) -> bool {
    domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
        && !domain.contains(char::is_whitespace)
}

pub fn require_field<'a>(value: Option<&'a str>, name: &str) -> Result<&'a str, String> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(format!("{name} is required")),
    }
}

pub fn parse_id(raw: &str) -> Option<u64> {
    raw.parse::<u64>().ok()
}

pub fn bad_request(message: &str) -> Response {
    error_response(StatusCode::BAD_REQUEST, message)
}

pub fn unprocessable(message: &str) -> Response {
    error_response(StatusCode::UNPROCESSABLE_ENTITY, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_formed_addresses_are_accepted() {
        assert!(is_valid_email("alice@example.com"));
        assert!(is_valid_email("a.b+tag@mail.example.co"));
    }

    #[test]
    fn malformed_addresses_are_rejected() {
        assert!(!is_valid_email("no-at-sign"));
        assert!(!is_valid_email("@example.com"));
        assert!(!is_valid_email("alice@localhost"));
        assert!(!is_valid_email("alice@exa mple.com"));
        assert!(!is_valid_email("alice@.com"));
        assert!(!is_valid_email("alice@example."));
        assert!(!is_valid_email("a b@example.com"));
    }

    #[test]
    fn domains_need_a_dot() {
        assert!(is_valid_domain("example.com"));
        assert!(!is_valid_domain("localhost"));
        assert!(!is_valid_domain("a..b.com"));
    }

    #[test]
    fn required_fields_are_checked() {
        assert_eq!(require_field(Some("x"), "name"), Ok("x"));
        assert!(require_field(Some("  "), "name").is_err());
        assert!(require_field(None, "name").is_err());
    }

    #[test]
    fn ids_round_trip_past_the_javascript_safe_integer() {
        assert_eq!(
            parse_id("340282366920938463"),
            Some(340_282_366_920_938_463)
        );
        assert_eq!(parse_id("not-a-number"), None);
        assert_eq!(parse_id("-1"), None);
        assert_eq!(parse_id(""), None);
    }

    #[test]
    fn the_error_helpers_carry_their_status() {
        assert_eq!(bad_request("x").status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            unprocessable("x").status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
}
