use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use calcard::common::PartialDateTime;
use calcard::vcard::{
    VCard, VCardEntry, VCardKind, VCardParameter, VCardParameterName, VCardParameterValue,
    VCardProperty, VCardValue,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

use irixmail_dav::parse::parse_vcf;

const KIND_X: &str = "X-ADDRESSBOOKSERVER-KIND";
const MEMBER_X: &str = "X-ADDRESSBOOKSERVER-MEMBER";
const UUID_SCHEME: &str = "urn:uuid:";

const FIELDS: [&str; 13] = [
    "kind",
    "name",
    "fullName",
    "nickname",
    "emails",
    "phones",
    "organization",
    "jobTitle",
    "addresses",
    "birthday",
    "note",
    "members",
    "photo",
];

pub fn card_fields(card: &VCard, member_ids: &BTreeMap<String, String>) -> Value {
    json!({
        "kind": kind_of(card),
        "name": name_of(card),
        "fullName": text_of(card, &VCardProperty::Fn),
        "nickname": text_of(card, &VCardProperty::Nickname),
        "emails": labelled(card, &VCardProperty::Email),
        "phones": labelled(card, &VCardProperty::Tel),
        "organization": text_of(card, &VCardProperty::Org),
        "jobTitle": text_of(card, &VCardProperty::Title),
        "addresses": addresses_of(card),
        "birthday": birthday_of(card),
        "note": text_of(card, &VCardProperty::Note),
        "members": members_of(card, member_ids),
        "photo": photo_of(card),
    })
}

pub fn build_card(fields: &Value, uid: &str) -> Result<String, String> {
    let mut card = VCard {
        entries: vec![
            text_entry(VCardProperty::Version, "3.0"),
            text_entry(VCardProperty::Uid, uid),
        ],
    };
    apply(&mut card, fields, fields)?;
    Ok(card.to_string())
}

pub fn patch_card(vcf: &str, patch: &Value) -> Result<String, String> {
    let (mut card, _) = parse_vcf(vcf).map_err(|_| "id".to_string())?;
    let merged = crate::calendar_object::merge_object(card_fields(&card, &BTreeMap::new()), patch);
    apply(&mut card, patch, &merged)?;
    Ok(card.to_string())
}

fn apply(card: &mut VCard, touched: &Value, merged: &Value) -> Result<(), String> {
    let touched = match touched.as_object() {
        Some(map) => map,
        None => return Err("update".to_string()),
    };
    for field in FIELDS {
        if !touched.contains_key(field) {
            continue;
        }
        let value = merged.get(field);
        match field {
            "fullName" => set_text(card, VCardProperty::Fn, value),
            "nickname" => set_text(card, VCardProperty::Nickname, value),
            "organization" => set_text(card, VCardProperty::Org, value),
            "jobTitle" => set_text(card, VCardProperty::Title, value),
            "note" => set_text(card, VCardProperty::Note, value),
            "birthday" => set_text(card, VCardProperty::Bday, value),
            "name" => set_name(card, value),
            "emails" => set_labelled(card, VCardProperty::Email, value)?,
            "phones" => set_labelled(card, VCardProperty::Tel, value)?,
            "addresses" => set_addresses(card, value)?,
            "members" => set_members(card, value),
            "kind" => set_kind(card, value),
            "photo" => set_photo(card, value)?,
            _ => {}
        }
    }
    Ok(())
}

fn set_text(card: &mut VCard, property: VCardProperty, value: Option<&Value>) {
    remove_property(card, &property);
    if let Some(text) = value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        card.entries.push(text_entry(property, text));
    }
}

fn set_name(card: &mut VCard, value: Option<&Value>) {
    remove_property(card, &VCardProperty::N);
    let Some(name) = value.and_then(Value::as_object) else {
        return;
    };
    let part = |key: &str| {
        VCardValue::Text(
            name.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        )
    };
    card.entries.push(VCardEntry {
        group: None,
        name: VCardProperty::N,
        params: Vec::new(),
        values: vec![
            part("surname"),
            part("given"),
            part("additional"),
            part("prefix"),
            part("suffix"),
        ],
    });
}

fn set_labelled(
    card: &mut VCard,
    property: VCardProperty,
    value: Option<&Value>,
) -> Result<(), String> {
    remove_property(card, &property);
    for item in value
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let text = item
            .get("value")
            .and_then(Value::as_str)
            .ok_or_else(|| property_name(&property))?;
        card.entries.push(VCardEntry {
            group: None,
            name: property.clone(),
            params: label_params(item),
            values: vec![VCardValue::Text(text.to_string())],
        });
    }
    Ok(())
}

