use crate::rec_mx::{DnsRecord, DnsRecordKind};

pub fn tlsrpt_record(domain: &str) -> DnsRecord {
    DnsRecord::txt(
        DnsRecordKind::TlsRpt,
        format!("_smtp._tls.{domain}"),
        format!("v=TLSRPTv1; rua=mailto:tlsrpt@{domain}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tlsrpt_record_sits_at_the_smtp_tls_label() {
        let record = tlsrpt_record("example.com");
        assert_eq!(record.kind, DnsRecordKind::TlsRpt);
        assert_eq!(record.name, "_smtp._tls.example.com");
        assert_eq!(record.value, "v=TLSRPTv1; rua=mailto:tlsrpt@example.com");
    }
}
