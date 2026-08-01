use crate::rec_mx::{fqdn, DnsRecord, DnsRecordKind};

const SERVICES: &[(&str, u16)] = &[
    ("_imaps._tcp", 993),
    ("_submission._tcp", 587),
    ("_submissions._tcp", 465),
    ("_pop3s._tcp", 995),
    ("_autodiscover._tcp", 443),
    ("_jmap._tcp", 443),
    ("_caldavs._tcp", 443),
    ("_carddavs._tcp", 443),
];

const DISCOVERY_HOSTS: &[&str] = &["autoconfig", "autodiscover"];

const DAV_PATHS: &[(&str, &str)] = &[
    ("_caldavs._tcp", "/.well-known/caldav"),
    ("_carddavs._tcp", "/.well-known/carddav"),
];

pub fn autoconfig_records(domain: &str, mail_host: &str) -> Vec<DnsRecord> {
    let target = fqdn(mail_host);
    SERVICES
        .iter()
        .map(|(service, port)| {
            DnsRecord::new(
                DnsRecordKind::Autoconfig,
                format!("{service}.{domain}"),
                "SRV",
                format!("0 1 {port} {target}"),
            )
        })
        .chain(DAV_PATHS.iter().map(|(service, path)| {
            DnsRecord::new(
                DnsRecordKind::Autoconfig,
                format!("{service}.{domain}"),
                "TXT",
                format!("\"path={path}\""),
            )
        }))
        .chain(DISCOVERY_HOSTS.iter().map(|host| {
            DnsRecord::new(
                DnsRecordKind::Autoconfig,
                format!("{host}.{domain}"),
                "CNAME",
                target.clone(),
            )
        }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autoconfig_emits_one_srv_per_service() {
        let records = autoconfig_records("example.com", "mail.example.com");
        let srvs: Vec<_> = records.iter().filter(|r| r.record_type == "SRV").collect();
        assert_eq!(srvs.len(), SERVICES.len());

        let imaps = &records[0];
        assert_eq!(imaps.kind, DnsRecordKind::Autoconfig);
        assert_eq!(imaps.name, "_imaps._tcp.example.com");
        assert_eq!(imaps.value, "0 1 993 mail.example.com.");

        let autodiscover = records
            .iter()
            .find(|r| r.name.starts_with("_autodiscover"))
            .unwrap();
        assert_eq!(autodiscover.value, "0 1 443 mail.example.com.");
    }

    #[test]
    fn the_jmap_srv_points_at_the_https_port() {
        let records = autoconfig_records("example.com", "mail.example.com");
        let jmap = records
            .iter()
            .find(|r| r.name.starts_with("_jmap._tcp"))
            .unwrap();
        assert_eq!(jmap.record_type, "SRV");
        assert_eq!(jmap.name, "_jmap._tcp.example.com");
        assert_eq!(jmap.value, "0 1 443 mail.example.com.");
    }

    #[test]
    fn dav_discovery_gets_srv_and_path_records() {
        let records = autoconfig_records("example.com", "mail.example.com");
        for service in ["_caldavs._tcp", "_carddavs._tcp"] {
            let srv = records
                .iter()
                .find(|r| r.name == format!("{service}.example.com") && r.record_type == "SRV")
                .unwrap();
            assert_eq!(srv.value, "0 1 443 mail.example.com.");
        }
        let caldav_txt = records
            .iter()
            .find(|r| r.name == "_caldavs._tcp.example.com" && r.record_type == "TXT")
            .unwrap();
        assert_eq!(caldav_txt.value, "\"path=/.well-known/caldav\"");
        let carddav_txt = records
            .iter()
            .find(|r| r.name == "_carddavs._tcp.example.com" && r.record_type == "TXT")
            .unwrap();
        assert_eq!(carddav_txt.value, "\"path=/.well-known/carddav\"");
    }

    #[test]
    fn autoconfig_points_the_discovery_hosts_at_the_mail_host() {
        let records = autoconfig_records("example.com", "mail.example.com");
        let cnames: Vec<_> = records
            .iter()
            .filter(|r| r.record_type == "CNAME")
            .collect();
        assert_eq!(cnames.len(), 2);
        assert_eq!(cnames[0].name, "autoconfig.example.com");
        assert_eq!(cnames[0].value, "mail.example.com.");
        assert_eq!(cnames[1].name, "autodiscover.example.com");
        assert_eq!(cnames[1].value, "mail.example.com.");
        assert!(cnames.iter().all(|r| r.kind == DnsRecordKind::Autoconfig));
    }
}
