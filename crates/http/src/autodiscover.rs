use axum::extract::State;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

use crate::app::AppState;

const DEPLOYMENT_ID: &str = "9f8b7a5e-4c31-4a52-9e0d-1b6f2c8d3a70";

pub async fn autodiscover(State(state): State<AppState>, body: String) -> Response {
    let Some(email) = extract_email_address(&body) else {
        return (StatusCode::BAD_REQUEST, "invalid autodiscover request").into_response();
    };
    let host = &state.hostname;
    let email = xml_escape(&email);
    let imap = state.listeners.imap.tls.unwrap_or(993);
    let pop3 = state.listeners.pop3.tls.unwrap_or(995);
    let smtp = state.listeners.submission.tls.unwrap_or(465);
    let protocols = [("IMAP", imap), ("POP3", pop3), ("SMTP", smtp)]
        .map(|(kind, port)| protocol_block(kind, host, port, &email))
        .join("");
    let xml = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Autodiscover xmlns="http://schemas.microsoft.com/exchange/autodiscover/responseschema/2006">
  <Response xmlns="http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a">
    <User>
      <DisplayName>{email}</DisplayName>
      <AutoDiscoverSMTPAddress>{email}</AutoDiscoverSMTPAddress>
      <DeploymentId>{DEPLOYMENT_ID}</DeploymentId>
    </User>
    <Account>
      <AccountType>email</AccountType>
      <Action>settings</Action>{protocols}
    </Account>
  </Response>
</Autodiscover>
"#
    );
    (
        [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
        xml,
    )
        .into_response()
}

fn protocol_block(kind: &str, host: &str, port: u16, email: &str) -> String {
    format!(
        r#"
      <Protocol>
        <Type>{kind}</Type>
        <Server>{host}</Server>
        <Port>{port}</Port>
        <LoginName>{email}</LoginName>
        <AuthRequired>on</AuthRequired>
        <DirectoryPort>0</DirectoryPort>
        <ReferralPort>0</ReferralPort>
        <SSL>on</SSL>
        <Encryption>TLS</Encryption>
        <SPA>off</SPA>
      </Protocol>"#
    )
}

fn extract_email_address(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let open = lower.find("<emailaddress")?;
    let start = open + lower[open..].find('>')? + 1;
    let end = start + lower[start..].find("</")?;
    let email = body.get(start..end)?.trim().to_ascii_lowercase();
    email.contains('@').then_some(email)
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub async fn autodiscover_json(State(state): State<AppState>, uri: Uri) -> Response {
    let query = uri.query().unwrap_or_default();
    let email = uri
        .path()
        .split('/')
        .map(|segment| segment.replace("%40", "@"))
        .find(|segment| segment.contains('@'))
        .or_else(|| query_param(query, "Email"))
        .unwrap_or_default();
    match email.rsplit_once('@') {
        Some((_, domain)) if !domain.is_empty() => {}
        _ => {
            return (StatusCode::BAD_REQUEST, "missing email address").into_response();
        }
    }
    let protocol = query_param(query, "Protocol").unwrap_or_default();
    if protocol.eq_ignore_ascii_case("autodiscoverv1") {
        let host = &state.hostname;
        let body = format!(
            r#"{{"Protocol":"AutodiscoverV1","Url":"https://{host}/autodiscover/autodiscover.xml"}}"#
        );
        (
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            body,
        )
            .into_response()
    } else {
        let safe: String = protocol
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect();
        let body = format!(
            r#"{{"ErrorCode":"InvalidProtocol","ErrorMessage":"The protocol '{safe}' is invalid. Supported values are 'AutodiscoverV1'"}}"#
        );
        (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            body,
        )
            .into_response()
    }
}

fn query_param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| value.replace("%40", "@"))
    })
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    use crate::tests_support::{state, TempDir};

    const OUTLOOK_REQUEST: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<Autodiscover xmlns="http://schemas.microsoft.com/exchange/autodiscover/outlook/requestschema/2006">
  <Request>
    <EMailAddress>User@Example.com</EMailAddress>
    <AcceptableResponseSchema>http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a</AcceptableResponseSchema>
  </Request>
