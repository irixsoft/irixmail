use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::context::JmapContext;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Invocation(pub String, pub Value, pub String);

impl Invocation {
    pub fn new(name: impl Into<String>, arguments: Value, call_id: impl Into<String>) -> Self {
        Invocation(name.into(), arguments, call_id.into())
    }

    pub fn name(&self) -> &str {
        &self.0
    }

    pub fn arguments(&self) -> &Value {
        &self.1
    }

    pub fn call_id(&self) -> &str {
        &self.2
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    pub using: Vec<String>,
    #[serde(rename = "methodCalls")]
    pub method_calls: Vec<Invocation>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Response {
    #[serde(rename = "methodResponses")]
    pub method_responses: Vec<Invocation>,
    #[serde(rename = "sessionState")]
    pub session_state: String,
}

pub const MAX_CALLS_IN_REQUEST: usize = 16;

pub fn method_error(kind: &str, call_id: &str) -> Invocation {
    Invocation::new("error", json!({ "type": kind }), call_id)
}

pub fn limit_problem(limit: &str) -> Value {
    json!({ "type": "urn:ietf:params:jmap:error:limit", "limit": limit, "status": 400 })
}

pub fn problem(error_type: &str) -> Value {
    json!({ "type": format!("urn:ietf:params:jmap:error:{error_type}"), "status": 400 })
}

pub fn unknown_capability_problem(capability: &str) -> Value {
    json!({
        "type": "urn:ietf:params:jmap:error:unknownCapability",
        "status": 400,
        "detail": format!("The Request object used capability '{capability}', which is not supported by this server."),
    })
}

pub type StatelessHandler = fn(&Value, &str) -> Invocation;
pub type StatefulHandler = fn(&JmapContext, &Value, &str) -> Invocation;

pub enum Handler {
    Stateless(StatelessHandler),
    Stateful(StatefulHandler),
}

#[derive(Default)]
pub struct Router {
    handlers: HashMap<String, Handler>,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, name: &str, handler: StatelessHandler) -> &mut Self {
        self.handlers
            .insert(name.to_string(), Handler::Stateless(handler));
        self
    }

    pub fn register_stateful(&mut self, name: &str, handler: StatefulHandler) -> &mut Self {
        self.handlers
            .insert(name.to_string(), Handler::Stateful(handler));
        self
    }

    pub fn handles(&self, name: &str) -> bool {
        self.handlers.contains_key(name)
    }

    pub fn process(&self, ctx: &JmapContext, request: &Request, session_state: &str) -> Response {
        let mut created_ids: HashMap<String, String> = HashMap::new();
        let mut method_responses = Vec::with_capacity(request.method_calls.len());
        for invocation in &request.method_calls {
            let response = match resolve_result_refs(invocation.arguments(), &method_responses) {
                Some(resolved) => {
                    let args = resolve_creation_refs(&resolved, &created_ids);
                    if over_object_limit(invocation.name(), &args) {
                        method_error("requestTooLarge", invocation.call_id())
                    } else {
                        match self.handlers.get(invocation.name()) {
                            Some(Handler::Stateless(handler)) => {
                                handler(&args, invocation.call_id())
                            }
                            Some(Handler::Stateful(handler)) => {
                                handler(ctx, &args, invocation.call_id())
                            }
                            None => method_error("unknownMethod", invocation.call_id()),
                        }
                    }
                }
                None => method_error("invalidResultReference", invocation.call_id()),
            };
            if let Some(created) = response
                .arguments()
                .get("created")
                .and_then(Value::as_object)
            {
                for (creation_id, object) in created {
                    if let Some(id) = object.get("id").and_then(Value::as_str) {
                        created_ids.insert(creation_id.clone(), id.to_string());
                    }
                }
            }
            method_responses.push(response);
        }
        Response {
            method_responses,
            session_state: session_state.to_string(),
        }
    }
}

fn over_object_limit(method: &str, args: &Value) -> bool {
    (method.ends_with("/get") && crate::reply::get_too_large(args))
        || (method.ends_with("/set") && crate::reply::set_too_large(args))
}

fn resolve_creation_refs(args: &Value, created: &HashMap<String, String>) -> Value {
    resolve_refs_at(args, created, false)
}

fn expects_id(key: &str) -> bool {
    key == "ids" || key == "destroy" || key.ends_with("Id") || key.ends_with("Ids")
}

fn resolve_refs_at(value: &Value, created: &HashMap<String, String>, id_position: bool) -> Value {
    match value {
        Value::String(text) if id_position => {
            match text
                .strip_prefix('#')
                .and_then(|creation_id| created.get(creation_id))
            {
                Some(id) => Value::String(id.clone()),
                None => value.clone(),
            }
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| resolve_refs_at(item, created, id_position))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| {
                    let resolved_key = key
                        .strip_prefix('#')
                        .and_then(|creation_id| created.get(creation_id))
                        .cloned()
                        .unwrap_or_else(|| key.clone());
                    let nested = resolve_refs_at(item, created, id_position || expects_id(key));
                    (resolved_key, nested)
                })
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn resolve_result_refs(args: &Value, prior: &[Invocation]) -> Option<Value> {
    let Value::Object(map) = args else {
        return Some(args.clone());
    };
    let mut out = serde_json::Map::with_capacity(map.len());
    for (key, value) in map {
        match key
            .strip_prefix('#')
            .and_then(|name| as_reference(value).map(|r| (name, r)))
        {
            Some((name, (result_of, method, path))) => {
                let resolved = resolve_reference(result_of, method, path, prior)?;
                out.insert(name.to_string(), resolved);
            }
            None => {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    Some(Value::Object(out))
}

fn as_reference(value: &Value) -> Option<(&str, &str, &str)> {
    let object = value.as_object()?;
    let result_of = object.get("resultOf")?.as_str()?;
    let name = object.get("name")?.as_str()?;
    let path = object.get("path")?.as_str()?;
    Some((result_of, name, path))
}

fn resolve_reference(
    result_of: &str,
    name: &str,
    path: &str,
    prior: &[Invocation],
) -> Option<Value> {
    let response = prior
        .iter()
        .rev()
        .find(|inv| inv.call_id() == result_of && inv.name() == name)?;
    eval_pointer(response.arguments(), &pointer_segments(path))
}

fn pointer_segments(path: &str) -> Vec<String> {
    if path.is_empty() {
        return Vec::new();
    }
    path.strip_prefix('/')
        .unwrap_or(path)
        .split('/')
        .map(|token| token.replace("~1", "/").replace("~0", "~"))
        .collect()
}

fn eval_pointer(value: &Value, segments: &[String]) -> Option<Value> {
    let Some((head, rest)) = segments.split_first() else {
        return Some(value.clone());
    };
    if head == "*" {
        let items = value.as_array()?;
        let mut out = Vec::new();
        for item in items {
            match eval_pointer(item, rest)? {
                Value::Array(nested) => out.extend(nested),
                other => out.push(other),
            }
        }
        return Some(Value::Array(out));
    }
    let next = match value {
        Value::Object(map) => map.get(head)?,
        Value::Array(items) => items.get(head.parse::<usize>().ok()?)?,
        _ => return None,
    };
    eval_pointer(next, rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo(args: &Value, call_id: &str) -> Invocation {
        Invocation::new("Echo/reply", args.clone(), call_id)
    }

    #[test]
    fn a_request_deserializes_from_jmap_json() {
        let body = r#"{
            "using": ["urn:ietf:params:jmap:core"],
            "methodCalls": [["Core/echo", {"hello": 1}, "c0"]]
        }"#;
        let request: Request = serde_json::from_str(body).unwrap();
        assert_eq!(request.using, vec!["urn:ietf:params:jmap:core"]);
        assert_eq!(request.method_calls[0].name(), "Core/echo");
        assert_eq!(request.method_calls[0].call_id(), "c0");
    }

    #[test]
    fn a_registered_handler_is_invoked() {
        let mut router = Router::new();
        router.register("Core/echo", echo);
        let request = Request {
            using: vec!["urn:ietf:params:jmap:core".into()],
            method_calls: vec![Invocation::new("Core/echo", json!({"x": 1}), "c1")],
        };
        let response = router.process(&crate::context::test_context(), &request, "state-1");
        assert_eq!(response.session_state, "state-1");
        assert_eq!(response.method_responses[0].name(), "Echo/reply");
        assert_eq!(response.method_responses[0].arguments(), &json!({"x": 1}));
        assert_eq!(response.method_responses[0].call_id(), "c1");
    }

    #[test]
    fn an_unknown_method_yields_an_error_invocation() {
        let router = Router::new();
        let request = Request {
            using: Vec::new(),
            method_calls: vec![Invocation::new("Mailbox/get", json!({}), "c2")],
        };
        let response = router.process(&crate::context::test_context(), &request, "s");
        assert_eq!(response.method_responses[0].name(), "error");
        assert_eq!(
            response.method_responses[0].arguments(),
            &json!({"type": "unknownMethod"})
        );
        assert_eq!(response.method_responses[0].call_id(), "c2");
    }

    #[test]
    fn a_limit_problem_names_the_breached_limit() {
        let problem = limit_problem("maxCallsInRequest");
        assert_eq!(problem["type"], "urn:ietf:params:jmap:error:limit");
        assert_eq!(problem["limit"], "maxCallsInRequest");
        assert_eq!(problem["status"], 400);
    }

    #[test]
    fn a_problem_carries_the_jmap_error_urn() {
        let problem = problem("notJSON");
        assert_eq!(problem["type"], "urn:ietf:params:jmap:error:notJSON");
        assert_eq!(problem["status"], 400);
    }

    #[test]
    fn the_response_serializes_to_jmap_json() {
        let response = Response {
            method_responses: vec![Invocation::new("Core/echo", json!({"ok": true}), "c0")],
            session_state: "s0".into(),
        };
        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(value["methodResponses"][0][0], "Core/echo");
        assert_eq!(value["methodResponses"][0][2], "c0");
        assert_eq!(value["sessionState"], "s0");
    }

    #[test]
    fn a_result_reference_feeds_a_query_result_into_a_later_get() {
        use crate::context::test_context_with_account;
        use irixmail_mail::{
            allocate_document_id, append_message, provision_mailboxes, AppendRequest, INBOX_ID,
        };

        let ctx = test_context_with_account();
        let account = ctx.account_id as u32;
        let record = ctx.directory.accounts().get(ctx.account_id).unwrap();
        let mailboxes = provision_mailboxes(record.created_at);
        let inbox = mailboxes.iter().find(|m| m.id == INBOX_ID).unwrap();
        let raw: &[u8] = b"Subject: Find me\r\nFrom: a@example.net\r\n\r\nbody\r\n";
        let doc = allocate_document_id(ctx.store.as_ref(), account).unwrap();
        append_message(
            ctx.store.as_ref(),
            ctx.blobs.as_ref(),
            ctx.notifier.as_ref(),
            &AppendRequest {
                account: &record,
                mailbox: inbox,
                flags: vec![],
                received_at: 0,
                document_id: doc,
                raw,
            },
        )
        .unwrap();

        let mut router = Router::new();
        router.register_stateful("Email/query", crate::email_query::email_query);
        router.register_stateful("Email/get", crate::email_get::email_get);
        let request = Request {
            using: Vec::new(),
            method_calls: vec![
                Invocation::new(
                    "Email/query",
                    json!({ "accountId": account.to_string() }),
                    "c1",
                ),
                Invocation::new(
                    "Email/get",
                    json!({
                        "accountId": account.to_string(),
                        "#ids": { "resultOf": "c1", "name": "Email/query", "path": "/ids" }
                    }),
                    "c2",
                ),
            ],
        };
        let response = router.process(&ctx, &request, "s");
        let get = &response.method_responses[1];
        assert_eq!(get.name(), "Email/get");
        let list = get.arguments()["list"].as_array().expect("a list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["id"], json!(doc.to_string()));
    }

    #[test]
    fn creation_references_resolve_as_id_set_keys_and_only_in_id_positions() {
        fn fake_set(_args: &Value, call_id: &str) -> Invocation {
            Invocation::new("Fake/set", json!({"created": {"a": {"id": "42"}}}), call_id)
        }
        let mut router = Router::new();
        router.register("Fake/set", fake_set);
        router.register("Core/echo", echo);
        let request = Request {
            using: Vec::new(),
            method_calls: vec![
                Invocation::new("Fake/set", json!({}), "c0"),
                Invocation::new(
                    "Core/echo",
                    json!({
                        "subject": "#a",
                        "emailId": "#a",
                        "ids": ["#a"],
                        "mailboxIds": {"#a": true},
                    }),
                    "c1",
                ),
            ],
        };
        let response = router.process(&crate::context::test_context(), &request, "s");
        let echoed = response.method_responses[1].arguments();
        assert_eq!(echoed["subject"], "#a");
        assert_eq!(echoed["emailId"], "42");
        assert_eq!(echoed["ids"], json!(["42"]));
        assert_eq!(echoed["mailboxIds"], json!({"42": true}));
    }

    #[test]
    fn a_creation_reference_key_files_the_email_into_the_new_mailbox() {
        use crate::context::test_context_with_account;
        use irixmail_mail::{mailbox_ops, provision_mailboxes};

        let ctx = test_context_with_account();
        let record = ctx.directory.accounts().get(ctx.account_id).unwrap();
        ctx.store
            .batch(&mailbox_ops(
                ctx.account_id as u32,
                &provision_mailboxes(record.created_at),
            ))
            .unwrap();
        let account = ctx.account_id.to_string();
        let mut router = Router::new();
        router.register_stateful("Mailbox/set", crate::mailbox_set::mailbox_set);
        router.register_stateful("Email/set", crate::email_set::email_set);
        router.register_stateful("Email/get", crate::email_get::email_get);
        let request = Request {
            using: Vec::new(),
            method_calls: vec![
                Invocation::new(
                    "Mailbox/set",
                    json!({"accountId": account, "create": {"newbox": {"name": "Projects"}}}),
                    "c0",
                ),
                Invocation::new(
                    "Email/set",
                    json!({
                        "accountId": account,
                        "create": {"d": {
                            "mailboxIds": {"#newbox": true},
                            "subject": "Filed by reference",
                        }},
                    }),
                    "c1",
                ),
            ],
        };
        let response = router.process(&ctx, &request, "s");
        let mailbox_id = response.method_responses[0].arguments()["created"]["newbox"]["id"]
            .as_str()
            .expect("mailbox created")
            .to_string();
        let email_id = response.method_responses[1].arguments()["created"]["d"]["id"]
            .as_str()
            .expect("email created")
            .to_string();

        let get = crate::email_get::email_get(
            &ctx,
            &json!({"accountId": ctx.account_id.to_string(), "ids": [email_id]}),
            "c2",
        );
        let list = get.arguments()["list"].as_array().expect("a list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["mailboxIds"][&mailbox_id], json!(true));
    }

    #[test]
    fn a_pointer_star_flattens_a_property_across_a_list() {
        let value = json!({ "list": [ { "id": "1" }, { "id": "2" } ] });
        let out = eval_pointer(&value, &pointer_segments("/list/*/id")).unwrap();
        assert_eq!(out, json!(["1", "2"]));
    }

    #[test]
    fn a_dangling_result_reference_yields_an_invalid_result_reference_error() {
        let ctx = crate::context::test_context();
        let mut router = Router::new();
        router.register_stateful("Email/get", crate::email_get::email_get);
        let request = Request {
            using: Vec::new(),
            method_calls: vec![Invocation::new(
                "Email/get",
                json!({
                    "accountId": "1",
                    "#ids": { "resultOf": "missing", "name": "Email/query", "path": "/ids" }
                }),
                "c2",
            )],
        };
        let response = router.process(&ctx, &request, "s");
        assert_eq!(response.method_responses[0].name(), "error");
        assert_eq!(
            response.method_responses[0].arguments()["type"],
            "invalidResultReference"
        );
    }
}
