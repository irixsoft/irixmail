#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QuotaLimitsView {
    pub used_bytes: u64,
    pub byte_limit: Option<u64>,
    pub used_messages: u64,
    pub message_limit: Option<u64>,
}

pub fn quotaroot_line(mailbox: &str) -> String {
    format!("* QUOTAROOT {} \"\"\r\n", crate::cmd_list::quoted(mailbox))
}

pub fn quota_line(view: &QuotaLimitsView) -> String {
    let mut resources = Vec::new();
    if let Some(limit) = view.byte_limit {
        resources.push(format!(
            "STORAGE {} {}",
            view.used_bytes / 1024,
            limit / 1024
        ));
    }
    if let Some(limit) = view.message_limit {
        resources.push(format!("MESSAGE {} {}", view.used_messages, limit));
    }
    format!("* QUOTA \"\" ({})\r\n", resources.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_resources_render_in_kib_and_counts() {
        let view = QuotaLimitsView {
            used_bytes: 2048,
            byte_limit: Some(102_400),
            used_messages: 2,
            message_limit: Some(10),
        };
        assert_eq!(
            quota_line(&view),
            "* QUOTA \"\" (STORAGE 2 100 MESSAGE 2 10)\r\n"
        );
    }

    #[test]
    fn an_unlimited_account_renders_an_empty_list() {
        let view = QuotaLimitsView {
            used_bytes: 2048,
            byte_limit: None,
            used_messages: 2,
            message_limit: None,
        };
        assert_eq!(quota_line(&view), "* QUOTA \"\" ()\r\n");
    }

    #[test]
    fn the_quotaroot_line_names_the_mailbox_and_root() {
        assert_eq!(quotaroot_line("INBOX"), "* QUOTAROOT \"INBOX\" \"\"\r\n");
    }
}