</Autodiscover>"#;

    async fn post_pox(path: &str, body: &str) -> (StatusCode, String, String) {
        let dir = TempDir::new();
        let app = crate::app::router(state(&dir));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|value| value.to_str().unwrap().to_string())
            .unwrap_or_default();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            content_type,
            String::from_utf8(bytes.to_vec()).unwrap(),
        )
    }

    async fn get_json(path: &str) -> (StatusCode, String) {
        let dir = TempDir::new();
        let app = crate::app::router(state(&dir));
        let response = app
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn the_pox_response_echoes_the_requested_address() {
        let (status, content_type, text) =
            post_pox("/autodiscover/autodiscover.xml", OUTLOOK_REQUEST).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type, "application/xml; charset=utf-8");
        assert!(
            text.contains("<AutoDiscoverSMTPAddress>user@example.com</AutoDiscoverSMTPAddress>")
        );
        assert!(text.contains("<LoginName>user@example.com</LoginName>"));
        assert!(text.contains("<Type>IMAP</Type>"));
        assert!(text.contains("<Type>POP3</Type>"));
        assert!(text.contains("<Type>SMTP</Type>"));
        assert!(text.contains("<Port>993</Port>"));
        assert!(text.contains("<Port>995</Port>"));
        assert!(text.contains("<Port>465</Port>"));
        assert!(text.contains("<AuthRequired>on</AuthRequired>"));
        assert!(text.contains("<Encryption>TLS</Encryption>"));
        assert!(text.contains("mail.example.com"));
    }

    #[tokio::test]
    async fn the_capital_a_paths_serve_the_pox_response() {
        for path in [
            "/Autodiscover/Autodiscover.xml",
            "/AutoDiscover/AutoDiscover.xml",
        ] {
            let (status, content_type, text) = post_pox(path, OUTLOOK_REQUEST).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(content_type, "application/xml; charset=utf-8");
            assert!(text.contains("<LoginName>user@example.com</LoginName>"));
        }
    }

    #[tokio::test]
    async fn a_request_without_a_parseable_address_is_refused() {
        let (status, _, _) = post_pox("/autodiscover/autodiscover.xml", "").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _, _) =
            post_pox("/autodiscover/autodiscover.xml", "<Request></Request>").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn the_echoed_address_is_xml_escaped() {
        let request = OUTLOOK_REQUEST.replace("User@Example.com", "a&b@example.com");
        let (status, _, text) = post_pox("/autodiscover/autodiscover.xml", &request).await;
        assert_eq!(status, StatusCode::OK);
        assert!(text.contains("<LoginName>a&amp;b@example.com</LoginName>"));
        assert!(!text.contains("<LoginName>a&b@example.com</LoginName>"));
    }

    #[tokio::test]
    async fn the_json_endpoint_points_at_the_pox_url() {
        let (status, text) = get_json(
            "/autodiscover/autodiscover.json/v1.0/user@example.com?Protocol=AutodiscoverV1",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(text.contains("\"Protocol\":\"AutodiscoverV1\""));
        assert!(text.contains("https://mail.example.com/autodiscover/autodiscover.xml"));
    }

    #[tokio::test]
    async fn the_json_endpoint_accepts_the_email_query_param() {
        let (status, text) = get_json(
            "/autodiscover/autodiscover.json?Email=user%40example.com&Protocol=AutodiscoverV1",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(text.contains("\"Protocol\":\"AutodiscoverV1\""));
    }

    #[tokio::test]
    async fn an_unknown_json_protocol_is_refused() {
        let (status, text) = get_json(
            "/autodiscover/autodiscover.json/v1.0/user@example.com?Protocol=Active-Sync.v2!",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(text.contains("\"ErrorCode\":\"InvalidProtocol\""));
        assert!(text.contains("'ActiveSyncv2'"));
        assert!(!text.contains("Active-Sync.v2!"));
    }

    #[tokio::test]
    async fn a_json_request_without_an_email_domain_is_refused() {
        let (status, _) = get_json("/autodiscover/autodiscover.json?Protocol=AutodiscoverV1").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
