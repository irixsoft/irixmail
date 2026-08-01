use crate::session::MessageEntry;

pub fn list_all(messages: &[MessageEntry]) -> String {
    let live: Vec<&MessageEntry> = messages.iter().filter(|message| !message.deleted).collect();
    let total: u64 = live.iter().map(|message| message.size).sum();
    let mut out = format!("+OK {} messages ({total} octets)\r\n", live.len());
    for message in &live {
        out.push_str(&format!("{} {}\r\n", message.number, message.size));
    }
    out.push_str(".\r\n");
    out
}

pub fn list_one(messages: &[MessageEntry], number: u32) -> String {
    match messages
        .iter()
        .find(|message| message.number == number && !message.deleted)
    {
        Some(message) => format!("+OK {} {}\r\n", message.number, message.size),
        None => "-ERR no such message\r\n".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(number: u32, size: u64, deleted: bool) -> MessageEntry {
        MessageEntry {
            number,
            size,
            uid: format!("uid{number}"),
            document_id: number,
            deleted,
        }
    }

    #[test]
    fn an_empty_maildrop_lists_nothing() {
        let listing = list_all(&[]);
        assert!(listing.starts_with("+OK 0 messages (0 octets)\r\n"));
        assert!(listing.ends_with(".\r\n"));
    }

    #[test]
    fn each_live_message_gets_a_line() {
        let drop = vec![
            message(1, 100, false),
            message(2, 200, true),
            message(3, 50, false),
        ];
        let listing = list_all(&drop);
        assert!(listing.contains("1 100\r\n"));
        assert!(listing.contains("3 50\r\n"));
        assert!(!listing.contains("2 200"));
        assert!(listing.starts_with("+OK 2 messages (150 octets)\r\n"));
    }

    #[test]
    fn a_single_message_is_reported() {
        let drop = vec![message(1, 100, false)];
        assert_eq!(list_one(&drop, 1), "+OK 1 100\r\n");
    }

    #[test]
    fn an_unknown_or_deleted_message_is_an_error() {
        let drop = vec![message(1, 100, true)];
        assert!(list_one(&drop, 1).starts_with("-ERR"));
        assert!(list_one(&drop, 9).starts_with("-ERR"));
    }
}
