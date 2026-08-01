use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::app::{error_response, AppState};

pub async fn delete(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return error_response(StatusCode::NOT_FOUND, "IP rule not found");
    };
    match state.directory.ip_rules().delete(id) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "IP rule not found"),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not delete the IP rule",
        ),
    }
}
