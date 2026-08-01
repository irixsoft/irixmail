use crate::dkim_keys::DkimKey;
use crate::rec_mx::{DnsRecord, DnsRecordKind};

pub fn dkim_record(domain: &str, key: &DkimKey) -> DnsRecord {
    DnsRecord::txt(
        DnsRecordKind::Dkim,
        format!("{}._domainkey.{domain}", key.selector),
        format!(
            "v=DKIM1; k={}; p={}",
            key.algorithm.record_tag(),
            key.public_key_b64
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dkim_keys::generate_ed25519;

    #[test]
    fn dkim_record_publishes_the_key_at_the_selector() {
        let key = generate_ed25519("mail").unwrap();
        let record = dkim_record("example.com", &key);
        assert_eq!(record.kind, DnsRecordKind::Dkim);
        assert_eq!(record.name, "mail._domainkey.example.com");
        assert!(record.value.starts_with("v=DKIM1; k=ed25519; p="));
        assert!(record.value.contains(&key.public_key_b64));
    }
}
