use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressEntry {
    pub address: String,
    pub target: Target,
}

impl AddressEntry {
    pub fn account(address: impl Into<String>, account_id: u64) -> Self {
        AddressEntry {
            address: address.into().to_ascii_lowercase(),
            target: Target::Account { account_id },
        }
    }

    pub fn forward(address: impl Into<String>, destination: impl Into<String>) -> Self {
        AddressEntry {
            address: address.into().to_ascii_lowercase(),
            target: Target::Forward {
                destination: destination.into(),
            },
        }
    }

    pub fn reject(address: impl Into<String>) -> Self {
        AddressEntry {
            address: address.into().to_ascii_lowercase(),
            target: Target::Reject,
        }
    }

    pub fn account_id(&self) -> Option<u64> {
        match self.target {
            Target::Account { account_id } => Some(account_id),
            Target::Forward { .. } | Target::Reject => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum Target {
    Account { account_id: u64 },
    Forward { destination: String },
    Reject,
}

impl Target {
    pub fn is_local(&self) -> bool {
        matches!(self, Target::Account { .. })
    }

    pub fn is_rejected(&self) -> bool {
        matches!(self, Target::Reject)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_account_entry_lowercases_its_address_and_reports_the_account() {
        let entry = AddressEntry::account("Alice@IriXSoft.CoM", 7);
        assert_eq!(entry.address, "alice@irixsoft.com");
        assert_eq!(entry.target, Target::Account { account_id: 7 });
        assert_eq!(entry.account_id(), Some(7));
        assert!(entry.target.is_local());
        assert!(!entry.target.is_rejected());
    }

    #[test]
    fn a_forward_entry_lowercases_the_address_but_keeps_the_destination() {
        let entry = AddressEntry::forward("Info@irixsoft.com", "Owner@Example.Org");
        assert_eq!(entry.address, "info@irixsoft.com");
        assert_eq!(
            entry.target,
            Target::Forward {
                destination: "Owner@Example.Org".to_string(),
            }
        );
        assert_eq!(entry.account_id(), None);
        assert!(!entry.target.is_local());
        assert!(!entry.target.is_rejected());
    }

    #[test]
    fn a_reject_entry_reports_rejection_and_no_account() {
        let entry = AddressEntry::reject("Blocked@irixsoft.com");
        assert_eq!(entry.address, "blocked@irixsoft.com");
        assert_eq!(entry.target, Target::Reject);
        assert_eq!(entry.account_id(), None);
        assert!(!entry.target.is_local());
        assert!(entry.target.is_rejected());
    }

    #[test]
    fn each_target_kind_round_trips_through_json() {
        for entry in [
            AddressEntry::account("alice@irixsoft.com", 7),
            AddressEntry::forward("info@irixsoft.com", "owner@example.org"),
            AddressEntry::reject("blocked@irixsoft.com"),
        ] {
            let encoded = serde_json::to_string(&entry).expect("entry serializes");
            let decoded: AddressEntry = serde_json::from_str(&encoded).expect("entry deserializes");
            assert_eq!(decoded, entry);
        }
    }

    #[test]
    fn the_target_kind_is_tagged_in_json() {
        let encoded = serde_json::to_string(&AddressEntry::account("alice@irixsoft.com", 7))
            .expect("entry serializes");
        assert!(encoded.contains("\"kind\":\"account\""));

        let encoded = serde_json::to_string(&AddressEntry::reject("blocked@irixsoft.com"))
            .expect("entry serializes");
        assert!(encoded.contains("\"kind\":\"reject\""));
    }
}
