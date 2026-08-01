use crate::session::MessageEntry;

pub fn dele(messages: &mut [MessageEntry], number: u32) -> String {
    match messages.iter_mut().find(|message| message.number == number) {
        Some(message) if !message.deleted => {
            message.deleted = true;
            format!("+OK message {number} deleted\r\n")
        }
        Some(_) => format!("-ERR message {number} already deleted\r\n"),
        None => "-ERR no such message\r\n".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(number: u32) -> MessageEntry {
        MessageEntry {
            number,
            size: 10,
            uid: format!("uid{number}"),
            document_id: number,
            deleted: false,
        }
    }

    #[test]
    fn a_live_message_is_marked_deleted() {
        let mut drop = vec![message(1), message(2)];
        assert!(dele(&mut drop, 1).starts_with("+OK"));
        assert!(drop[0].deleted);
        assert!(!drop[1].deleted);
    }

    #[test]
    fn deleting_twice_is_an_error() {
        let mut drop = vec![message(1)];
        dele(&mut drop, 1);
        assert!(dele(&mut drop, 1).contains("already deleted"));
    }

    #[test]
    fn an_unknown_message_is_an_error() {
        let mut drop = vec![message(1)];
        assert!(dele(&mut drop, 9).starts_with("-ERR no such message"));
    }
}
