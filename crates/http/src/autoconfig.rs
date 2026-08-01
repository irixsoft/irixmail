use axum::extract::State;
use axum::http::{header, Uri};
use axum::response::{IntoResponse, Response};

use crate::app::AppState;

pub async fn autoconfig(State(state): State<AppState>, uri: Uri) -> Response {
    respond(&state, uri, false)
}

pub async fn autoconfig_well_known(State(state): State<AppState>, uri: Uri) -> Response {
    respond(&state, uri, true)
}

fn respond(state: &AppState, uri: Uri, cors: bool) -> Response {
    let host = &state.hostname;
    let requested = uri
        .query()
        .unwrap_or_default()
        .split('&')
        .find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == "emailaddress").then(|| value.replace("%40", "@").to_ascii_lowercase())
        })
        .filter(|address| address.contains('@'));
    let (username, domain) = match &requested {
        Some(address) => (
            xml_escape(address),
            xml_escape(address.rsplit_once('@').map(|(_, d)| d).unwrap_or(host)),
        ),
        None => ("%EMAILADDRESS%".to_string(), host.clone()),
    };
    let imap = state.listeners.imap.tls.unwrap_or(993);
    let pop3 = state.listeners.pop3.tls.unwrap_or(995);
    let smtp = state.listeners.submission.tls.unwrap_or(465);
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<clientConfig version="1.1">
  <emailProvider id="{domain}">
    <domain>{domain}</domain>
    <displayName>IRIXMAIL</displayName>
    <incomingServer type="imap">
      <hostname>{host}</hostname>
      <port>{imap}</port>
      <socketType>SSL</socketType>
      <authentication>password-cleartext</authentication>
      <username>{username}</username>
    </incomingServer>
    <incomingServer type="pop3">
      <hostname>{host}</hostname>
      <port>{pop3}</port>
      <socketType>SSL</socketType>
      <authentication>password-cleartext</authentication>
      <username>{username}</username>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>{host}</hostname>
      <port>{smtp}</port>
      <socketType>SSL</socketType>
      <authentication>password-cleartext</authentication>
      <username>{username}</username>
    </outgoingServer>
  </emailProvider>
</clientConfig>
"#
    );
    let mut response = (
        [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
        xml,
    )
        .into_response();
    if cors {
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            header::HeaderValue::from_static("*"),
        );
    }
    response
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::tests_support::{state, TempDir};

    async fn get_config(uri: &str) -> (StatusCode, Option<String>, String) {
        let dir = TempDir::new();
        let app = crate::app::router(state(&dir));
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let cors = response
            .headers()
            .get("access-control-allow-origin")
            .map(|value| value.to_str().unwrap().to_string());
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, cors, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn the_autoconfig_xml_names_the_servers() {
        let (status, _, text) = get_config("/mail/config-v1.1.xml").await;
        assert_eq!(status, StatusCode::OK);
        assert!(text.contains("clientConfig"));
        assert!(text.contains("mail.example.com"));
        assert!(text.contains("<port>993</port>"));
        assert!(text.contains(r#"<incomingServer type="pop3">"#));
        assert!(text.contains("<port>995</port>"));
    }

    #[tokio::test]
    async fn the_emailaddress_query_sets_the_domain_and_username() {
        let (status, _, text) =
            get_config("/mail/config-v1.1.xml?emailaddress=User%40Example.com").await;
        assert_eq!(status, StatusCode::OK);
        assert!(text.contains(r#"<emailProvider id="example.com">"#));
        assert!(text.contains("<domain>example.com</domain>"));
        assert!(text.contains("<username>user@example.com</username>"));
        assert!(!text.contains("%EMAILADDRESS%"));
    }

    #[tokio::test]
    async fn a_request_without_an_address_keeps_the_placeholder() {
        let (status, _, text) = get_config("/mail/config-v1.1.xml").await;
        assert_eq!(status, StatusCode::OK);
        assert!(text.contains("<domain>mail.example.com</domain>"));
        assert!(text.contains("<username>%EMAILADDRESS%</username>"));
    }

    #[tokio::test]
    async fn the_well_known_autoconfig_route_serves_with_cors() {
        let (status, cors, text) =
            get_config("/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress=a%40b.example")
                .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(cors.as_deref(), Some("*"));
        assert!(text.contains("<domain>b.example</domain>"));
    }

    #[tokio::test]
    async fn the_well_known_mail_v1_route_is_mounted() {
        let (status, _, text) = get_config("/.well-known/mail-v1.xml").await;
        assert_eq!(status, StatusCode::OK);
        assert!(text.contains("clientConfig"));
    }

    #[tokio::test]
    async fn the_plain_http_router_serves_the_autoconfig_xml() {
        let dir = TempDir::new();
        let app = crate::serve::redirect_router(state(&dir), 443);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/mail/config-v1.1.xml?emailaddress=a%40b.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("<domain>b.example</domain>"));
    }
}
