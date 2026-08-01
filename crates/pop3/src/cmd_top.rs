use crate::cmd_retr::wire_body;

pub fn take_lines(body: &[u8], count: usize) -> Vec<u8> {
    if count == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut seen = 0;
    for &byte in body {
        out.push(byte);
        if byte == b'\n' {
            seen += 1;
            if seen >= count {
                break;
            }
        }
    }
    out
}

pub fn top_response(headers: &[u8], body: &[u8], lines: usize) -> Vec<u8> {
    let mut out = b"+OK\r\n".to_vec();
    out.extend_from_slice(&wire_body(headers));
    out.extend_from_slice(&wire_body(&take_lines(body, lines)));
    if !out.ends_with(b"\r\n") {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b".\r\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_lines_returns_nothing() {
        assert!(take_lines(b"a\r\nb\r\n", 0).is_empty());
    }

    #[test]
    fn only_the_requested_number_of_lines_are_kept() {
        assert_eq!(take_lines(b"a\r\nb\r\nc\r\n", 2), b"a\r\nb\r\n");
        assert_eq!(take_lines(b"a\r\nb\r\n", 5), b"a\r\nb\r\n");
    }

    #[test]
    fn the_response_frames_headers_and_body() {
        let response = top_response(b"Subject: hi\r\n", b"line1\r\nline2\r\n", 1);
        assert!(response.starts_with(b"+OK\r\n"));
        assert!(response.windows(11).any(|w| w == b"Subject: hi"));
        assert!(response.windows(5).any(|w| w == b"line1"));
        assert!(!response.windows(5).any(|w| w == b"line2"));
        assert!(response.ends_with(b".\r\n"));
    }
}
