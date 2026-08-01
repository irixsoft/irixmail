use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use irixmail_directory::IpRule;

use crate::app::{error_response, AppState};

pub async fn list(State(state): State<AppState>) -> Response {
    match state.directory.ip_rules().list() {
        Ok(rules) => {
            let rules: Vec<_> = rules.iter().map(rule_json).collect();
            Json(json!({ "rules": rules })).into_response()
        }
        Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "could not list IP rules"),
    }
}

pub fn rule_json(rule: &IpRule) -> serde_json::Value {
    json!({
        "id": rule.id.to_string(),
        "cidr": rule.cidr,
        "action": rule.action,
    })
}
