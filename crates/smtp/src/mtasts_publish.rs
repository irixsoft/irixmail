use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use irixmail_dns::rec_mtasts::DEFAULT_MAX_AGE;
use irixmail_dns::{mtasts_policy, mtasts_record, DnsRecord, MtaStsMode};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedPolicy {
    pub id: String,
    pub body: String,
    pub record: DnsRecord,
}

pub fn publish(domain: &str, mail_host: &str) -> PublishedPolicy {
    publish_with(domain, mail_host, MtaStsMode::Enforce, DEFAULT_MAX_AGE)
}

pub fn publish_with(
    domain: &str,
    mail_host: &str,
    mode: MtaStsMode,
    max_age: u32,
) -> PublishedPolicy {
    let id = policy_id(mail_host, mode, max_age);
    let body = mtasts_policy(mail_host, mode, max_age);
    let record = mtasts_record(domain, &id);

    PublishedPolicy { id, body, record }
}

fn policy_id(mail_host: &str, mode: MtaStsMode, max_age: u32) -> String {
    let mut hasher = DefaultHasher::new();
    mode_token(mode).hash(&mut hasher);
    max_age.hash(&mut hasher);
    mail_host.hash(&mut hasher);
    hasher.finish().to_string()
}

fn mode_token(mode: MtaStsMode) -> u8 {
    match mode {
        MtaStsMode::Enforce => 0,
        MtaStsMode::Testing => 1,
        MtaStsMode::None => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use irixmail_dns::DnsRecordKind;

    #[test]
    fn the_published_policy_enforces_and_names_the_mail_host() {
        let published = publish("example.com", "mail.example.com");
        assert!(published.body.contains("mode: enforce"));
        assert!(published.body.contains("mx: mail.example.com"));
        assert!(published.body.starts_with("version: STSv1"));
    }

    #[test]
    fn the_record_sits_under_mta_sts_and_carries_the_policy_id() {
        let published = publish("example.com", "mail.example.com");
        assert_eq!(published.record.kind, DnsRecordKind::MtaSts);
        assert_eq!(published.record.name, "_mta-sts.example.com");
        assert_eq!(
            published.record.value,
            format!("v=STSv1; id={}", published.id)
        );
    }

    #[test]
    fn the_same_policy_yields_the_same_id() {
        let first = publish("example.com", "mail.example.com");
        let second = publish("example.com", "mail.example.com");
        assert_eq!(first.id, second.id);
    }

    #[test]
    fn a_changed_exchange_changes_the_id() {
        let first = publish("example.com", "mail.example.com");
        let second = publish("example.com", "mx.example.com");
        assert_ne!(first.id, second.id);
    }

    #[test]
    fn a_changed_mode_changes_the_id() {
        let enforce = publish_with(
            "example.com",
            "mail.example.com",
            MtaStsMode::Enforce,
            DEFAULT_MAX_AGE,
        );
        let testing = publish_with(
            "example.com",
            "mail.example.com",
            MtaStsMode::Testing,
            DEFAULT_MAX_AGE,
        );
        assert_ne!(enforce.id, testing.id);
    }

    #[test]
    fn a_changed_lifetime_changes_the_id() {
        let week = publish_with(
            "example.com",
            "mail.example.com",
            MtaStsMode::Enforce,
            DEFAULT_MAX_AGE,
        );
        let day = publish_with(
            "example.com",
            "mail.example.com",
            MtaStsMode::Enforce,
            86_400,
        );
        assert_ne!(week.id, day.id);
    }

    #[test]
    fn the_id_does_not_depend_on_the_domain_only_on_the_served_policy() {
        let one = publish("example.com", "mail.shared.net");
        let two = publish("example.org", "mail.shared.net");
        assert_eq!(one.id, two.id);
        assert_ne!(one.record.name, two.record.name);
    }
}
