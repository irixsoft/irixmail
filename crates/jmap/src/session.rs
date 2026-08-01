use serde_json::{json, Value};

pub const CORE: &str = "urn:ietf:params:jmap:core";
pub const MAIL: &str = "urn:ietf:params:jmap:mail";
pub const SUBMISSION: &str = "urn:ietf:params:jmap:submission";
pub const VACATION: &str = "urn:ietf:params:jmap:vacationresponse";
pub const CALENDARS: &str = "urn:ietf:params:jmap:calendars";
pub const CONTACTS: &str = "urn:ietf:params:jmap:contacts";
pub const WEBPUSH: &str = "urn:irixmail:webpush";

pub const MAX_SIZE_REQUEST: usize = 10_000_000;
pub const MAX_SIZE_UPLOAD: usize = 50_000_000;
pub const MAX_OBJECTS_IN_GET: usize = 500;
pub const MAX_OBJECTS_IN_SET: usize = 500;
pub const MAX_MAILBOX_DEPTH: usize = 10;
pub const MAX_SIZE_MAILBOX_NAME: usize = 255;
pub const MAX_SIZE_ATTACHMENTS: usize = 50_000_000;

pub fn unknown_capability(using: &[String]) -> Option<&str> {
    using
        .iter()
        .find(|uri| {
            ![
                CORE, MAIL, SUBMISSION, VACATION, CALENDARS, CONTACTS, WEBPUSH,
            ]
            .contains(&uri.as_str())
        })
        .map(String::as_str)
}

pub fn session_resource(
    account_id: &str,
    username: &str,
    state: &str,
    webpush_key: Option<&str>,
) -> Value {
    let mut resource = json!({
        "capabilities": {
            CORE: {
                "maxSizeUpload": MAX_SIZE_UPLOAD,
                "maxConcurrentUpload": 4,
                "maxSizeRequest": MAX_SIZE_REQUEST,
                "maxConcurrentRequests": 4,
                "maxCallsInRequest": 16,
                "maxObjectsInGet": MAX_OBJECTS_IN_GET,
                "maxObjectsInSet": MAX_OBJECTS_IN_SET,
                "collationAlgorithms": ["i;ascii-casemap", "i;unicode-casemap"]
            },
            MAIL: {
                "maxMailboxesPerEmail": 10,
                "maxMailboxDepth": MAX_MAILBOX_DEPTH,
                "maxSizeMailboxName": MAX_SIZE_MAILBOX_NAME,
                "maxSizeAttachmentsPerEmail": MAX_SIZE_ATTACHMENTS,
                "emailQuerySortOptions": ["receivedAt", "sentAt", "size"],
                "mayCreateTopLevelMailbox": true
            },
            SUBMISSION: {
                "maxDelayedSend": 0,
                "submissionExtensions": {}
            },
            VACATION: {},
            CALENDARS: {},
            CONTACTS: {},
            WEBPUSH: {
                "applicationServerKey": webpush_key,
            }
        },
        "accounts": {
            account_id: {
                "name": username,
                "isPersonal": true,
                "isReadOnly": false,
                "accountCapabilities": {
                    CORE: {},
                    MAIL: {},
                    SUBMISSION: {},
                    VACATION: {},
                    CALENDARS: {},
                    CONTACTS: {}
                }
            }
        },
        "primaryAccounts": {
            MAIL: account_id,
            SUBMISSION: account_id,
            VACATION: account_id,
            CALENDARS: account_id,
            CONTACTS: account_id
        },
        "username": username,
        "apiUrl": "/jmap/",
        "downloadUrl": "/jmap/download/{accountId}/{blobId}/{name}?accept={type}",
        "uploadUrl": "/jmap/upload/{accountId}/",
        "eventSourceUrl": "/jmap/eventsource?types={types}&closeafter={closeafter}&ping={ping}",
        "state": state
    });
    if webpush_key.is_none() {
        if let Some(capabilities) = resource["capabilities"].as_object_mut() {
            capabilities.remove(WEBPUSH);
        }
    }
    resource
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_core_capabilities_are_advertised() {
        let resource = session_resource("a1", "alice@example.com", "s0", None);
        assert!(resource["capabilities"][CORE]["maxCallsInRequest"].is_number());
        assert!(resource["capabilities"][MAIL]["maxMailboxesPerEmail"].is_number());
        assert!(resource["capabilities"][SUBMISSION].is_object());
        assert!(resource["capabilities"][VACATION].is_object());
        assert!(resource["capabilities"][CALENDARS].is_object());
        assert!(resource["capabilities"][CONTACTS].is_object());
    }

    #[test]
    fn the_calendar_and_contact_capabilities_are_accepted_and_primary() {
        let resource = session_resource("a1", "alice@example.com", "s0", None);
        assert_eq!(resource["primaryAccounts"][CALENDARS], "a1");
        assert_eq!(resource["primaryAccounts"][CONTACTS], "a1");
        assert!(resource["accounts"]["a1"]["accountCapabilities"][CALENDARS].is_object());
        assert!(resource["accounts"]["a1"]["accountCapabilities"][CONTACTS].is_object());
        assert_eq!(
            unknown_capability(&[CALENDARS.to_string(), CONTACTS.to_string()]),
            None
        );
    }

    #[test]
    fn the_account_is_listed_and_primary() {
        let resource = session_resource("a1", "alice@example.com", "s0", None);
        assert_eq!(resource["accounts"]["a1"]["name"], "alice@example.com");
        assert_eq!(resource["accounts"]["a1"]["isPersonal"], true);
        assert_eq!(resource["primaryAccounts"][MAIL], "a1");
    }

    #[test]
    fn the_endpoints_and_state_are_present() {
        let resource = session_resource("a1", "alice@example.com", "state-9", None);
        assert_eq!(resource["apiUrl"], "/jmap/");
        assert!(resource["downloadUrl"]
            .as_str()
            .unwrap()
            .contains("{blobId}"));
        assert!(resource["uploadUrl"]
            .as_str()
            .unwrap()
            .contains("{accountId}"));
        assert!(resource["eventSourceUrl"]
            .as_str()
            .unwrap()
            .contains("eventsource"));
        assert_eq!(resource["state"], "state-9");
    }

    #[test]
    fn the_request_size_limit_is_named_max_size_request() {
        let resource = session_resource("a1", "alice@example.com", "s0", None);
        assert!(resource["capabilities"][CORE]["maxSizeRequest"].is_number());
        assert!(resource["capabilities"][CORE]
            .as_object()
            .unwrap()
            .get("maxSizeRequestObject")
            .is_none());
    }

    #[test]
    fn the_download_url_template_carries_the_type_variable() {
        let resource = session_resource("a1", "alice@example.com", "s0", None);
        assert!(resource["downloadUrl"].as_str().unwrap().contains("{type}"));
    }

    #[test]
    fn an_unsupported_capability_is_detected() {
        assert_eq!(
            unknown_capability(&["urn:bogus".to_string()]),
            Some("urn:bogus")
        );
        assert_eq!(
            unknown_capability(&[CORE.to_string(), MAIL.to_string()]),
            None
        );
    }

    #[test]
    fn the_username_is_echoed() {
        let resource = session_resource("a1", "bob@example.com", "s", None);
        assert_eq!(resource["username"], "bob@example.com");
    }
}
