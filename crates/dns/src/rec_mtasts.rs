use crate::rec_mx::{fqdn, DnsRecord, DnsRecordKind};

pub const DEFAULT_MAX_AGE: u32 = 604_800;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MtaStsMode {
    Enforce,
    Testing,
    None,
}

impl MtaStsMode {
    fn as_str(self) -> &'static str {
        match self {
            MtaStsMode::Enforce => "enforce",
            MtaStsMode::Testing => "testing",
            MtaStsMode::None => "none",
        }
    }
}

pub fn mtasts_record(domain: &str, id: &str) -> DnsRecord {
    DnsRecord::txt(
        DnsRecordKind::MtaSts,
        format!("_mta-sts.{domain}"),
        format!("v=STSv1; id={id}"),
    )
}

pub fn mtasts_host_record(domain: &str, mail_host: &str) -> DnsRecord {
    DnsRecord::new(
        DnsRecordKind::MtaSts,
        format!("mta-sts.{domain}"),
        "CNAME",
        fqdn(mail_host),
    )
}

pub fn mtasts_policy(mail_host: &str, mode: MtaStsMode, max_age: u32) -> String {
    format!(
        "version: STSv1\nmode: {}\nmx: {mail_host}\nmax_age: {max_age}\n",
        mode.as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_record_carries_the_policy_id() {
        let record = mtasts_record("example.com", "20260101000000");
        assert_eq!(record.kind, DnsRecordKind::MtaSts);
        assert_eq!(record.name, "_mta-sts.example.com");
        assert_eq!(record.value, "v=STSv1; id=20260101000000");
    }

    #[test]
    fn the_host_record_points_the_policy_host_at_the_mail_host() {
        let record = mtasts_host_record("example.com", "mail.example.com");
        assert_eq!(record.kind, DnsRecordKind::MtaSts);
        assert_eq!(record.name, "mta-sts.example.com");
        assert_eq!(record.record_type, "CNAME");
        assert_eq!(record.value, "mail.example.com.");
    }

    #[test]
    fn the_policy_lists_the_mode_and_mx() {
        let policy = mtasts_policy("mail.example.com", MtaStsMode::Enforce, DEFAULT_MAX_AGE);
        assert_eq!(
            policy,
            "version: STSv1\nmode: enforce\nmx: mail.example.com\nmax_age: 604800\n"
        );
    }
}
