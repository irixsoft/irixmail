use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Redirect, Response};
use rust_embed::RustEmbed;

use crate::app::error_response;

#[derive(RustEmbed)]
#[folder = "assets/"]
struct Assets;

pub fn rewrite_base_href(html: &str, base: &str) -> String {
    html.replace("<base href=\"/\" />", &format!("<base href=\"{base}\" />"))
        .replace("<base href=\"/\">", &format!("<base href=\"{base}\">"))
}

pub fn serve_asset(path: &str) -> Response {
    let trimmed = path.trim_start_matches('/');
    let lookup = if trimmed.is_empty() {
        "index.html"
    } else {
        trimmed
    };
    match Assets::get(lookup) {
        Some(file) => (
            [
                (header::CONTENT_TYPE, mime_for(lookup)),
                (header::CACHE_CONTROL, cache_control_for(lookup)),
            ],
            file.data.into_owned(),
        )
            .into_response(),
        None => index_for(path),
    }
}

fn cache_control_for(lookup: &str) -> &'static str {
    if lookup.contains("/assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    }
}

fn index_for(path: &str) -> Response {
    let candidate = if path.starts_with("/admin") {
        "admin/index.html"
    } else if path.starts_with("/webmail") {
        "webmail/index.html"
    } else {
        "index.html"
    };
    match Assets::get(candidate).or_else(|| Assets::get("index.html")) {
        Some(file) => (
            [
                (header::CONTENT_TYPE, "text/html"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            file.data.into_owned(),
        )
            .into_response(),
        None => error_response(StatusCode::NOT_FOUND, "asset not found"),
    }
}

pub async fn spa_fallback(uri: Uri) -> Response {
    if uri.path() == "/" {
        return Redirect::permanent("/webmail/").into_response();
    }
    if is_api_path(uri.path()) {
        return error_response(StatusCode::NOT_FOUND, "resource not found");
    }
    serve_asset(uri.path())
}

fn is_api_path(path: &str) -> bool {
    path.starts_with("/api")
        || path.starts_with("/jmap")
        || path.starts_with("/healthz")
        || path.starts_with("/.well-known")
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html",
        Some("js") => "application/javascript",
        Some("css") => "text/css",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("webmanifest") => "application/manifest+json",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn body_string(response: Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn an_embedded_asset_is_served_with_its_mime() {
        let response = serve_asset("/app.js");
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/javascript"
        );
    }

    #[tokio::test]
    async fn an_unknown_route_falls_back_to_index() {
        let response = serve_asset("/inbox/123");
        assert_eq!(response.headers()[header::CONTENT_TYPE], "text/html");
        let body = body_string(response).await;
        assert!(body.contains("<div id=\"root\">"));
    }

    #[tokio::test]
    async fn an_api_path_does_not_serve_the_spa() {
        let response = spa_fallback("/api/unknown".parse().unwrap()).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_non_api_path_serves_the_spa() {
        let response = spa_fallback("/dashboard".parse().unwrap()).await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn manifest_font_and_image_mimes_are_known() {
        assert_eq!(
            mime_for("webmail/manifest.webmanifest"),
            "application/manifest+json"
        );
        assert_eq!(mime_for("a.woff"), "font/woff");
        assert_eq!(mime_for("a.jpg"), "image/jpeg");
        assert_eq!(mime_for("a.jpeg"), "image/jpeg");
        assert_eq!(mime_for("a.webp"), "image/webp");
    }

    #[tokio::test]
    async fn the_root_redirects_to_webmail() {
        let response = spa_fallback("/".parse().unwrap()).await;
        assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
        assert_eq!(response.headers()[header::LOCATION], "/webmail/");
    }

    #[test]
    fn hashed_assets_cache_immutably_and_shells_revalidate() {
        assert_eq!(
            cache_control_for("webmail/assets/index-B3x.js"),
            "public, max-age=31536000, immutable"
        );
        assert_eq!(cache_control_for("webmail/index.html"), "no-cache");
        assert_eq!(cache_control_for("webmail/service-worker.js"), "no-cache");
    }

    #[tokio::test]
    async fn a_served_asset_carries_a_cache_control_header() {
        let response = serve_asset("/app.js");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
    }

    #[test]
    fn the_base_href_is_rewritten() {
        let html = "<base href=\"/\" />";
        assert_eq!(
            rewrite_base_href(html, "/admin/"),
            "<base href=\"/admin/\" />"
        );
    }
}
