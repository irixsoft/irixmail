use irixmail_store::Collection;
use serde_json::{json, Value};

pub fn state_change_single(account_id: &str, type_name: &str, state: &str) -> Value {
    json!({
        "@type": "StateChange",
        "changed": {
            account_id: {
                type_name: state,
            }
        }
    })
}

pub fn type_name(collection: Collection) -> Option<&'static str> {
    match collection {
        Collection::Mailbox => Some("Mailbox"),
        Collection::Email => Some("Email"),
        Collection::Thread => Some("Thread"),
        Collection::Identity => Some("Identity"),
        Collection::EmailSubmission => Some("EmailSubmission"),
        Collection::Calendar => Some("Calendar"),
        Collection::CalendarEvent => Some("CalendarEvent"),
        Collection::AddressBook => Some("AddressBook"),
        Collection::ContactCard => Some("ContactCard"),
        Collection::SieveScript | Collection::EmailVanished | Collection::PushSubscription => None,
    }
}

pub fn sse_event(event_type: &str, id: Option<&str>, data: &Value) -> String {
    match id {
        Some(id) => format!("event: {event_type}\nid: {id}\ndata: {data}\n\n"),
        None => format!("event: {event_type}\ndata: {data}\n\n"),
    }
}

pub fn ping_event(interval_secs: u64) -> String {
    sse_event("ping", None, &json!({ "interval": interval_secs }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dav_collections_have_jmap_type_names() {
        assert_eq!(type_name(Collection::Calendar), Some("Calendar"));
        assert_eq!(type_name(Collection::CalendarEvent), Some("CalendarEvent"));
        assert_eq!(type_name(Collection::AddressBook), Some("AddressBook"));
        assert_eq!(type_name(Collection::ContactCard), Some("ContactCard"));
    }

    #[test]
    fn an_sse_event_is_framed_with_a_blank_line() {
        let event = sse_event("state", None, &json!({"x": 1}));
        assert!(event.starts_with("event: state\n"));
        assert!(event.contains("data: "));
        assert!(event.ends_with("\n\n"));
    }

    #[test]
    fn an_sse_event_can_carry_an_id_for_resume() {
        let event = sse_event("state", Some("42"), &json!({"x": 1}));
        assert!(event.contains("\nid: 42\n"));
    }

    #[test]
    fn a_ping_is_a_named_event_with_the_interval() {
        let ping = ping_event(30);
        assert!(ping.starts_with("event: ping\n"));
        assert!(ping.contains("data: {\"interval\":30}"));
        assert!(ping.ends_with("\n\n"));
    }

    #[test]
    fn a_single_state_change_names_one_type() {
        let change = state_change_single("a1", "Mailbox", "s5");
        assert_eq!(change["@type"], "StateChange");
        assert_eq!(change["changed"]["a1"]["Mailbox"], "s5");
        assert!(change["changed"]["a1"]["Email"].is_null());
    }

    #[test]
    fn collections_map_to_their_jmap_type_names() {
        assert_eq!(type_name(Collection::Email), Some("Email"));
        assert_eq!(type_name(Collection::Mailbox), Some("Mailbox"));
        assert_eq!(
            type_name(Collection::EmailSubmission),
            Some("EmailSubmission")
        );
        assert_eq!(type_name(Collection::SieveScript), None);
    }
}
