use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: u64,
    pub local_part: String,
    pub domain_id: u64,
    pub display_name: String,
    pub enabled: bool,
    pub role: Role,
    pub aliases: Vec<String>,
    pub forwarding: Forwarding,
    pub quota_bytes: u64,
    pub quota_messages: u64,
    pub locale: String,
    pub timezone: String,
    pub signature: String,
    pub vacation: VacationResponder,
    pub created_at: u64,
}

impl Account {
    pub fn is_active(&self) -> bool {
        self.enabled
    }

    pub fn is_admin(&self) -> bool {
        matches!(self.role, Role::Admin)
    }

    pub fn byte_quota(&self) -> Option<u64> {
        (self.quota_bytes != 0).then_some(self.quota_bytes)
    }

    pub fn message_quota(&self) -> Option<u64> {
        (self.quota_messages != 0).then_some(self.quota_messages)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    Admin,
    #[default]
    User,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Forwarding {
    pub destinations: Vec<String>,
    pub keep_local_copy: bool,
}

impl Forwarding {
    pub fn is_active(&self) -> bool {
        !self.destinations.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VacationResponder {
    pub enabled: bool,
    pub subject: String,
    pub body: String,
    pub active_from: Option<u64>,
    pub active_to: Option<u64>,
}

impl VacationResponder {
    pub fn is_active_at(&self, now: u64) -> bool {
        if !self.enabled {
            return false;
        }
        if let Some(from) = self.active_from {
            if now < from {
                return false;
            }
        }
        if let Some(to) = self.active_to {
            if now > to {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_account() -> Account {
        Account {
            id: 7,
            local_part: "alice".to_string(),
            domain_id: 42,
            display_name: "Alice Adams".to_string(),
            enabled: true,
            role: Role::User,
            aliases: Vec::new(),
            forwarding: Forwarding::default(),
            quota_bytes: 0,
            quota_messages: 0,
            locale: String::new(),
            timezone: String::new(),
            signature: String::new(),
            vacation: VacationResponder::default(),
            created_at: 1_700_000_000_000,
        }
    }

    #[test]
    fn a_disabled_account_is_not_active() {
        let mut account = sample_account();
        assert!(account.is_active());
        account.enabled = false;
        assert!(!account.is_active());
    }

    #[test]
    fn admin_role_reports_admin_and_user_does_not() {
        let mut account = sample_account();
        assert!(!account.is_admin());
        account.role = Role::Admin;
        assert!(account.is_admin());
    }

    #[test]
    fn role_defaults_to_user() {
        assert_eq!(Role::default(), Role::User);
    }

    #[test]
    fn a_zero_quota_means_unlimited() {
        let mut account = sample_account();
        assert_eq!(account.byte_quota(), None);
        assert_eq!(account.message_quota(), None);
        account.quota_bytes = 1024;
        account.quota_messages = 50;
        assert_eq!(account.byte_quota(), Some(1024));
        assert_eq!(account.message_quota(), Some(50));
    }

    #[test]
    fn forwarding_is_active_only_with_a_destination() {
        let mut forwarding = Forwarding::default();
        assert!(!forwarding.is_active());
        forwarding
            .destinations
            .push("alice@example.org".to_string());
        forwarding.keep_local_copy = true;
        assert!(forwarding.is_active());
    }

    #[test]
    fn a_disabled_responder_never_fires() {
        let responder = VacationResponder {
            enabled: false,
            active_from: None,
            active_to: None,
            ..VacationResponder::default()
        };
        assert!(!responder.is_active_at(1_700_000_000_000));
    }

    #[test]
    fn an_enabled_responder_without_a_window_is_always_active() {
        let responder = VacationResponder {
            enabled: true,
            ..VacationResponder::default()
        };
        assert!(responder.is_active_at(0));
        assert!(responder.is_active_at(u64::MAX));
    }

    #[test]
    fn a_windowed_responder_fires_only_inside_the_window() {
        let responder = VacationResponder {
            enabled: true,
            active_from: Some(100),
            active_to: Some(200),
            ..VacationResponder::default()
        };
        assert!(!responder.is_active_at(99));
        assert!(responder.is_active_at(100));
        assert!(responder.is_active_at(150));
        assert!(responder.is_active_at(200));
        assert!(!responder.is_active_at(201));
    }

    #[test]
    fn an_account_round_trips_through_json() {
        let mut account = sample_account();
        account.role = Role::Admin;
        account.aliases = vec!["a.adams@irixsoft.com".to_string()];
        account.forwarding = Forwarding {
            destinations: vec!["alice@example.org".to_string()],
            keep_local_copy: true,
        };
        account.quota_bytes = 10_000_000;
        account.quota_messages = 5_000;
        account.locale = "en-US".to_string();
        account.timezone = "Europe/Berlin".to_string();
        account.signature = "-- Alice".to_string();
        account.vacation = VacationResponder {
            enabled: true,
            subject: "Out of office".to_string(),
            body: "Back next week.".to_string(),
            active_from: Some(1_700_000_500_000),
            active_to: Some(1_700_100_000_000),
        };

        let encoded = serde_json::to_string(&account).expect("account serializes");
        let decoded: Account = serde_json::from_str(&encoded).expect("account deserializes");
        assert_eq!(decoded, account);
    }
}
