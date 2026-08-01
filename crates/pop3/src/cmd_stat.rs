use crate::session::MessageEntry;

pub fn stat_response(messages: &[MessageEntry]) -> String {
    let count = messages.iter().filter(|message| !message.deleted).count();
    let total: u64 = messages
        .iter()
        .filter(|message| !message.deleted)
        .map(|message| message.size)
        .sum();
    format!("+OK {count} {total}\r\n")
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
    fn an_empty_maildrop_reports_zero() {
        assert_eq!(stat_response(&[]), "+OK 0 0\r\n");
    }

    #[test]
    fn live_messages_are_counted_and_summed() {
        let drop = vec![message(1, 100, false), message(2, 250, false)];
        assert_eq!(stat_response(&drop), "+OK 2 350\r\n");
    }

    #[test]
    fn deleted_messages_are_excluded() {
        let drop = vec![message(1, 100, true), message(2, 250, false)];
        assert_eq!(stat_response(&drop), "+OK 1 250\r\n");
    }
}
