use std::net::{Ipv4Addr, Ipv6Addr};

use crate::rec_mx::{DnsRecord, DnsRecordKind};

pub fn a_record(mail_host: &str, address: Ipv4Addr) -> DnsRecord {
    DnsRecord::new(DnsRecordKind::A, mail_host, "A", address.to_string())
}

pub fn aaaa_record(mail_host: &str, address: Ipv6Addr) -> DnsRecord {
    DnsRecord::new(DnsRecordKind::Aaaa, mail_host, "AAAA", address.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_a_record_carries_the_ipv4_address() {
        let record = a_record("mail.example.com", Ipv4Addr::new(192, 0, 2, 1));
        assert_eq!(record.kind, DnsRecordKind::A);
        assert_eq!(record.name, "mail.example.com");
        assert_eq!(record.record_type, "A");
        assert_eq!(record.value, "192.0.2.1");
    }

    #[test]
    fn the_aaaa_record_carries_the_ipv6_address() {
        let record = aaaa_record(
            "mail.example.com",
            Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1),
        );
        assert_eq!(record.kind, DnsRecordKind::Aaaa);
        assert_eq!(record.record_type, "AAAA");
        assert_eq!(record.value, "2001:db8::1");
    }
}
