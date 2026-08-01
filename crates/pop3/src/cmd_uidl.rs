use crate::session::MessageEntry;

pub fn uidl_all(messages: &[MessageEntry]) -> String {
    let mut out = String::from("+OK\r\n");
    for message in messages.iter().filter(|message| !message.deleted) {
        out.push_str(&format!("{} {}\r\n", message.number, message.uid));
    }
    out.push_str(".\r\n");
    out
}

pub fn uidl_one(messages: &[MessageEntry], number: u32) -> String {
    match messages
        .iter()
        .find(|message| message.number == number && !message.deleted)
    {
        Some(message) => format!("+OK {} {}\r\n", message.number, message.uid),
        None => "-ERR no such message\r\n".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(number: u32, deleted: bool) -> MessageEntry {
        MessageEntry {
            number,
            size: 10,
            uid: format!("uid{number}"),
            document_id: number,
            deleted,
        }
    }

    #[test]
    fn an_empty_maildrop_lists_no_uids() {
        assert_eq!(uidl_all(&[]), "+OK\r\n.\r\n");
    }

    #[test]
    fn each_live_message_reports_its_uid() {
        let drop = vec![message(1, false), message(2, true), message(3, false)];
        let listing = uidl_all(&drop);
        assert!(listing.contains("1 uid1\r\n"));
        assert!(listing.contains("3 uid3\r\n"));
        assert!(!listing.contains("2 uid2"));
    }

    #[test]
    fn a_single_uid_is_reported() {
        let drop = vec![message(1, false)];
        assert_eq!(uidl_one(&drop, 1), "+OK 1 uid1\r\n");
        assert!(uidl_one(&drop, 2).starts_with("-ERR"));
    }
}
