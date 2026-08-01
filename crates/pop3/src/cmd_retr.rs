pub fn no_such_message() -> &'static [u8] {
    b"-ERR no such message\r\n"
}

pub fn wire_body(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len());
    let mut at_line_start = true;
    let mut last = 0u8;
    for &byte in body {
        if byte == b'\n' && last != b'\r' {
            out.push(b'\r');
        }
        if at_line_start && byte == b'.' {
            out.push(b'.');
        }
        out.push(byte);
        at_line_start = byte == b'\n';
        last = byte;
    }
    out
}

pub fn retr_response(size: u64, body: &[u8]) -> Vec<u8> {
    let mut out = format!("+OK {size} octets\r\n").into_bytes();
    let stuffed = wire_body(body);
    let terminated = stuffed.ends_with(b"\r\n");
    out.extend_from_slice(&stuffed);
    if !terminated {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b".\r\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leading_dot_is_doubled() {
        assert_eq!(wire_body(b".hidden\r\n"), b"..hidden\r\n");
        assert_eq!(wire_body(b"line\r\n.dot\r\n"), b"line\r\n..dot\r\n");
    }

    #[test]
    fn ordinary_bytes_are_untouched() {
        assert_eq!(wire_body(b"no dots here\r\n"), b"no dots here\r\n");
    }

    #[test]
    fn bare_lf_becomes_crlf_and_existing_crlf_is_untouched() {
        assert_eq!(wire_body(b"one\ntwo\n"), b"one\r\ntwo\r\n");
        assert_eq!(wire_body(b"one\r\ntwo\n"), b"one\r\ntwo\r\n");
        assert_eq!(wire_body(b"\n.dot\n"), b"\r\n..dot\r\n");
    }

    #[test]
    fn the_response_frames_size_and_terminator() {
        let response = retr_response(5, b"hello");
        assert!(response.starts_with(b"+OK 5 octets\r\n"));
        assert!(response.ends_with(b"hello\r\n.\r\n"));
    }

    #[test]
    fn an_already_terminated_body_is_not_double_wrapped() {
        let response = retr_response(7, b"hello\r\n");
        assert!(response.ends_with(b"hello\r\n.\r\n"));
    }

    #[test]
    fn the_error_is_an_err() {
        assert!(no_such_message().starts_with(b"-ERR"));
    }
}
