use crate::rec_mx::{DnsRecord, DnsRecordKind};

pub fn spf_record(domain: &str, mail_host: &str) -> DnsRecord {
    DnsRecord::txt(
        DnsRecordKind::Spf,
        domain,
        format!("v=spf1 a:{mail_host} mx -all"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spf_authorizes_the_mail_host_and_hard_fails_the_rest() {
        let record = spf_record("example.com", "mail.example.com");
        assert_eq!(record.kind, DnsRecordKind::Spf);
        assert_eq!(record.name, "example.com");
        assert_eq!(record.record_type, "TXT");
        assert_eq!(record.value, "v=spf1 a:mail.example.com mx -all");
    }
}
