use crate::rec_mx::{DnsRecord, DnsRecordKind};

pub fn dmarc_record(domain: &str) -> DnsRecord {
    DnsRecord::txt(
        DnsRecordKind::Dmarc,
        format!("_dmarc.{domain}"),
        format!("v=DMARC1; p=quarantine; rua=mailto:postmaster@{domain}; adkim=s; aspf=s"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dmarc_record_sits_at_the_dmarc_label_with_a_quarantine_policy() {
        let record = dmarc_record("example.com");
        assert_eq!(record.kind, DnsRecordKind::Dmarc);
        assert_eq!(record.name, "_dmarc.example.com");
        assert_eq!(
            record.value,
            "v=DMARC1; p=quarantine; rua=mailto:postmaster@example.com; adkim=s; aspf=s"
        );
    }
}