fn set_addresses(card: &mut VCard, value: Option<&Value>) -> Result<(), String> {
    remove_property(card, &VCardProperty::Adr);
    for item in value
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let object = item
            .as_object()
            .ok_or_else(|| "addresses".to_string())?
            .clone();
        let part = |key: &str| {
            VCardValue::Text(
                object
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            )
        };
        card.entries.push(VCardEntry {
            group: None,
            name: VCardProperty::Adr,
            params: label_params(item),
            values: vec![
                VCardValue::Text(String::new()),
                VCardValue::Text(String::new()),
                part("street"),
                part("city"),
                part("region"),
                part("postcode"),
                part("country"),
            ],
        });
    }
    Ok(())
}

fn set_members(card: &mut VCard, value: Option<&Value>) {
    remove_property(card, &VCardProperty::Member);
    remove_other(card, MEMBER_X);
    for uid in value
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let Some(uid) = uid.as_str() else { continue };
        card.entries.push(text_entry(
            VCardProperty::Other(MEMBER_X.to_string()),
            &format!("{UUID_SCHEME}{uid}"),
        ));
    }
}

fn set_kind(card: &mut VCard, value: Option<&Value>) {
    remove_property(card, &VCardProperty::Kind);
    remove_other(card, KIND_X);
    if value.and_then(Value::as_str) == Some("group") {
        card.entries.push(text_entry(
            VCardProperty::Other(KIND_X.to_string()),
            "group",
        ));
    }
}

fn set_photo(card: &mut VCard, value: Option<&Value>) -> Result<(), String> {
    remove_property(card, &VCardProperty::Photo);
    let Some(photo) = value.and_then(Value::as_object) else {
        return Ok(());
    };
    let data = photo
        .get("data")
        .and_then(Value::as_str)
        .and_then(|text| STANDARD.decode(text).ok())
        .ok_or_else(|| "photo".to_string())?;
    let media_type = photo
        .get("mediaType")
        .and_then(Value::as_str)
        .unwrap_or("application/octet-stream")
        .to_string();
    card.entries.push(VCardEntry {
        group: None,
        name: VCardProperty::Photo,
        params: Vec::new(),
        values: vec![VCardValue::Text(format!(
            "data:{media_type};base64,{}",
            STANDARD.encode(&data)
        ))],
    });
    Ok(())
}

fn label_params(item: &Value) -> Vec<VCardParameter> {
    match item
        .get("label")
        .and_then(Value::as_str)
        .filter(|label| !label.is_empty())
    {
        Some(label) => vec![VCardParameter {
            name: VCardParameterName::Type,
            value: VCardParameterValue::Text(label.to_ascii_uppercase()),
        }],
        None => Vec::new(),
    }
}

fn text_entry(name: VCardProperty, text: &str) -> VCardEntry {
    VCardEntry {
        group: None,
        name,
        params: Vec::new(),
        values: vec![VCardValue::Text(text.to_string())],
    }
}

fn remove_property(card: &mut VCard, property: &VCardProperty) {
    card.entries.retain(|entry| &entry.name != property);
}

fn remove_other(card: &mut VCard, name: &str) {
    card.entries.retain(|entry| !is_other(&entry.name, name));
}

fn is_other(property: &VCardProperty, name: &str) -> bool {
    matches!(property, VCardProperty::Other(other) if other.eq_ignore_ascii_case(name))
}

fn property_name(property: &VCardProperty) -> String {
    match property {
        VCardProperty::Email => "emails".to_string(),
        VCardProperty::Tel => "phones".to_string(),
        _ => "properties".to_string(),
    }
}

fn value_text(value: &VCardValue) -> Option<String> {
    match value {
        VCardValue::Text(text) => Some(text.clone()),
        VCardValue::Component(items) => items.first().cloned(),
        VCardValue::PartialDateTime(value) => Some(date_string(value)),
        VCardValue::Kind(VCardKind::Group) => Some("group".to_string()),
        _ => None,
    }
}

fn text_of(card: &VCard, property: &VCardProperty) -> Value {
    card.property(property)
        .and_then(|entry| entry.values.first())
        .and_then(value_text)
        .filter(|text| !text.is_empty())
        .map(Value::String)
        .unwrap_or(Value::Null)
}

