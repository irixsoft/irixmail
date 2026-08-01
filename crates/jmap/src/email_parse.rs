use serde_json::{json, Map, Value};

use crate::context::JmapContext;
use crate::email_get::parse_email;
use crate::reply::account_id;
use crate::request::Invocation;

pub fn email_parse(ctx: &JmapContext, args: &Value, call_id: &str) -> Invocation {
    let mut parsed = Map::new();
    let mut not_parsable = Vec::new();
    let mut not_found = Vec::new();

    if let Some(blob_ids) = args.get("blobIds").and_then(Value::as_array) {
        for blob_id in blob_ids.iter().filter_map(Value::as_str) {
            match crate::fetch_blob(ctx.blobs.as_ref(), blob_id) {
                Ok(Some(raw)) => match parse_email(&raw, blob_id, raw.len()) {
                    Some(object) => {
                        parsed.insert(blob_id.to_string(), Value::Object(object));
                    }
                    None => not_parsable.push(Value::String(blob_id.to_string())),
                },
                Ok(None) | Err(_) => not_found.push(Value::String(blob_id.to_string())),
            }
        }
    }

    Invocation::new(
        "Email/parse",
        json!({
            "accountId": account_id(args),
            "parsed": parsed,
            "notParsable": not_parsable,
            "notFound": not_found,
        }),
        call_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob_upload::store_upload;
    use crate::context::test_context;

    #[test]
    fn email_parse_extracts_headers_from_an_uploaded_blob() {
        let ctx = test_context();
        let raw: &[u8] = b"Subject: Parsed\r\nFrom: bob@example.net\r\n\r\nhello parse\r\n";
        let blob_id = store_upload(ctx.store.as_ref(), ctx.blobs.as_ref(), 1, raw, 0).unwrap();

        let response = email_parse(&ctx, &json!({"accountId": "1", "blobIds": [blob_id]}), "c0");
        let email = &response.arguments()["parsed"][blob_id.as_str()];
        assert_eq!(email["subject"], "Parsed");
        assert_eq!(email["from"][0]["email"], "bob@example.net");
        assert!(email.get("id").is_none(), "a parsed email has no stored id");
    }

    #[test]
    fn email_parse_reports_a_missing_blob_as_not_found() {
        let ctx = test_context();
        let response = email_parse(
            &ctx,
            &json!({"accountId": "1", "blobIds": ["deadbeef"]}),
            "c0",
        );
        assert_eq!(response.arguments()["notFound"], json!(["deadbeef"]));
    }
}