fn kind_of(card: &VCard) -> Value {
    let declared = card
        .entries
        .iter()
        .find(|entry| entry.name == VCardProperty::Kind || is_other(&entry.name, KIND_X))
        .and_then(|entry| entry.values.first())
        .and_then(value_text)
        .unwrap_or_default();
    if declared.eq_ignore_ascii_case("group") {
        json!("group")
    } else {
        json!("individual")
    }
}

fn name_of(card: &VCard) -> Value {
    let values = card
        .property(&VCardProperty::N)
        .map(|entry| entry.values.as_slice())
        .unwrap_or_default();
    let part = |index: usize| values.get(index).and_then(value_text).unwrap_or_default();
    json!({
        "prefix": part(3),
        "given": part(1),
        "additional": part(2),
        "surname": part(0),
        "suffix": part(4),
    })
}

fn label_of(entry: &VCardEntry) -> String {
    entry
        .params
        .iter()
        .find(|param| param.name == VCardParameterName::Type)
        .and_then(|param| param.value.as_text())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

fn labelled(card: &VCard, property: &VCardProperty) -> Value {
    let items: Vec<Value> = card
        .properties(property)
        .filter_map(|entry| {
            entry
                .values
                .first()
                .and_then(value_text)
                .map(|text| json!({"value": text, "label": label_of(entry)}))
        })
        .collect();
    Value::Array(items)
}

fn addresses_of(card: &VCard) -> Value {
    let items: Vec<Value> = card
        .properties(&VCardProperty::Adr)
        .map(|entry| {
            let part = |index: usize| {
                entry
                    .values
                    .get(index)
                    .and_then(value_text)
                    .unwrap_or_default()
            };
            json!({
                "street": part(2),
                "city": part(3),
                "region": part(4),
                "postcode": part(5),
                "country": part(6),
                "label": label_of(entry),
            })
        })
        .collect();
    Value::Array(items)
}

fn birthday_of(card: &VCard) -> Value {
    match text_of(card, &VCardProperty::Bday) {
        Value::String(text) if text.len() >= 10 => Value::String(text[..10].to_string()),
        Value::String(text) if text.len() == 8 && text.chars().all(|c| c.is_ascii_digit()) => {
            Value::String(format!("{}-{}-{}", &text[..4], &text[4..6], &text[6..8]))
        }
        other => other,
    }
}

fn members_of(card: &VCard, member_ids: &BTreeMap<String, String>) -> Value {
    let items: Vec<Value> = card
        .entries
        .iter()
        .filter(|entry| entry.name == VCardProperty::Member || is_other(&entry.name, MEMBER_X))
        .filter_map(|entry| entry.values.first().and_then(value_text))
        .map(|value| {
            value
                .strip_prefix(UUID_SCHEME)
                .map(str::to_string)
                .unwrap_or(value)
        })
        .map(|uid| match member_ids.get(&uid) {
            Some(id) => Value::String(id.clone()),
            None => Value::String(uid),
        })
        .collect();
    Value::Array(items)
}

fn photo_of(card: &VCard) -> Value {
    let Some(entry) = card.property(&VCardProperty::Photo) else {
        return Value::Null;
    };
    let media_type = entry
        .params
        .iter()
        .find(|param| param.name == VCardParameterName::Mediatype)
        .and_then(|param| param.value.as_text())
        .map(str::to_string);
    match entry.values.first() {
        Some(VCardValue::Binary(data)) => json!({
            "mediaType": media_type
                .or_else(|| data.content_type.clone())
                .unwrap_or_else(|| "application/octet-stream".to_string()),
            "data": STANDARD.encode(&data.data),
        }),
        Some(VCardValue::Text(text)) => match text.split_once(";base64,") {
            Some((head, body)) => json!({
                "mediaType": head.strip_prefix("data:").unwrap_or(head),
                "data": body,
            }),
            None => Value::Null,
        },
        _ => Value::Null,
    }
}

fn date_string(value: &PartialDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}",
        value.year.unwrap_or(0),
        value.month.unwrap_or(1),
        value.day.unwrap_or(1)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(fields: &Value) -> Value {
        let vcf = build_card(fields, "c-1").unwrap();
        let (card, _) = parse_vcf(&vcf).unwrap();
        card_fields(&card, &BTreeMap::new())
    }

    #[test]
    fn a_full_contact_survives_a_json_round_trip() {
        let fields = json!({
            "kind": "individual",
            "name": {
                "prefix": "Dr",
                "given": "Ada",
                "additional": "M",
                "surname": "Lovelace",
                "suffix": "Jr",
            },
            "fullName": "Ada Lovelace",
            "nickname": "Ada",
            "emails": [{"value": "ada@example.com", "label": "work"}],
            "phones": [{"value": "+123456", "label": "cell"}],
            "organization": "Analytical Engines",
            "jobTitle": "Programmer",
            "addresses": [{
                "street": "1 Loop",
                "city": "London",
                "region": "Greater London",
                "postcode": "E1",
                "country": "UK",
                "label": "home",
            }],
            "birthday": "1815-12-10",
            "note": "first programmer",
            "members": [],
            "photo": null,
        });
        let back = round_trip(&fields);
        assert_eq!(back["kind"], "individual");
        assert_eq!(back["name"], fields["name"]);
        assert_eq!(back["fullName"], "Ada Lovelace");
        assert_eq!(back["nickname"], "Ada");
        assert_eq!(back["emails"], fields["emails"]);
        assert_eq!(back["phones"], fields["phones"]);
        assert_eq!(back["organization"], "Analytical Engines");
        assert_eq!(back["jobTitle"], "Programmer");
        assert_eq!(back["addresses"], fields["addresses"]);
        assert_eq!(back["birthday"], "1815-12-10");
        assert_eq!(back["note"], "first programmer");
        assert_eq!(back["photo"], Value::Null);
    }

    #[test]
    fn a_group_card_lists_its_member_uids() {
        let fields = json!({
            "kind": "group",
            "fullName": "Team",
            "members": ["uid-a", "uid-b"],
        });
        let vcf = build_card(&fields, "c-2").unwrap();
        assert!(vcf.contains("X-ADDRESSBOOKSERVER-KIND:group"), "{vcf}");
        assert!(
            vcf.contains("X-ADDRESSBOOKSERVER-MEMBER:urn:uuid:uid-a"),
            "{vcf}"
        );
        let back = round_trip(&fields);
        assert_eq!(back["kind"], "group");
        assert_eq!(back["members"], json!(["uid-a", "uid-b"]));
    }

    #[test]
    fn member_uids_resolve_to_card_ids_when_known() {
        let vcf = build_card(
            &json!({"kind": "group", "fullName": "Team", "members": ["uid-a"]}),
            "c-3",
        )
        .unwrap();
        let (card, _) = parse_vcf(&vcf).unwrap();
        let mut index = BTreeMap::new();
        index.insert("uid-a".to_string(), "42".to_string());
        assert_eq!(card_fields(&card, &index)["members"], json!(["42"]));
    }

    #[test]
    fn a_photo_round_trips_as_base64_with_its_media_type() {
        let fields = json!({
            "fullName": "Ada",
            "photo": {"mediaType": "image/png", "data": STANDARD.encode([1u8, 2, 3, 4])},
        });
        let back = round_trip(&fields);
        assert_eq!(back["photo"]["mediaType"], "image/png");
        assert_eq!(back["photo"]["data"], STANDARD.encode([1u8, 2, 3, 4]));
    }

    #[test]
    fn a_patch_keeps_unknown_properties_and_untouched_fields() {
        let vcf = concat!(
            "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:c-4\r\nFN:Ada Lovelace\r\n",
            "X-ABLABEL:favourite\r\nCATEGORIES:friends\r\nEMAIL:ada@example.com\r\nEND:VCARD\r\n"
        );
        let patched = patch_card(vcf, &json!({"nickname": "Ada"})).unwrap();
        assert!(patched.contains("X-ABLABEL:favourite"), "{patched}");
        assert!(patched.contains("CATEGORIES:friends"), "{patched}");
        assert!(patched.contains("NICKNAME:Ada"), "{patched}");
        let (card, _) = parse_vcf(&patched).unwrap();
        let back = card_fields(&card, &BTreeMap::new());
        assert_eq!(back["fullName"], "Ada Lovelace");
        assert_eq!(
            back["emails"],
            json!([{"value": "ada@example.com", "label": ""}])
        );
    }

    #[test]
    fn a_null_patch_value_clears_the_property() {
        let vcf = build_card(&json!({"fullName": "Ada", "nickname": "A"}), "c-5").unwrap();
        let patched = patch_card(&vcf, &json!({"nickname": null})).unwrap();
        assert!(!patched.contains("NICKNAME"), "{patched}");
    }

    #[test]
    fn an_email_without_a_value_is_rejected() {
        let fields = json!({"fullName": "Ada", "emails": [{"label": "work"}]});
        assert_eq!(build_card(&fields, "c-6"), Err("emails".to_string()));
    }
}
